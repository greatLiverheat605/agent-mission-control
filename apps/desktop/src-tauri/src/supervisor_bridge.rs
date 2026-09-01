use std::fs::{File, OpenOptions};
use std::io;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::thread;
use std::time::Duration;

use mission_protocol::credential::{WindowsCredentialInstallSecret, secure_random};
use mission_protocol::frame::{read_frame, write_frame};
use mission_protocol::handshake::{
    ClientMessage, CommandRequest, Handshake, InstallSecretProvider, NONCE_BYTES,
    PRODUCT_INSTALL_ID, PROTOCOL_VERSION, ProtocolErrorCode, ServerMessage, handshake_proof,
};
use mission_supervisor::single_instance::{current_user_sid, production_pipe_name};
use serde::Serialize;
use windows_sys::Win32::Foundation::{CloseHandle, ERROR_NOT_FOUND, HANDLE};
use windows_sys::Win32::System::IO::CancelSynchronousIo;
use windows_sys::Win32::System::Threading::{
    CREATE_NO_WINDOW, GetCurrentThreadId, OpenThread, THREAD_TERMINATE,
};

const HANDSHAKE_IO_TIMEOUT: Duration = Duration::from_secs(5);
const PING_IO_TIMEOUT: Duration = Duration::from_secs(1);
const COMMAND_IO_TIMEOUT: Duration = Duration::from_secs(5);
const LONG_COMMAND_IO_TIMEOUT: Duration = Duration::from_secs(15);
const CANCELLATION_POLL_INTERVAL: Duration = Duration::from_millis(10);

pub(crate) fn command_io_timeout(command: &str) -> Duration {
    match command {
        "launch_route"
        | "build_recovery_package"
        | "verify_recovery"
        | "resolve_recovery"
        | "handoff_provider" => LONG_COMMAND_IO_TIMEOUT,
        _ => COMMAND_IO_TIMEOUT,
    }
}

macro_rules! supervisor_commands {
    ($callback:ident) => {
        $callback!(supervisor_status, ping_supervisor)
    };
}
#[allow(unused_imports)]
pub(crate) use supervisor_commands;

macro_rules! command_names {
    ($($command:ident),+ $(,)?) => {
        [$(stringify!($command)),+]
    };
}

pub const ALLOWED_COMMANDS: [&str; 2] = supervisor_commands!(command_names);

macro_rules! mission_commands {
    ($callback:ident) => {
        $callback!(
            create_mission,
            update_mission_contract,
            launch_route,
            subscribe_mission,
            request_safe_pause,
            request_force_termination,
            force_terminate,
            resolve_approval,
            build_recovery_package,
            verify_recovery,
            resolve_recovery,
            review_memory,
            handoff_provider,
            provider_capabilities,
            storage_preview,
            export_preview,
            diagnostic_preview,
            archive_mission,
            delete_mission,
            materialize_export
        )
    };
}
#[allow(unused_imports)]
pub(crate) use mission_commands;
#[allow(dead_code)]
pub const MISSION_ALLOWED_COMMANDS: [&str; 20] = mission_commands!(command_names);

#[allow(unused_imports)]
pub use mission_supervisor::mission_service::{MissionCommandRequest, MissionCommandResult};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum BridgeError {
    Authentication,
    Protocol,
    Timeout,
    Unavailable,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PublicSupervisorStatus {
    pub connection: &'static str,
    pub version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_code: Option<&'static str>,
}

pub trait SupervisorTransport {
    fn ping(&mut self) -> Result<String, BridgeError>;

    #[allow(dead_code)]
    fn command(
        &mut self,
        _command: &str,
        _request: serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        Err("SUPERVISOR_UNAVAILABLE".to_owned())
    }

    fn start_packaged(&mut self) -> Result<(), BridgeError> {
        Err(BridgeError::Unavailable)
    }
}

pub struct LocalSupervisorTransport<P = WindowsCredentialInstallSecret> {
    pipe_name: String,
    pipe_path: String,
    secret_provider: P,
    session: Option<File>,
    supervisor_path: Option<PathBuf>,
    data_dir: Option<PathBuf>,
    instance_scope: Option<String>,
    child: Option<Child>,
}

impl LocalSupervisorTransport<WindowsCredentialInstallSecret> {
    #[cfg_attr(test, allow(dead_code))]
    pub fn production(data_dir: PathBuf) -> io::Result<Self> {
        let current_exe = std::env::current_exe()?;
        let install_dir = current_exe.parent().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                "desktop install directory is missing",
            )
        })?;
        #[cfg(debug_assertions)]
        let pipe_name = std::env::var("MISSION_PIPE_NAME")
            .ok()
            .filter(|value| !value.is_empty())
            .unwrap_or(production_pipe_name(&current_user_sid()?));
        #[cfg(not(debug_assertions))]
        let pipe_name = production_pipe_name(&current_user_sid()?);
        #[cfg(debug_assertions)]
        let instance_scope = std::env::var("MISSION_INSTANCE_SCOPE")
            .ok()
            .filter(|value| !value.is_empty());
        #[cfg(not(debug_assertions))]
        let instance_scope = None;
        Ok(Self {
            pipe_name: pipe_name.clone(),
            pipe_path: format!(r"\\.\pipe\{pipe_name}"),
            secret_provider: WindowsCredentialInstallSecret::default(),
            session: None,
            supervisor_path: Some(install_dir.join("mission-control-supervisor.exe")),
            data_dir: Some(data_dir),
            instance_scope,
            child: None,
        })
    }
}

impl<P> LocalSupervisorTransport<P> {
    #[cfg(test)]
    #[allow(dead_code)]
    pub fn for_test(pipe_name: &str, secret_provider: P) -> Self {
        Self {
            pipe_name: pipe_name.to_owned(),
            pipe_path: format!(r"\\.\pipe\{pipe_name}"),
            secret_provider,
            session: None,
            supervisor_path: None,
            data_dir: None,
            instance_scope: None,
            child: None,
        }
    }
}

impl<P: InstallSecretProvider> SupervisorTransport for LocalSupervisorTransport<P> {
    fn ping(&mut self) -> Result<String, BridgeError> {
        if self.session.is_none() {
            self.session = Some(self.connect_and_authenticate()?);
        }

        let result = ping_session(self.session.as_mut().expect("session was established"));
        if result.is_err() {
            self.session = None;
        }
        result
    }

    fn start_packaged(&mut self) -> Result<(), BridgeError> {
        if let Some(child) = &mut self.child
            && child
                .try_wait()
                .map_err(|_| BridgeError::Unavailable)?
                .is_none()
        {
            return Ok(());
        }
        let supervisor_path = self
            .supervisor_path
            .as_ref()
            .filter(|path| path.is_file())
            .ok_or(BridgeError::Unavailable)?;
        let data_dir = self.data_dir.as_ref().ok_or(BridgeError::Unavailable)?;
        let child = packaged_supervisor_command(
            supervisor_path,
            data_dir,
            &self.pipe_name,
            self.instance_scope.as_deref(),
        )
        .spawn()
        .map_err(|_| BridgeError::Unavailable)?;
        self.child = Some(child);
        Ok(())
    }

    fn command(
        &mut self,
        command: &str,
        request: serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        if self.session.is_none() {
            self.session = Some(
                self.connect_and_authenticate()
                    .map_err(|error| match error {
                        BridgeError::Timeout => "SUPERVISOR_HANDSHAKE_TIMEOUT".to_owned(),
                        BridgeError::Authentication => "SUPERVISOR_AUTH_FAILED".to_owned(),
                        BridgeError::Protocol => "SUPERVISOR_PROTOCOL_ERROR".to_owned(),
                        BridgeError::Unavailable => "SUPERVISOR_UNAVAILABLE".to_owned(),
                    })?,
            );
        }
        let pipe = self.session.as_mut().expect("session was established");
        let message = ClientMessage::Command(CommandRequest {
            command: command.to_owned(),
            request,
        });
        let result = with_io_deadline(command_io_timeout(command), || {
            write_frame(&mut *pipe, &message).map_err(|_| BridgeError::Unavailable)?;
            read_frame(pipe).map_err(|_| BridgeError::Unavailable)
        });
        match result {
            Ok(ServerMessage::Command(response)) if response.command == command => {
                response.result.ok_or_else(|| {
                    response
                        .error
                        .unwrap_or_else(|| "SUPERVISOR_COMMAND_FAILED".to_owned())
                })
            }
            Ok(_) => Err("SUPERVISOR_PROTOCOL_ERROR".to_owned()),
            Err(BridgeError::Timeout) => {
                self.session = None;
                Err("SUPERVISOR_COMMAND_TIMEOUT".to_owned())
            }
            Err(_) => {
                self.session = None;
                Err("SUPERVISOR_UNAVAILABLE".to_owned())
            }
        }
    }
}

pub(crate) fn packaged_supervisor_command(
    supervisor_path: &Path,
    data_dir: &Path,
    pipe_name: &str,
    instance_scope: Option<&str>,
) -> Command {
    let mut command = Command::new(supervisor_path);
    command
        .arg("--data-dir")
        .arg(data_dir)
        .arg("--pipe-name")
        .arg(pipe_name);
    if let Some(instance_scope) = instance_scope {
        command.arg("--instance-scope").arg(instance_scope);
    }
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .creation_flags(CREATE_NO_WINDOW);
    command
}

impl<P: InstallSecretProvider> LocalSupervisorTransport<P> {
    fn connect_and_authenticate(&self) -> Result<File, BridgeError> {
        let mut pipe = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&self.pipe_path)
            .map_err(|_| BridgeError::Unavailable)?;
        let secret = self
            .secret_provider
            .install_secret()
            .map_err(|_| BridgeError::Authentication)?;
        let nonce = secure_random::<NONCE_BYTES>()?.to_vec();
        let versions = vec![PROTOCOL_VERSION];
        let proof = handshake_proof(&secret, PRODUCT_INSTALL_ID, &nonce, &versions)
            .map_err(|_| BridgeError::Authentication)?;
        let handshake = ClientMessage::Handshake(Handshake {
            install_id: PRODUCT_INSTALL_ID.to_owned(),
            protocol_versions: versions,
            nonce,
            proof,
        });
        with_io_deadline(HANDSHAKE_IO_TIMEOUT, || {
            write_frame(&mut pipe, &handshake).map_err(|_| BridgeError::Unavailable)?;
            match read_frame(&mut pipe).map_err(|_| BridgeError::Unavailable)? {
                ServerMessage::HandshakeAccepted(accepted)
                    if accepted.protocol_version == PROTOCOL_VERSION =>
                {
                    Ok(())
                }
                ServerMessage::Error(error) => Err(protocol_error(error.code)),
                _ => Err(BridgeError::Protocol),
            }
        })?;
        Ok(pipe)
    }
}

impl From<io::Error> for BridgeError {
    fn from(_: io::Error) -> Self {
        Self::Unavailable
    }
}

fn ping_session(pipe: &mut File) -> Result<String, BridgeError> {
    with_io_deadline(PING_IO_TIMEOUT, || {
        write_frame(&mut *pipe, &ClientMessage::Ping).map_err(|_| BridgeError::Unavailable)?;
        match read_frame(pipe).map_err(|_| BridgeError::Unavailable)? {
            ServerMessage::Pong(pong) if pong.protocol_version == PROTOCOL_VERSION => {
                Ok(pong.supervisor_version)
            }
            ServerMessage::Error(error) => Err(protocol_error(error.code)),
            _ => Err(BridgeError::Protocol),
        }
    })
}

struct CancellationTarget(HANDLE);

unsafe impl Send for CancellationTarget {}

impl CancellationTarget {
    fn current() -> io::Result<Self> {
        let handle = unsafe { OpenThread(THREAD_TERMINATE, 0, GetCurrentThreadId()) };
        if handle.is_null() {
            return Err(io::Error::last_os_error());
        }
        Ok(Self(handle))
    }

    fn cancel(&self) -> io::Result<()> {
        if unsafe { CancelSynchronousIo(self.0) } != 0 {
            return Ok(());
        }
        let error = io::Error::last_os_error();
        if error.raw_os_error() == Some(ERROR_NOT_FOUND as i32) {
            return Ok(());
        }
        Err(error)
    }
}

impl Drop for CancellationTarget {
    fn drop(&mut self) {
        unsafe { CloseHandle(self.0) };
    }
}

pub(crate) fn with_io_deadline<T>(
    timeout: Duration,
    operation: impl FnOnce() -> Result<T, BridgeError>,
) -> Result<T, BridgeError> {
    let target = CancellationTarget::current()?;
    let (done_tx, done_rx) = mpsc::channel();
    let timed_out = Arc::new(AtomicBool::new(false));
    let watchdog_timed_out = Arc::clone(&timed_out);
    let watchdog = thread::Builder::new()
        .name("mission-desktop-ipc-watchdog".to_owned())
        .spawn(move || -> io::Result<()> {
            match done_rx.recv_timeout(timeout) {
                Ok(()) | Err(mpsc::RecvTimeoutError::Disconnected) => return Ok(()),
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    watchdog_timed_out.store(true, Ordering::Release);
                }
            }

            let mut cancellation_error = None;
            loop {
                if let Err(error) = target.cancel()
                    && cancellation_error.is_none()
                {
                    cancellation_error = Some(error);
                }
                match done_rx.recv_timeout(CANCELLATION_POLL_INTERVAL) {
                    Ok(()) | Err(mpsc::RecvTimeoutError::Disconnected) => {
                        return cancellation_error.map_or(Ok(()), Err);
                    }
                    Err(mpsc::RecvTimeoutError::Timeout) => {}
                }
            }
        })
        .map_err(|_| BridgeError::Unavailable)?;
    let result = operation();
    let _ = done_tx.send(());
    let watchdog_result = watchdog
        .join()
        .map_err(|_| BridgeError::Unavailable)?
        .map_err(|_| BridgeError::Unavailable);
    if timed_out.load(Ordering::Acquire) {
        return Err(BridgeError::Timeout);
    }
    watchdog_result?;
    result
}

fn protocol_error(code: ProtocolErrorCode) -> BridgeError {
    match code {
        ProtocolErrorCode::AuthFailed | ProtocolErrorCode::ReplayedNonce => {
            BridgeError::Authentication
        }
        ProtocolErrorCode::IncompatibleProtocol => BridgeError::Protocol,
        ProtocolErrorCode::FrameTooLarge | ProtocolErrorCode::InvalidFrame => {
            BridgeError::Unavailable
        }
    }
}

pub struct SupervisorBridge<T> {
    inner: Mutex<BridgeInner<T>>,
}

struct BridgeInner<T> {
    transport: T,
    launch_attempted: bool,
}

impl<T: SupervisorTransport> SupervisorBridge<T> {
    pub const fn new(transport: T) -> Self {
        Self {
            inner: Mutex::new(BridgeInner {
                transport,
                launch_attempted: false,
            }),
        }
    }

    pub fn supervisor_status(&self) -> PublicSupervisorStatus {
        let mut inner = self.inner.lock().expect("supervisor bridge mutex poisoned");
        let first = inner.transport.ping();
        if matches!(first, Err(BridgeError::Unavailable)) && !inner.launch_attempted {
            inner.launch_attempted = true;
            if let Err(error) = inner.transport.start_packaged() {
                return public_status(Err(error));
            }
            for attempt in 0..10 {
                if attempt != 0 {
                    thread::sleep(Duration::from_millis(100));
                }
                let result = inner.transport.ping();
                if !matches!(result, Err(BridgeError::Unavailable)) || attempt == 9 {
                    return public_status(result);
                }
            }
        }
        public_status(first)
    }

    pub fn ping_supervisor(&self) -> PublicSupervisorStatus {
        let result = self
            .inner
            .lock()
            .expect("supervisor bridge mutex poisoned")
            .transport
            .ping();
        public_status(result)
    }

    #[allow(dead_code)]
    pub fn dispatch_mission(
        &self,
        command: &str,
        request: serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        if !MISSION_ALLOWED_COMMANDS.contains(&command) {
            return Err("COMMAND_NOT_ALLOWED".to_owned());
        }
        self.inner
            .lock()
            .map_err(|_| "SUPERVISOR_BRIDGE_POISONED".to_owned())?
            .transport
            .command(command, request)
    }
}

impl<T: SupervisorTransport + Send + 'static> SupervisorBridge<T> {
    pub async fn supervisor_status_async(self: Arc<Self>) -> PublicSupervisorStatus {
        tauri::async_runtime::spawn_blocking(move || self.supervisor_status())
            .await
            .expect("supervisor status worker panicked")
    }

    pub async fn ping_supervisor_async(self: Arc<Self>) -> PublicSupervisorStatus {
        tauri::async_runtime::spawn_blocking(move || self.ping_supervisor())
            .await
            .expect("supervisor ping worker panicked")
    }

    #[allow(dead_code)]
    pub async fn dispatch_mission_async(
        self: Arc<Self>,
        command: &'static str,
        request: serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        tauri::async_runtime::spawn_blocking(move || self.dispatch_mission(command, request))
            .await
            .map_err(|_| "SUPERVISOR_COMMAND_WORKER_FAILED".to_owned())?
    }
}

fn public_status(result: Result<String, BridgeError>) -> PublicSupervisorStatus {
    match result {
        Ok(version) => PublicSupervisorStatus {
            connection: "connected",
            version: Some(version),
            error_code: None,
        },
        Err(BridgeError::Authentication) => PublicSupervisorStatus {
            connection: "disconnected",
            version: None,
            error_code: Some("SUPERVISOR_AUTH_FAILED"),
        },
        Err(BridgeError::Protocol) => PublicSupervisorStatus {
            connection: "disconnected",
            version: None,
            error_code: Some("SUPERVISOR_PROTOCOL_INCOMPATIBLE"),
        },
        Err(BridgeError::Timeout) => PublicSupervisorStatus {
            connection: "degraded",
            version: None,
            error_code: Some("SUPERVISOR_TIMEOUT"),
        },
        Err(BridgeError::Unavailable) => PublicSupervisorStatus {
            connection: "disconnected",
            version: None,
            error_code: None,
        },
    }
}
