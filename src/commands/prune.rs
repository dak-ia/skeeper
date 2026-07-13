use crate::{paths, session};

pub(crate) fn run() -> anyhow::Result<()> {
    let base_dir = paths::runtime_dir()?;
    let sessions = session::list_all_meta(&base_dir).unwrap_or_default();
    if sessions.is_empty() {
        println!("No sessions to check");
        return Ok(());
    }

    let mut pruned = 0usize;
    let mut alive = 0usize;
    for m in sessions {
        match session::is_orphan(&m) {
            Ok(true) => {
                let _ = std::fs::remove_file(paths::ctl_path(&base_dir, &m.id));
                let _ = std::fs::remove_file(paths::socket_path(&base_dir, &m.id));
                let _ = std::fs::remove_file(paths::meta_path(&base_dir, &m.id));
                pruned += 1;
                println!("Pruned '{}' (orphan)", m.name);
            }
            Ok(false) => {
                alive += 1;
            }
            Err(e) => {
                eprintln!("Skipped '{}': {e}", m.name);
            }
        }
    }
    println!("Pruned {pruned} session(s), {alive} still alive");
    Ok(())
}

#[cfg(test)]
#[path = "prune_tests.rs"]
mod tests;
