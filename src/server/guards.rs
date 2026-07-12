use std::path::Path;

/// runを抜けるときにソケット/メタファイルを掃除する。パニック時も動く。
/// _pathサフィックスはpath型のフィールドで一般的な命名なのでclippyの警告は抑止
#[allow(clippy::struct_field_names)]
pub(super) struct SessionFileGuard<'a> {
    pub(super) meta_path: &'a Path,
    pub(super) socket_path: &'a Path,
    pub(super) ctl_socket_path: &'a Path,
}

impl Drop for SessionFileGuard<'_> {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(self.ctl_socket_path);
        let _ = std::fs::remove_file(self.socket_path);
        let _ = std::fs::remove_file(self.meta_path);
    }
}

#[cfg(test)]
#[path = "guards_tests.rs"]
mod tests;
