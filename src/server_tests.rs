use super::*;

fn dummy_handle(cols: u16, rows: u16) -> ClientHandle {
    let (event_tx, _event_rx) = mpsc::channel();
    // Receiverはdropされるがsendしないので実害なし。attach_idはtest内でユニークならOK
    ClientHandle {
        attach_id: 1,
        cols,
        rows,
        should_detach: Arc::new(AtomicBool::new(false)),
        event_tx,
    }
}

#[test]
fn aggregate_min_size_zero_clients_returns_none() {
    let acl: HashMap<u32, ClientHandle> = HashMap::new();
    assert_eq!(aggregate_min_size(&acl), None);
}

#[test]
fn aggregate_min_size_single_client_returns_its_size() {
    let mut acl: HashMap<u32, ClientHandle> = HashMap::new();
    acl.insert(1, dummy_handle(80, 24));
    assert_eq!(aggregate_min_size(&acl), Some((80, 24)));
}

#[test]
fn aggregate_min_size_multiple_clients_returns_min_of_each_axis() {
    let mut acl: HashMap<u32, ClientHandle> = HashMap::new();
    acl.insert(1, dummy_handle(100, 40));
    acl.insert(2, dummy_handle(80, 50));
    acl.insert(3, dummy_handle(120, 30));
    // colsは80(client2)、rowsは30(client3)がそれぞれ最小
    assert_eq!(aggregate_min_size(&acl), Some((80, 30)));
}
