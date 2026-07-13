use super::*;

use tempfile::TempDir;
use uuid::Uuid;

const ENV_KEY: &str = "SKEEPER_SESSION_ID";

/// 全ケースを1つの#[test]に集約している理由:
/// current_session_idはSKEEPER_SESSION_IDをprocess-globalなenvから読むため、
/// テストを分けるとcargo testの並列実行で他ケースのset/removeと干渉する
#[test]
fn current_session_id_reflects_env_and_meta_json() {
    let _guard = crate::test_helpers::env_lock();
    let dir = TempDir::new().unwrap();
    let base = dir.path();

    // env未セット: Noneを返す
    // SAFETY: このテストはenvの直列アクセスを前提に集約している
    unsafe { std::env::remove_var(ENV_KEY) };
    assert_eq!(current_session_id(base), None);

    // envは有効UUIDだがmeta.jsonが実在しない(bashrc誤export相当): None
    let id = Uuid::new_v4();
    unsafe { std::env::set_var(ENV_KEY, id.to_string()) };
    assert_eq!(current_session_id(base), None);

    // envの示すidに対応するmeta.jsonが実在する: Some(id)
    std::fs::write(paths::meta_path(base, &id), "").unwrap();
    assert_eq!(current_session_id(base), Some(id));

    // envの値がUUIDとしてparseできない: None(meta.jsonの有無に関わらず)
    unsafe { std::env::set_var(ENV_KEY, "not-a-uuid") };
    assert_eq!(current_session_id(base), None);

    unsafe { std::env::remove_var(ENV_KEY) };
}
