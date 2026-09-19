use super::*;
use std::sync::{
    Barrier,
    atomic::{AtomicUsize, Ordering},
};
use tokio::sync::{mpsc, oneshot};

fn policy(generation: u64, auto_check: bool, include_prereleases: bool) -> PolicySnapshot {
    PolicySnapshot {
        generation,
        settings: UpdatesSettings {
            auto_check,
            include_prereleases,
        },
    }
}

fn lifecycle() -> Lifecycle<String> {
    let mut lifecycle = Lifecycle::new(DesktopUpdateState::Idle);
    assert!(lifecycle.observe(policy(1, false, false)));
    lifecycle
}

fn candidate(version: &str) -> PendingUpdate<String> {
    PendingUpdate {
        version: version.into(),
        update: version.into(),
        bytes: None,
    }
}

fn available(lifecycle: &mut Lifecycle<String>, version: &str) -> u64 {
    assert!(lifecycle.begin_check());
    let (operation, _) = lifecycle.claim_work().unwrap();
    lifecycle.complete(
        operation,
        Completion::Check(Ok(Some(candidate(version)))),
        "1.0.0",
    );
    operation.generation
}

#[test]
fn concurrent_check_admission_has_one_winner() {
    let shared = Arc::new(Shared::<String>::new(DesktopUpdateState::Idle));
    let barrier = Arc::new(Barrier::new(8));
    let winners = std::thread::scope(|threads| {
        let handles: Vec<_> = (0..8)
            .map(|_| {
                let shared = shared.clone();
                let barrier = barrier.clone();
                threads.spawn(move || {
                    barrier.wait();
                    usize::from(shared.transition(|_| {}, |lifecycle| lifecycle.begin_check()))
                })
            })
            .collect();
        handles
            .into_iter()
            .map(|handle| handle.join().unwrap())
            .sum::<usize>()
    });
    assert_eq!(
        winners, 1,
        "manual and automatic triggers must share one admission"
    );
}

#[test]
fn automatic_empty_check_settles_overlapping_manual_request_up_to_date() {
    let mut lifecycle = lifecycle();
    lifecycle.observe(policy(2, true, false));
    assert!(
        !lifecycle.begin_check(),
        "manual request joins the automatic operation"
    );
    let (operation, _) = lifecycle.claim_work().unwrap();
    lifecycle.complete(operation, Completion::Check(Ok(None)), "1.2.3");
    assert_eq!(
        lifecycle.state,
        DesktopUpdateState::UpToDate {
            version: "1.2.3".into()
        }
    );
    assert!(lifecycle.begin_check());
    let (operation, _) = lifecycle.claim_work().unwrap();
    lifecycle.complete(operation, Completion::Check(Ok(None)), "1.2.3");
    assert_eq!(lifecycle.state, no_update_state("1.2.3".into()));
}

#[test]
fn snapshot_watermark_rejects_old_and_conflicting_equal_generations() {
    let mut lifecycle = lifecycle();
    lifecycle.observe(policy(10, false, true));
    lifecycle.observe(policy(20, false, true)); // Unrelated snapshot still advances watermark.
    assert!(!lifecycle.observe(policy(19, true, false)));
    assert!(!lifecycle.observe(policy(20, true, false)));
    assert_eq!(lifecycle.settings_generation, Some(20));
    assert_eq!(lifecycle.settings, policy(20, false, true).settings);
    assert!(lifecycle.observe(policy(20, false, true)));
    assert!(lifecycle.claim_work().is_none());
}

#[test]
fn manual_check_with_older_snapshot_uses_current_policy_and_keeps_user_intent() {
    let mut lifecycle = lifecycle();
    lifecycle.observe(policy(20, false, true));
    assert!(lifecycle.request_check(policy(19, true, false)));
    assert_eq!(lifecycle.settings_generation, Some(20));
    assert_eq!(lifecycle.settings, policy(20, false, true).settings);
    let (_, work) = lifecycle.claim_work().unwrap();
    assert!(matches!(work, Work::Check(true)));
}

#[test]
fn download_with_older_snapshot_keeps_consent_but_never_rolls_back_candidate_or_policy() {
    let mut lifecycle = lifecycle();
    lifecycle.observe(policy(20, false, true));
    let generation = available(&mut lifecycle, "2.0.0");
    lifecycle.observe(policy(21, false, true)); // Unrelated Core commit.
    assert_eq!(
        lifecycle.request_download(policy(20, false, true), generation),
        Ok(())
    );
    assert!(matches!(
        lifecycle.state,
        DesktopUpdateState::Downloading { .. }
    ));

    let mut changed = self::lifecycle();
    let stale_generation = available(&mut changed, "2.0.0");
    changed.observe(policy(20, false, true)); // Actual channel change invalidates consent.
    let replacement = available(&mut changed, "2.1.0-beta.1");
    assert_eq!(
        changed.request_download(policy(19, false, false), stale_generation),
        Err(())
    );
    assert!(
        matches!(changed.state, DesktopUpdateState::Available { generation, .. } if generation == replacement)
    );
    assert!(changed.settings.include_prereleases);
}

#[test]
fn both_channel_directions_fence_late_success_empty_and_failure() {
    for initial in [false, true] {
        for auto_check in [false, true] {
            for result in [Ok(Some(candidate("2.0.0"))), Ok(None), Err(())] {
                let mut lifecycle = lifecycle();
                lifecycle.observe(policy(2, false, initial));
                assert!(lifecycle.begin_check());
                let (old, _) = lifecycle.claim_work().unwrap();
                lifecycle.observe(policy(3, auto_check, !initial));
                let expected = lifecycle.state.clone();
                assert!(!lifecycle.complete(old, Completion::Check(result), "1.0.0"));
                assert_eq!(lifecycle.state, expected);
                assert!(lifecycle.pending.is_none());
                if auto_check {
                    let (new, work) = lifecycle.claim_work().unwrap();
                    assert_ne!(old.generation, new.generation);
                    assert!(matches!(work, Work::Check(channel) if channel == !initial));
                } else {
                    assert_eq!(lifecycle.state, DesktopUpdateState::Idle);
                    assert!(lifecycle.claim_work().is_none());
                }
            }
        }
    }
}

#[test]
fn channel_change_clears_available_and_stale_consent_cannot_download_replacement() {
    for initial in [false, true] {
        let mut lifecycle = lifecycle();
        lifecycle.observe(policy(2, false, initial));
        let old = available(&mut lifecycle, "2.0.0");
        lifecycle.observe(policy(3, false, !initial));
        assert_eq!(lifecycle.state, DesktopUpdateState::Idle);
        assert!(lifecycle.pending.is_none());
        assert!(lifecycle.claim_work().is_none());
        assert!(lifecycle.begin_download(old).is_err());
        // Same version still represents a different candidate/check operation.
        let new = available(&mut lifecycle, "2.0.0");
        assert_ne!(old, new);
        assert!(lifecycle.begin_download(old).is_err());
        assert!(lifecycle.begin_download(new).is_ok());
        assert!(lifecycle.begin_download(new).is_err());
        let (_, work) = lifecycle.claim_work().unwrap();
        assert!(matches!(work, Work::Download(pending) if pending.update == "2.0.0"));
        assert!(lifecycle.claim_work().is_none());
    }
}

#[test]
fn policy_toggle_invalidates_unapproved_but_freezes_all_consented_phases() {
    for channel in [false, true] {
        let mut lifecycle = lifecycle();
        lifecycle.observe(policy(2, false, channel));
        let generation = available(&mut lifecycle, "2.0.0");
        lifecycle.begin_download(generation).unwrap();
        let (download, work) = lifecycle.claim_work().unwrap();
        let Work::Download(mut artifact) = work else {
            panic!("download");
        };
        lifecycle.observe(policy(3, true, !channel));
        assert_eq!(lifecycle.operation(), Some(download));
        assert!(!lifecycle.begin_check());
        assert!(lifecycle.begin_install().is_err());
        artifact.bytes = Some(vec![1, 2, 3]);
        lifecycle.complete(download, Completion::Download(Ok(artifact)), "1.0.0");
        lifecycle.observe(policy(4, false, channel));
        assert!(matches!(lifecycle.state, DesktopUpdateState::Ready { .. }));
        assert_eq!(
            lifecycle.pending.as_ref().unwrap().bytes,
            Some(vec![1, 2, 3])
        );
        lifecycle.begin_install().unwrap();
        assert!(lifecycle.begin_install().is_err());
        let (install, work) = lifecycle.claim_work().unwrap();
        assert!(
            matches!(work, Work::Install(pending) if pending.update == "2.0.0" && pending.bytes == Some(vec![1, 2, 3]))
        );
        lifecycle.observe(policy(5, true, !channel));
        assert_eq!(lifecycle.operation(), Some(install));
        assert!(!lifecycle.complete(install, Completion::Install(Err(())), "1.0.0"));
        assert_eq!(
            lifecycle.state,
            DesktopUpdateState::Failed {
                stage: DesktopUpdateFailureStage::Install
            }
        );
        assert!(lifecycle.begin_check());
        assert!(
            matches!(lifecycle.claim_work().unwrap().1, Work::Check(value) if value == !channel)
        );
    }
}

#[test]
fn auto_toggles_preserve_available_candidate_without_starting_work() {
    let mut lifecycle = lifecycle();
    lifecycle.observe(policy(2, true, false));
    let (check, _) = lifecycle.claim_work().unwrap();
    lifecycle.complete(
        check,
        Completion::Check(Ok(Some(candidate("2.0.0")))),
        "1.0.0",
    );
    let offered = lifecycle.state.clone();
    lifecycle.observe(policy(3, false, false));
    assert_eq!(lifecycle.state, offered);
    assert_eq!(lifecycle.pending.as_ref().unwrap().update, "2.0.0");
    lifecycle.observe(policy(4, true, false));
    assert_eq!(lifecycle.state, offered);
    assert_eq!(lifecycle.pending.as_ref().unwrap().update, "2.0.0");
    assert!(lifecycle.claim_work().is_none());
}

#[test]
fn auto_toggles_preserve_active_manual_check_and_enabling_idle_starts_once() {
    let mut lifecycle = lifecycle();
    assert!(lifecycle.begin_check());
    let (manual, _) = lifecycle.claim_work().unwrap();
    lifecycle.observe(policy(2, true, false));
    lifecycle.observe(policy(3, false, false));
    assert_eq!(lifecycle.operation(), Some(manual));
    assert!(lifecycle.claim_work().is_none());
    lifecycle.complete(manual, Completion::Check(Ok(None)), "1.0.0");
    assert_eq!(lifecycle.state, no_update_state("1.0.0".into()));
    lifecycle.observe(policy(4, true, false));
    let (automatic, _) = lifecycle.claim_work().unwrap();
    assert_ne!(automatic, manual);
    lifecycle.observe(policy(5, true, false));
    assert_eq!(lifecycle.operation(), Some(automatic));
    assert!(lifecycle.claim_work().is_none());
}

#[test]
fn completions_require_claim_generation_and_phase_and_cannot_replay() {
    let mut lifecycle = lifecycle();
    assert!(lifecycle.begin_check());
    let operation = lifecycle.operation().unwrap();
    lifecycle.complete(
        operation,
        Completion::Check(Ok(Some(candidate("2.0.0")))),
        "1.0.0",
    );
    assert_eq!(lifecycle.state, DesktopUpdateState::Checking); // Not claimed yet.
    let (operation, _) = lifecycle.claim_work().unwrap();
    lifecycle.complete(operation, Completion::Download(Err(())), "1.0.0");
    assert_eq!(lifecycle.state, DesktopUpdateState::Checking);
    lifecycle.complete(
        Operation {
            phase: Phase::Download,
            ..operation
        },
        Completion::Check(Err(())),
        "1.0.0",
    );
    assert_eq!(lifecycle.state, DesktopUpdateState::Checking);
    lifecycle.complete(
        operation,
        Completion::Check(Ok(Some(candidate("2.0.0")))),
        "1.0.0",
    );
    lifecycle.complete(operation, Completion::Check(Err(())), "1.0.0");
    assert!(matches!(
        lifecycle.state,
        DesktopUpdateState::Available { .. }
    ));
    lifecycle.begin_download(operation.generation).unwrap();
    let (download, _) = lifecycle.claim_work().unwrap();
    lifecycle.complete(operation, Completion::Check(Err(())), "1.0.0");
    assert!(matches!(
        lifecycle.state,
        DesktopUpdateState::Downloading { .. }
    ));
    let mut pending = candidate("2.0.0");
    pending.bytes = Some(vec![1]);
    lifecycle.complete(download, Completion::Download(Ok(pending)), "1.0.0");
    lifecycle.begin_install().unwrap();
    let (install, _) = lifecycle.claim_work().unwrap();
    assert!(lifecycle.complete(install, Completion::Install(Ok(())), "1.0.0"));
    assert!(!lifecycle.complete(install, Completion::Install(Ok(())), "1.0.0"));
}

#[test]
fn state_events_follow_atomic_transition_order() {
    let shared = Arc::new(Shared::<String>::new(DesktopUpdateState::Idle));
    let events = Arc::new(Mutex::new(Vec::new()));
    std::thread::scope(|threads| {
        let (entered_tx, entered_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let first = shared.clone();
        let first_events = events.clone();
        threads.spawn(move || {
            first.transition(
                |state| {
                    entered_tx.send(()).unwrap();
                    release_rx.recv().unwrap();
                    first_events.lock().unwrap().push(state);
                },
                |lifecycle| lifecycle.begin_check(),
            )
        });
        entered_rx.recv().unwrap();
        let second = shared.clone();
        let second_events = events.clone();
        threads.spawn(move || {
            second.transition(
                |state| second_events.lock().unwrap().push(state),
                |lifecycle| {
                    lifecycle.observe(policy(1, false, true));
                },
            )
        });
        release_tx.send(()).unwrap();
    });
    assert_eq!(
        *events.lock().unwrap(),
        vec![DesktopUpdateState::Checking, DesktopUpdateState::Idle]
    );
}

#[test]
fn wire_state_includes_only_available_candidate_generation() {
    assert_eq!(
        serde_json::to_value(DesktopUpdateState::Available {
            version: "2.0.0".into(),
            generation: 7
        })
        .unwrap(),
        serde_json::json!({"kind": "available", "version": "2.0.0", "generation": 7})
    );
    for (state, kind) in [
        (
            DesktopUpdateState::Ready {
                version: "2.0.0".into(),
            },
            "ready",
        ),
        (
            DesktopUpdateState::Downloading {
                version: "2.0.0".into(),
            },
            "downloading",
        ),
        (
            DesktopUpdateState::Installing {
                version: "2.0.0".into(),
            },
            "installing",
        ),
        (no_update_state("2.0.0".into()), "up_to_date"),
    ] {
        assert_eq!(
            serde_json::to_value(state).unwrap(),
            serde_json::json!({"kind": kind, "version": "2.0.0"})
        );
    }
    assert_eq!(
        serde_json::to_value(DesktopUpdateState::Failed {
            stage: DesktopUpdateFailureStage::DownloadOrVerify
        })
        .unwrap(),
        serde_json::json!({"kind": "failed", "stage": "download_or_verify"})
    );
}

#[cfg(target_os = "macos")]
#[test]
fn semver_candidate_selection_handles_prerelease_ordering() {
    assert!(candidate_version_is_newer("1.2.0-beta.2", "1.2.0-beta.10"));
    assert!(candidate_version_is_newer("1.2.0-beta.10", "1.2.0"));
    assert!(!candidate_version_is_newer("1.2.0", "1.2.0+build.1"));
    assert!(!candidate_version_is_newer("1.3.0", "1.2.0-rc.1"));
}

struct DropSignal(Option<oneshot::Sender<()>>);
impl Drop for DropSignal {
    fn drop(&mut self) {
        if let Some(sender) = self.0.take() {
            let _ = sender.send(());
        }
    }
}
struct Started {
    work: Work<String>,
    complete: oneshot::Sender<Completion<String>>,
    dropped: oneshot::Receiver<()>,
}
struct FakeBackend {
    started: mpsc::UnboundedSender<Started>,
    events: mpsc::UnboundedSender<DesktopUpdateState>,
    restarts: Arc<AtomicUsize>,
    restarted: watch::Sender<bool>,
}
impl Backend<String> for FakeBackend {
    fn start(&self, work: Work<String>) -> UpdateFuture<'static, Completion<String>> {
        let (tx, rx) = oneshot::channel();
        let (dropped_tx, dropped_rx) = oneshot::channel();
        let guard = DropSignal(Some(dropped_tx));
        self.started
            .send(Started {
                work,
                complete: tx,
                dropped: dropped_rx,
            })
            .ok()
            .unwrap();
        Box::pin(async move {
            let _guard = guard;
            rx.await.expect("test controls completion")
        })
    }
    fn emit(&self, state: DesktopUpdateState) {
        self.events.send(state).unwrap();
    }
    fn current_version(&self) -> &str {
        "1.0.0"
    }
    fn restart(&self) {
        self.restarts.fetch_add(1, Ordering::SeqCst);
        self.restarted.send_replace(true);
    }
}
struct FakeSource(watch::Receiver<Option<PolicySnapshot>>);
impl SettingsSource for FakeSource {
    fn next(&mut self) -> UpdateFuture<'_, Option<PolicySnapshot>> {
        Box::pin(async move {
            self.0.changed().await.ok()?;
            self.0.borrow_and_update().clone()
        })
    }
}
struct Harness {
    shared: Arc<Shared<String>>,
    policies: watch::Sender<Option<PolicySnapshot>>,
    started: mpsc::UnboundedReceiver<Started>,
    events: mpsc::UnboundedReceiver<DesktopUpdateState>,
    restarts: Arc<AtomicUsize>,
    restarted: watch::Receiver<bool>,
}
impl Harness {
    fn new() -> Self {
        let shared = Arc::new(Shared::new(DesktopUpdateState::Idle));
        shared.transition(
            |_| {},
            |lifecycle| {
                lifecycle.observe(policy(1, true, false));
            },
        );
        let (policies, source) = watch::channel(None);
        let (started_tx, started) = mpsc::unbounded_channel();
        let (events_tx, events) = mpsc::unbounded_channel();
        let restarts = Arc::new(AtomicUsize::new(0));
        let (restarted_tx, restarted) = watch::channel(false);
        start_owner(
            shared.clone(),
            FakeBackend {
                started: started_tx,
                events: events_tx,
                restarts: restarts.clone(),
                restarted: restarted_tx,
            },
            FakeSource(source),
        );
        Self {
            shared,
            policies,
            started,
            events,
            restarts,
            restarted,
        }
    }
    async fn started(&mut self) -> Started {
        bounded(self.started.recv()).await.unwrap()
    }
    async fn state(&mut self, expected: DesktopUpdateState) {
        while self.shared.state() != expected {
            bounded(self.events.recv()).await.unwrap();
        }
    }
}
async fn bounded<T>(future: impl Future<Output = T>) -> T {
    tokio::time::timeout(std::time::Duration::from_secs(5), future)
        .await
        .expect("updater test deadline")
}

#[tokio::test]
async fn watcher_observes_channel_changes_during_check_and_owns_replacement() {
    let mut harness = Harness::new();
    let mut old = harness.started().await;
    assert!(matches!(old.work, Work::Check(false)));
    harness.policies.send_replace(Some(policy(2, true, true)));
    bounded(old.dropped).await.unwrap();
    // The guard may signal before the async frame drops its receiver. Wait for
    // actual receiver closure before asserting that stale completion is rejected.
    bounded(old.complete.closed()).await;
    assert!(
        old.complete
            .send(Completion::Check(Ok(Some(candidate("9.0.0")))))
            .is_err()
    );
    let replacement = harness.started().await;
    assert!(matches!(replacement.work, Work::Check(true)));
    replacement
        .complete
        .send(Completion::Check(Ok(None)))
        .ok()
        .unwrap();
    harness.state(no_update_state("1.0.0".into())).await;
    assert!(harness.started.try_recv().is_err());
    bounded(harness.shared.shutdown()).await;
    assert!(harness.shared.lifecycle.lock().unwrap().owner.is_none());
}

#[tokio::test]
async fn disabled_auto_channel_change_cancels_check_without_network_replacement() {
    let mut harness = Harness::new();
    let mut old = harness.started().await;
    harness.policies.send_replace(Some(policy(2, false, true)));
    bounded(old.dropped).await.unwrap();
    // DropSignal alone is not a barrier for the completion receiver's drop.
    bounded(old.complete.closed()).await;
    harness.state(DesktopUpdateState::Idle).await;
    assert!(harness.started.try_recv().is_err());
    assert!(old.complete.send(Completion::Check(Err(()))).is_err());
    bounded(harness.shared.shutdown()).await;
}

#[tokio::test]
async fn shutdown_cancels_check_and_download_and_joins_owner() {
    for download in [false, true] {
        let mut harness = Harness::new();
        let check = harness.started().await;
        let active = if download {
            check
                .complete
                .send(Completion::Check(Ok(Some(candidate("2.0.0")))))
                .ok()
                .unwrap();
            let generation = harness.shared.lifecycle.lock().unwrap().generation;
            harness
                .state(DesktopUpdateState::Available {
                    version: "2.0.0".into(),
                    generation,
                })
                .await;
            harness
                .shared
                .transition(|_| {}, |lifecycle| lifecycle.begin_download(generation))
                .unwrap();
            harness.started().await
        } else {
            check
        };
        bounded(harness.shared.shutdown()).await;
        bounded(active.dropped).await.unwrap();
        assert!(harness.shared.lifecycle.lock().unwrap().owner.is_none());
        assert!(harness.shared.lifecycle.lock().unwrap().pending.is_none());
        assert_eq!(harness.restarts.load(Ordering::SeqCst), 0);
    }
}

#[tokio::test]
async fn shutdown_joins_installer_even_if_first_shutdown_waiter_is_cancelled() {
    let mut harness = Harness::new();
    let check = harness.started().await;
    let generation = harness.shared.lifecycle.lock().unwrap().generation;
    check
        .complete
        .send(Completion::Check(Ok(Some(candidate("2.0.0")))))
        .ok()
        .unwrap();
    harness
        .state(DesktopUpdateState::Available {
            version: "2.0.0".into(),
            generation,
        })
        .await;
    harness
        .shared
        .transition(|_| {}, |lifecycle| lifecycle.begin_download(generation))
        .unwrap();
    let download = harness.started().await;
    let mut artifact = candidate("2.0.0");
    artifact.bytes = Some(vec![1, 2, 3]);
    download
        .complete
        .send(Completion::Download(Ok(artifact)))
        .ok()
        .unwrap();
    harness
        .state(DesktopUpdateState::Ready {
            version: "2.0.0".into(),
        })
        .await;
    harness
        .shared
        .transition(|_| {}, |lifecycle| lifecycle.begin_install())
        .unwrap();
    let mut install = harness.started().await;
    assert!(matches!(install.work, Work::Install(_)));
    // Poll shutdown once to establish cancellation and prove it awaits the installer.
    let mut shutdown = Box::pin(harness.shared.shutdown());
    assert!(
        std::future::poll_fn(|cx| std::task::Poll::Ready(shutdown.as_mut().poll(cx).is_pending()))
            .await
    );
    drop(shutdown);
    assert!(install.dropped.try_recv().is_err());
    assert!(harness.shared.lifecycle.lock().unwrap().owner.is_some());
    install
        .complete
        .send(Completion::Install(Ok(())))
        .ok()
        .unwrap();
    let second = harness.shared.clone();
    bounded(async {
        tokio::join!(harness.shared.shutdown(), second.shutdown());
    })
    .await;
    bounded(install.dropped).await.unwrap();
    assert!(harness.shared.lifecycle.lock().unwrap().owner.is_none());
    assert_eq!(harness.restarts.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn source_close_cancels_owned_work_and_rejects_future_admission() {
    let mut harness = Harness::new();
    let check = harness.started().await;
    drop(harness.policies);
    bounded(check.dropped).await.unwrap();
    assert!(
        !harness
            .shared
            .transition(|_| {}, |lifecycle| lifecycle.begin_check())
    );
    bounded(harness.shared.shutdown()).await;
}

#[tokio::test]
async fn successful_install_requests_restart_once_and_owner_can_be_joined() {
    let mut harness = Harness::new();
    let check = harness.started().await;
    let generation = harness.shared.lifecycle.lock().unwrap().generation;
    check
        .complete
        .send(Completion::Check(Ok(Some(candidate("2.0.0")))))
        .ok()
        .unwrap();
    harness
        .state(DesktopUpdateState::Available {
            version: "2.0.0".into(),
            generation,
        })
        .await;
    harness
        .shared
        .transition(|_| {}, |lifecycle| lifecycle.begin_download(generation))
        .unwrap();
    let download = harness.started().await;
    let mut artifact = candidate("2.0.0");
    artifact.bytes = Some(vec![1]);
    download
        .complete
        .send(Completion::Download(Ok(artifact)))
        .ok()
        .unwrap();
    harness
        .state(DesktopUpdateState::Ready {
            version: "2.0.0".into(),
        })
        .await;
    harness
        .shared
        .transition(|_| {}, |lifecycle| lifecycle.begin_install())
        .unwrap();
    let install = harness.started().await;
    install
        .complete
        .send(Completion::Install(Ok(())))
        .ok()
        .unwrap();
    bounded(harness.restarted.changed()).await.unwrap();
    assert!(*harness.restarted.borrow());
    bounded(harness.shared.shutdown()).await;
    assert_eq!(harness.restarts.load(Ordering::SeqCst), 1);
    assert!(harness.shared.lifecycle.lock().unwrap().owner.is_none());
}
