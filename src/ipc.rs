use std::io::{self, Read, Write};
use std::path::PathBuf;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// 1メッセージあたりの最大サイズ。DoS防御のため
pub const MAX_FRAME_BYTES: u32 = 16 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ClientMsg {
    Hello {
        client_pid: u32,
        cols: u16,
        rows: u16,
    },
    Resize {
        cols: u16,
        rows: u16,
    },
    Stdin(Vec<u8>),
    Detach,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ServerMsg {
    HelloOk {
        session_id: Uuid,
        name: String,
    },
    Stdout(Vec<u8>),
    DetachAck,
    SessionEnded {
        exit_status: Option<i32>,
    },
    /// 接続中clientに現接続を閉じてtarget_socket_pathへ繋ぎ直させる指示(session内`skeeper attach`用)
    SwitchTo {
        target_socket_path: PathBuf,
    },
}

/// 制御ソケット(<uuid>.ctl)経由でサーバに送るメッセージ。データ用socketとは別
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ControlMsg {
    /// 現在接続中のクライアントをdetachするようサーバに依頼する
    RequestDetach,
    /// セッション名を変更するようサーバに依頼する
    RequestRename { new_name: String },
    /// 直近stdinを送ったclientのpidを問い合わせる(応答はControlResponse::CurrentClient)
    QueryCurrentClient,
    /// LAST_STDIN_CLIENTのclientに `SwitchTo(target_socket_path)` を配信するよう依頼する
    SwitchClient { target_socket_path: PathBuf },
}

/// 制御ソケットでサーバがclientに返す応答。fire-and-forgetでないqueryだけがここに来る
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ControlResponse {
    /// QueryCurrentClientへの応答。pid=0は「まだ誰もstdin送っていない」を意味する
    CurrentClient { pid: u32 },
    /// RequestRenameへの応答
    Rename(RenameResponse),
}

/// rename処理の結果。server側でuniqueness判定を済ませてからclientに返す
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum RenameResponse {
    /// rename成功。metaは新しい名前で永続化された
    Ok,
    /// 現在の名前と同じなので何もしていない(no-op)
    Unchanged,
    /// 他のsessionが同じ名前を既に使用している
    Conflict,
    /// server内部の障害(lock取得失敗・meta書き込み失敗など)
    Failed,
}

pub fn write_client_msg<W: Write>(w: &mut W, msg: &ClientMsg) -> io::Result<()> {
    write_frame(w, msg)
}

pub fn read_client_msg<R: Read>(r: &mut R) -> io::Result<ClientMsg> {
    read_frame(r)
}

pub fn write_server_msg<W: Write>(w: &mut W, msg: &ServerMsg) -> io::Result<()> {
    write_frame(w, msg)
}

pub fn read_server_msg<R: Read>(r: &mut R) -> io::Result<ServerMsg> {
    read_frame(r)
}

pub fn write_control_msg<W: Write>(w: &mut W, msg: &ControlMsg) -> io::Result<()> {
    write_frame(w, msg)
}

pub fn read_control_msg<R: Read>(r: &mut R) -> io::Result<ControlMsg> {
    read_frame(r)
}

pub fn write_control_response<W: Write>(w: &mut W, msg: &ControlResponse) -> io::Result<()> {
    write_frame(w, msg)
}

pub fn read_control_response<R: Read>(r: &mut R) -> io::Result<ControlResponse> {
    read_frame(r)
}

fn write_frame<W: Write, T: Serialize>(w: &mut W, msg: &T) -> io::Result<()> {
    let body =
        postcard::to_allocvec(msg).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    let len = u32::try_from(body.len())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "frame too large"))?;
    w.write_all(&len.to_be_bytes())?;
    w.write_all(&body)?;
    w.flush()?;
    Ok(())
}

fn read_frame<R: Read, T: DeserializeOwned>(r: &mut R) -> io::Result<T> {
    let mut len_buf = [0u8; 4];
    r.read_exact(&mut len_buf)?;
    let len = u32::from_be_bytes(len_buf);
    if len > MAX_FRAME_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("frame size {len} exceeds MAX_FRAME_BYTES {MAX_FRAME_BYTES}"),
        ));
    }
    let mut body = vec![0u8; len as usize];
    r.read_exact(&mut body)?;
    postcard::from_bytes(&body).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

#[cfg(test)]
#[path = "ipc_tests.rs"]
mod tests;
