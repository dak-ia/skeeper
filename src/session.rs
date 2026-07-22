use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use uuid::Uuid;

pub type SessionId = Uuid;

/// SessionMetaのschemaバージョン。旧schema(pidsのVecのみ)は1、ClientInfo拡張後は2
pub const SCHEMA_VERSION_CURRENT: u32 = 2;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionMeta {
    pub id: SessionId,
    pub name: String,
    pub cwd: PathBuf,
    pub shell: PathBuf,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339::option")]
    pub last_attached_at: Option<OffsetDateTime>,
    pub server_pid: u32,
    #[serde(with = "time::serde::rfc3339")]
    pub server_started_at: OffsetDateTime,
    /// このmetaを書いたserverが使うschemaのversion。読み側でmigrationの分岐に使う
    #[serde(default = "default_schema_version_old")]
    pub schema_version: u32,
    /// このserverが話すIPCプロトコルのversion。clientはlistで差分を検知して"outdated"表示できる
    #[serde(default)]
    pub ipc_protocol_version: u32,
    #[serde(default)]
    pub attached_clients: Vec<ClientInfo>,
}

/// attach中の1 clientの情報。pidだけでは同一session内に同居する複数attachを識別できないので、
/// tty/SSH_CONNECTION/attach時刻でどのterminalからかまで示せるようにする
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClientInfo {
    pub pid: u32,
    #[serde(default)]
    pub tty: Option<String>,
    #[serde(default)]
    pub ssh_connection: Option<String>,
    #[serde(with = "time::serde::rfc3339")]
    pub attached_at: OffsetDateTime,
}

fn default_schema_version_old() -> u32 {
    1
}

/// 旧schema(v1)のmetaをserdeで読むためのraw構造。attached_clientsは持たずattached_client_pidsを持つ
#[derive(Deserialize)]
struct MetaV1 {
    id: SessionId,
    name: String,
    cwd: PathBuf,
    shell: PathBuf,
    #[serde(with = "time::serde::rfc3339")]
    created_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339::option", default)]
    last_attached_at: Option<OffsetDateTime>,
    server_pid: u32,
    #[serde(with = "time::serde::rfc3339")]
    server_started_at: OffsetDateTime,
    #[serde(default)]
    attached_client_pids: Vec<u32>,
}

impl MetaV1 {
    fn upgrade(self) -> SessionMeta {
        // v1にはtty/ssh/attach時刻が無いので、pidだけ引き継いで既知情報で補う
        let attached_clients = self
            .attached_client_pids
            .into_iter()
            .map(|pid| ClientInfo {
                pid,
                tty: None,
                ssh_connection: None,
                attached_at: self.created_at,
            })
            .collect();
        SessionMeta {
            id: self.id,
            name: self.name,
            cwd: self.cwd,
            shell: self.shell,
            created_at: self.created_at,
            last_attached_at: self.last_attached_at,
            server_pid: self.server_pid,
            server_started_at: self.server_started_at,
            schema_version: SCHEMA_VERSION_CURRENT,
            // v1にはIPC versionという概念が無く、書き手のprotocolも不明。
            // 0を"unknown"のsentinelとして残し、現行の値を偽らない
            ipc_protocol_version: 0,
            attached_clients,
        }
    }
}

/// メタ情報を原子的にファイルに書き込む(tmpに書いてからrenameで置き換え)
pub fn write_meta_atomic(path: &Path, meta: &SessionMeta) -> anyhow::Result<()> {
    // Unixのrenameは原子的なので、path側に中途半端なJSONが残らない
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp_path = PathBuf::from(tmp);
    let json = serde_json::to_string_pretty(meta)?;
    std::fs::write(&tmp_path, json)?;
    std::fs::rename(&tmp_path, path)?;
    Ok(())
}

/// メタ情報をファイルから読み込む。旧schema(v1)はsilent auto migrationで新schemaに変換する
pub fn read_meta(path: &Path) -> anyhow::Result<SessionMeta> {
    let json = std::fs::read_to_string(path)?;
    // 現行schema(SCHEMA_VERSION_CURRENT)以上ならそのまま。未来のversion(v3+)も
    // 現状のfieldさえ揃っていれば読める前提で受け入れる(壊れれば下のfrom_strが即失敗する)
    let meta: SessionMeta = serde_json::from_str(&json)?;
    if meta.schema_version >= SCHEMA_VERSION_CURRENT {
        return Ok(meta);
    }
    let v1: MetaV1 = serde_json::from_str(&json)?;
    Ok(v1.upgrade())
}

/// 指定ディレクトリ配下の全セッションメタを読み出す
///
/// 存在しない/壊れたJSON/非json拡張子は無視して残りを返す
pub fn list_all_meta(dir: &Path) -> anyhow::Result<Vec<SessionMeta>> {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e.into()),
    };
    let mut result = Vec::new();
    for entry in entries {
        let Ok(entry) = entry else { continue };
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("json") {
            continue;
        }
        if let Ok(meta) = read_meta(&path) {
            result.push(meta);
        }
    }
    Ok(result)
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
pub fn is_orphan(meta: &SessionMeta) -> anyhow::Result<bool> {
    if !is_pid_alive(meta.server_pid)? {
        return Ok(true);
    }
    // is_pid_aliveとの間でプロセスが終了する競合を、Ok(None)で受ける
    let Some(actual) = process_start_time(meta.server_pid)? else {
        return Ok(true);
    };
    // clock tickの丸めで小さなズレは許容する
    let diff = (actual - meta.server_started_at).abs();
    Ok(diff > time::Duration::seconds(1))
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
pub fn is_orphan(_meta: &SessionMeta) -> anyhow::Result<bool> {
    anyhow::bail!("Orphan detection is not supported on this platform")
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn is_pid_alive(pid: u32) -> anyhow::Result<bool> {
    use nix::errno::Errno;
    use nix::sys::signal;
    use nix::unistd::Pid;

    // pid=0はkill(2)で「呼び出し元のプロセスグループ全体」を指す特殊値。生存判定にはならないので拒否
    if pid == 0 {
        return Ok(false);
    }
    let Ok(pid_i32) = i32::try_from(pid) else {
        return Ok(false);
    };
    match signal::kill(Pid::from_raw(pid_i32), None) {
        Err(Errno::ESRCH) => Ok(false),
        Ok(()) | Err(Errno::EPERM) => Ok(true),
        Err(e) => Err(e.into()),
    }
}

#[cfg(target_os = "macos")]
pub(crate) fn process_start_time(pid: u32) -> anyhow::Result<Option<OffsetDateTime>> {
    use libproc::proc_pid::pidinfo;
    use libproc::task_info::TaskAllInfo;

    let pid_i32 = i32::try_from(pid)?;
    // 存在しないPIDや権限不足はErr→Ok(None)扱いにして、呼び出し側で「孤児」扱いにする
    let info = match pidinfo::<TaskAllInfo>(pid_i32, 0) {
        Ok(i) => i,
        Err(_) => return Ok(None),
    };
    let sec = i64::try_from(info.pbsd.pbi_start_tvsec)?;
    let usec = i64::try_from(info.pbsd.pbi_start_tvusec)?;
    let dt = OffsetDateTime::from_unix_timestamp(sec)? + time::Duration::microseconds(usec);
    Ok(Some(dt))
}

#[cfg(target_os = "linux")]
pub(crate) fn process_start_time(pid: u32) -> anyhow::Result<Option<OffsetDateTime>> {
    // commフィールドはprctl(PR_SET_NAME)で任意バイトが入り得るのでバイト列で読む
    // 存在しないPIDはOk(None)で返し、呼び出し側で「孤児」扱いにする
    let raw = match std::fs::read(format!("/proc/{pid}/stat")) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.into()),
    };
    // /proc/<pid>/stat: pid (comm) state ppid ... starttime ...
    // commには空白や')'や非UTF-8バイトが入り得るので、バイト列で最後の')'を探す
    let last_paren = raw
        .iter()
        .rposition(|&b| b == b')')
        .ok_or_else(|| anyhow::anyhow!("/proc/{pid}/stat: unexpected format"))?;
    // 最後の')'以降は数値とspaceのみ(ASCII)なのでUTF-8として安全に扱える
    let after = std::str::from_utf8(&raw[last_paren + 1..])?;
    let fields: Vec<&str> = after.split_whitespace().collect();
    // last_paren以降のフィールド番号0がstate(3番目)、starttimeは22番目 → 22-3=19番目のオフセット
    let starttime_ticks: u64 = fields
        .get(19)
        .ok_or_else(|| anyhow::anyhow!("/proc/{pid}/stat: missing starttime"))?
        .parse()?;

    let btime = read_btime()?;
    let hz = clock_ticks_per_second()?;
    let start_secs = btime + starttime_ticks / hz;
    let start_nanos = (starttime_ticks % hz) * 1_000_000_000 / hz;

    let dt = OffsetDateTime::from_unix_timestamp(i64::try_from(start_secs)?)?
        + time::Duration::nanoseconds(i64::try_from(start_nanos)?);
    Ok(Some(dt))
}

#[cfg(target_os = "linux")]
fn read_btime() -> anyhow::Result<u64> {
    let stat = std::fs::read_to_string("/proc/stat")?;
    for line in stat.lines() {
        if let Some(rest) = line.strip_prefix("btime ") {
            return Ok(rest.trim().parse()?);
        }
    }
    anyhow::bail!("/proc/stat: btime line not found")
}

#[cfg(target_os = "linux")]
fn clock_ticks_per_second() -> anyhow::Result<u64> {
    use nix::unistd::{SysconfVar, sysconf};
    let hz =
        sysconf(SysconfVar::CLK_TCK)?.ok_or_else(|| anyhow::anyhow!("Failed to get CLK_TCK"))?;
    if hz <= 0 {
        anyhow::bail!("Invalid CLK_TCK value: {hz}");
    }
    Ok(u64::try_from(hz)?)
}

#[cfg(test)]
#[path = "session_tests.rs"]
mod tests;
