use std::io::{self, Read, Write};

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
    HelloOk { session_id: Uuid, name: String },
    Stdout(Vec<u8>),
    DetachAck,
    SessionEnded { exit_status: Option<i32> },
}

/// 制御ソケット(<uuid>.ctl)経由でサーバに送るメッセージ。データ用socketとは別
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ControlMsg {
    /// 現在接続中のクライアントをdetachするようサーバに依頼する
    RequestDetach,
    /// セッション名を変更するようサーバに依頼する
    RequestRename { new_name: String },
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
