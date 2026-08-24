#![cfg(windows)]

use std::fs::{self, File};
use std::io::Read;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use windows_sys::Win32::System::Console::{CTRL_BREAK_EVENT, GenerateConsoleCtrlEvent};
use windows_sys::Win32::System::Threading::CREATE_NEW_PROCESS_GROUP;

struct TestProcess(Child);

static TEST_NONCE: AtomicU64 = AtomicU64::new(0);
static PROCESS_TEST_LOCK: Mutex<()> = Mutex::new(());

impl TestProcess {
    fn spawn(pipe_name: &str, data_dir: &Path) -> Self {
        Self::spawn_with_flags(pipe_name, data_dir, 0)
    }

    fn spawn_process_group(pipe_name: &str, data_dir: &Path) -> Self {
        Self::spawn_with_flags(pipe_name, data_dir, CREATE_NEW_PROCESS_GROUP)
    }

    fn spawn_with_flags(pipe_name: &str, data_dir: &Path, creation_flags: u32) -> Self {
        Self::spawn_args_with_flags(
            [
                "--pipe-name",
                pipe_name,
                "--data-dir",
                data_dir.to_str().expect("test data dir is valid Unicode"),
                "--parent-pid",
                &std::process::id().to_string(),
                "--log-level",
                "debug",
            ],
            creation_flags,
            Stdio::piped(),
        )
    }

    fn spawn_with_stdout(pipe_name: &str, data_dir: &Path, stdout: Stdio) -> Self {
        Self::spawn_args_with_flags(
            [
                "--pipe-name",
                pipe_name,
                "--data-dir",
                data_dir.to_str().expect("test data dir is valid Unicode"),
            ],
            0,
            stdout,
        )
    }

    fn spawn_args(args: impl IntoIterator<Item = impl AsRef<std::ffi::OsStr>>) -> Self {
        Self::spawn_args_with_flags(args, 0, Stdio::piped())
    }

    fn spawn_args_with_flags(
        args: impl IntoIterator<Item = impl AsRef<std::ffi::OsStr>>,
        creation_flags: u32,
        stdout: Stdio,
    ) -> Self {
        let mut command = Command::new(env!("CARGO_BIN_EXE_mission-control-supervisor"));
        command
            .args(args)
            .stdin(Stdio::null())
            .stdout(stdout)
            .stderr(Stdio::piped())
            .creation_flags(creation_flags);
        let child = command.spawn().expect("supervisor starts");

        Self(child)
    }

    fn id(&self) -> u32 {
        self.0.id()
    }

    fn wait_for_exit(&mut self, timeout: Duration) -> Option<std::process::ExitStatus> {
        let deadline = Instant::now() + timeout;
        loop {
            if let Some(status) = self.0.try_wait().expect("query child status") {
                return Some(status);
            }
            if Instant::now() >= deadline {
                return None;
            }
            thread::sleep(Duration::from_millis(10));
        }
    }

    fn terminate(&mut self) {
        if self.0.try_wait().expect("query child status").is_none() {
            self.0.kill().expect("terminate exact child process");
        }
        self.0.wait().expect("reap child process");
    }

    fn read_stderr(&mut self) -> String {
        let mut stderr = String::new();
        self.0
            .stderr
            .take()
            .expect("child stderr is captured")
            .read_to_string(&mut stderr)
            .expect("read child stderr");
        stderr
    }

    fn read_stdout(&mut self) -> String {
        let mut stdout = String::new();
        self.0
            .stdout
            .take()
            .expect("child stdout is captured")
            .read_to_string(&mut stdout)
            .expect("read child stdout");
        stdout
    }

    fn send_ctrl_break(&self) {
        if unsafe { GenerateConsoleCtrlEvent(CTRL_BREAK_EVENT, self.id()) } == 0 {
            panic!(
                "GenerateConsoleCtrlEvent failed for process group {}: {}",
                self.id(),
                std::io::Error::last_os_error()
            );
        }
    }
}

impl Drop for TestProcess {
    fn drop(&mut self) {
        if self.0.try_wait().is_ok_and(|status| status.is_none()) {
            let _ = self.0.kill();
        }
        let _ = self.0.wait();
    }
}

fn unique_test_dir() -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock is after the Unix epoch")
        .as_nanos();
    std::env::temp_dir().join(format!(
        "mission-supervisor-single-instance-{}-{nonce}-{}",
        std::process::id(),
        TEST_NONCE.fetch_add(1, Ordering::Relaxed)
    ))
}

fn wait_for_ready(path: &Path, expected_pid: u32, expected_pipe: &str) {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Ok(contents) = fs::read_to_string(path)
            && let Ok(ready) = serde_json::from_str::<serde_json::Value>(&contents)
            && ready["pid"] == expected_pid
            && ready["pipe"] == expected_pipe
        {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "supervisor did not write the expected ready file"
        );
        thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn a_named_instance_excludes_competitors_and_releases_after_exit() {
    let _serial = PROCESS_TEST_LOCK.lock().expect("lock process tests");
    let data_dir = unique_test_dir();
    fs::create_dir_all(&data_dir).expect("create isolated test data dir");
    let ready_path = data_dir.join("supervisor.ready");
    let pipe_name = format!(
        "mission-control-test-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock is after the Unix epoch")
            .as_nanos()
    );

    let mut first = TestProcess::spawn(&pipe_name, &data_dir);
    wait_for_ready(&ready_path, first.id(), &pipe_name);

    let competing_pipe_name = format!("{pipe_name}-competitor");
    let mut second = TestProcess::spawn(&competing_pipe_name, &data_dir);
    let second_status = second
        .wait_for_exit(Duration::from_secs(2))
        .expect("competing supervisor exits within two seconds");
    assert_eq!(second_status.code(), Some(23));

    first.terminate();
    fs::write(&ready_path, b"{malformed").expect("seed malformed stale ready after crash");

    let mut third = TestProcess::spawn_process_group(&pipe_name, &data_dir);
    wait_for_ready(&ready_path, third.id(), &pipe_name);
    assert!(
        fs::read_dir(&data_dir)
            .expect("read test data dir")
            .all(|entry| !entry
                .expect("read test data entry")
                .file_name()
                .to_string_lossy()
                .ends_with(".tmp")),
        "ready publish leaves no temp files"
    );
    third.send_ctrl_break();
    let third_status = third
        .wait_for_exit(Duration::from_secs(2))
        .expect("graceful supervisor exits within two seconds");
    assert_eq!(third_status.code(), Some(0));
    let events: Vec<serde_json::Value> = third
        .read_stdout()
        .lines()
        .map(|line| serde_json::from_str(line).expect("stdout line is valid JSON"))
        .collect();
    assert_eq!(events.len(), 2);
    assert_eq!(events[0]["event"], "supervisor.ready");
    assert_eq!(events[1]["event"], "supervisor.stopped");
    assert!(!ready_path.exists(), "graceful shutdown removes ready file");

    fs::remove_dir_all(&data_dir).expect("remove isolated test data dir");
}

#[test]
fn output_error_after_ready_publish_removes_the_ready_file() {
    let _serial = PROCESS_TEST_LOCK.lock().expect("lock process tests");
    let data_dir = unique_test_dir();
    fs::create_dir_all(&data_dir).expect("create isolated test data dir");
    let stdout_path = data_dir.join("read-only-stdout");
    fs::write(&stdout_path, b"").expect("create stdout target");
    let stdout = Stdio::from(File::open(&stdout_path).expect("open read-only stdout target"));
    let ready_path = data_dir.join("supervisor.ready");
    let mut process =
        TestProcess::spawn_with_stdout("mission-control-output-error-test", &data_dir, stdout);

    let status = process
        .wait_for_exit(Duration::from_secs(2))
        .expect("output failure exits within two seconds");
    assert_eq!(status.code(), Some(2));
    let error: serde_json::Value =
        serde_json::from_str(process.read_stderr().trim()).expect("stderr is structured JSON");
    assert_eq!(error["errorCode"], "io_error");
    assert!(
        !ready_path.exists(),
        "published ready is removed when ready log output fails"
    );

    fs::remove_dir_all(&data_dir).expect("remove isolated test data dir");
}

#[test]
fn invalid_arguments_emit_one_safe_structured_error() {
    for invalid_args in [
        vec!["--parent-pid", "sensitive-parent-value"],
        vec!["--sensitive-unknown-argument"],
        vec!["--log-level", "sensitive-log-level"],
    ] {
        let mut process = TestProcess::spawn_args(invalid_args);
        let status = process
            .wait_for_exit(Duration::from_secs(2))
            .expect("invalid arguments exit within two seconds");
        assert_eq!(status.code(), Some(2));

        let stderr = process.read_stderr();
        assert_eq!(stderr.lines().count(), 1);
        let error: serde_json::Value =
            serde_json::from_str(stderr.trim()).expect("stderr is valid JSON");
        assert_eq!(
            error,
            serde_json::json!({
                "event": "supervisor.error",
                "errorCode": "invalid_arguments"
            })
        );
        assert!(!stderr.contains("sensitive"));
    }
}
