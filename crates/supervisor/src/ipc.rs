use std::fs::{File, OpenOptions};
use std::io;
use std::os::windows::io::{AsRawHandle, FromRawHandle};
use std::ptr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use mission_protocol::frame::{FrameError, read_frame, write_frame};
use mission_protocol::handshake::{
    ClientMessage, HandshakeVerifier, InstallSecretProvider, Pong, ProtocolError,
    ProtocolErrorCode, ServerMessage,
};
use windows_sys::Win32::Foundation::{
    ERROR_NO_DATA, ERROR_OPERATION_ABORTED, ERROR_PIPE_CONNECTED, INVALID_HANDLE_VALUE, LocalFree,
};
use windows_sys::Win32::Security::Authorization::{
    ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
};
use windows_sys::Win32::Security::{PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES};
use windows_sys::Win32::Storage::FileSystem::{
    FILE_FLAG_FIRST_PIPE_INSTANCE, FlushFileBuffers, PIPE_ACCESS_DUPLEX,
};
use windows_sys::Win32::System::IO::CancelSynchronousIo;
use windows_sys::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, DisconnectNamedPipe, PIPE_READMODE_BYTE,
    PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_BYTE, PIPE_WAIT,
};

use crate::single_instance::current_user_sid;

const PIPE_BUFFER_BYTES: u32 = 64 * 1024;

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
        let sid = current_user_sid()?;
        let pipe_path = format!(r"\\.\pipe\{pipe_name}");
        let first = PipeInstance::create(&pipe_path, &sid)?;
        let stop = Arc::new(AtomicBool::new(false));
        let thread_stop = Arc::clone(&stop);
        let thread_pipe_path = pipe_path.clone();
        let thread_sid = sid;
        let thread_install_id = install_id.to_owned();
        let server_thread = thread::Builder::new()
            .name("mission-supervisor-ipc".to_owned())
            .spawn(move || {
                run_server(
                    first,
                    &thread_pipe_path,
                    &thread_sid,
                    &thread_install_id,
                    secret_provider,
                    &thread_stop,
                )
            })?;

        Ok(Self {
            stop,
            pipe_path,
            thread: Some(server_thread),
        })
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

fn run_server<P: InstallSecretProvider>(
    first: PipeInstance,
    pipe_path: &str,
    sid: &str,
    install_id: &str,
    secret_provider: P,
    stop: &AtomicBool,
) -> io::Result<()> {
    let mut next = Some(first);
    let mut verifier = HandshakeVerifier::new(install_id, secret_provider);

    while !stop.load(Ordering::SeqCst) {
        let instance = match next.take() {
            Some(instance) => instance,
            None => PipeInstance::create(pipe_path, sid)?,
        };
        let Some(mut pipe) = instance.connect()? else {
            continue;
        };
        if !stop.load(Ordering::SeqCst) {
            serve_connection(&mut pipe, &mut verifier);
        }
        if !stop.load(Ordering::SeqCst) {
            unsafe {
                FlushFileBuffers(pipe.as_raw_handle().cast());
            }
        }
        unsafe {
            DisconnectNamedPipe(pipe.as_raw_handle().cast());
        }
        drop(pipe);
    }

    Ok(())
}

fn serve_connection<P: InstallSecretProvider>(
    pipe: &mut File,
    verifier: &mut HandshakeVerifier<P>,
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
                let response = ServerMessage::Pong(Pong {
                    supervisor_version: env!("CARGO_PKG_VERSION").to_owned(),
                    protocol_version,
                });
                if write_frame(pipe, &response).is_err() {
                    return;
                }
            }
            (None, ClientMessage::Ping) | (Some(_), ClientMessage::Handshake(_)) => {
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
        let security = PipeSecurity::for_user(sid)?;
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
                &security.attributes,
            )
        };
        if handle == INVALID_HANDLE_VALUE {
            return Err(last_error("CreateNamedPipeW"));
        }

        Ok(Self(unsafe { File::from_raw_handle(handle.cast()) }))
    }

    fn connect(self) -> io::Result<Option<File>> {
        let connected = unsafe { ConnectNamedPipe(self.0.as_raw_handle().cast(), ptr::null_mut()) };
        if connected == 0 {
            let error = io::Error::last_os_error();
            if matches!(
                error.raw_os_error(),
                Some(code) if code == ERROR_NO_DATA as i32 || code == ERROR_OPERATION_ABORTED as i32
            ) {
                return Ok(None);
            }
            if error.raw_os_error() != Some(ERROR_PIPE_CONNECTED as i32) {
                return Err(io::Error::new(
                    error.kind(),
                    format!("ConnectNamedPipe failed: {error}"),
                ));
            }
        }
        Ok(Some(self.0))
    }
}

struct PipeSecurity {
    attributes: SECURITY_ATTRIBUTES,
    descriptor: PSECURITY_DESCRIPTOR,
}

impl PipeSecurity {
    fn for_user(sid: &str) -> io::Result<Self> {
        let sddl = pipe_sddl(sid);
        let wide_sddl: Vec<u16> = sddl.encode_utf16().chain(Some(0)).collect();
        let mut descriptor = ptr::null_mut();
        if unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                wide_sddl.as_ptr(),
                SDDL_REVISION_1,
                &mut descriptor,
                ptr::null_mut(),
            )
        } == 0
        {
            return Err(last_error(
                "ConvertStringSecurityDescriptorToSecurityDescriptorW",
            ));
        }

        Ok(Self {
            attributes: SECURITY_ATTRIBUTES {
                nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
                lpSecurityDescriptor: descriptor,
                bInheritHandle: 0,
            },
            descriptor,
        })
    }
}

fn pipe_sddl(sid: &str) -> String {
    format!("D:P(A;;GA;;;{sid})")
}

impl Drop for PipeSecurity {
    fn drop(&mut self) {
        unsafe {
            LocalFree(self.descriptor);
        }
    }
}

fn last_error(operation: &str) -> io::Error {
    let error = io::Error::last_os_error();
    io::Error::new(error.kind(), format!("{operation} failed: {error}"))
}

#[cfg(test)]
mod tests {
    use super::pipe_sddl;

    #[test]
    fn pipe_acl_is_protected_and_only_grants_the_current_user() {
        let sid = "S-1-5-21-111-222-333-1001";

        assert_eq!(pipe_sddl(sid), "D:P(A;;GA;;;S-1-5-21-111-222-333-1001)");
    }
}
