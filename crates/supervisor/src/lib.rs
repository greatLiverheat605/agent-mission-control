pub mod single_instance;

use std::ffi::OsString;
use std::fmt;
use std::fs;
use std::io::{self, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::Duration;

use serde_json::json;
use windows_sys::Win32::System::Console::{CTRL_BREAK_EVENT, CTRL_C_EVENT, SetConsoleCtrlHandler};

use single_instance::{AcquireResult, SingleInstance, current_user_sid, production_pipe_name};

static RUNNING: AtomicBool = AtomicBool::new(true);

#[derive(Debug)]
pub enum RunError {
    AlreadyRunning,
    InvalidArguments(String),
    Io(std::io::Error),
}

impl fmt::Display for RunError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AlreadyRunning => formatter.write_str("supervisor is already running"),
            Self::InvalidArguments(message) => formatter.write_str(message),
            Self::Io(error) => error.fmt(formatter),
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
}

fn parse_args(args: impl IntoIterator<Item = OsString>) -> Result<Config, RunError> {
    let mut args = args.into_iter();
    let mut pipe_name = None;
    let mut data_dir = None;

    while let Some(argument) = args.next() {
        match argument.to_str() {
            Some("--pipe-name") => {
                pipe_name = Some(
                    args.next()
                        .and_then(|value| value.into_string().ok())
                        .filter(|value| !value.is_empty())
                        .ok_or_else(|| invalid("--pipe-name requires a non-empty value"))?,
                );
            }
            Some("--data-dir") => {
                data_dir = Some(PathBuf::from(
                    args.next()
                        .ok_or_else(|| invalid("--data-dir requires a value"))?,
                ));
            }
            Some("--parent-pid") => {
                let value = args
                    .next()
                    .and_then(|value| value.into_string().ok())
                    .and_then(|value| value.parse::<u32>().ok())
                    .filter(|value| *value != 0);
                if value.is_none() {
                    return Err(invalid("--parent-pid requires a nonzero u32"));
                }
            }
            Some("--log-level") => {
                args.next()
                    .ok_or_else(|| invalid("--log-level requires a value"))?;
            }
            Some(argument) => return Err(invalid(&format!("unknown argument: {argument}"))),
            None => return Err(invalid("arguments must be valid Unicode")),
        }
    }

    Ok(Config {
        pipe_name,
        data_dir: data_dir.ok_or_else(|| invalid("--data-dir is required"))?,
    })
}

fn invalid(message: &str) -> RunError {
    RunError::InvalidArguments(message.to_owned())
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

fn write_stopped_log(writer: &mut impl Write, pid: u32, pipe_name: &str) -> io::Result<()> {
    serde_json::to_writer(
        &mut *writer,
        &json!({ "event": "supervisor.stopped", "pid": pid, "pipe": pipe_name }),
    )?;
    writeln!(writer)
}

pub fn run(args: impl IntoIterator<Item = OsString>) -> Result<(), RunError> {
    let config = parse_args(args)?;
    let sid = current_user_sid()?;
    let pipe_name = config
        .pipe_name
        .unwrap_or_else(|| production_pipe_name(&sid));
    let _instance = match SingleInstance::acquire(&pipe_name, &sid)? {
        AcquireResult::Acquired(instance) => instance,
        AcquireResult::AlreadyRunning => return Err(RunError::AlreadyRunning),
    };
    let _console_handler = ConsoleHandler::install()?;

    fs::create_dir_all(&config.data_dir)?;
    let pid = std::process::id();
    let ready_path = config.data_dir.join("supervisor.ready");
    let ready = json!({ "pid": pid, "pipe": pipe_name });
    fs::write(
        &ready_path,
        serde_json::to_vec(&ready).expect("serializing fixed ready fields cannot fail"),
    )?;
    let mut stdout = io::stdout().lock();
    serde_json::to_writer(
        &mut stdout,
        &json!({ "event": "supervisor.ready", "pid": pid, "pipe": pipe_name }),
    )
    .map_err(io::Error::other)?;
    writeln!(stdout)?;

    while RUNNING.load(Ordering::SeqCst) {
        thread::sleep(Duration::from_millis(50));
    }

    let stopped_result = write_stopped_log(&mut stdout, pid, &pipe_name);
    let cleanup_result = fs::remove_file(ready_path);
    stopped_result?;
    cleanup_result?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::write_stopped_log;

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
