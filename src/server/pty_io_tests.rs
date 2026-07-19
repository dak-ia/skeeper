use super::*;
use std::collections::{HashMap, VecDeque};
use std::io::Cursor;
use std::sync::atomic::AtomicBool;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};

#[test]
fn fills_scrollback_from_reader() {
    // active_clientsが空のとき、readerから読んだ内容がそのままscrollbackに積まれる
    let data: Vec<u8> = b"hello scrollback".to_vec();
    let reader: Box<dyn std::io::Read + Send> = Box::new(Cursor::new(data.clone()));
    let sb: Scrollback = Arc::new(Mutex::new(VecDeque::new()));
    let ac: Arc<Mutex<HashMap<u32, ClientHandle>>> = Arc::new(Mutex::new(HashMap::new()));

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
    let ac: Arc<Mutex<HashMap<u32, ClientHandle>>> = Arc::new(Mutex::new(HashMap::new()));

    pty_reader_loop(reader, ac, Arc::clone(&sb));

    let stored: Vec<u8> = sb.lock().unwrap().iter().copied().collect();
    assert_eq!(stored.len(), SCROLLBACK_MAX_BYTES);
    // 最古のバイトが捨てられ、残っているのは元データの末尾SCROLLBACK_MAX_BYTES分と一致する
    let expected_tail = &data[data.len() - SCROLLBACK_MAX_BYTES..];
    assert_eq!(stored.as_slice(), expected_tail);
}

#[test]
fn fans_out_pty_chunks_to_all_client_channels() {
    // active_clientsに登録した全clientのevent_txにPtyChunkが届くことを確認
    let data: Vec<u8> = b"broadcast".to_vec();
    let reader: Box<dyn std::io::Read + Send> = Box::new(Cursor::new(data.clone()));
    let sb: Scrollback = Arc::new(Mutex::new(VecDeque::new()));

    let (tx1, rx1) = mpsc::channel::<ClientEvent>();
    let (tx2, rx2) = mpsc::channel::<ClientEvent>();
    let mut map: HashMap<u32, ClientHandle> = HashMap::new();
    map.insert(
        11,
        ClientHandle {
            should_detach: Arc::new(AtomicBool::new(false)),
            event_tx: tx1,
        },
    );
    map.insert(
        22,
        ClientHandle {
            should_detach: Arc::new(AtomicBool::new(false)),
            event_tx: tx2,
        },
    );
    let ac = Arc::new(Mutex::new(map));

    pty_reader_loop(reader, Arc::clone(&ac), Arc::clone(&sb));

    let stored: Vec<u8> = sb.lock().unwrap().iter().copied().collect();
    assert_eq!(stored, data);

    // 両clientのchannelに同じchunkが届いている
    let ev1 = rx1.try_recv().expect("client 1 should receive PtyChunk");
    match ev1 {
        ClientEvent::PtyChunk(bytes) => assert_eq!(bytes.as_slice(), data.as_slice()),
        ClientEvent::ClientMsg(_) => panic!("expected PtyChunk, got ClientMsg"),
    }
    let ev2 = rx2.try_recv().expect("client 2 should receive PtyChunk");
    match ev2 {
        ClientEvent::PtyChunk(bytes) => assert_eq!(bytes.as_slice(), data.as_slice()),
        ClientEvent::ClientMsg(_) => panic!("expected PtyChunk, got ClientMsg"),
    }
}

#[test]
fn removes_client_whose_channel_receiver_is_dropped() {
    // event_rxを先にdropしたclientはfanout時にsendが失敗し、active_clientsから外れる
    let data: Vec<u8> = b"hi".to_vec();
    let reader: Box<dyn std::io::Read + Send> = Box::new(Cursor::new(data.clone()));
    let sb: Scrollback = Arc::new(Mutex::new(VecDeque::new()));

    let (tx_alive, rx_alive) = mpsc::channel::<ClientEvent>();
    let (tx_dead, rx_dead) = mpsc::channel::<ClientEvent>();
    // dead側のReceiverを先に落として、send時にErrになる状態にする
    drop(rx_dead);

    let mut map: HashMap<u32, ClientHandle> = HashMap::new();
    map.insert(
        1,
        ClientHandle {
            should_detach: Arc::new(AtomicBool::new(false)),
            event_tx: tx_alive,
        },
    );
    map.insert(
        2,
        ClientHandle {
            should_detach: Arc::new(AtomicBool::new(false)),
            event_tx: tx_dead,
        },
    );
    let ac = Arc::new(Mutex::new(map));

    pty_reader_loop(reader, Arc::clone(&ac), Arc::clone(&sb));

    let ev = rx_alive.try_recv().expect("alive client should receive");
    match ev {
        ClientEvent::PtyChunk(bytes) => assert_eq!(bytes.as_slice(), data.as_slice()),
        ClientEvent::ClientMsg(_) => panic!("expected PtyChunk"),
    }

    let map_after = ac.lock().unwrap();
    assert!(map_after.contains_key(&1));
    assert!(!map_after.contains_key(&2));
}
