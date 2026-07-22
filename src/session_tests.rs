use super::*;
use time::macros::datetime;

fn sample() -> SessionMeta {
    SessionMeta {
        id: Uuid::from_u128(0x550e_8400_e29b_41d4_a716_4466_5544_0000),
        name: "brave-otter".to_string(),
        cwd: PathBuf::from("/home/user/projects/foo"),
        shell: PathBuf::from("/bin/bash"),
        created_at: datetime!(2026-07-04 12:34:56.789012345 UTC),
        last_attached_at: Some(datetime!(2026-07-04 13:00:00 UTC)),
        server_pid: 12345,
        server_started_at: datetime!(2026-07-04 12:34:56.789012345 UTC),
        schema_version: SCHEMA_VERSION_CURRENT,
        ipc_protocol_version: crate::ipc::IPC_PROTOCOL_VERSION,
        attached_clients: Vec::new(),
    }
}

#[test]
fn roundtrips_via_json() {
    let m = sample();
    let json = serde_json::to_string(&m).unwrap();
    let back: SessionMeta = serde_json::from_str(&json).unwrap();
    assert_eq!(m, back);
}

#[test]
fn preserves_nanosecond_precision() {
    let m = sample();
    let json = serde_json::to_string(&m).unwrap();
    assert!(
        json.contains(".789012345"),
        "expected nanosecond precision, got: {json}"
    );
    let back: SessionMeta = serde_json::from_str(&json).unwrap();
    assert_eq!(m.created_at, back.created_at);
}

#[test]
fn nulls_optionals_when_none() {
    let mut m = sample();
    m.last_attached_at = None;
    m.attached_clients = Vec::new();
    let json = serde_json::to_string(&m).unwrap();
    assert!(json.contains("\"last_attached_at\":null"));
    assert!(json.contains("\"attached_clients\":[]"));
}

#[test]
fn atomic_write_creates_file_with_correct_json() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("meta.json");
    let m = sample();
    write_meta_atomic(&path, &m).unwrap();
    assert!(path.exists());
    let back = read_meta(&path).unwrap();
    assert_eq!(m, back);
}

#[test]
fn atomic_write_replaces_existing_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("meta.json");
    std::fs::write(&path, "OLD CONTENT").unwrap();
    let m = sample();
    write_meta_atomic(&path, &m).unwrap();
    let content = std::fs::read_to_string(&path).unwrap();
    assert!(!content.contains("OLD CONTENT"));
    assert!(content.contains("brave-otter"));
}

#[test]
fn list_all_meta_nonexistent_dir_returns_empty() {
    let list = list_all_meta(Path::new("/nonexistent/skeeper/xyz")).unwrap();
    assert!(list.is_empty());
}

#[test]
fn list_all_meta_empty_dir_returns_empty() {
    let dir = tempfile::tempdir().unwrap();
    let list = list_all_meta(dir.path()).unwrap();
    assert!(list.is_empty());
}

#[test]
fn list_all_meta_reads_valid_files() {
    let dir = tempfile::tempdir().unwrap();
    let m = sample();
    let path = dir.path().join(format!("{}.json", m.id));
    write_meta_atomic(&path, &m).unwrap();
    let list = list_all_meta(dir.path()).unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].name, m.name);
}

#[test]
fn list_all_meta_ignores_non_json_files() {
    let dir = tempfile::tempdir().unwrap();
    let m = sample();
    let path = dir.path().join(format!("{}.json", m.id));
    write_meta_atomic(&path, &m).unwrap();
    std::fs::write(dir.path().join("something.sock"), b"").unwrap();
    std::fs::write(dir.path().join("something.tmp"), b"garbage").unwrap();
    let list = list_all_meta(dir.path()).unwrap();
    assert_eq!(list.len(), 1);
}

#[test]
fn list_all_meta_skips_corrupted_json() {
    let dir = tempfile::tempdir().unwrap();
    let m = sample();
    let path = dir.path().join(format!("{}.json", m.id));
    write_meta_atomic(&path, &m).unwrap();
    std::fs::write(dir.path().join("bad.json"), b"not valid json").unwrap();
    let list = list_all_meta(dir.path()).unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].name, m.name);
}

#[test]
fn read_meta_migrates_v1_schema_silently() {
    // schema_version未存在(=v1扱い)のJSONを、silent migrationでClientInfo付きの
    // 最新schemaに引き上げる。tty/ssh_connectionはunknown、attached_atはcreated_atで補う
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("v1.json");
    let json = r#"{
        "id": "550e8400-e29b-41d4-a716-446655440000",
        "name": "legacy",
        "cwd": "/tmp",
        "shell": "/bin/sh",
        "created_at": "2000-01-02T03:04:05Z",
        "last_attached_at": null,
        "server_pid": 12345,
        "server_started_at": "2000-01-02T03:04:05Z",
        "attached_client_pids": [111, 222]
    }"#;
    std::fs::write(&path, json).unwrap();
    let meta = read_meta(&path).unwrap();
    assert_eq!(meta.schema_version, SCHEMA_VERSION_CURRENT);
    // v1にはIPC versionという概念自体が無いのでunknown扱いの0で埋める
    assert_eq!(meta.ipc_protocol_version, 0);
    assert_eq!(meta.last_attached_at, None);
    assert_eq!(meta.attached_clients.len(), 2);
    assert_eq!(meta.attached_clients[0].pid, 111);
    assert_eq!(meta.attached_clients[0].tty, None);
    assert_eq!(meta.attached_clients[0].ssh_connection, None);
    // v1には個別のattached_atが無いのでcreated_atで補う
    assert_eq!(meta.attached_clients[0].attached_at, meta.created_at);
    assert_eq!(meta.attached_clients[1].pid, 222);
}

#[test]
fn atomic_write_does_not_leave_tmp_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("meta.json");
    let m = sample();
    write_meta_atomic(&path, &m).unwrap();
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp_path = PathBuf::from(tmp);
    assert!(!tmp_path.exists(), "tmp file should be gone after rename");
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
mod orphan_tests {
    use super::*;

    fn self_meta() -> SessionMeta {
        let self_pid = std::process::id();
        let start = process_start_time(self_pid).unwrap().unwrap();
        SessionMeta {
            id: Uuid::new_v4(),
            name: "test".to_string(),
            cwd: PathBuf::from("/"),
            shell: PathBuf::from("/bin/sh"),
            created_at: start,
            last_attached_at: None,
            server_pid: self_pid,
            server_started_at: start,
            schema_version: SCHEMA_VERSION_CURRENT,
            ipc_protocol_version: crate::ipc::IPC_PROTOCOL_VERSION,
            attached_clients: Vec::new(),
        }
    }

    #[test]
    fn own_process_is_not_orphan() {
        let m = self_meta();
        assert!(!is_orphan(&m).unwrap());
    }

    #[test]
    fn mismatched_start_time_is_orphan() {
        let mut m = self_meta();
        m.server_started_at -= time::Duration::minutes(1);
        assert!(is_orphan(&m).unwrap());
    }

    #[test]
    fn nonexistent_pid_is_orphan() {
        let mut m = self_meta();
        m.server_pid = 10_000_000; // 通常のPID_MAX_LIMIT(4194304)を超える値
        assert!(is_orphan(&m).unwrap());
    }

    #[test]
    fn pid_zero_is_orphan() {
        // pid=0はkill(2)特殊値なので早期に「存在しない」扱いにする(誤判定でsignal誤爆を防ぐ)
        let mut m = self_meta();
        m.server_pid = 0;
        assert!(is_orphan(&m).unwrap());
    }
}
