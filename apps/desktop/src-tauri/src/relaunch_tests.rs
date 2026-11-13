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
