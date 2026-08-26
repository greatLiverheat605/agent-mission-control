#[path = "../src/supervisor_bridge.rs"]
mod supervisor_bridge;

use std::fs::OpenOptions;
use std::io::{self, Read, Write};
use std::os::windows::io::{AsRawHandle, FromRawHandle};
use std::path::Path;
use std::ptr;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, mpsc};
use std::thread;
use std::time::{Duration, Instant};

use mission_protocol::frame::{read_frame, write_frame};
use mission_protocol::handshake::{
    ClientMessage, InstallSecretProvider, PRODUCT_INSTALL_ID, ServerMessage,
};
use mission_supervisor::ipc::{IpcDispatcher, IpcServer};
use windows_sys::Win32::Foundation::{ERROR_PIPE_CONNECTED, GetLastError, INVALID_HANDLE_VALUE};
use windows_sys::Win32::Storage::FileSystem::PIPE_ACCESS_DUPLEX;
use windows_sys::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, PIPE_READMODE_BYTE, PIPE_REJECT_REMOTE_CLIENTS,
    PIPE_TYPE_BYTE, PIPE_WAIT,
};

use supervisor_bridge::{
    BridgeError, LocalSupervisorTransport, PublicSupervisorStatus, SupervisorBridge,
    SupervisorTransport,
};

macro_rules! command_names {
    ($($command:ident),+ $(,)?) => {
        [$(stringify!($command)),+]
    };
}

static PIPE_NONCE: AtomicU64 = AtomicU64::new(0);
const FIXTURE_SECRET: &[u8] = b"desktop-bridge-fixture-secret";

struct FakePipe {
    response: Result<String, BridgeError>,
}

struct EchoDispatcher;

impl IpcDispatcher for EchoDispatcher {
    fn dispatch(
        &self,
        command: &str,
        request: serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        Ok(serde_json::json!({ "command": command, "request": request }))
    }

    fn touch_ui(&self) {}
}

impl SupervisorTransport for FakePipe {
    fn ping(&mut self) -> Result<String, BridgeError> {
        self.response.clone()
    }
}

#[derive(Clone)]
struct FixtureSecret;

impl InstallSecretProvider for FixtureSecret {
    fn install_secret(&self) -> io::Result<Vec<u8>> {
        Ok(FIXTURE_SECRET.to_vec())
    }
}

#[derive(Clone)]
struct CountingSecret(Arc<AtomicUsize>);

impl InstallSecretProvider for CountingSecret {
    fn install_secret(&self) -> io::Result<Vec<u8>> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(FIXTURE_SECRET.to_vec())
    }
}

fn unique_pipe_name() -> String {
    format!(
        "mission-control-desktop-bridge-{}-{}",
        std::process::id(),
        PIPE_NONCE.fetch_add(1, Ordering::Relaxed)
    )
}

fn spawn_partial_handshake_server(
    pipe_name: &str,
) -> (thread::JoinHandle<()>, mpsc::Sender<()>, mpsc::Receiver<()>) {
    let pipe_path: Vec<u16> = format!(r"\\.\pipe\{pipe_name}")
        .encode_utf16()
        .chain(Some(0))
        .collect();
    let handle = unsafe {
        CreateNamedPipeW(
            pipe_path.as_ptr(),
            PIPE_ACCESS_DUPLEX,
            PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
            1,
            64 * 1024,
            64 * 1024,
            0,
            ptr::null(),
        )
    };
    assert_ne!(handle, INVALID_HANDLE_VALUE, "create stalled named pipe");
    let mut pipe = unsafe { std::fs::File::from_raw_handle(handle.cast()) };
    let (release_tx, release_rx) = mpsc::channel();
    let (partial_tx, partial_rx) = mpsc::channel();
    let server = thread::spawn(move || {
        if unsafe { ConnectNamedPipe(pipe.as_raw_handle().cast(), ptr::null_mut()) } == 0 {
            assert_eq!(unsafe { GetLastError() }, ERROR_PIPE_CONNECTED);
        }
        let _: ClientMessage = read_frame(&mut pipe).expect("read desktop handshake");
        pipe.write_all(&32_u32.to_le_bytes())
            .expect("write partial response length");
        pipe.write_all(b"{").expect("write partial response body");
        pipe.flush().expect("flush partial response");
        let _ = partial_tx.send(());
        let _ = release_rx.recv_timeout(Duration::from_secs(3));
    });
    (server, release_tx, partial_rx)
}

struct HeaderBoundaryReader {
    pipe: std::fs::File,
    header_bytes: usize,
    delayed: bool,
}

impl Read for HeaderBoundaryReader {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if self.header_bytes == size_of::<u32>() && !self.delayed {
            self.delayed = true;
            thread::sleep(Duration::from_millis(300));
        }
        let read = self.pipe.read(buffer)?;
        self.header_bytes = (self.header_bytes + read).min(size_of::<u32>());
        Ok(read)
    }
}

struct StartsThenConnects {
    started: bool,
}

struct ThreadReportingPipe(std::thread::ThreadId);

impl SupervisorTransport for ThreadReportingPipe {
    fn ping(&mut self) -> Result<String, BridgeError> {
        if std::thread::current().id() == self.0 {
            Err(BridgeError::Protocol)
        } else {
            Ok("0.1.0".to_owned())
        }
    }
}

impl SupervisorTransport for StartsThenConnects {
    fn ping(&mut self) -> Result<String, BridgeError> {
        if self.started {
            Ok("0.1.0".to_owned())
        } else {
            Err(BridgeError::Unavailable)
        }
    }

    fn start_packaged(&mut self) -> Result<(), BridgeError> {
        self.started = true;
        Ok(())
    }
}

#[test]
fn bridge_exposes_only_the_two_supervisor_commands() {
    let expected = ["supervisor_status", "ping_supervisor"];

    assert_eq!(
        supervisor_bridge::supervisor_commands!(command_names),
        expected
    );
    assert_eq!(supervisor_bridge::ALLOWED_COMMANDS, expected);
}

#[test]
fn authentication_failures_map_to_a_fixed_renderer_error() {
    let bridge = SupervisorBridge::new(FakePipe {
        response: Err(BridgeError::Authentication),
    });

    assert_eq!(
        bridge.ping_supervisor(),
        PublicSupervisorStatus {
            connection: "disconnected",
            version: None,
            error_code: Some("SUPERVISOR_AUTH_FAILED"),
        }
    );
}

#[test]
fn real_transport_dispatches_allowlisted_mission_commands_over_the_authenticated_pipe() {
    let pipe_name = unique_pipe_name();
    let server = IpcServer::spawn_with_dispatcher(
        &pipe_name,
        PRODUCT_INSTALL_ID,
        FixtureSecret,
        Arc::new(EchoDispatcher),
    )
    .expect("start mission command server");
    let bridge = SupervisorBridge::new(LocalSupervisorTransport::for_test(
        &pipe_name,
        FixtureSecret,
    ));

    let result = bridge
        .dispatch_mission(
            "subscribe_mission",
            serde_json::json!({ "missionId": "mission-1" }),
        )
        .expect("dispatch mission command");

    assert_eq!(result["command"], "subscribe_mission");
    assert_eq!(result["request"]["missionId"], "mission-1");
    assert_eq!(
        bridge.dispatch_mission("arbitrary_shell", serde_json::json!({})),
        Err("COMMAND_NOT_ALLOWED".to_owned())
    );
    server.shutdown().expect("stop mission command server");
}

#[test]
fn mission_allowlist_exposes_recovery_package_command() {
    let expected = [
        "create_mission",
        "update_mission_contract",
        "launch_route",
        "subscribe_mission",
        "request_safe_pause",
        "force_terminate",
        "build_recovery_package",
        "review_memory",
        "handoff_provider",
        "provider_capabilities",
    ];

    assert_eq!(
        supervisor_bridge::mission_commands!(command_names),
        expected
    );
    assert_eq!(supervisor_bridge::MISSION_ALLOWED_COMMANDS, expected);
}

#[test]
fn initial_status_starts_the_packaged_supervisor_then_retries() {
    let bridge = SupervisorBridge::new(StartsThenConnects { started: false });

    assert_eq!(
        bridge.supervisor_status(),
        PublicSupervisorStatus {
            connection: "connected",
            version: Some("0.1.0".to_owned()),
            error_code: None,
        }
    );
}

#[test]
fn packaged_supervisor_command_has_an_independent_lifetime() {
    let data_dir = Path::new(r"C:\ProgramData\Agent Mission Control");
    let command = supervisor_bridge::packaged_supervisor_command(
        Path::new("mission-control-supervisor.exe"),
        data_dir,
        "mission-control-test",
        Some("mission-control-e2e-profile"),
    );

    assert_eq!(
        command.get_args().collect::<Vec<_>>(),
        [
            std::ffi::OsStr::new("--data-dir"),
            data_dir.as_os_str(),
            std::ffi::OsStr::new("--pipe-name"),
            std::ffi::OsStr::new("mission-control-test"),
            std::ffi::OsStr::new("--instance-scope"),
            std::ffi::OsStr::new("mission-control-e2e-profile"),
        ]
    );
}

#[test]
fn async_bridge_runs_blocking_transport_off_the_calling_thread() {
    let bridge = Arc::new(SupervisorBridge::new(ThreadReportingPipe(
        std::thread::current().id(),
    )));

    let initial = tauri::async_runtime::block_on(Arc::clone(&bridge).supervisor_status_async());
    let heartbeat = tauri::async_runtime::block_on(Arc::clone(&bridge).ping_supervisor_async());

    assert_eq!(initial.connection, "connected");
    assert_eq!(heartbeat.connection, "connected");
}

#[test]
fn real_transport_caches_the_session_and_reconnects_after_disconnect() {
    let pipe_name = unique_pipe_name();
    let first_handshakes = Arc::new(AtomicUsize::new(0));
    let server = IpcServer::spawn(
        &pipe_name,
        PRODUCT_INSTALL_ID,
        CountingSecret(Arc::clone(&first_handshakes)),
    )
    .expect("start first real pipe server");
    let mut transport = LocalSupervisorTransport::for_test(&pipe_name, FixtureSecret);

    assert_eq!(transport.ping(), Ok("0.1.0".to_owned()));
    assert_eq!(transport.ping(), Ok("0.1.0".to_owned()));
    assert_eq!(first_handshakes.load(Ordering::SeqCst), 1);
    server.shutdown().expect("stop first pipe server");
    assert_eq!(transport.ping(), Err(BridgeError::Unavailable));

    let second_handshakes = Arc::new(AtomicUsize::new(0));
    let replacement = IpcServer::spawn(
        &pipe_name,
        PRODUCT_INSTALL_ID,
        CountingSecret(Arc::clone(&second_handshakes)),
    )
    .expect("start replacement pipe server");
    assert_eq!(transport.ping(), Ok("0.1.0".to_owned()));
    assert_eq!(second_handshakes.load(Ordering::SeqCst), 1);
    replacement
        .shutdown()
        .expect("stop replacement pipe server");
}

#[test]
fn real_transport_times_out_partial_responses_and_reconnects() {
    let pipe_name = unique_pipe_name();
    let (stalled_server, release_stall, _partial_sent) = spawn_partial_handshake_server(&pipe_name);
    let mut transport = LocalSupervisorTransport::for_test(&pipe_name, FixtureSecret);

    let started = Instant::now();
    assert_eq!(transport.ping(), Err(BridgeError::Unavailable));
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "partial response exceeded the client deadline"
    );

    let _ = release_stall.send(());
    stalled_server.join().expect("join stalled pipe server");
    let replacement = IpcServer::spawn(&pipe_name, PRODUCT_INSTALL_ID, FixtureSecret)
        .expect("start replacement pipe server");
    assert_eq!(transport.ping(), Ok("0.1.0".to_owned()));
    replacement
        .shutdown()
        .expect("stop replacement pipe server");
}

#[test]
fn io_deadline_retries_cancellation_across_frame_read_boundary() {
    let pipe_name = unique_pipe_name();
    let (stalled_server, release_stall, partial_sent) = spawn_partial_handshake_server(&pipe_name);
    let mut pipe = OpenOptions::new()
        .read(true)
        .write(true)
        .open(format!(r"\\.\pipe\{pipe_name}"))
        .expect("connect boundary fixture pipe");
    write_frame(&mut pipe, &ClientMessage::Ping).expect("write boundary fixture request");
    partial_sent
        .recv_timeout(Duration::from_secs(1))
        .expect("partial frame is buffered");
    let mut reader = HeaderBoundaryReader {
        pipe,
        header_bytes: 0,
        delayed: false,
    };

    let started = Instant::now();
    let result = supervisor_bridge::with_io_deadline(Duration::from_millis(100), || {
        read_frame::<_, ServerMessage>(&mut reader)
            .map(|_| ())
            .map_err(|_| BridgeError::Unavailable)
    });
    let elapsed = started.elapsed();

    let _ = release_stall.send(());
    stalled_server.join().expect("join boundary fixture server");
    assert_eq!(result, Err(BridgeError::Unavailable));
    assert!(
        elapsed < Duration::from_secs(1),
        "frame boundary cancellation took {elapsed:?}"
    );
}

#[test]
fn real_transport_maps_a_rejected_handshake_to_authentication() {
    let pipe_name = unique_pipe_name();
    let server = IpcServer::spawn(
        &pipe_name,
        PRODUCT_INSTALL_ID,
        CountingSecret(Arc::new(AtomicUsize::new(0))),
    )
    .expect("start authenticated pipe server");
    let mut transport =
        LocalSupervisorTransport::for_test(&pipe_name, FakePipeSecret(b"different-desktop-secret"));

    assert_eq!(transport.ping(), Err(BridgeError::Authentication));
    server.shutdown().expect("stop authenticated pipe server");
}

#[derive(Clone, Copy)]
struct FakePipeSecret(&'static [u8]);

impl InstallSecretProvider for FakePipeSecret {
    fn install_secret(&self) -> io::Result<Vec<u8>> {
        Ok(self.0.to_vec())
    }
}
