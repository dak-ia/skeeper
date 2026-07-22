use std::collections::HashMap;
use std::path::Path;
use std::sync::Mutex;

use portable_pty::{MasterPty, PtySize};

use crate::session::{self, SessionMeta};

use super::{ClientHandle, aggregate_min_size};

/// runを抜けるときにソケット/メタファイルを掃除する。パニック時も動く。
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

/// 各clientのattach中に持っている共有状態(active_clients登録 + meta.attached_clients)を、
/// 早期return/パニックのどの経路でも一度だけ確実に解除するためのRAII guard
pub(super) struct AttachStateGuard<'a> {
    pub(super) client_pid: u32,
    pub(super) attach_id: u64,
    pub(super) active_clients: &'a Mutex<HashMap<u32, ClientHandle>>,
    pub(super) meta: &'a Mutex<SessionMeta>,
    pub(super) meta_path: &'a Path,
    pub(super) master: Option<&'a Mutex<Box<dyn MasterPty + Send>>>,
    pub(super) armed: bool,
}

impl AttachStateGuard<'_> {
    pub(super) fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for AttachStateGuard<'_> {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        // pty_reader_loopがactive_clientsのlockを保持したまま書き込む設計に依存して、
        // ここでmapから外せば「in-flight書き込みは完了→以降新規書き込みなし」となる。
        // ただし同一pidで新しいattachに置換された後の後始末では、今のslotは他者所有なので触らない
        let is_still_current = {
            let mut acl = self.active_clients.lock().unwrap();
            let owned =
                matches!(acl.get(&self.client_pid), Some(h) if h.attach_id == self.attach_id);
            if owned {
                acl.remove(&self.client_pid);
                // 残clientの min サイズでpty再集約。masterはOption(testでNone可)
                if let Some(m) = self.master
                    && let Some((c, r)) = aggregate_min_size(&acl)
                {
                    let _ = m.lock().unwrap().resize(PtySize {
                        cols: c,
                        rows: r,
                        pixel_width: 0,
                        pixel_height: 0,
                    });
                }
            }
            owned
        };
        if is_still_current {
            let mut m = self.meta.lock().unwrap();
            m.attached_clients.retain(|c| c.pid != self.client_pid);
            let _ = session::write_meta_atomic(self.meta_path, &m);
        }
    }
}

#[cfg(test)]
#[path = "guards_tests.rs"]
mod tests;
