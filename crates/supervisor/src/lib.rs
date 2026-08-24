#[cfg(windows)]
pub mod ipc;
pub mod single_instance;

use std::ffi::OsString;
use std::fs;
use std::io::{self, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::thread;
use std::time::Duration;

use serde_json::json;
use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, WAIT_TIMEOUT};
use windows_sys::Win32::System::Console::{CTRL_BREAK_EVENT, CTRL_C_EVENT, SetConsoleCtrlHandler};
use windows_sys::Win32::System::Threading::{
    OpenProcess, PROCESS_SYNCHRONIZE, WaitForSingleObject,
};

use mission_protocol::credential::WindowsCredentialInstallSecret;
use mission_protocol::handshake::PRODUCT_INSTALL_ID;

use ipc::IpcServer;
use single_instance::{AcquireResult, SingleInstance, current_user_sid, production_pipe_name};

static RUNNING: AtomicBool = AtomicBool::new(true);
static READY_NONCE: AtomicU64 = AtomicU64::new(0);
const READY_TEMP_CREATE_ATTEMPTS: usize = 16;

#[derive(Debug)]
pub enum RunError {
    AlreadyRunning,
    InvalidArguments,
    Io(std::io::Error),
}

impl RunError {
    pub const fn code(&self) -> &'static str {
        match self {
            Self::AlreadyRunning => "already_running",
            Self::InvalidArguments => "invalid_arguments",
            Self::Io(_) => "io_error",
        }
    }

    pub const fn exit_code(&self) -> u8 {
        match self {
            Self::AlreadyRunning => 23,
            Self::InvalidArguments | Self::Io(_) => 2,
        }
    }
}

impl From<std::io::Error> for RunError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

struct Config {
    pipe_name: Option<String>,
    data_dir: PathBuf,
    parent_pid: Option<u32>,
    #[cfg(debug_assertions)]
    credential_target: Option<String>,
}

fn parse_args(args: impl IntoIterator<Item = OsString>) -> Result<Config, RunError> {
    let mut args = args.into_iter();
    let mut pipe_name = None;
    let mut data_dir = None;
    let mut parent_pid = None;
    #[cfg(debug_assertions)]
    let mut credential_target = None;

    while let Some(argument) = args.next() {
        match argument.to_str() {
            Some("--pipe-name") => {
                pipe_name = Some(
                    args.next()
                        .and_then(|value| value.into_string().ok())
                        .filter(|value| !value.is_empty())
                        .ok_or_else(invalid)?,
                );
            }
            Some("--data-dir") => {
                data_dir = Some(PathBuf::from(args.next().ok_or_else(invalid)?));
            }
            Some("--parent-pid") => {
                parent_pid = args
                    .next()
                    .and_then(|value| value.into_string().ok())
                    .and_then(|value| value.parse::<u32>().ok())
                    .filter(|value| *value != 0);
                if parent_pid.is_none() {
                    return Err(invalid());
                }
            }
            Some("--log-level") => {
                // Phase 1 validates the level but does not retain it until log filtering exists.
                let value = args.next().and_then(|value| value.into_string().ok());
                if !matches!(
                    value.as_deref(),
                    Some("trace" | "debug" | "info" | "warn" | "error")
                ) {
                    return Err(invalid());
                }
            }
            #[cfg(debug_assertions)]
            Some("--credential-target") => {
                credential_target = Some(
                    args.next()
                        .and_then(|value| value.into_string().ok())
                        .filter(|value| !value.is_empty())
                        .ok_or_else(invalid)?,
                );
            }
            Some(_) | None => return Err(invalid()),
        }
    }

    Ok(Config {
        pipe_name,
        data_dir: data_dir.ok_or_else(invalid)?,
        parent_pid,
        #[cfg(debug_assertions)]
        credential_target,
    })
}

fn invalid() -> RunError {
    RunError::InvalidArguments
}

unsafe extern "system" fn console_handler(event: u32) -> i32 {
    if matches!(event, CTRL_C_EVENT | CTRL_BREAK_EVENT) {
        RUNNING.store(false, Ordering::SeqCst);
        1
    } else {
        0
    }
}

struct ConsoleHandler;

impl ConsoleHandler {
    fn install() -> io::Result<Self> {
        RUNNING.store(true, Ordering::SeqCst);
        if unsafe { SetConsoleCtrlHandler(Some(console_handler), 1) } == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(Self)
    }
}

impl Drop for ConsoleHandler {
    fn drop(&mut self) {
        unsafe {
            SetConsoleCtrlHandler(Some(console_handler), 0);
        }
    }
}

struct ParentProcess(HANDLE);

impl ParentProcess {
    fn open(pid: u32) -> io::Result<Self> {
        let handle = unsafe { OpenProcess(PROCESS_SYNCHRONIZE, 0, pid) };
        if handle.is_null() {
            return Err(io::Error::last_os_error());
        }
        Ok(Self(handle))
    }

    fn is_running(&self) -> bool {
        (unsafe { WaitForSingleObject(self.0, 0) }) == WAIT_TIMEOUT
    }
}

impl Drop for ParentProcess {
    fn drop(&mut self) {
        unsafe { CloseHandle(self.0) };
    }
}

fn write_stopped_log(writer: &mut impl Write, pid: u32, pipe_name: &str) -> io::Result<()> {
    serde_json::to_writer(
        &mut *writer,
        &json!({ "event": "supervisor.stopped", "pid": pid, "pipe": pipe_name }),
    )?;
    writeln!(writer)
}

fn resolve_pipe_name(pipe_name: Option<String>, sid: &str) -> String {
    pipe_name.unwrap_or_else(|| production_pipe_name(sid))
}

fn remove_file_if_exists(path: &std::path::Path) -> io::Result<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

fn write_ready_file(data_dir: &std::path::Path, pid: u32, pipe_name: &str) -> io::Result<()> {
    let ready_path = data_dir.join("supervisor.ready");
    remove_file_if_exists(&ready_path)?;

    let ready =
        serde_json::to_vec(&json!({ "pid": pid, "pipe": pipe_name })).map_err(io::Error::other)?;
    let mut created_temp = None;
    for _ in 0..READY_TEMP_CREATE_ATTEMPTS {
        let temp_path = data_dir.join(format!(
            "supervisor.ready.{pid}.{}.tmp",
            READY_NONCE.fetch_add(1, Ordering::Relaxed)
        ));
        match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp_path)
        {
            Ok(temp) => {
                created_temp = Some((temp_path, temp));
                break;
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    }
    let (temp_path, mut temp) = created_temp.ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::AlreadyExists,
            "ready temp create attempts exhausted",
        )
    })?;
    let publish = (|| {
        temp.write_all(&ready)?;
        temp.flush()?;
        temp.sync_all()?;
        drop(temp);
        fs::rename(&temp_path, &ready_path)
    })();
    if publish.is_err() {
        let _ = remove_file_if_exists(&temp_path);
    }
    publish
}

struct ReadyFile(Option<PathBuf>);

impl ReadyFile {
    fn publish(data_dir: &std::path::Path, pid: u32, pipe_name: &str) -> io::Result<Self> {
        write_ready_file(data_dir, pid, pipe_name)?;
        Ok(Self(Some(data_dir.join("supervisor.ready"))))
    }

    fn cleanup(mut self) -> io::Result<()> {
        remove_file_if_exists(self.0.as_ref().expect("ready path is armed"))?;
        self.0 = None;
        Ok(())
    }
}

impl Drop for ReadyFile {
    fn drop(&mut self) {
        if let Some(path) = &self.0 {
            let _ = remove_file_if_exists(path);
        }
    }
}

pub fn run(args: impl IntoIterator<Item = OsString>) -> Result<(), RunError> {
    let config = parse_args(args)?;
    let sid = current_user_sid()?;
    let pipe_name = resolve_pipe_name(config.pipe_name, &sid);
    let _instance = match SingleInstance::acquire(&sid)? {
        AcquireResult::Acquired(instance) => instance,
        AcquireResult::AlreadyRunning => return Err(RunError::AlreadyRunning),
    };
    let _console_handler = ConsoleHandler::install()?;
    let parent = config.parent_pid.map(ParentProcess::open).transpose()?;

    fs::create_dir_all(&config.data_dir)?;
    let pid = std::process::id();
    #[cfg(debug_assertions)]
    let secret_provider = config
        .credential_target
        .map(WindowsCredentialInstallSecret::for_test_target)
        .transpose()?
        .unwrap_or_default();
    #[cfg(not(debug_assertions))]
    let secret_provider = WindowsCredentialInstallSecret::default();
    let ipc_server = IpcServer::spawn(&pipe_name, PRODUCT_INSTALL_ID, secret_provider)?;
    let ready_file = ReadyFile::publish(&config.data_dir, pid, &pipe_name)?;
    let mut stdout = io::stdout().lock();
    serde_json::to_writer(
        &mut stdout,
        &json!({ "event": "supervisor.ready", "pid": pid, "pipe": pipe_name }),
    )
    .map_err(io::Error::other)?;
    writeln!(stdout)?;

    while RUNNING.load(Ordering::SeqCst)
        && ipc_server.is_running()
        && parent.as_ref().is_none_or(ParentProcess::is_running)
    {
        thread::sleep(Duration::from_millis(50));
    }

    let shutdown = ipc_server.shutdown();
    let stopped = write_stopped_log(&mut stdout, pid, &pipe_name);
    let cleanup = ready_file.cleanup();
    shutdown?;
    stopped?;
    cleanup?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::{
        READY_NONCE, remove_file_if_exists, resolve_pipe_name, write_ready_file, write_stopped_log,
    };

    static TEST_NONCE: AtomicU64 = AtomicU64::new(0);
    static READY_TEST_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn stale_ready_is_replaced_without_leaving_a_temp_file() {
        let _serial = READY_TEST_LOCK.lock().expect("lock ready tests");
        let data_dir = std::env::temp_dir().join(format!(
            "mission-supervisor-ready-unit-{}-{}",
            std::process::id(),
            TEST_NONCE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&data_dir).expect("create test data dir");
        fs::write(data_dir.join("supervisor.ready"), b"{malformed").expect("seed malformed ready");

        write_ready_file(&data_dir, 42, "test-pipe").expect("replace stale ready");

        let ready: serde_json::Value = serde_json::from_slice(
            &fs::read(data_dir.join("supervisor.ready")).expect("read ready"),
        )
        .expect("ready is valid JSON");
        assert_eq!(ready, serde_json::json!({ "pid": 42, "pipe": "test-pipe" }));
        let entries: Vec<_> = fs::read_dir(&data_dir)
            .expect("read test data dir")
            .map(|entry| entry.expect("read entry").file_name())
            .collect();
        assert_eq!(entries, ["supervisor.ready"]);

        fs::remove_file(data_dir.join("supervisor.ready")).expect("remove ready");
        remove_file_if_exists(&data_dir.join("supervisor.ready"))
            .expect("missing ready cleanup succeeds");

        fs::remove_dir_all(data_dir).expect("remove test data dir");
    }

    #[test]
    fn stale_temp_nonce_collision_does_not_block_ready_publish() {
        let _serial = READY_TEST_LOCK.lock().expect("lock ready tests");
        READY_NONCE.store(0, Ordering::Relaxed);
        let data_dir = std::env::temp_dir().join(format!(
            "mission-supervisor-ready-collision-{}-{}",
            std::process::id(),
            TEST_NONCE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&data_dir).expect("create test data dir");
        fs::write(data_dir.join("supervisor.ready.42.0.tmp"), b"stale").expect("seed stale temp");

        let publish = write_ready_file(&data_dir, 42, "test-pipe");
        if let Err(error) = publish {
            fs::remove_dir_all(&data_dir).expect("remove test data dir after publish failure");
            panic!("stale temp blocked ready publish: {error}");
        }
        let ready: serde_json::Value = serde_json::from_slice(
            &fs::read(data_dir.join("supervisor.ready")).expect("read ready"),
        )
        .expect("ready is valid JSON");
        assert_eq!(ready["pid"], 42);

        fs::remove_file(data_dir.join("supervisor.ready")).expect("remove published ready");
        fs::remove_file(data_dir.join("supervisor.ready.42.0.tmp"))
            .expect("remove seeded stale temp");
        fs::remove_dir(data_dir).expect("remove empty test data dir");
    }

    #[test]
    fn temp_nonce_retry_returns_error_after_a_finite_limit() {
        const EXPECTED_ATTEMPTS: u64 = 16;

        let _serial = READY_TEST_LOCK.lock().expect("lock ready tests");
        READY_NONCE.store(0, Ordering::Relaxed);
        let data_dir = std::env::temp_dir().join(format!(
            "mission-supervisor-ready-exhausted-{}-{}",
            std::process::id(),
            TEST_NONCE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&data_dir).expect("create test data dir");
        let stale_paths: Vec<_> = (0..EXPECTED_ATTEMPTS)
            .map(|nonce| data_dir.join(format!("supervisor.ready.42.{nonce}.tmp")))
            .collect();
        for path in &stale_paths {
            fs::write(path, b"stale").expect("seed stale temp");
        }

        let error_kind = write_ready_file(&data_dir, 42, "test-pipe")
            .err()
            .map(|error| error.kind());

        let ready_path = data_dir.join("supervisor.ready");
        if ready_path.exists() {
            fs::remove_file(ready_path).expect("remove unexpectedly published ready");
        }
        for path in stale_paths {
            fs::remove_file(path).expect("remove seeded stale temp");
        }
        fs::remove_dir(data_dir).expect("remove empty test data dir");

        assert_eq!(error_kind, Some(std::io::ErrorKind::AlreadyExists));
    }

    #[test]
    fn omitted_pipe_name_resolves_to_the_users_production_digest() {
        let sid = "S-1-5-21-111-222-333-1001";

        let pipe_name = resolve_pipe_name(None, sid);

        assert_eq!(pipe_name, "mission-control-4c51f3baadf41ed2");
    }

    #[test]
    fn shutdown_helper_writes_structured_stopped_log() {
        let mut output = Vec::new();

        write_stopped_log(&mut output, 42, "test-pipe").expect("write stopped log");

        let event: serde_json::Value =
            serde_json::from_slice(&output).expect("stopped log is valid JSON");
        assert_eq!(
            event,
            serde_json::json!({
                "event": "supervisor.stopped",
                "pid": 42,
                "pipe": "test-pipe"
            })
        );
    }
}
