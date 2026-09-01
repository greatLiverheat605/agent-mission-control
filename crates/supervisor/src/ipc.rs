use std::fs::{File, OpenOptions};
use std::io;
use std::os::windows::io::{AsRawHandle, FromRawHandle};
use std::ptr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use mission_protocol::frame::{FrameError, read_frame, write_frame};
use mission_protocol::handshake::{
    ClientMessage, CommandResponse, HandshakeVerifier, InstallSecretProvider, Pong, ProtocolError,
    ProtocolErrorCode, ServerMessage,
};
use mission_protocol::windows_security::{SecurityAttributes, current_user_sid};
use windows_sys::Win32::Foundation::{
    ERROR_NO_DATA, ERROR_OPERATION_ABORTED, ERROR_PIPE_CONNECTED, INVALID_HANDLE_VALUE,
};
use windows_sys::Win32::Storage::FileSystem::{
    FILE_FLAG_FIRST_PIPE_INSTANCE, FlushFileBuffers, PIPE_ACCESS_DUPLEX,
};
use windows_sys::Win32::System::IO::CancelSynchronousIo;
use windows_sys::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, DisconnectNamedPipe, PIPE_READMODE_BYTE,
    PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_BYTE, PIPE_WAIT,
};

const PIPE_BUFFER_BYTES: u32 = 64 * 1024;
const DEFAULT_IDLE_TIMEOUT: Duration = Duration::from_secs(30);
const WATCHDOG_POLL_INTERVAL: Duration = Duration::from_millis(10);
const MIN_COMMAND_HEARTBEAT_INTERVAL: Duration = Duration::from_millis(10);

pub trait IpcDispatcher: Send + Sync {
    fn dispatch(
        &self,
        command: &str,
        request: serde_json::Value,
    ) -> Result<serde_json::Value, String>;
    fn touch_ui(&self);
}

struct NoopDispatcher;

impl IpcDispatcher for NoopDispatcher {
    fn dispatch(
        &self,
        _command: &str,
        _request: serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        Err("COMMAND_NOT_AVAILABLE".to_owned())
    }

    fn touch_ui(&self) {}
}

pub struct IpcServer {
    stop: Arc<AtomicBool>,
    pipe_path: String,
    thread: Option<JoinHandle<io::Result<()>>>,
}

impl IpcServer {
    pub fn spawn<P>(pipe_name: &str, install_id: &str, secret_provider: P) -> io::Result<Self>
    where
        P: InstallSecretProvider + Send + 'static,
    {
        Self::spawn_with_dispatcher(
            pipe_name,
            install_id,
            secret_provider,
            Arc::new(NoopDispatcher),
        )
    }

    pub fn spawn_with_dispatcher<P>(
        pipe_name: &str,
        install_id: &str,
        secret_provider: P,
        dispatcher: Arc<dyn IpcDispatcher>,
    ) -> io::Result<Self>
    where
        P: InstallSecretProvider + Send + 'static,
    {
        Self::spawn_with_idle_timeout_and_dispatcher(
            pipe_name,
            install_id,
            secret_provider,
            DEFAULT_IDLE_TIMEOUT,
            dispatcher,
        )
    }

    #[cfg(test)]
    fn spawn_with_idle_timeout<P>(
        pipe_name: &str,
        install_id: &str,
        secret_provider: P,
        idle_timeout: Duration,
    ) -> io::Result<Self>
    where
        P: InstallSecretProvider + Send + 'static,
    {
        Self::spawn_with_idle_timeout_and_dispatcher(
            pipe_name,
            install_id,
            secret_provider,
            idle_timeout,
            Arc::new(NoopDispatcher),
        )
    }

    fn spawn_with_idle_timeout_and_dispatcher<P>(
        pipe_name: &str,
        install_id: &str,
        secret_provider: P,
        idle_timeout: Duration,
        dispatcher: Arc<dyn IpcDispatcher>,
    ) -> io::Result<Self>
    where
        P: InstallSecretProvider + Send + 'static,
    {
        let sid = current_user_sid()?;
        let pipe_path = format!(r"\\.\pipe\{pipe_name}");
        let first = PipeInstance::create(&pipe_path, &sid)?;
        let stop = Arc::new(AtomicBool::new(false));
        let thread_stop = Arc::clone(&stop);
        let thread_install_id = install_id.to_owned();
        let thread_dispatcher = Arc::clone(&dispatcher);
        let server_thread = thread::Builder::new()
            .name("mission-supervisor-ipc".to_owned())
            .spawn(move || {
                run_server(
                    first,
                    &thread_install_id,
                    secret_provider,
                    thread_stop,
                    idle_timeout,
                    thread_dispatcher,
                )
            })?;

        Ok(Self {
            stop,
            pipe_path,
            thread: Some(server_thread),
        })
    }

    pub fn is_running(&self) -> bool {
        self.thread
            .as_ref()
            .is_some_and(|thread| !thread.is_finished())
    }

    pub fn shutdown(mut self) -> io::Result<()> {
        self.stop_and_join()
    }

    fn stop_and_join(&mut self) -> io::Result<()> {
        let Some(thread) = self.thread.as_ref() else {
            return Ok(());
        };

        self.stop.store(true, Ordering::SeqCst);
        while !thread.is_finished() {
            unsafe {
                CancelSynchronousIo(thread.as_raw_handle().cast());
            }
            let _ = OpenOptions::new()
                .read(true)
                .write(true)
                .open(&self.pipe_path);
            thread::sleep(Duration::from_millis(10));
        }

        self.thread
            .take()
            .expect("finished pipe server thread is present")
            .join()
            .map_err(|_| io::Error::other("pipe server thread panicked"))?
    }
}

impl Drop for IpcServer {
    fn drop(&mut self) {
        let _ = self.stop_and_join();
    }
}

fn run_server<P>(
    mut instance: PipeInstance,
    install_id: &str,
    secret_provider: P,
    stop: Arc<AtomicBool>,
    idle_timeout: Duration,
    dispatcher: Arc<dyn IpcDispatcher>,
) -> io::Result<()>
where
    P: InstallSecretProvider + Send + 'static,
{
    let mut verifier = HandshakeVerifier::new(install_id, secret_provider);

    while !stop.load(Ordering::SeqCst) {
        if !instance.connect()? {
            instance.disconnect();
            continue;
        }
        if stop.load(Ordering::SeqCst) {
            instance.disconnect();
            break;
        }

        (instance, verifier) = serve_with_idle_timeout(
            instance,
            verifier,
            &stop,
            idle_timeout,
            Arc::clone(&dispatcher),
        )?;
        instance.disconnect();
    }

    Ok(())
}

fn serve_with_idle_timeout<P: InstallSecretProvider + Send + 'static>(
    mut instance: PipeInstance,
    mut verifier: HandshakeVerifier<P>,
    stop: &AtomicBool,
    idle_timeout: Duration,
    dispatcher: Arc<dyn IpcDispatcher>,
) -> io::Result<(PipeInstance, HandshakeVerifier<P>)> {
    let (activity_tx, activity_rx) = mpsc::channel();
    let worker = thread::Builder::new()
        .name("mission-supervisor-ipc-connection".to_owned())
        .spawn(move || {
            serve_connection(
                &mut instance.0,
                &mut verifier,
                &activity_tx,
                idle_timeout,
                dispatcher,
            );
            unsafe {
                FlushFileBuffers(instance.0.as_raw_handle().cast());
            }
            (instance, verifier)
        })?;
    let mut deadline = Instant::now() + idle_timeout;
    let mut cancelling = false;

    while !worker.is_finished() {
        if stop.load(Ordering::SeqCst) || Instant::now() >= deadline {
            cancelling = true;
        }
        if cancelling {
            unsafe {
                CancelSynchronousIo(worker.as_raw_handle().cast());
            }
        }

        let until_deadline = deadline.saturating_duration_since(Instant::now());
        let wait = if cancelling {
            WATCHDOG_POLL_INTERVAL
        } else {
            until_deadline.min(WATCHDOG_POLL_INTERVAL)
        };
        match activity_rx.recv_timeout(wait) {
            Ok(()) if !cancelling => deadline = Instant::now() + idle_timeout,
            Ok(()) => {}
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }
    }

    worker
        .join()
        .map_err(|_| io::Error::other("pipe connection thread panicked"))
}

fn serve_connection<P: InstallSecretProvider>(
    pipe: &mut File,
    verifier: &mut HandshakeVerifier<P>,
    activity: &mpsc::Sender<()>,
    idle_timeout: Duration,
    dispatcher: Arc<dyn IpcDispatcher>,
) {
    let mut authenticated_protocol = None;
    loop {
        let message = match read_frame(pipe) {
            Ok(message) => message,
            Err(FrameError::FrameTooLarge) => {
                send_error(pipe, ProtocolErrorCode::FrameTooLarge);
                return;
            }
            Err(FrameError::InvalidUtf8 | FrameError::InvalidJson) => {
                send_error(pipe, ProtocolErrorCode::InvalidFrame);
                return;
            }
            Err(FrameError::Io(_)) => return,
        };
        let _ = activity.send(());

        match (authenticated_protocol, message) {
            (None, ClientMessage::Handshake(handshake)) => {
                match verifier.verify_at(&handshake, Instant::now()) {
                    Ok(accepted) => {
                        authenticated_protocol = Some(accepted.protocol_version);
                        if write_frame(pipe, &ServerMessage::HandshakeAccepted(accepted)).is_err() {
                            return;
                        }
                    }
                    Err(error) => {
                        let _ = write_frame(pipe, &ServerMessage::Error(error));
                        return;
                    }
                }
            }
            (Some(protocol_version), ClientMessage::Ping) => {
                dispatcher.touch_ui();
                let response = ServerMessage::Pong(Pong {
                    supervisor_version: env!("CARGO_PKG_VERSION").to_owned(),
                    protocol_version,
                });
                if write_frame(pipe, &response).is_err() {
                    return;
                }
            }
            (Some(_), ClientMessage::Command(command)) => {
                dispatcher.touch_ui();
                let heartbeat_interval = (idle_timeout / 3).max(MIN_COMMAND_HEARTBEAT_INTERVAL);
                let (heartbeat_stop_tx, heartbeat_stop_rx) = mpsc::channel();
                let heartbeat_activity = activity.clone();
                let heartbeat_dispatcher = Arc::clone(&dispatcher);
                let heartbeat = thread::Builder::new()
                    .name("mission-supervisor-ipc-command-heartbeat".to_owned())
                    .spawn(move || {
                        loop {
                            match heartbeat_stop_rx.recv_timeout(heartbeat_interval) {
                                Ok(()) | Err(RecvTimeoutError::Disconnected) => break,
                                Err(RecvTimeoutError::Timeout) => {
                                    let _ = heartbeat_activity.send(());
                                    heartbeat_dispatcher.touch_ui();
                                }
                            }
                        }
                    })
                    .ok();
                let response = match dispatcher.dispatch(&command.command, command.request) {
                    Ok(result) => ServerMessage::Command(CommandResponse {
                        command: command.command,
                        result: Some(result),
                        error: None,
                    }),
                    Err(error) => ServerMessage::Command(CommandResponse {
                        command: command.command,
                        result: None,
                        error: Some(error),
                    }),
                };
                let _ = heartbeat_stop_tx.send(());
                if let Some(heartbeat) = heartbeat {
                    let _ = heartbeat.join();
                }
                if write_frame(pipe, &response).is_err() {
                    return;
                }
            }
            (None, ClientMessage::Ping | ClientMessage::Command(_))
            | (Some(_), ClientMessage::Handshake(_)) => {
                send_error(pipe, ProtocolErrorCode::AuthFailed);
                return;
            }
        }
    }
}

fn send_error(pipe: &mut File, code: ProtocolErrorCode) {
    let _ = write_frame(pipe, &ServerMessage::Error(ProtocolError::new(code)));
}

struct PipeInstance(File);

impl PipeInstance {
    fn create(pipe_path: &str, sid: &str) -> io::Result<Self> {
        let security = SecurityAttributes::from_sddl(&pipe_sddl(sid))?;
        let wide_path: Vec<u16> = pipe_path.encode_utf16().chain(Some(0)).collect();
        let handle = unsafe {
            CreateNamedPipeW(
                wide_path.as_ptr(),
                PIPE_ACCESS_DUPLEX | FILE_FLAG_FIRST_PIPE_INSTANCE,
                PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
                1,
                PIPE_BUFFER_BYTES,
                PIPE_BUFFER_BYTES,
                0,
                security.as_ptr(),
            )
        };
        if handle == INVALID_HANDLE_VALUE {
            return Err(last_error("CreateNamedPipeW"));
        }

        Ok(Self(unsafe { File::from_raw_handle(handle.cast()) }))
    }

    fn connect(&self) -> io::Result<bool> {
        let connected = unsafe { ConnectNamedPipe(self.0.as_raw_handle().cast(), ptr::null_mut()) };
        if connected == 0 {
            let error = io::Error::last_os_error();
            if matches!(
                error.raw_os_error(),
                Some(code) if code == ERROR_NO_DATA as i32 || code == ERROR_OPERATION_ABORTED as i32
            ) {
                return Ok(false);
            }
            if error.raw_os_error() != Some(ERROR_PIPE_CONNECTED as i32) {
                return Err(io::Error::new(
                    error.kind(),
                    format!("ConnectNamedPipe failed: {error}"),
                ));
            }
        }
        Ok(true)
    }

    fn disconnect(&self) {
        unsafe {
            DisconnectNamedPipe(self.0.as_raw_handle().cast());
        }
    }
}

fn pipe_sddl(sid: &str) -> String {
    format!("D:P(A;;GA;;;{sid})")
}

fn last_error(operation: &str) -> io::Error {
    let error = io::Error::last_os_error();
    io::Error::new(error.kind(), format!("{operation} failed: {error}"))
}

#[cfg(test)]
mod tests {
    use std::fs::{File, OpenOptions};
    use std::io::{self, Write};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
    use std::thread;
    use std::time::{Duration, Instant};

    use mission_protocol::frame::{read_frame, write_frame};
    use mission_protocol::handshake::{
        ClientMessage, Handshake, InstallSecretProvider, NONCE_BYTES, PROTOCOL_VERSION,
        ServerMessage, handshake_proof,
    };

    use super::{IpcServer, pipe_sddl};

    const INSTALL_ID: &str = "9f3628e6-2c77-4815-91cc-213e92e07726";
    const FIXTURE_SECRET: &[u8] = b"phase-1-idle-fixture-secret-never-log";
    const TEST_IDLE_TIMEOUT: Duration = Duration::from_millis(100);
    static PIPE_NONCE: AtomicU64 = AtomicU64::new(0);

    #[derive(Clone, Copy)]
    struct FixtureSecret;

    impl InstallSecretProvider for FixtureSecret {
        fn install_secret(&self) -> io::Result<Vec<u8>> {
            Ok(FIXTURE_SECRET.to_vec())
        }
    }

    struct SlowDispatcher {
        delay: Duration,
        heartbeats: Arc<AtomicUsize>,
    }

    impl super::IpcDispatcher for SlowDispatcher {
        fn dispatch(
            &self,
            command: &str,
            request: serde_json::Value,
        ) -> Result<serde_json::Value, String> {
            thread::sleep(self.delay);
            Ok(serde_json::json!({ "command": command, "request": request }))
        }

        fn touch_ui(&self) {
            self.heartbeats.fetch_add(1, Ordering::SeqCst);
        }
    }

    fn unique_pipe_name() -> String {
        format!(
            "mission-control-ipc-idle-{}-{}",
            std::process::id(),
            PIPE_NONCE.fetch_add(1, Ordering::Relaxed)
        )
    }

    fn connect(pipe_name: &str) -> File {
        let path = format!(r"\\.\pipe\{pipe_name}");
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            match OpenOptions::new().read(true).write(true).open(&path) {
                Ok(pipe) => return pipe,
                Err(error) => {
                    assert!(Instant::now() < deadline, "connect to {path}: {error}");
                    thread::sleep(Duration::from_millis(5));
                }
            }
        }
    }

    fn handshake(index: u8) -> ClientMessage {
        let nonce = vec![index; NONCE_BYTES];
        let versions = vec![PROTOCOL_VERSION];
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

    fn assert_idle_client_is_released(prepare: impl FnOnce(&mut File), nonce: u8) {
        let pipe_name = unique_pipe_name();
        let server = IpcServer::spawn_with_idle_timeout(
            &pipe_name,
            INSTALL_ID,
            FixtureSecret,
            TEST_IDLE_TIMEOUT,
        )
        .expect("start short-timeout pipe server");
        let mut stalled = connect(&pipe_name);
        prepare(&mut stalled);

        let mut next = connect(&pipe_name);
        assert!(server.is_running(), "idle client must not stop the server");
        assert!(matches!(
            exchange(&mut next, &handshake(nonce)),
            ServerMessage::HandshakeAccepted(_)
        ));
        assert!(matches!(
            exchange(&mut next, &ClientMessage::Ping),
            ServerMessage::Pong(_)
        ));

        drop(stalled);
        drop(next);
        server.shutdown().expect("stop short-timeout server");
    }

    #[test]
    fn pipe_acl_is_protected_and_only_grants_the_current_user() {
        let sid = "S-1-5-21-111-222-333-1001";

        assert_eq!(pipe_sddl(sid), "D:P(A;;GA;;;S-1-5-21-111-222-333-1001)");
    }

    #[test]
    fn idle_deadline_releases_partial_and_authenticated_clients() {
        assert_idle_client_is_released(
            |client| client.write_all(&[4, 0]).expect("write partial header"),
            1,
        );
        assert_idle_client_is_released(
            |client| {
                client
                    .write_all(&10_u32.to_le_bytes())
                    .expect("write body length");
                client.write_all(b"{").expect("write partial body");
            },
            2,
        );
        assert_idle_client_is_released(
            |client| {
                assert!(matches!(
                    exchange(client, &handshake(3)),
                    ServerMessage::HandshakeAccepted(_)
                ));
            },
            4,
        );
    }

    #[test]
    fn active_messages_reset_the_idle_deadline() {
        let pipe_name = unique_pipe_name();
        let server = IpcServer::spawn_with_idle_timeout(
            &pipe_name,
            INSTALL_ID,
            FixtureSecret,
            TEST_IDLE_TIMEOUT,
        )
        .expect("start short-timeout pipe server");
        let mut client = connect(&pipe_name);
        assert!(matches!(
            exchange(&mut client, &handshake(5)),
            ServerMessage::HandshakeAccepted(_)
        ));

        for _ in 0..5 {
            thread::sleep(TEST_IDLE_TIMEOUT / 2);
            assert!(matches!(
                exchange(&mut client, &ClientMessage::Ping),
                ServerMessage::Pong(_)
            ));
        }

        assert!(server.is_running());
        drop(client);
        server.shutdown().expect("stop short-timeout server");
    }

    #[test]
    fn in_flight_command_emits_heartbeats_until_response() {
        let pipe_name = unique_pipe_name();
        let heartbeats = Arc::new(AtomicUsize::new(0));
        let server = IpcServer::spawn_with_idle_timeout_and_dispatcher(
            &pipe_name,
            INSTALL_ID,
            FixtureSecret,
            TEST_IDLE_TIMEOUT,
            Arc::new(SlowDispatcher {
                delay: Duration::from_millis(250),
                heartbeats: Arc::clone(&heartbeats),
            }),
        )
        .expect("start long-command fixture server");
        let mut client = connect(&pipe_name);
        assert!(matches!(
            exchange(&mut client, &handshake(6)),
            ServerMessage::HandshakeAccepted(_)
        ));

        let started = Instant::now();
        let response = exchange(
            &mut client,
            &ClientMessage::Command(mission_protocol::handshake::CommandRequest {
                command: "launch_route".to_owned(),
                request: serde_json::json!({ "fixture": true }),
            }),
        );

        assert!(matches!(response, ServerMessage::Command(_)));
        assert!(started.elapsed() >= Duration::from_millis(200));
        assert!(
            heartbeats.load(Ordering::SeqCst) >= 2,
            "long command did not refresh the IPC/UI lease"
        );
        assert!(server.is_running());
        drop(client);
        server.shutdown().expect("stop long-command fixture server");
    }
}
