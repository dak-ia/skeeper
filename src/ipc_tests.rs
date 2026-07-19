use super::*;

fn roundtrip_client(msg: &ClientMsg) {
    let bytes = postcard::to_allocvec(msg).unwrap();
    let back: ClientMsg = postcard::from_bytes(&bytes).unwrap();
    assert_eq!(msg, &back);
}

fn roundtrip_server(msg: &ServerMsg) {
    let bytes = postcard::to_allocvec(msg).unwrap();
    let back: ServerMsg = postcard::from_bytes(&bytes).unwrap();
    assert_eq!(msg, &back);
}

#[test]
fn client_hello_roundtrip() {
    roundtrip_client(&ClientMsg::Hello {
        client_pid: 42,
        cols: 80,
        rows: 24,
    });
}

#[test]
fn client_resize_roundtrip() {
    roundtrip_client(&ClientMsg::Resize {
        cols: 120,
        rows: 40,
    });
}

#[test]
fn client_stdin_roundtrip() {
    roundtrip_client(&ClientMsg::Stdin(vec![]));
    roundtrip_client(&ClientMsg::Stdin(vec![0, 1, 2, 0xff, 0]));
}

#[test]
fn client_detach_roundtrip() {
    roundtrip_client(&ClientMsg::Detach);
}

#[test]
fn server_hello_ok_roundtrip() {
    roundtrip_server(&ServerMsg::HelloOk {
        session_id: Uuid::from_u128(0x1234_5678_9abc_def0_1234_5678_9abc_def0),
        name: "brave-otter".to_string(),
    });
}

#[test]
fn server_stdout_roundtrip() {
    roundtrip_server(&ServerMsg::Stdout(vec![]));
    roundtrip_server(&ServerMsg::Stdout(vec![0, 1, 2, 0xff, b'\n']));
}

#[test]
fn server_detach_ack_roundtrip() {
    roundtrip_server(&ServerMsg::DetachAck);
}

#[test]
fn server_session_ended_roundtrip() {
    roundtrip_server(&ServerMsg::SessionEnded {
        exit_status: Some(0),
    });
    roundtrip_server(&ServerMsg::SessionEnded {
        exit_status: Some(-1),
    });
    roundtrip_server(&ServerMsg::SessionEnded { exit_status: None });
}

#[test]
fn framing_roundtrip_client() {
    let msg = ClientMsg::Stdin(vec![1, 2, 3, 0xff, 0]);
    let mut buf = Vec::new();
    write_client_msg(&mut buf, &msg).unwrap();
    let back = read_client_msg(&mut &buf[..]).unwrap();
    assert_eq!(msg, back);
}

#[test]
fn framing_roundtrip_server() {
    let msg = ServerMsg::Stdout(vec![0, 0xff, b'\n']);
    let mut buf = Vec::new();
    write_server_msg(&mut buf, &msg).unwrap();
    let back = read_server_msg(&mut &buf[..]).unwrap();
    assert_eq!(msg, back);
}

#[test]
fn framing_multiple_messages_can_be_read_in_sequence() {
    let mut buf = Vec::new();
    write_client_msg(&mut buf, &ClientMsg::Stdin(vec![1])).unwrap();
    write_client_msg(&mut buf, &ClientMsg::Resize { cols: 80, rows: 24 }).unwrap();
    write_client_msg(&mut buf, &ClientMsg::Detach).unwrap();
    let mut reader = &buf[..];
    assert_eq!(
        read_client_msg(&mut reader).unwrap(),
        ClientMsg::Stdin(vec![1])
    );
    assert_eq!(
        read_client_msg(&mut reader).unwrap(),
        ClientMsg::Resize { cols: 80, rows: 24 }
    );
    assert_eq!(read_client_msg(&mut reader).unwrap(), ClientMsg::Detach);
}

#[test]
fn framing_rejects_oversized_length() {
    let mut buf = Vec::new();
    let too_big: u32 = MAX_FRAME_BYTES + 1;
    buf.extend_from_slice(&too_big.to_be_bytes());
    let err = read_client_msg(&mut &buf[..]).unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::InvalidData);
}

#[test]
fn framing_returns_unexpected_eof_on_truncated_body() {
    let mut buf = Vec::new();
    // 大きめのlenだけ書いて、bodyは空
    let len: u32 = 100;
    buf.extend_from_slice(&len.to_be_bytes());
    let err = read_client_msg(&mut &buf[..]).unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::UnexpectedEof);
}

#[test]
fn framing_returns_unexpected_eof_on_truncated_length() {
    // 4バイト未満のlength
    let buf = [0u8, 0u8];
    let err = read_client_msg(&mut &buf[..]).unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::UnexpectedEof);
}

#[test]
fn control_request_detach_roundtrip_postcard() {
    let msg = ControlMsg::RequestDetach;
    let bytes = postcard::to_allocvec(&msg).unwrap();
    let back: ControlMsg = postcard::from_bytes(&bytes).unwrap();
    assert_eq!(msg, back);
}

#[test]
fn control_request_rename_roundtrip_postcard() {
    let msg = ControlMsg::RequestRename {
        new_name: "brave-otter".to_string(),
    };
    let bytes = postcard::to_allocvec(&msg).unwrap();
    let back: ControlMsg = postcard::from_bytes(&bytes).unwrap();
    assert_eq!(msg, back);
}

#[test]
fn control_msg_framing_roundtrip_detach() {
    let msg = ControlMsg::RequestDetach;
    let mut buf = Vec::new();
    write_control_msg(&mut buf, &msg).unwrap();
    let back = read_control_msg(&mut &buf[..]).unwrap();
    assert_eq!(msg, back);
}

#[test]
fn control_msg_framing_roundtrip_rename() {
    let msg = ControlMsg::RequestRename {
        new_name: "new-name".to_string(),
    };
    let mut buf = Vec::new();
    write_control_msg(&mut buf, &msg).unwrap();
    let back = read_control_msg(&mut &buf[..]).unwrap();
    assert_eq!(msg, back);
}

#[test]
fn control_query_current_client_roundtrip_postcard() {
    let msg = ControlMsg::QueryCurrentClient;
    let bytes = postcard::to_allocvec(&msg).unwrap();
    let back: ControlMsg = postcard::from_bytes(&bytes).unwrap();
    assert_eq!(msg, back);
}

#[test]
fn control_msg_framing_roundtrip_query_current_client() {
    let msg = ControlMsg::QueryCurrentClient;
    let mut buf = Vec::new();
    write_control_msg(&mut buf, &msg).unwrap();
    let back = read_control_msg(&mut &buf[..]).unwrap();
    assert_eq!(msg, back);
}

#[test]
fn control_response_current_client_roundtrip_postcard() {
    let msg = ControlResponse::CurrentClient { pid: 12345 };
    let bytes = postcard::to_allocvec(&msg).unwrap();
    let back: ControlResponse = postcard::from_bytes(&bytes).unwrap();
    assert_eq!(msg, back);
}

#[test]
fn control_response_framing_roundtrip_current_client() {
    let msg = ControlResponse::CurrentClient { pid: 4242 };
    let mut buf = Vec::new();
    write_control_response(&mut buf, &msg).unwrap();
    let back = read_control_response(&mut &buf[..]).unwrap();
    assert_eq!(msg, back);
}

#[test]
fn control_response_framing_roundtrip_current_client_zero() {
    // pid=0(まだ誰もstdin送ってない)も普通に往復できる
    let msg = ControlResponse::CurrentClient { pid: 0 };
    let mut buf = Vec::new();
    write_control_response(&mut buf, &msg).unwrap();
    let back = read_control_response(&mut &buf[..]).unwrap();
    assert_eq!(msg, back);
}
