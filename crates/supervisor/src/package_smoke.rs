use std::fs::{File, OpenOptions};
use std::io;
use std::sync::mpsc::{self, RecvTimeoutError};
use std::thread;
use std::time::{Duration, Instant};

use mission_protocol::credential::{WindowsCredentialInstallSecret, secure_random};
use mission_protocol::frame::{FrameError, read_frame, write_frame};
use mission_protocol::handshake::{
    ClientMessage, Handshake, InstallSecretProvider, NONCE_BYTES, PRODUCT_INSTALL_ID,
    PROTOCOL_VERSION, ProtocolErrorCode, ServerMessage, handshake_proof,
};
use windows_sys::Win32::Foundation::{CloseHandle, ERROR_NOT_FOUND, ERROR_PIPE_BUSY, HANDLE};
use windows_sys::Win32::System::IO::CancelSynchronousIo;
use windows_sys::Win32::System::Threading::{GetCurrentThreadId, OpenThread, THREAD_TERMINATE};

const CONNECT_RETRY_INTERVAL: Duration = Duration::from_millis(10);
const CANCELLATION_POLL_INTERVAL: Duration = Duration::from_millis(10);

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PackageSmokeSuccess {
    pub protocol_version: u32,
    pub supervisor_version: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PackageSmokeClientError {
    Timeout,
    Authentication,
    Protocol,
    Unavailable,
    WorkerDidNotStop,
}

pub fn authenticated_ping(
    pipe_name: &str,
    timeout: Duration,
) -> Result<PackageSmokeSuccess, PackageSmokeClientError> {
    authenticated_ping_with_provider(
        pipe_name,
        WindowsCredentialInstallSecret::default(),
        timeout,
    )
}

pub fn authenticated_ping_with_provider<P>(
    pipe_name: &str,
    provider: P,
    timeout: Duration,
) -> Result<PackageSmokeSuccess, PackageSmokeClientError>
where
    P: InstallSecretProvider + Send + 'static,
{
    if pipe_name.is_empty() || pipe_name.contains('\0') || timeout.is_zero() {
        return Err(PackageSmokeClientError::Unavailable);
    }
    let deadline = Instant::now()
        .checked_add(timeout)
        .ok_or(PackageSmokeClientError::Unavailable)?;
    let pipe_name = pipe_name.to_owned();
    let (ready_tx, ready_rx) = mpsc::channel();
    let (done_tx, done_rx) = mpsc::channel();
    let worker = thread::Builder::new()
        .name("mission-package-smoke-client".to_owned())
        .spawn(move || {
            let target = CancellationTarget::current();
            let can_run = target.is_ok();
            let _ = ready_tx.send(target.map_err(|_| PackageSmokeClientError::Unavailable));
            if !can_run {
                return;
            }
            let result = authenticated_ping_worker(&pipe_name, provider, deadline);
            let _ = done_tx.send(result);
        })
        .map_err(|_| PackageSmokeClientError::Unavailable)?;

    let target = match ready_rx.recv_timeout(remaining(deadline)) {
        Ok(Ok(target)) => target,
        Ok(Err(error)) => return join_worker(worker, error),
        Err(RecvTimeoutError::Disconnected) => {
            return join_worker(worker, PackageSmokeClientError::WorkerDidNotStop);
        }
        Err(RecvTimeoutError::Timeout) => {
            return join_worker(worker, PackageSmokeClientError::Timeout);
        }
    };

    match done_rx.recv_timeout(remaining(deadline)) {
        Ok(result) => join_worker_result(worker, result),
        Err(RecvTimeoutError::Disconnected) => {
            join_worker(worker, PackageSmokeClientError::WorkerDidNotStop)
        }
        Err(RecvTimeoutError::Timeout) => {
            while !worker.is_finished() {
                let _ = target.cancel();
                match done_rx.recv_timeout(CANCELLATION_POLL_INTERVAL) {
                    Ok(_) | Err(RecvTimeoutError::Disconnected) => break,
                    Err(RecvTimeoutError::Timeout) => {}
                }
            }
            join_worker(worker, PackageSmokeClientError::Timeout)
        }
    }
}

fn authenticated_ping_worker<P>(
    pipe_name: &str,
    provider: P,
    deadline: Instant,
) -> Result<PackageSmokeSuccess, PackageSmokeClientError>
where
    P: InstallSecretProvider,
{
    let mut secret = provider
        .install_secret()
        .map_err(|_| PackageSmokeClientError::Authentication)?;
    let nonce = secure_random::<NONCE_BYTES>()
        .map_err(|_| PackageSmokeClientError::Authentication)?
        .to_vec();
    let versions = vec![PROTOCOL_VERSION];
    let proof = handshake_proof(&secret, PRODUCT_INSTALL_ID, &nonce, &versions)
        .map_err(|_| PackageSmokeClientError::Authentication)?;
    secret.fill(0);

    let mut pipe = connect_pipe(pipe_name, deadline)?;
    let handshake = ClientMessage::Handshake(Handshake {
        install_id: PRODUCT_INSTALL_ID.to_owned(),
        protocol_versions: versions,
        nonce,
        proof,
    });
    ensure_before_deadline(deadline)?;
    write_frame(&mut pipe, &handshake).map_err(map_write_error)?;
    ensure_before_deadline(deadline)?;
    match read_frame(&mut pipe).map_err(map_read_error)? {
        ServerMessage::HandshakeAccepted(accepted)
            if accepted.protocol_version == PROTOCOL_VERSION => {}
        ServerMessage::Error(error) => return Err(map_protocol_error(error.code)),
        _ => return Err(PackageSmokeClientError::Protocol),
    }

    ensure_before_deadline(deadline)?;
    write_frame(&mut pipe, &ClientMessage::Ping).map_err(map_write_error)?;
    ensure_before_deadline(deadline)?;
    match read_frame(&mut pipe).map_err(map_read_error)? {
        ServerMessage::Pong(pong) if pong.protocol_version == PROTOCOL_VERSION => {
            Ok(PackageSmokeSuccess {
                protocol_version: pong.protocol_version,
                supervisor_version: pong.supervisor_version,
            })
        }
        ServerMessage::Error(error) => Err(map_protocol_error(error.code)),
        _ => Err(PackageSmokeClientError::Protocol),
    }
}

fn connect_pipe(pipe_name: &str, deadline: Instant) -> Result<File, PackageSmokeClientError> {
    let pipe_path = format!(r"\\.\pipe\{pipe_name}");
    loop {
        ensure_before_deadline(deadline)?;
        match OpenOptions::new().read(true).write(true).open(&pipe_path) {
            Ok(pipe) => return Ok(pipe),
            Err(error)
                if error.kind() == io::ErrorKind::NotFound
                    || error.raw_os_error() == Some(ERROR_PIPE_BUSY as i32) =>
            {
                thread::sleep(remaining(deadline).min(CONNECT_RETRY_INTERVAL));
            }
            Err(_) => return Err(PackageSmokeClientError::Unavailable),
        }
    }
}

fn ensure_before_deadline(deadline: Instant) -> Result<(), PackageSmokeClientError> {
    if Instant::now() >= deadline {
        Err(PackageSmokeClientError::Timeout)
    } else {
        Ok(())
    }
}

fn remaining(deadline: Instant) -> Duration {
    deadline.saturating_duration_since(Instant::now())
}

fn map_write_error(_: FrameError) -> PackageSmokeClientError {
    PackageSmokeClientError::Unavailable
}

fn map_read_error(error: FrameError) -> PackageSmokeClientError {
    match error {
        FrameError::FrameTooLarge | FrameError::InvalidUtf8 | FrameError::InvalidJson => {
            PackageSmokeClientError::Protocol
        }
        FrameError::Io(_) => PackageSmokeClientError::Unavailable,
    }
}

fn map_protocol_error(code: ProtocolErrorCode) -> PackageSmokeClientError {
    match code {
        ProtocolErrorCode::AuthFailed | ProtocolErrorCode::ReplayedNonce => {
            PackageSmokeClientError::Authentication
        }
        ProtocolErrorCode::FrameTooLarge
        | ProtocolErrorCode::IncompatibleProtocol
        | ProtocolErrorCode::InvalidFrame => PackageSmokeClientError::Protocol,
    }
}

fn join_worker(
    worker: thread::JoinHandle<()>,
    result: PackageSmokeClientError,
) -> Result<PackageSmokeSuccess, PackageSmokeClientError> {
    worker
        .join()
        .map_err(|_| PackageSmokeClientError::WorkerDidNotStop)?;
    Err(result)
}

fn join_worker_result(
    worker: thread::JoinHandle<()>,
    result: Result<PackageSmokeSuccess, PackageSmokeClientError>,
) -> Result<PackageSmokeSuccess, PackageSmokeClientError> {
    worker
        .join()
        .map_err(|_| PackageSmokeClientError::WorkerDidNotStop)?;
    result
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
