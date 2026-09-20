//! Isolate the cache fixture from other tests' legitimate Core shutdowns.

use std::io::{BufRead, Read, Write};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

const CHILD_ENV: &str = "KOUSHI_THUMBNAIL_PROTOCOL_TEST_CHILD";
const STORED: &str = "thumbnail_protocol_fixture_stored";
const TEST: &str = "tests::renderable_thumbnail_protocol_serves_known_cached_bytes";

pub(super) fn is_child() -> bool {
    std::env::var_os(CHILD_ENV).is_some_and(|value| value == "1")
}

pub(super) fn after_thumbnail_stored() {
    // The parent performs a real Core shutdown in the exact store/lookup gap.
    println!("{STORED}");
    std::io::stdout()
        .flush()
        .expect("thumbnail_child_signal_failed");
    let mut release = [0];
    std::io::stdin()
        .read_exact(&mut release)
        .expect("thumbnail_child_release_failed");
    assert_eq!(release, [1], "thumbnail_child_release_invalid");
}

struct ReapedChild(Child);

impl Drop for ReapedChild {
    fn drop(&mut self) {
        // On timeout or assertion failure, unblock the pipe reader and reap.
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

pub(super) fn run_isolated() {
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut command = Command::new(std::env::current_exe().expect("thumbnail_test_binary_missing"));
    command
        .args(["--exact", TEST, "--nocapture"])
        .env_clear()
        .env(CHILD_ENV, "1")
        .env("RUST_MIN_STACK", "4194304")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    // Windows process startup may require this non-secret OS directory.
    if let Some(system_root) = std::env::var_os("SYSTEMROOT") {
        command.env("SYSTEMROOT", system_root);
    }
    std::thread::scope(|scope| {
        // The guard is inside the scope so unwinding kills the child before
        // scope teardown joins the stdout reader. No child/thread is detached.
        let mut child = ReapedChild(command.spawn().expect("thumbnail_child_spawn_failed"));
        let stdout = child
            .0
            .stdout
            .take()
            .expect("thumbnail_child_stdout_missing");
        let (events, received) = mpsc::channel();
        scope.spawn(move || {
            for line in std::io::BufReader::new(stdout).lines() {
                let Ok(line) = line else { break };
                if line.ends_with(STORED) {
                    let _ = events.send(true);
                }
            }
            let _ = events.send(false);
        });
        assert_eq!(
            received.recv_timeout(deadline.saturating_duration_since(Instant::now())),
            Ok(true),
            "thumbnail_child_store_barrier_failed"
        );
        shutdown_unrelated_runtime(deadline);
        child
            .0
            .stdin
            .take()
            .expect("thumbnail_child_stdin_missing")
            .write_all(&[1])
            .expect("thumbnail_child_release_failed");
        assert_eq!(
            received.recv_timeout(deadline.saturating_duration_since(Instant::now())),
            Ok(false),
            "thumbnail_child_completion_deadline"
        );
        // EOF need not imply process exit. Bound the final OS reap too; pipe
        // handshakes above, not this supervision interval, order cache access.
        loop {
            if let Some(status) = child.0.try_wait().expect("thumbnail_child_wait_failed") {
                assert!(status.success(), "thumbnail_protocol_child_failed");
                break;
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            assert!(!remaining.is_zero(), "thumbnail_child_exit_deadline");
            std::thread::park_timeout(remaining.min(Duration::from_millis(5)));
        }
    });
}

fn shutdown_unrelated_runtime(deadline: Instant) {
    let directory = tempfile::tempdir().expect("isolated runtime directory");
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("test runtime")
        .block_on(async {
            let core = koushi_core::CoreRuntime::start_with_data_dir(directory.path().to_owned());
            tokio::time::timeout(
                deadline.saturating_duration_since(Instant::now()),
                core.shutdown(),
            )
            .await
            .expect("thumbnail_parent_shutdown_deadline");
        });
}
