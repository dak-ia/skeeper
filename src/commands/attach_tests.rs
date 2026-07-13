use super::*;

use crate::cli::AttachArgs;
use uuid::Uuid;

#[test]
fn run_errors_on_early_paths() {
    let _guard = crate::test_helpers::env_lock();

    // XDG_RUNTIME_DIR配下に"skeeper"が付与される仕様に合わせて、
    // 実体はtempdir/skeeperを掘って使う
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path().join("skeeper");
    std::fs::create_dir_all(&base).unwrap();

    unsafe {
        std::env::set_var("XDG_RUNTIME_DIR", dir.path());
    }

    // ケース1: SKEEPER_SESSION_IDが有効UUIDで、対応meta.jsonが実在 → 「既にsession内」でErr
    let id = Uuid::from_u128(0xdead_beef);
    let meta_path = base.join(format!("{id}.json"));
    std::fs::File::create(&meta_path).unwrap();
    unsafe {
        std::env::set_var("SKEEPER_SESSION_ID", id.to_string());
    }

    assert!(run(AttachArgs { name: None }).is_err());

    // ケース2: env未セット + 名前指定が存在しない → Err
    std::fs::remove_file(&meta_path).unwrap();
    unsafe {
        std::env::remove_var("SKEEPER_SESSION_ID");
    }

    assert!(
        run(AttachArgs {
            name: Some("does-not-exist".to_string()),
        })
        .is_err()
    );
}
