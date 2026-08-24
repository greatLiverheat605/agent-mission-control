use std::fs::{File, OpenOptions};
use std::io;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex, mpsc};
use std::thread;
use std::time::Duration;

use mission_protocol::credential::{WindowsCredentialInstallSecret, secure_random};
use mission_protocol::frame::{read_frame, write_frame};
use mission_protocol::handshake::{
    ClientMessage, Handshake, InstallSecretProvider, NONCE_BYTES, PRODUCT_INSTALL_ID,
    PROTOCOL_VERSION, ProtocolErrorCode, ServerMessage, handshake_proof,
};
use mission_supervisor::single_instance::{current_user_sid, production_pipe_name};
use serde::Serialize;
use windows_sys::Win32::Foundation::{CloseHandle, ERROR_NOT_FOUND, HANDLE};
use windows_sys::Win32::System::IO::CancelSynchronousIo;
use windows_sys::Win32::System::Threading::{
    CREATE_NO_WINDOW, GetCurrentThreadId, OpenThread, THREAD_TERMINATE,
};

const CLIENT_IO_TIMEOUT: Duration = Duration::from_secs(1);
const CANCELLATION_POLL_INTERVAL: Duration = Duration::from_millis(10);

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
            force_terminate
        )
    };
}
#[allow(unused_imports)]
pub(crate) use mission_commands;
#[allow(dead_code)]
pub const MISSION_ALLOWED_COMMANDS: [&str; 6] = mission_commands!(command_names);

#[allow(dead_code)]
#[derive(Clone, Debug, Eq, PartialEq, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MissionCommandRequest {
    pub mission_id: Option<String>,
    pub route_id: Option<String>,
    pub expected_version: Option<u64>,
    pub project_root: Option<String>,
    pub goal: Option<String>,
    pub reason: Option<String>,
    pub confirmation_token: Option<String>,
}

#[allow(dead_code)]
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MissionCommandResult {
    pub accepted: bool,
    pub mission_id: Option<String>,
    pub sequence: Option<u64>,
    pub error_code: Option<&'static str>,
}

#[allow(dead_code)]
pub fn validate_mission_request(request: &MissionCommandRequest) -> Result<(), &'static str> {
    if request
        .project_root
        .as_ref()
        .is_some_and(|value| value.len() > 4096)
    {
        return Err("PROJECT_ROOT_TOO_LONG");
    }
    if request
        .goal
        .as_ref()
        .is_some_and(|value| value.len() > 32_000)
    {
        return Err("GOAL_TOO_LONG");
    }
    if request
        .confirmation_token
        .as_ref()
        .is_some_and(|value| value.len() > 256)
    {
        return Err("CONFIRMATION_TOKEN_TOO_LONG");
    }
    Ok(())
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum BridgeError {
    Authentication,
    Protocol,
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

    fn start_packaged(&mut self) -> Result<(), BridgeError> {
        Err(BridgeError::Unavailable)
    }
}

pub struct LocalSupervisorTransport<P = WindowsCredentialInstallSecret> {
    pipe_path: String,
    secret_provider: P,
    session: Option<File>,
    supervisor_path: Option<PathBuf>,
    data_dir: Option<PathBuf>,
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
        let pipe_name = production_pipe_name(&current_user_sid()?);
        Ok(Self {
            pipe_path: format!(r"\\.\pipe\{pipe_name}"),
            secret_provider: WindowsCredentialInstallSecret::default(),
            session: None,
            supervisor_path: Some(install_dir.join("mission-control-supervisor.exe")),
            data_dir: Some(data_dir),
            child: None,
        })
    }
}

impl<P> LocalSupervisorTransport<P> {
    #[cfg(test)]
    #[allow(dead_code)]
    pub fn for_test(pipe_name: &str, secret_provider: P) -> Self {
        Self {
            pipe_path: format!(r"\\.\pipe\{pipe_name}"),
            secret_provider,
            session: None,
            supervisor_path: None,
            data_dir: None,
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
        let child = packaged_supervisor_command(supervisor_path, data_dir)
            .spawn()
            .map_err(|_| BridgeError::Unavailable)?;
        self.child = Some(child);
        Ok(())
    }
}

pub(crate) fn packaged_supervisor_command(supervisor_path: &Path, data_dir: &Path) -> Command {
    let mut command = Command::new(supervisor_path);
    command
        .arg("--data-dir")
        .arg(data_dir)
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
        with_io_deadline(CLIENT_IO_TIMEOUT, || {
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
    with_io_deadline(CLIENT_IO_TIMEOUT, || {
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
    let watchdog = thread::Builder::new()
        .name("mission-desktop-ipc-watchdog".to_owned())
        .spawn(move || -> io::Result<()> {
            match done_rx.recv_timeout(timeout) {
                Ok(()) | Err(mpsc::RecvTimeoutError::Disconnected) => return Ok(()),
                Err(mpsc::RecvTimeoutError::Timeout) => {}
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
    watchdog
        .join()
        .map_err(|_| BridgeError::Unavailable)?
        .map_err(|_| BridgeError::Unavailable)?;
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
        Err(BridgeError::Unavailable) => PublicSupervisorStatus {
            connection: "disconnected",
            version: None,
            error_code: None,
        },
    }
}
