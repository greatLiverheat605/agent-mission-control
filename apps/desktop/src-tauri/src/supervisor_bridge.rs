use std::fs::{File, OpenOptions};
use std::io;
use std::os::windows::process::CommandExt;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
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
use windows_sys::Win32::System::Threading::CREATE_NO_WINDOW;

pub const ALLOWED_COMMANDS: [&str; 2] = ["supervisor_status", "ping_supervisor"];

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
        let child = Command::new(supervisor_path)
            .arg("--data-dir")
            .arg(data_dir)
            .arg("--parent-pid")
            .arg(std::process::id().to_string())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .creation_flags(CREATE_NO_WINDOW)
            .spawn()
            .map_err(|_| BridgeError::Unavailable)?;
        self.child = Some(child);
        Ok(())
    }
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
        write_frame(&mut pipe, &handshake).map_err(|_| BridgeError::Unavailable)?;
        match read_frame(&mut pipe).map_err(|_| BridgeError::Unavailable)? {
            ServerMessage::HandshakeAccepted(accepted)
                if accepted.protocol_version == PROTOCOL_VERSION =>
            {
                Ok(pipe)
            }
            ServerMessage::Error(error) => Err(protocol_error(error.code)),
            _ => Err(BridgeError::Protocol),
        }
    }
}

impl From<io::Error> for BridgeError {
    fn from(_: io::Error) -> Self {
        Self::Unavailable
    }
}

fn ping_session(pipe: &mut File) -> Result<String, BridgeError> {
    write_frame(&mut *pipe, &ClientMessage::Ping).map_err(|_| BridgeError::Unavailable)?;
    match read_frame(pipe).map_err(|_| BridgeError::Unavailable)? {
        ServerMessage::Pong(pong) if pong.protocol_version == PROTOCOL_VERSION => {
            Ok(pong.supervisor_version)
        }
        ServerMessage::Error(error) => Err(protocol_error(error.code)),
        _ => Err(BridgeError::Protocol),
    }
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
