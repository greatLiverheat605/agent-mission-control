#[path = "../src/supervisor_bridge.rs"]
mod supervisor_bridge;

use std::io;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

use mission_protocol::handshake::{InstallSecretProvider, PRODUCT_INSTALL_ID};
use mission_supervisor::ipc::IpcServer;

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
