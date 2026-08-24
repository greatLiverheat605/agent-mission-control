#![cfg(windows)]

use std::fs::{File, OpenOptions};
use std::io;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use mission_protocol::frame::{read_frame, write_frame};
use mission_protocol::handshake::{
    ClientMessage, Handshake, InstallSecretProvider, NONCE_BYTES, PROTOCOL_VERSION,
    ProtocolErrorCode, ServerMessage, handshake_proof,
};
use mission_supervisor::ipc::IpcServer;

const INSTALL_ID: &str = "9f3628e6-2c77-4815-91cc-213e92e07726";
const FIXTURE_SECRET: &[u8] = b"phase-1-ipc-fixture-secret-never-log";

#[derive(Clone, Copy)]
struct FixtureSecret;

impl InstallSecretProvider for FixtureSecret {
    fn install_secret(&self) -> io::Result<Vec<u8>> {
        Ok(FIXTURE_SECRET.to_vec())
    }
}

fn unique_pipe_name() -> String {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time is after Unix epoch")
        .as_nanos();
    format!("mission-control-ipc-smoke-{}-{nonce}", std::process::id())
}

fn connect(pipe_name: &str) -> File {
    let path = format!(r"\\.\pipe\{pipe_name}");
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        match OpenOptions::new().read(true).write(true).open(&path) {
            Ok(pipe) => return pipe,
            Err(error) => {
                assert!(Instant::now() < deadline, "connect to {path}: {error}");
                thread::sleep(Duration::from_millis(10));
            }
        }
    }
}

fn handshake(index: u8, versions: Vec<u32>) -> ClientMessage {
    let nonce = vec![index; NONCE_BYTES];
    let proof = handshake_proof(FIXTURE_SECRET, INSTALL_ID, &nonce, &versions)
        .expect("fixture handshake can be signed");
    ClientMessage::Handshake(Handshake {
        install_id: INSTALL_ID.to_owned(),
        protocol_versions: versions,
        nonce,
        proof,
    })
}

fn exchange(pipe: &mut File, message: &ClientMessage) -> ServerMessage {
    write_frame(&mut *pipe, message).expect("write client frame");
    read_frame(pipe).expect("read server frame")
}

fn assert_shutdown_completes_while_client_stalls(prepare_client: impl FnOnce(&mut File)) {
    let pipe_name = unique_pipe_name();
    let server = IpcServer::spawn(&pipe_name, INSTALL_ID, FixtureSecret)
        .expect("start authenticated local pipe server");
    let mut client = connect(&pipe_name);
    prepare_client(&mut client);

    let (result_tx, result_rx) = mpsc::channel();
    let shutdown = thread::spawn(move || {
        let _ = result_tx.send(server.shutdown());
    });
    let result = result_rx
        .recv_timeout(Duration::from_secs(2))
        .expect("stalled client cannot block server shutdown");
    result.expect("stalled pipe I/O is cancelled during shutdown");
    shutdown.join().expect("join shutdown caller");
    drop(client);
}

#[test]
fn authenticated_pipe_serves_ping_and_survives_client_disconnects() {
    let pipe_name = unique_pipe_name();
    let server = IpcServer::spawn(&pipe_name, INSTALL_ID, FixtureSecret)
        .expect("start authenticated local pipe server");

    let mut unauthenticated = connect(&pipe_name);
    let response = exchange(&mut unauthenticated, &ClientMessage::Ping);
    assert!(matches!(
        response,
        ServerMessage::Error(ref error) if error.code == ProtocolErrorCode::AuthFailed
    ));
    drop(unauthenticated);

    let mut authenticated = connect(&pipe_name);
    assert!(matches!(
        exchange(
            &mut authenticated,
            &handshake(1, vec![PROTOCOL_VERSION])
        ),
        ServerMessage::HandshakeAccepted(ref accepted)
            if accepted.protocol_version == PROTOCOL_VERSION
    ));
    assert!(matches!(
        exchange(&mut authenticated, &ClientMessage::Ping),
        ServerMessage::Pong(ref pong)
            if pong.supervisor_version == env!("CARGO_PKG_VERSION")
                && pong.protocol_version == PROTOCOL_VERSION
    ));
    drop(authenticated);

    let mut reconnected = connect(&pipe_name);
    assert!(matches!(
        exchange(&mut reconnected, &handshake(2, vec![PROTOCOL_VERSION])),
        ServerMessage::HandshakeAccepted(_)
    ));
    assert!(matches!(
        exchange(&mut reconnected, &ClientMessage::Ping),
        ServerMessage::Pong(_)
    ));
    drop(reconnected);

    server.shutdown().expect("stop pipe server");
}

#[test]
fn pipe_returns_fixed_rejection_codes() {
    let pipe_name = unique_pipe_name();
    let server = IpcServer::spawn(&pipe_name, INSTALL_ID, FixtureSecret)
        .expect("start authenticated local pipe server");

    let mut wrong_proof = connect(&pipe_name);
    let mut message = handshake(3, vec![PROTOCOL_VERSION]);
    let ClientMessage::Handshake(payload) = &mut message else {
        unreachable!();
    };
    payload.proof[0] ^= 0xff;
    assert!(matches!(
        exchange(&mut wrong_proof, &message),
        ServerMessage::Error(ref error) if error.code == ProtocolErrorCode::AuthFailed
    ));
    drop(wrong_proof);

    let mut incompatible = connect(&pipe_name);
    assert!(matches!(
        exchange(&mut incompatible, &handshake(4, vec![2, 3])),
        ServerMessage::Error(ref error)
            if error.code == ProtocolErrorCode::IncompatibleProtocol
    ));
    drop(incompatible);

    let mut first_nonce = connect(&pipe_name);
    assert!(matches!(
        exchange(&mut first_nonce, &handshake(5, vec![PROTOCOL_VERSION])),
        ServerMessage::HandshakeAccepted(_)
    ));
    drop(first_nonce);

    let mut replay = connect(&pipe_name);
    assert!(matches!(
        exchange(&mut replay, &handshake(5, vec![PROTOCOL_VERSION])),
        ServerMessage::Error(ref error) if error.code == ProtocolErrorCode::ReplayedNonce
    ));
    drop(replay);

    let mut oversized = connect(&pipe_name);
    use std::io::Write;
    oversized
        .write_all(&((mission_protocol::frame::MAX_FRAME_SIZE + 1) as u32).to_le_bytes())
        .expect("write oversized frame header");
    assert!(matches!(
        read_frame(&mut oversized).expect("read structured oversized-frame rejection"),
        ServerMessage::Error(ref error) if error.code == ProtocolErrorCode::FrameTooLarge
    ));
    drop(oversized);

    for payload in [vec![0xff], vec![b'{']] {
        let mut malformed = connect(&pipe_name);
        malformed
            .write_all(&(payload.len() as u32).to_le_bytes())
            .expect("write malformed frame length");
        malformed
            .write_all(&payload)
            .expect("write malformed frame payload");
        assert!(matches!(
            read_frame(&mut malformed).expect("read structured malformed-frame rejection"),
            ServerMessage::Error(ref error) if error.code == ProtocolErrorCode::InvalidFrame
        ));
        drop(malformed);
    }

    server.shutdown().expect("stop pipe server");
}

#[test]
fn stalled_client_io_does_not_block_server_shutdown() {
    use std::io::Write;

    assert_shutdown_completes_while_client_stalls(|client| {
        client
            .write_all(&[4, 0])
            .expect("write partial frame header");
    });
    assert_shutdown_completes_while_client_stalls(|client| {
        client
            .write_all(&10_u32.to_le_bytes())
            .expect("write frame length");
        client.write_all(b"{").expect("write partial frame payload");
    });
    assert_shutdown_completes_while_client_stalls(|client| {
        write_frame(client, &ClientMessage::Ping).expect("write unauthenticated ping");
    });
}

#[test]
fn disconnect_before_server_accept_does_not_stop_the_accept_loop() {
    let pipe_name = unique_pipe_name();
    let server = IpcServer::spawn(&pipe_name, INSTALL_ID, FixtureSecret)
        .expect("start authenticated local pipe server");

    drop(connect(&pipe_name));

    let mut next_client = connect(&pipe_name);
    assert!(matches!(
        exchange(&mut next_client, &handshake(9, vec![PROTOCOL_VERSION])),
        ServerMessage::HandshakeAccepted(_)
    ));
    drop(next_client);
    server.shutdown().expect("stop pipe server");
}
