//! Bounded capture process waits and cleanup for race tests.

use std::fs;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::thread;
use std::time::{Duration, Instant};

/// Bounds process waits and test-side race synchronization.
const TIMEOUT: Duration = Duration::from_secs(20);

/// Keeps command output in files so process pipes cannot block a race test.
pub(super) struct CaptureChild {
    child: Child,
    stdout: PathBuf,
    stderr: PathBuf,
}

impl CaptureChild {
    /// Starts a process group that the test can stop on failure.
    pub(super) fn spawn(mut command: Command, work: &Path, job: &str) -> Self {
        let stdout = work.join(format!("{job}.stdout"));
        let stderr = work.join(format!("{job}.stderr"));
        let child = command
            .stdout(fs::File::create(&stdout).expect("create capture stdout log"))
            .stderr(fs::File::create(&stderr).expect("create capture stderr log"))
            .process_group(0)
            .spawn()
            .expect("start fixture capture");
        Self {
            child,
            stdout,
            stderr,
        }
    }

    /// Reaps the command before returning its status and output.
    pub(super) fn wait(mut self) -> Output {
        let start = Instant::now();
        loop {
            if let Some(status) = self.child.try_wait().expect("check capture status") {
                return Output {
                    status,
                    stdout: fs::read(&self.stdout).expect("read capture stdout"),
                    stderr: fs::read(&self.stderr).expect("read capture stderr"),
                };
            }
            assert!(
                start.elapsed() < TIMEOUT,
                "fixture capture timed out: {}",
                fs::read_to_string(&self.stderr).expect("read capture timeout log")
            );
            thread::sleep(Duration::from_millis(10));
        }
    }
}

impl Drop for CaptureChild {
    fn drop(&mut self) {
        if self
            .child
            .try_wait()
            .expect("check child before cleanup")
            .is_some()
        {
            return;
        }
        let status = Command::new("/bin/kill")
            .args(["-KILL", "--"])
            .arg(format!("-{}", self.child.id()))
            .stderr(Stdio::null())
            .status()
            .expect("stop capture process group");
        if !status.success() {
            assert!(
                self.child
                    .try_wait()
                    .expect("check stopped capture")
                    .is_some(),
                "could not stop capture process group"
            );
        }
        self.child
            .wait()
            .expect("reap capture process during cleanup");
    }
}

/// Requires every producer to reach exclusive directory creation before release.
pub(super) fn wait_ready(gate: &Path, jobs: &[&str]) {
    let start = Instant::now();
    while !jobs
        .iter()
        .all(|job| gate.join(format!("ready-{job}")).exists())
    {
        assert!(start.elapsed() < TIMEOUT, "capture race gate timed out");
        thread::sleep(Duration::from_millis(10));
    }
}

/// A failed test reaps its child instead of leaving capture processes running.
#[test]
fn dropping_capture_child_stops_its_process_group() {
    let temporary = tempfile::tempdir().expect("capture process test directory");
    let mut command = Command::new("bash");
    command.args(["-c", "sleep 60"]);
    let mut child = CaptureChild::spawn(command, temporary.path(), "cleanup");
    let id = child.child.id();
    assert!(
        child
            .child
            .try_wait()
            .expect("check sleeping child")
            .is_none()
    );
    drop(child);
    let alive = Command::new("/bin/kill")
        .args(["-0", "--"])
        .arg(id.to_string())
        .stderr(Stdio::null())
        .status()
        .expect("check reaped capture child");
    assert!(!alive.success());
}
