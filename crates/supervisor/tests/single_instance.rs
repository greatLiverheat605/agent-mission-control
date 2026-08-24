#![cfg(windows)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

struct TestProcess(Child);

impl TestProcess {
    fn spawn(pipe_name: &str, data_dir: &Path) -> Self {
        Self::spawn_args([
            "--pipe-name",
            pipe_name,
            "--data-dir",
            data_dir.to_str().expect("test data dir is valid Unicode"),
            "--parent-pid",
            &std::process::id().to_string(),
            "--log-level",
            "debug",
        ])
    }

    fn spawn_args<const N: usize>(args: [&str; N]) -> Self {
        let child = Command::new(env!("CARGO_BIN_EXE_mission-control-supervisor"))
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("supervisor starts");

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
        "mission-supervisor-single-instance-{}-{nonce}",
        std::process::id()
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

fn wait_for_default_ready(path: &Path, expected_pid: u32) -> String {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Ok(contents) = fs::read_to_string(path)
            && let Ok(ready) = serde_json::from_str::<serde_json::Value>(&contents)
            && ready["pid"] == expected_pid
            && let Some(pipe_name) = ready["pipe"].as_str()
        {
            return pipe_name.to_owned();
        }
        assert!(
            Instant::now() < deadline,
            "supervisor did not write a default ready file"
        );
        thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn a_named_instance_excludes_competitors_and_releases_after_exit() {
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

    let mut second = TestProcess::spawn(&pipe_name, &data_dir);
    let second_status = second
        .wait_for_exit(Duration::from_secs(2))
        .expect("competing supervisor exits within two seconds");
    assert_eq!(second_status.code(), Some(23));

    first.terminate();
    fs::remove_file(&ready_path).expect("remove stale ready file from terminated process");
    assert!(
        !ready_path.exists(),
        "old ready file is gone before restart"
    );

    let mut third = TestProcess::spawn(&pipe_name, &data_dir);
    wait_for_ready(&ready_path, third.id(), &pipe_name);
    third.terminate();

    fs::remove_dir_all(&data_dir).expect("remove isolated test data dir");
}

#[test]
fn omitted_pipe_name_uses_the_current_users_stable_digest() {
    let data_dir = unique_test_dir();
    fs::create_dir_all(&data_dir).expect("create isolated test data dir");

    let mut process = TestProcess::spawn_args([
        "--data-dir",
        data_dir.to_str().expect("test data dir is valid Unicode"),
    ]);
    let pipe_name = wait_for_default_ready(&data_dir.join("supervisor.ready"), process.id());

    assert!(pipe_name.starts_with("mission-control-"));
    assert_eq!(pipe_name.len(), "mission-control-".len() + 16);

    process.terminate();
    fs::remove_dir_all(&data_dir).expect("remove isolated test data dir");
}

#[test]
fn invalid_parent_pid_exits_nonzero() {
    let data_dir = unique_test_dir();
    let mut process = TestProcess::spawn_args([
        "--pipe-name",
        "mission-control-invalid-parent-test",
        "--data-dir",
        data_dir.to_str().expect("test data dir is valid Unicode"),
        "--parent-pid",
        "not-a-pid",
    ]);

    let status = process.wait_for_exit(Duration::from_secs(2));
    if status.is_none() {
        process.terminate();
    }
    assert!(
        status.is_some_and(|status| !status.success()),
        "invalid parent PID must exit nonzero"
    );

    if data_dir.exists() {
        fs::remove_dir_all(&data_dir).expect("remove isolated test data dir");
    }
}
