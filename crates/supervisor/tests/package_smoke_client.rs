#![cfg(windows)]

use std::fs::File;
use std::io::Write;
use std::os::windows::io::{AsRawHandle, FromRawHandle};
use std::ptr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use mission_protocol::frame::{read_frame, write_frame};
use mission_protocol::handshake::{
    ClientMessage, HandshakeAccepted, InstallSecretProvider, PROTOCOL_VERSION, Pong, ServerMessage,
};
use mission_supervisor::package_smoke::{
    PackageSmokeClientError, authenticated_ping_with_provider,
};
use windows_sys::Win32::Foundation::{ERROR_PIPE_CONNECTED, INVALID_HANDLE_VALUE};
use windows_sys::Win32::Storage::FileSystem::PIPE_ACCESS_DUPLEX;
use windows_sys::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, PIPE_READMODE_BYTE, PIPE_REJECT_REMOTE_CLIENTS,
    PIPE_TYPE_BYTE, PIPE_WAIT,
};

const FIXTURE_SECRET: &[u8] = b"package-smoke-client-fixture-secret";
static TEST_PIPE_NONCE: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Copy)]
enum Scenario {
    StagedResponses,
    StalledHeader,
    StalledBody,
    StalledWrite,
}

struct DropSignalProvider {
    dropped: Option<mpsc::Sender<()>>,
}

impl InstallSecretProvider for DropSignalProvider {
    fn install_secret(&self) -> std::io::Result<Vec<u8>> {
        Ok(FIXTURE_SECRET.to_vec())
    }
}

impl Drop for DropSignalProvider {
    fn drop(&mut self) {
        if let Some(dropped) = self.dropped.take() {
            let _ = dropped.send(());
        }
    }
}

fn unique_pipe_name(label: &str) -> String {
    format!(
        "mission-package-client-{label}-{}-{}",
        std::process::id(),
        TEST_PIPE_NONCE.fetch_add(1, Ordering::Relaxed)
    )
}

fn create_pipe(name: &str, buffer_size: u32) -> File {
    let path: Vec<u16> = format!(r"\\.\pipe\{name}")
        .encode_utf16()
        .chain(Some(0))
        .collect();
    let handle = unsafe {
        CreateNamedPipeW(
            path.as_ptr(),
            PIPE_ACCESS_DUPLEX,
            PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
            1,
            buffer_size,
            buffer_size,
            0,
            ptr::null(),
        )
    };
    assert_ne!(handle, INVALID_HANDLE_VALUE, "create fixture named pipe");
    unsafe { File::from_raw_handle(handle.cast()) }
}

fn connect_server(pipe: &File) {
    if unsafe { ConnectNamedPipe(pipe.as_raw_handle().cast(), ptr::null_mut()) } == 0 {
        assert_eq!(
            std::io::Error::last_os_error().raw_os_error(),
            Some(ERROR_PIPE_CONNECTED as i32),
            "connect fixture named pipe"
        );
    }
}

fn spawn_server(name: String, scenario: Scenario) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        let mut pipe = create_pipe(&name, 32);
        connect_server(&pipe);
        if matches!(scenario, Scenario::StalledWrite) {
            thread::sleep(Duration::from_millis(300));
            return;
        }

        let _: ClientMessage = read_frame(&mut pipe).expect("read client handshake");
        match scenario {
            Scenario::StagedResponses => {
                thread::sleep(Duration::from_millis(75));
                write_frame(
                    &mut pipe,
                    &ServerMessage::HandshakeAccepted(HandshakeAccepted {
                        protocol_version: PROTOCOL_VERSION,
                    }),
                )
                .expect("write staged handshake acceptance");
                let _: ClientMessage = read_frame(&mut pipe).expect("read staged ping");
                thread::sleep(Duration::from_millis(75));
                let _ = write_frame(
                    &mut pipe,
                    &ServerMessage::Pong(Pong {
                        supervisor_version: "fixture".to_owned(),
                        protocol_version: PROTOCOL_VERSION,
                    }),
                );
            }
            Scenario::StalledHeader => {
                pipe.write_all(&[10, 0])
                    .expect("write partial frame header");
                thread::sleep(Duration::from_millis(300));
            }
            Scenario::StalledBody => {
                pipe.write_all(&10_u32.to_le_bytes())
                    .expect("write stalled body length");
                pipe.write_all(b"{").expect("write partial frame body");
                thread::sleep(Duration::from_millis(300));
            }
            Scenario::StalledWrite => unreachable!(),
        }
    })
}

fn assert_total_timeout(scenario: Scenario, timeout: Duration, upper_bound: Duration) {
    let pipe_name = unique_pipe_name(match scenario {
        Scenario::StagedResponses => "staged",
        Scenario::StalledHeader => "header",
        Scenario::StalledBody => "body",
        Scenario::StalledWrite => "write",
    });
    let server = spawn_server(pipe_name.clone(), scenario);
    let (dropped_tx, dropped_rx) = mpsc::channel();
    let started = Instant::now();

    let result = authenticated_ping_with_provider(
        &pipe_name,
        DropSignalProvider {
            dropped: Some(dropped_tx),
        },
        timeout,
    );
    let elapsed = started.elapsed();

    assert_eq!(result, Err(PackageSmokeClientError::Timeout));
    assert!(
        elapsed < upper_bound,
        "single total deadline exceeded its bound: {elapsed:?}"
    );
    dropped_rx
        .recv_timeout(Duration::from_millis(10))
        .expect("client worker joined before timeout returned");
    server.join().expect("join fixture server");
}

#[test]
fn one_deadline_covers_handshake_and_ping_stages() {
    assert_total_timeout(
        Scenario::StagedResponses,
        Duration::from_millis(120),
        Duration::from_millis(220),
    );
}

#[test]
fn timeout_cancels_stalled_header_and_joins_worker() {
    assert_total_timeout(
        Scenario::StalledHeader,
        Duration::from_millis(80),
        Duration::from_millis(180),
    );
}

#[test]
fn timeout_cancels_stalled_body_and_joins_worker() {
    assert_total_timeout(
        Scenario::StalledBody,
        Duration::from_millis(80),
        Duration::from_millis(180),
    );
}

#[test]
fn timeout_cancels_stalled_write_and_joins_worker() {
    assert_total_timeout(
        Scenario::StalledWrite,
        Duration::from_millis(80),
        Duration::from_millis(180),
    );
}
