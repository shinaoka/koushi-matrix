//! Replacement-launch ordering for the macOS relaunch handoff (#1864).
//!
//! The Dock symptom needs a real bundle, but the ordering contract does not: a
//! real child process stands in for the old instance, and the launcher is a
//! stub, so the wait, the timeout, the argument boundaries, and the failure
//! behavior are all observable headlessly.

use super::*;
use crate::{
    ApplicationExit, CoreExitOutcome, QuitStage, claim_core_shutdown, finish_application_shutdown,
    request_application_restart_with,
};
#[cfg(unix)]
use std::time::Instant;
use std::{
    path::Path,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU8, AtomicUsize, Ordering},
    },
    time::Duration,
};

use tokio::sync::oneshot;

fn plan(bundle: &str, args: Vec<OsString>) -> RelaunchPlan {
    RelaunchPlan {
        old_pid: std::process::id(),
        bundle: Path::new(bundle).to_path_buf(),
        args,
    }
}

/// Bounded wait for a condition that the test expects to become true.
#[cfg(unix)]
fn wait_until(deadline: Duration, mut condition: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + deadline;
    while Instant::now() < deadline {
        if condition() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    condition()
}

/// A real child process standing in for the old instance. Always reaped.
#[cfg(unix)]
struct ChildGuard(std::process::Child);

#[cfg(unix)]
impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
fn only_bundled_executables_get_a_detached_relaunch() {
    assert_eq!(
        bundle_root(Path::new("/Applications/Koushi.app/Contents/MacOS/koushi")),
        Some(Path::new("/Applications/Koushi.app"))
    );
    assert_eq!(
        bundle_root(Path::new(
            "/Users/x/koushi-desktop/target/debug/koushi-desktop"
        )),
        None
    );
    assert_eq!(
        bundle_root(Path::new("/Applications/Koushi.app/Contents/koushi")),
        None
    );
    assert_eq!(
        current_plan(
            Path::new("/Users/x/koushi-desktop/target/debug/koushi-desktop"),
            7,
            Vec::new()
        ),
        None
    );
    assert_eq!(
        current_plan(
            Path::new("/Applications/Koushi.app/Contents/MacOS/koushi"),
            7,
            vec![OsString::from("--flag")]
        )
        .map(|plan| plan.bundle),
        Some(Path::new("/Applications/Koushi.app").to_path_buf())
    );
}

#[test]
fn the_default_wait_is_bounded_and_polls() {
    assert!(RELAUNCH_WAIT.timeout > Duration::ZERO);
    assert!(RELAUNCH_WAIT.timeout <= Duration::from_secs(60));
    assert!(RELAUNCH_WAIT.poll > Duration::ZERO);
    assert!(RELAUNCH_WAIT.poll < RELAUNCH_WAIT.timeout);
}

#[test]
fn helper_and_launch_arguments_carry_paths_and_arguments_as_data() {
    let bundle = OsString::from("/Applications/Koushi 2; $(touch pwned) `id` 'q'.app");
    let args = vec![
        OsString::from("--flag=a b"),
        OsString::from(";rm -rf /"),
        OsString::from("\"quoted\" --args"),
    ];
    let plan = RelaunchPlan {
        old_pid: 4242,
        bundle: Path::new(&bundle).to_path_buf(),
        args: args.clone(),
    };

    let helper = plan.helper_arguments();
    assert_eq!(helper[0], OsString::from(RELAUNCH_HELPER_FLAG));
    assert_eq!(helper[1], OsString::from("4242"));
    assert_eq!(helper[2], bundle);
    assert_eq!(helper.len(), args.len() + 3);
    assert_eq!(
        RelaunchPlan::from_helper_arguments(&helper),
        Some(plan.clone())
    );

    // Each path and argument stays exactly one argv entry: no splitting on
    // spaces and no shell interpretation of metacharacters.
    assert_eq!(
        plan.launch_arguments(),
        vec![
            OsString::from("-n"),
            bundle,
            OsString::from("--args"),
            args[0].clone(),
            args[1].clone(),
            args[2].clone(),
        ]
    );

    assert_eq!(RelaunchPlan::from_helper_arguments(&[]), None);
    assert_eq!(
        RelaunchPlan::from_helper_arguments(&[OsString::from("--other"), OsString::from("1")]),
        None
    );
    assert_eq!(
        RelaunchPlan::from_helper_arguments(&[
            OsString::from(RELAUNCH_HELPER_FLAG),
            OsString::from("not-a-pid"),
            OsString::from("/Applications/Koushi.app"),
        ]),
        None
    );
    assert_eq!(
        RelaunchPlan::from_helper_arguments(&[
            OsString::from(RELAUNCH_HELPER_FLAG),
            OsString::from("1"),
            OsString::new(),
        ]),
        None
    );
}

#[cfg(unix)]
#[test]
fn the_liveness_probe_follows_a_real_process() {
    let child = ChildGuard(
        std::process::Command::new("/bin/sleep")
            .arg("30")
            .spawn()
            .unwrap(),
    );
    let pid = child.0.id();
    assert!(process_is_alive(pid));
    let mut child = child;
    child.0.kill().unwrap();
    child.0.wait().unwrap();
    assert!(wait_until(Duration::from_secs(5), || !process_is_alive(
        pid
    )));
}

#[cfg(unix)]
#[test]
fn the_replacement_launches_once_only_after_the_old_process_exits() {
    let mut child = ChildGuard(
        std::process::Command::new("/bin/sleep")
            .arg("30")
            .spawn()
            .unwrap(),
    );
    let old_pid = child.0.id();
    let launches = Arc::new(Mutex::new(Vec::<RelaunchPlan>::new()));
    let probes = Arc::new(AtomicUsize::new(0));
    let plan = RelaunchPlan {
        old_pid,
        bundle: Path::new("/Applications/Koushi 2; $(id).app").to_path_buf(),
        args: vec![OsString::from("--relaunched")],
    };
    let handoff = {
        let launches = Arc::clone(&launches);
        let probes = Arc::clone(&probes);
        let plan = plan.clone();
        std::thread::spawn(move || {
            relaunch_after_exit(
                &plan,
                RelaunchWait {
                    timeout: Duration::from_secs(20),
                    poll: Duration::from_millis(2),
                },
                |pid| {
                    probes.fetch_add(1, Ordering::AcqRel);
                    process_is_alive(pid)
                },
                |plan| {
                    launches.lock().unwrap().push(plan.clone());
                    true
                },
            )
        })
    };

    // The old process is alive: the helper must keep polling and not launch.
    assert!(wait_until(Duration::from_secs(10), || probes
        .load(Ordering::Acquire)
        >= 20));
    assert!(launches.lock().unwrap().is_empty());

    child.0.kill().unwrap();
    child.0.wait().unwrap();
    assert!(wait_until(Duration::from_secs(20), || launches
        .lock()
        .unwrap()
        .len()
        == 1));
    assert_eq!(handoff.join().unwrap(), RelaunchOutcome::Launched);
    // The handoff thread has returned, so the one launch above is the only one.
    assert_eq!(*launches.lock().unwrap(), vec![plan]);
}

#[test]
fn an_expired_wait_never_launches_concurrently() {
    let launches = AtomicUsize::new(0);
    let probes = AtomicUsize::new(0);
    let outcome = relaunch_after_exit(
        &plan("/Applications/Koushi.app", Vec::new()),
        RelaunchWait {
            timeout: Duration::from_millis(20),
            poll: Duration::from_millis(2),
        },
        |_| {
            probes.fetch_add(1, Ordering::AcqRel);
            true
        },
        |_| {
            launches.fetch_add(1, Ordering::AcqRel);
            true
        },
    );
    assert_eq!(outcome, RelaunchOutcome::TimedOut);
    assert_eq!(launches.load(Ordering::Acquire), 0);
    assert!(probes.load(Ordering::Acquire) > 1);
}

#[test]
fn a_failed_launch_is_reported_and_not_retried() {
    let attempts = AtomicUsize::new(0);
    let outcome = relaunch_after_exit(
        &plan("/Applications/Koushi.app", Vec::new()),
        RelaunchWait {
            timeout: Duration::from_secs(1),
            poll: Duration::from_millis(2),
        },
        |_| false,
        |_| {
            attempts.fetch_add(1, Ordering::AcqRel);
            false
        },
    );
    assert_eq!(outcome, RelaunchOutcome::LaunchFailed);
    assert_eq!(attempts.load(Ordering::Acquire), 1);
}

#[test]
fn a_failed_handoff_stays_coarse_and_never_relaunches_concurrently() {
    let logged = Arc::new(Mutex::new(Vec::<String>::new()));
    let sink = {
        let logged = Arc::clone(&logged);
        move |line: &str| logged.lock().unwrap().push(line.to_owned())
    };
    let handed = hand_off(
        Some(plan("/Applications/Koushi.app", Vec::new())),
        |_| Err(std::io::Error::other("no helper")),
        sink,
    );
    assert_eq!(handed, RelaunchHandoff::Failed);
    // The durable trace is the only thing anyone can find after the process
    // exits: one line, a reason token, and nothing from the bundle or argv.
    assert_eq!(
        *logged.lock().unwrap(),
        vec!["relaunch_failed helper_spawn_failed".to_owned()]
    );

    let records = koushi_diagnostics::snapshot().records;
    let failure = records
        .iter()
        .find(|record| {
            record.event.stage == "relaunch_failed"
                && koushi_diagnostics::format_event(&record.event).contains("helper_spawn_failed")
        })
        .expect("a failed handoff records a diagnostic");
    assert_eq!(
        failure.event.level,
        koushi_diagnostics::DiagnosticLevel::Warn
    );
    let text = koushi_diagnostics::format_event(&failure.event);
    assert!(!text.contains("Koushi.app"));
    assert!(!text.contains("/Applications"));

    let spawned = AtomicUsize::new(0);
    let mut logged_unbundled = 0;
    let handed = hand_off(
        None,
        |_| {
            spawned.fetch_add(1, Ordering::AcqRel);
            Ok(())
        },
        |_| logged_unbundled += 1,
    );
    assert_eq!(handed, RelaunchHandoff::Unbundled);
    assert_eq!(spawned.load(Ordering::Acquire), 0);
    assert_eq!(logged_unbundled, 0);

    assert_eq!(
        hand_off(
            Some(plan("/Applications/Koushi.app", Vec::new())),
            |_| Ok(()),
            |_| panic!("a successful handoff logs no failure"),
        ),
        RelaunchHandoff::Detached
    );
}

#[test]
fn only_an_unbundled_build_keeps_tauris_relaunch() {
    assert!(uses_tauri_relaunch(RelaunchHandoff::Unbundled));
    assert!(!uses_tauri_relaunch(RelaunchHandoff::Detached));
    // A failed handoff must never fall back to the overlapping launch.
    assert!(!uses_tauri_relaunch(RelaunchHandoff::Failed));
}

#[test]
fn a_helper_failure_exits_ordinarily_with_a_coarse_reason() {
    assert_eq!(helper_exit(RelaunchOutcome::Launched), (0, None));
    assert_eq!(
        helper_exit(RelaunchOutcome::TimedOut),
        (1, Some("wait_timeout"))
    );
    assert_eq!(
        helper_exit(RelaunchOutcome::LaunchFailed),
        (1, Some("launch_failed"))
    );

    for reason in ["wait_timeout", "launch_failed", "invalid_helper_arguments"] {
        let logged = Arc::new(Mutex::new(Vec::<String>::new()));
        let sink = {
            let logged = Arc::clone(&logged);
            move |line: &str| logged.lock().unwrap().push(line.to_owned())
        };
        record_handoff_failure(reason, sink);
        let logged = logged.lock().unwrap();
        assert_eq!(*logged, vec![format!("relaunch_failed {reason}")]);
        assert!(!logged[0].contains('/'));
        assert!(!logged[0].contains(std::process::id().to_string().as_str()));
    }

    let records = koushi_diagnostics::snapshot().records;
    let failure = records
        .iter()
        .find(|record| {
            record.event.stage == "relaunch_failed"
                && koushi_diagnostics::format_event(&record.event).contains("launch_failed")
        })
        .expect("a failed launch records a coarse diagnostic");
    let text = koushi_diagnostics::format_event(&failure.event);
    assert!(!text.contains("Koushi.app"));
    assert!(!text.contains("/Applications"));
}

#[derive(Clone, Default)]
struct HandoffRecorder(Arc<Mutex<Vec<u32>>>);

impl ApplicationExit for HandoffRecorder {
    fn ordinary_exit(&self) {}
    fn final_restart(&self) {
        let handed = Arc::clone(&self.0);
        assert_eq!(
            hand_off(
                Some(plan("/Applications/Koushi.app", Vec::new())),
                move |plan| {
                    handed.lock().unwrap().push(plan.old_pid);
                    Ok(())
                },
                |_| panic!("a successful handoff logs no failure"),
            ),
            RelaunchHandoff::Detached
        );
    }
}

#[tokio::test]
async fn duplicate_restart_requests_hand_off_exactly_once() {
    let stage = AtomicU8::new(QuitStage::Idle.repr());
    let restart = AtomicBool::new(false);
    let exit = HandoffRecorder::default();
    assert!(claim_core_shutdown(&stage));
    let (updater_tx, updater_rx) = oneshot::channel();
    let (core_tx, core_rx) = oneshot::channel();
    let mut finish = Box::pin(finish_application_shutdown(
        &stage,
        &restart,
        async {
            updater_rx.await.unwrap();
        },
        async {
            core_rx.await.unwrap();
            CoreExitOutcome::Completed
        },
        &exit,
    ));
    assert!(
        std::future::poll_fn(|cx| {
            std::task::Poll::Ready(finish.as_mut().poll(cx).is_pending())
        })
        .await
    );
    request_application_restart_with(&stage, &restart, &exit);
    request_application_restart_with(&stage, &restart, &exit);
    updater_tx.send(()).unwrap();
    core_tx.send(()).unwrap();
    finish.await;
    request_application_restart_with(&stage, &restart, &exit);
    assert_eq!(*exit.0.lock().unwrap(), vec![std::process::id()]);
}

/// A `#!/bin/sh` stand-in for the macOS-only helper, launcher and helper-mode
/// entry points (#1896). Nothing here reaches `/usr/bin/open`, `/usr/bin/logger`
/// or LaunchServices: each stub records what it was handed and exits on its own,
/// because a real helper is detached on purpose and must never linger.
#[cfg(target_os = "macos")]
fn write_stub(dir: &Path, name: &str, body: &str) -> std::path::PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let path = dir.join(name);
    std::fs::write(&path, body).expect("write stub");
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).expect("chmod stub");
    path
}

/// The stub's `$@`, one entry per NUL, as the strings it was really given.
#[cfg(target_os = "macos")]
fn recorded_arguments(path: &Path) -> Vec<String> {
    let bytes = std::fs::read(path).expect("read recorded arguments");
    bytes
        .split(|byte| *byte == 0)
        .filter(|entry| !entry.is_empty())
        .map(|entry| String::from_utf8(entry.to_vec()).expect("utf-8 argument"))
        .collect()
}

/// A launcher stub that records its argv once and exits with `status`.
#[cfg(target_os = "macos")]
fn recording_launcher(dir: &Path, status: u8) -> std::path::PathBuf {
    let runs = dir.join("launched-runs");
    let argv = dir.join("launched-argv");
    write_stub(
        dir,
        "open-stub",
        &format!(
            "#!/bin/sh\nprintf '.' >> '{runs}'\nfor arg in \"$@\"; do printf '%s\\0' \"$arg\" >> '{argv}'; done\nexit {status}\n",
            runs = runs.display(),
            argv = argv.display()
        ),
    )
}

#[cfg(target_os = "macos")]
fn launch_count(dir: &Path) -> usize {
    std::fs::read(dir.join("launched-runs"))
        .map(|runs| runs.len())
        .unwrap_or(0)
}

#[cfg(target_os = "macos")]
fn failure_sink() -> (Arc<Mutex<Vec<String>>>, impl Fn(&str)) {
    let logged = Arc::new(Mutex::new(Vec::<String>::new()));
    let sink = {
        let logged = Arc::clone(&logged);
        move |line: &str| logged.lock().unwrap().push(line.to_owned())
    };
    (logged, sink)
}

#[cfg(target_os = "macos")]
#[test]
fn the_helper_is_detached_in_its_own_group_with_null_stdio() {
    let dir = tempfile::tempdir().expect("tempdir");
    let observed = dir.path().join("observed");
    let argv_log = dir.path().join("helper-argv");
    let stub = write_stub(
        dir.path(),
        "helper-stub",
        &format!(
            // The three standard streams are duplicated first: a redirect or a
            // command substitution of its own would otherwise be what `stat`
            // sees on /dev/fd/N instead of the descriptor the spawn inherited.
            "#!/bin/sh\nexec 3>&1 4>&0 5>&2\n: > '{argv_log}'\nfor arg in \"$@\"; do printf '%s\\0' \"$arg\" >> '{argv_log}'; done\n\
             {{\n\
             printf 'pid %s\\n' \"$$\"\n\
             printf 'pgid %s\\n' \"$(ps -o pgid= -p $$ | tr -d ' ')\"\n\
             printf 'null %s\\n' \"$(stat -L -f '%d:%i' /dev/null)\"\n\
             printf 'fd 0 %s\\n' \"$(stat -L -f '%d:%i' /dev/fd/4)\"\n\
             printf 'fd 1 %s\\n' \"$(stat -L -f '%d:%i' /dev/fd/3)\"\n\
             printf 'fd 2 %s\\n' \"$(stat -L -f '%d:%i' /dev/fd/5)\"\n\
             printf 'done\\n'\n\
             }} >> '{observed}'\n",
            observed = observed.display(),
            argv_log = argv_log.display()
        ),
    );
    let plan = RelaunchPlan {
        old_pid: std::process::id(),
        bundle: Path::new("/Applications/Koushi 2; $(id).app").to_path_buf(),
        args: vec![OsString::from("--relaunched"), OsString::from("a b")],
    };

    spawn_helper(&stub, &plan).expect("spawn the helper stub");
    assert!(
        wait_until(Duration::from_secs(10), || std::fs::read_to_string(
            &observed
        )
        .is_ok_and(|text| text.ends_with("done\n"))),
        "the helper stub never finished recording"
    );

    // The helper's argv is the cross-version protocol, entry by entry.
    assert_eq!(
        recorded_arguments(&argv_log),
        plan.helper_arguments()
            .iter()
            .map(|argument| argument.to_string_lossy().into_owned())
            .collect::<Vec<_>>()
    );

    let text = std::fs::read_to_string(&observed).expect("read observations");
    let field = |name: &str| {
        text.lines()
            .find_map(|line| line.strip_prefix(&format!("{name} ")))
            .unwrap_or_else(|| panic!("{name} in {text}"))
            .to_owned()
    };
    let helper_pid: u32 = field("pid").parse().expect("helper pid");
    assert_eq!(
        field("pgid"),
        helper_pid.to_string(),
        "the helper leads its own process group"
    );
    // SAFETY: `getpgrp` takes no arguments and has no side effects.
    let own_group = unsafe { libc::getpgrp() };
    assert_ne!(
        field("pgid").parse::<i32>().expect("pgid"),
        own_group,
        "the helper must not stay in the test's process group"
    );
    let null = field("null");
    // fd 1 and fd 2 prove the null stdio on any harness, because the harness's
    // own streams are a pipe or a file. fd 0 proves it only when this process's
    // stdin is not itself /dev/null, as a terminal-less runner may arrange.
    for fd in ["0", "1", "2"] {
        assert_eq!(
            text.lines()
                .find_map(|line| line.strip_prefix(&format!("fd {fd} ")))
                .unwrap_or_else(|| panic!("fd {fd} in {text}")),
            null,
            "fd {fd} must be /dev/null, not the harness's stream"
        );
    }
}

#[cfg(target_os = "macos")]
#[test]
fn launch_services_receives_bundle_and_arguments_as_separate_entries() {
    let dir = tempfile::tempdir().expect("tempdir");
    let launcher = recording_launcher(dir.path(), 0);
    let argv_log = dir.path().join("launched-argv");
    let plan = RelaunchPlan {
        old_pid: std::process::id(),
        bundle: Path::new("/Applications/Koushi 2; $(id).app").to_path_buf(),
        args: vec![
            OsString::from("--relaunched"),
            OsString::from("a b"),
            OsString::from("--flag=1 2"),
        ],
    };

    assert!(
        launch_bundle_with(&launcher, &plan),
        "a zero exit from LaunchServices is a successful launch"
    );
    assert_eq!(
        recorded_arguments(&argv_log),
        vec![
            "-n".to_owned(),
            "/Applications/Koushi 2; $(id).app".to_owned(),
            "--args".to_owned(),
            "--relaunched".to_owned(),
            "a b".to_owned(),
            "--flag=1 2".to_owned(),
        ]
    );

    let failing = tempfile::tempdir().expect("tempdir");
    let failing_launcher = recording_launcher(failing.path(), 1);
    assert!(
        !launch_bundle_with(&failing_launcher, &plan),
        "a non-zero exit from LaunchServices is a failed launch"
    );
}

#[cfg(target_os = "macos")]
#[test]
fn helper_mode_launches_once_after_the_real_old_process_exits() {
    let dir = tempfile::tempdir().expect("tempdir");
    let launcher = recording_launcher(dir.path(), 0);
    let mut child = ChildGuard(
        std::process::Command::new("/bin/sleep")
            .arg("30")
            .spawn()
            .unwrap(),
    );
    let plan = RelaunchPlan {
        old_pid: child.0.id(),
        bundle: Path::new("/Applications/Koushi 2; $(id).app").to_path_buf(),
        args: vec![OsString::from("--relaunched")],
    };
    let arguments = plan.helper_arguments();
    let (logged, sink) = failure_sink();
    let helper_dir = dir.path().to_path_buf();
    let helper = std::thread::spawn(move || {
        run_helper_with(
            &arguments,
            RelaunchWait {
                timeout: Duration::from_secs(20),
                poll: Duration::from_millis(2),
            },
            |plan| launch_bundle_with(&launcher, plan),
            sink,
        )
    });

    // The old process is alive: no replacement may start yet. An early launch
    // would also make the final count two, which the last assertion catches.
    assert!(
        !wait_until(Duration::from_millis(200), || launch_count(&helper_dir) > 0),
        "helper mode launched while the old process was alive"
    );

    child.0.kill().unwrap();
    child.0.wait().unwrap();
    assert_eq!(helper.join().expect("helper thread"), 0);
    assert_eq!(
        launch_count(&helper_dir),
        1,
        "exactly one replacement launch"
    );
    assert_eq!(*logged.lock().unwrap(), Vec::<String>::new());
}

#[cfg(target_os = "macos")]
#[test]
fn helper_mode_reports_coarse_exit_codes() {
    let mut lines: Vec<String> = Vec::new();
    let bundle = "/Applications/Koushi 2; $(id).app";

    // Invalid helper argv: exit before any wait or launch.
    let (logged, sink) = failure_sink();
    let exit = run_helper_with(
        &[
            OsString::from(RELAUNCH_HELPER_FLAG),
            OsString::from("not-a-pid"),
            OsString::from(bundle),
        ],
        RELAUNCH_WAIT,
        |_| panic!("invalid arguments must not launch"),
        sink,
    );
    assert_eq!(exit, 2, "invalid helper arguments exit without launching");
    let invalid = logged.lock().unwrap().clone();
    assert_eq!(
        invalid,
        vec!["relaunch_failed invalid_helper_arguments".to_owned()]
    );
    lines.extend(invalid);

    // A dead old process and a launcher that fails: one ordinary exit code,
    // its own coarse reason.
    let dir = tempfile::tempdir().expect("tempdir");
    let failing_launcher = recording_launcher(dir.path(), 1);
    let dead = ChildGuard(
        std::process::Command::new("/bin/sleep")
            .arg("30")
            .spawn()
            .unwrap(),
    );
    let dead_pid = dead.0.id();
    drop(dead);
    let (logged, sink) = failure_sink();
    let exit = run_helper_with(
        &RelaunchPlan {
            old_pid: dead_pid,
            bundle: Path::new(bundle).to_path_buf(),
            args: Vec::new(),
        }
        .helper_arguments(),
        RELAUNCH_WAIT,
        |plan| launch_bundle_with(&failing_launcher, plan),
        sink,
    );
    assert_eq!(exit, 1, "a failed launch exits ordinarily");
    let failed = logged.lock().unwrap().clone();
    assert_eq!(failed, vec!["relaunch_failed launch_failed".to_owned()]);
    lines.extend(failed);
    assert_eq!(launch_count(dir.path()), 1, "the launcher ran once");

    // A live old process and an expired deadline: no launch at all.
    let dir = tempfile::tempdir().expect("tempdir");
    let launcher = recording_launcher(dir.path(), 0);
    let mut child = ChildGuard(
        std::process::Command::new("/bin/sleep")
            .arg("30")
            .spawn()
            .unwrap(),
    );
    let arguments = RelaunchPlan {
        old_pid: child.0.id(),
        bundle: Path::new(bundle).to_path_buf(),
        args: Vec::new(),
    }
    .helper_arguments();
    let (logged, sink) = failure_sink();
    let exit = run_helper_with(
        &arguments,
        RelaunchWait {
            timeout: Duration::from_millis(50),
            poll: Duration::from_millis(2),
        },
        |plan| launch_bundle_with(&launcher, plan),
        sink,
    );
    assert_eq!(exit, 1, "an expired wait exits ordinarily");
    let timed_out = logged.lock().unwrap().clone();
    assert_eq!(timed_out, vec!["relaunch_failed wait_timeout".to_owned()]);
    lines.extend(timed_out);
    assert_eq!(
        launch_count(dir.path()),
        0,
        "a timed-out wait never launches concurrently"
    );
    child.0.kill().unwrap();
    child.0.wait().unwrap();

    for line in lines {
        assert!(
            !line.contains("/Applications") && !line.contains("Koushi"),
            "the durable line carries a reason token, never a path or argv: {line}"
        );
    }
}

#[cfg(target_os = "macos")]
#[test]
fn an_unbundled_test_binary_never_spawns_a_helper() {
    // The test binary lives under `target-*/ci/deps`, not in an app bundle, so
    // the real macOS entry point stops before spawning anything.
    let handoff = hand_off_relaunch();
    assert_eq!(handoff, RelaunchHandoff::Unbundled);
    assert!(uses_tauri_relaunch(handoff));
    assert_eq!(
        run_helper_if_requested(),
        None,
        "the harness argv does not start with the helper flag"
    );
}
