#![cfg(windows)]

use std::fs::{self, File};
use std::io::Read;
use std::os::windows::fs::OpenOptionsExt;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::ptr;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use windows_sys::Win32::Foundation::ERROR_NOT_FOUND;
use windows_sys::Win32::Security::Credentials::{
    CRED_TYPE_GENERIC, CREDENTIALW, CredDeleteW, CredFree, CredReadW,
};
use windows_sys::Win32::System::Console::{CTRL_BREAK_EVENT, GenerateConsoleCtrlEvent};
use windows_sys::Win32::System::Threading::CREATE_NEW_PROCESS_GROUP;

use mission_protocol::credential::WindowsCredentialInstallSecret;
use mission_protocol::frame::{read_frame, write_frame};
use mission_protocol::handshake::{
    ClientMessage, Handshake, InstallSecretProvider, NONCE_BYTES, PRODUCT_INSTALL_ID,
    PROTOCOL_VERSION, ServerMessage, handshake_proof,
};

struct TestProcess {
    child: Child,
    credential_target: Option<String>,
}

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
        let credential_target = test_credential_target(pipe_name);
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
                "--credential-target",
                &credential_target,
            ],
            creation_flags,
            Stdio::piped(),
            Some(credential_target.clone()),
        )
    }

    fn spawn_with_stdout(pipe_name: &str, data_dir: &Path, stdout: Stdio) -> Self {
        let credential_target = test_credential_target(pipe_name);
        Self::spawn_args_with_flags(
            [
                "--pipe-name",
                pipe_name,
                "--data-dir",
                data_dir.to_str().expect("test data dir is valid Unicode"),
                "--credential-target",
                &credential_target,
            ],
            0,
            stdout,
            Some(credential_target.clone()),
        )
    }

    fn spawn_with_parent(pipe_name: &str, data_dir: &Path, parent_pid: u32) -> Self {
        let credential_target = test_credential_target(pipe_name);
        Self::spawn_args_with_flags(
            [
                "--pipe-name",
                pipe_name,
                "--data-dir",
                data_dir.to_str().expect("test data dir is valid Unicode"),
                "--parent-pid",
                &parent_pid.to_string(),
                "--credential-target",
                &credential_target,
            ],
            0,
            Stdio::piped(),
            Some(credential_target.clone()),
        )
    }

    fn spawn_args(args: impl IntoIterator<Item = impl AsRef<std::ffi::OsStr>>) -> Self {
        Self::spawn_args_with_flags(args, 0, Stdio::piped(), None)
    }

    fn spawn_args_with_flags(
        args: impl IntoIterator<Item = impl AsRef<std::ffi::OsStr>>,
        creation_flags: u32,
        stdout: Stdio,
        credential_target: Option<String>,
    ) -> Self {
        let mut command = Command::new(env!("CARGO_BIN_EXE_mission-control-supervisor"));
        command
            .args(args)
            .stdin(Stdio::null())
            .stdout(stdout)
            .stderr(Stdio::piped())
            .creation_flags(creation_flags);
        let child = command.spawn().expect("supervisor starts");

        Self {
            child,
            credential_target,
        }
    }

    fn id(&self) -> u32 {
        self.child.id()
    }

    fn wait_for_exit(&mut self, timeout: Duration) -> Option<std::process::ExitStatus> {
        let deadline = Instant::now() + timeout;
        loop {
            if let Some(status) = self.child.try_wait().expect("query child status") {
                return Some(status);
            }
            if Instant::now() >= deadline {
                return None;
            }
            thread::sleep(Duration::from_millis(10));
        }
    }

    fn terminate(&mut self) {
        if self.child.try_wait().expect("query child status").is_none() {
            self.child.kill().expect("terminate exact child process");
        }
        self.child.wait().expect("reap child process");
    }

    fn read_stderr(&mut self) -> String {
        let mut stderr = String::new();
        self.child
            .stderr
            .take()
            .expect("child stderr is captured")
            .read_to_string(&mut stderr)
            .expect("read child stderr");
        stderr
    }

    fn read_stdout(&mut self) -> String {
        let mut stdout = String::new();
        self.child
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
        if self.child.try_wait().is_ok_and(|status| status.is_none()) {
            let _ = self.child.kill();
        }
        let _ = self.child.wait();
        if let Some(target) = &self.credential_target {
            delete_test_credential(target);
        }
    }
}

fn test_credential_target(pipe_name: &str) -> String {
    format!(
        "Agent Mission Control/Test/Supervisor/{}/{}",
        std::process::id(),
        pipe_name
    )
}

fn delete_test_credential(target: &str) {
    let target: Vec<u16> = target.encode_utf16().chain(Some(0)).collect();
    unsafe { CredDeleteW(target.as_ptr(), CRED_TYPE_GENERIC, 0) };
}

fn assert_test_credential_missing(target: &str) {
    let target: Vec<u16> = target.encode_utf16().chain(Some(0)).collect();
    let mut credential: *mut CREDENTIALW = ptr::null_mut();
    assert_eq!(
        unsafe { CredReadW(target.as_ptr(), CRED_TYPE_GENERIC, 0, &mut credential) },
        0
    );
    assert!(credential.is_null());
    assert_eq!(
        std::io::Error::last_os_error().raw_os_error(),
        Some(ERROR_NOT_FOUND as i32)
    );
    if !credential.is_null() {
        unsafe { CredFree(credential.cast()) };
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

fn assert_authenticated_ping(pipe_name: &str) {
    let pipe_path = format!(r"\\.\pipe\{pipe_name}");
    let deadline = Instant::now() + Duration::from_secs(2);
    let mut pipe = loop {
        match fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&pipe_path)
        {
            Ok(pipe) => break pipe,
            Err(error) => {
                assert!(Instant::now() < deadline, "connect to ready pipe: {error}");
                thread::sleep(Duration::from_millis(5));
            }
        }
    };
    let secret = WindowsCredentialInstallSecret::for_test_target(test_credential_target(pipe_name))
        .expect("accept unique non-secret process test target")
        .install_secret()
        .expect("desktop and supervisor share the install secret");
    let nonce = vec![7; NONCE_BYTES];
    let versions = vec![PROTOCOL_VERSION];
    let proof = handshake_proof(&secret, PRODUCT_INSTALL_ID, &nonce, &versions)
        .expect("sign production handshake");
    write_frame(
        &mut pipe,
        &ClientMessage::Handshake(Handshake {
            install_id: PRODUCT_INSTALL_ID.to_owned(),
            protocol_versions: versions,
            nonce,
            proof,
        }),
    )
    .expect("write production handshake");
    assert!(matches!(
        read_frame(&mut pipe).expect("read handshake acceptance"),
        ServerMessage::HandshakeAccepted(_)
    ));
    write_frame(&mut pipe, &ClientMessage::Ping).expect("write ping");
    assert!(matches!(
        read_frame(&mut pipe).expect("read pong"),
        ServerMessage::Pong(_)
    ));
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
    assert_authenticated_ping(&pipe_name);

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
    assert!(
        fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(format!(r"\\.\pipe\{pipe_name}"))
            .is_err(),
        "graceful shutdown releases the pipe"
    );

    let credential_target = test_credential_target(&pipe_name);
    drop(third);
    drop(second);
    drop(first);
    assert_test_credential_missing(&credential_target);
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
fn graceful_shutdown_propagates_ready_cleanup_failure() {
    let _serial = PROCESS_TEST_LOCK.lock().expect("lock process tests");
    let data_dir = unique_test_dir();
    fs::create_dir_all(&data_dir).expect("create isolated test data dir");
    let ready_path = data_dir.join("supervisor.ready");
    let mut process =
        TestProcess::spawn_process_group("mission-control-cleanup-failure-test", &data_dir);
    wait_for_ready(
        &ready_path,
        process.id(),
        "mission-control-cleanup-failure-test",
    );
    let locked_ready = fs::OpenOptions::new()
        .read(true)
        .share_mode(0)
        .open(&ready_path)
        .expect("open ready without delete sharing");

    process.send_ctrl_break();
    let status = process
        .wait_for_exit(Duration::from_secs(2))
        .expect("supervisor exits within two seconds");
    let stdout = process.read_stdout();
    let stderr = process.read_stderr();
    let ready_remained = ready_path.exists();
    drop(locked_ready);
    fs::remove_file(&ready_path).expect("remove locked ready after releasing handle");
    fs::remove_dir_all(&data_dir).expect("remove isolated test data dir");

    assert!(stdout.contains("\"event\":\"supervisor.stopped\""));
    assert!(
        ready_remained,
        "failed cleanup leaves the ready path in place"
    );
    assert_ne!(
        status.code(),
        Some(0),
        "ready cleanup failure must not report successful exit; stderr={stderr}"
    );
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

#[test]
fn supervisor_exits_when_its_parent_process_is_gone() {
    let _serial = PROCESS_TEST_LOCK.lock().expect("lock process tests");
    let data_dir = unique_test_dir();
    fs::create_dir_all(&data_dir).expect("create isolated test data dir");
    let pipe_name = "mission-control-departed-parent-test";
    let mut departed_parent = Command::new("cmd.exe")
        .args(["/d", "/c", "exit", "0"])
        .spawn()
        .expect("start short-lived parent fixture");
    let departed_pid = departed_parent.id();
    departed_parent.wait().expect("fixture parent exits");
    let mut supervisor = TestProcess::spawn_with_parent(pipe_name, &data_dir, departed_pid);

    let status = supervisor
        .wait_for_exit(Duration::from_secs(2))
        .expect("supervisor cannot outlive a departed parent");
    let events: Vec<serde_json::Value> = supervisor
        .read_stdout()
        .lines()
        .map(|line| serde_json::from_str(line).expect("stdout line is valid JSON"))
        .collect();

    assert_eq!(status.code(), Some(0));
    assert_eq!(events.len(), 2);
    assert_eq!(events[0]["event"], "supervisor.ready");
    assert_eq!(events[1]["event"], "supervisor.stopped");
    assert!(!data_dir.join("supervisor.ready").exists());
    assert!(
        fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(format!(r"\\.\pipe\{pipe_name}"))
            .is_err(),
        "parent shutdown releases the pipe"
    );

    let credential_target = test_credential_target(pipe_name);
    drop(supervisor);
    assert_test_credential_missing(&credential_target);
    fs::remove_dir_all(&data_dir).expect("remove isolated test data dir");
}
