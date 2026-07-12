use super::*;
use std::collections::VecDeque;
use std::io::Cursor;
use std::os::unix::net::UnixStream;
use std::sync::{Arc, Mutex};

#[test]
fn fills_scrollback_from_reader() {
    // active_client=Noneのとき、readerから読んだ内容がそのままscrollbackに積まれる
    let data: Vec<u8> = b"hello scrollback".to_vec();
    let reader: Box<dyn std::io::Read + Send> = Box::new(Cursor::new(data.clone()));
    let sb: Scrollback = Arc::new(Mutex::new(VecDeque::new()));
    let ac: Arc<Mutex<Option<Arc<Mutex<UnixStream>>>>> = Arc::new(Mutex::new(None));

    pty_reader_loop(reader, ac, Arc::clone(&sb));

    let stored: Vec<u8> = sb.lock().unwrap().iter().copied().collect();
    assert_eq!(stored, data);
}

#[test]
fn evicts_oldest_bytes_when_scrollback_full() {
    // SCROLLBACK_MAX_BYTES超のデータを流し、上限を保ったまま古いバイトから捨てられることを検証
    let total = SCROLLBACK_MAX_BYTES + PTY_BUF_SIZE * 2;
    // 位置を追えるように251(素数)で循環する繰り返しにして、末尾一致で検証する
    // i % 251 < 251 なのでu8に必ず収まる
    let data: Vec<u8> = (0..total).map(|i| u8::try_from(i % 251).unwrap()).collect();

    let reader: Box<dyn std::io::Read + Send> = Box::new(Cursor::new(data.clone()));
    let sb: Scrollback = Arc::new(Mutex::new(VecDeque::new()));
    let ac: Arc<Mutex<Option<Arc<Mutex<UnixStream>>>>> = Arc::new(Mutex::new(None));

    pty_reader_loop(reader, ac, Arc::clone(&sb));

    let stored: Vec<u8> = sb.lock().unwrap().iter().copied().collect();
    assert_eq!(stored.len(), SCROLLBACK_MAX_BYTES);
    // 最古のバイトが捨てられ、残っているのは元データの末尾SCROLLBACK_MAX_BYTES分と一致する
    let expected_tail = &data[data.len() - SCROLLBACK_MAX_BYTES..];
    assert_eq!(stored.as_slice(), expected_tail);
}
