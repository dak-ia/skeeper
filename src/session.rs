use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use uuid::Uuid;

pub type SessionId = Uuid;

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
    #[serde(default)]
    pub attached_client_pids: Vec<u32>,
}

/// メタ情報を原子的にファイルに書き込む(tmpに書いてからrenameで置き換え)
pub fn write_meta_atomic(path: &Path, meta: &SessionMeta) -> anyhow::Result<()> {
    // <path>.tmp を作って完全に書ききってから rename する。
    // rename はUnix上で原子的なのでpath側に中途半端なJSONは残らない
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp_path = PathBuf::from(tmp);
    let json = serde_json::to_string_pretty(meta)?;
    std::fs::write(&tmp_path, json)?;
    std::fs::rename(&tmp_path, path)?;
    Ok(())
}

/// メタ情報をファイルから読み込む
pub fn read_meta(path: &Path) -> anyhow::Result<SessionMeta> {
    let json = std::fs::read_to_string(path)?;
    let meta: SessionMeta = serde_json::from_str(&json)?;
    Ok(meta)
}

/// 指定ディレクトリ配下の全セッションメタを読み出す
///
/// 存在しないディレクトリは空Vecとして扱う。
/// パースに失敗するファイル(書込中や壊れたJSON)は黙って飛ばして残りを返す。
/// 非jsonファイル(*.sock, *.tmp等)は対象外
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

#[cfg(target_os = "linux")]
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

#[cfg(not(target_os = "linux"))]
pub fn is_orphan(_meta: &SessionMeta) -> anyhow::Result<bool> {
    anyhow::bail!("Orphan detection is Linux-only for MVP-1")
}

#[cfg(target_os = "linux")]
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
    // commフィールドはprctl(PR_SET_NAME)で任意バイトが入り得るのでバイト列で読む。
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
