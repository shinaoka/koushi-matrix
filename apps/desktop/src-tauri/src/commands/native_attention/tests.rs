use super::*;
use koushi_core::{CoreConnection, CoreRuntime, executor};
use koushi_state::{
    AppAction, CurrentDeviceTrustState, NativeAttentionCandidate, NativeAttentionCapabilities,
    NativeAttentionDispatchState, NativeAttentionState, NativeAttentionSummary, RoomAttentionKind,
    SessionInfo,
};

async fn seed_ready(runtime: &CoreRuntime, connection: &mut CoreConnection) {
    runtime
        .inject_actions(vec![
            AppAction::AppStarted,
            AppAction::RestoreSessionSucceeded(SessionInfo {
                homeserver: "https://example.invalid".to_owned(),
                user_id: "@me:example.invalid".to_owned(),
                device_id: "DEVICE".to_owned(),
                authentication_method: koushi_state::SessionAuthenticationMethod::Unknown,
            }),
            AppAction::CurrentDeviceTrustChanged(CurrentDeviceTrustState::Verified),
        ])
        .await;
    executor::timeout(Duration::from_secs(1), async {
        loop {
            if matches!(
                connection.snapshot().session,
                koushi_state::SessionState::Ready(_)
            ) {
                break;
            }
            connection
                .next_versioned_snapshot()
                .await
                .expect("snapshot stream");
        }
    })
    .await
    .expect("canonical Ready fixture must reach reducer");
}
use std::cell::Cell;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;
use tokio::sync::Notify;

struct FakeBackend {
    calls: Cell<u32>,
    outcome: NativeAttentionSoundOutcome,
}

struct ControlledBackend {
    calls: AtomicU32,
    entered: Notify,
    release: Notify,
}

impl NativeAttentionSoundBackend for ControlledBackend {
    async fn play(&self) -> NativeAttentionSoundOutcome {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.entered.notify_one();
        self.release.notified().await;
        NativeAttentionSoundOutcome::Played
    }
}

impl NativeAttentionSoundBackend for FakeBackend {
    async fn play(&self) -> NativeAttentionSoundOutcome {
        self.calls.set(self.calls.get() + 1);
        self.outcome
    }
}

#[tokio::test]
async fn available_backend_is_invoked_once_and_returns_typed_outcome() {
    let backend = FakeBackend {
        calls: Cell::new(0),
        outcome: NativeAttentionSoundOutcome::Played,
    };
    assert_eq!(backend.play().await, NativeAttentionSoundOutcome::Played);
    assert_eq!(backend.calls.get(), 1);
}

#[test]
fn failure_and_unsupported_outcomes_are_fixed_and_private_safe() {
    assert_eq!(
        serde_json::to_value(NativeAttentionSoundOutcome::Failed).unwrap(),
        "failed"
    );
    assert_eq!(
        serde_json::to_value(NativeAttentionSoundOutcome::Unsupported).unwrap(),
        "unsupported"
    );
}

#[test]
fn native_badge_labels_clear_zero_and_preserve_positive_counts() {
    assert_eq!(native_attention_badge_label(None), None);
    assert_eq!(native_attention_badge_label(Some(0)), None);
    assert_eq!(native_attention_badge_label(Some(7)), Some("7".to_owned()));
}

#[test]
fn native_badge_outcomes_are_typed_and_private_safe() {
    assert_eq!(
        serde_json::to_value(NativeAttentionBadgeOutcome::Applied).unwrap(),
        "applied"
    );
    assert_eq!(
        serde_json::to_value(NativeAttentionBadgeOutcome::Unsupported).unwrap(),
        "unsupported"
    );
    assert_eq!(
        serde_json::to_value(NativeAttentionBadgeOutcome::Mismatch).unwrap(),
        "mismatch"
    );
}

#[cfg(target_os = "macos")]
#[test]
fn macos_alert_sound_source_resolution_handles_all_setting_rows() {
    assert_eq!(
        resolve_macos_alert_sound_source(None),
        MacosAlertSoundSource::Named(MACOS_DEFAULT_ALERT_SOUND_NAME.to_owned())
    );
    assert_eq!(
        resolve_macos_alert_sound_source(Some("")),
        MacosAlertSoundSource::Muted
    );
    assert_eq!(
        resolve_macos_alert_sound_source(Some("/System/Library/Sounds/Ping.aiff")),
        MacosAlertSoundSource::Path("/System/Library/Sounds/Ping.aiff".to_owned())
    );
    assert_eq!(
        resolve_macos_alert_sound_source(Some("Ping")),
        MacosAlertSoundSource::Named("Ping".to_owned())
    );
    // Fail-closed: a malformed value is treated as a name, which fails to
    // load and maps to `Failed` instead of silently playing.
    assert_eq!(
        resolve_macos_alert_sound_source(Some("  ")),
        MacosAlertSoundSource::Named("  ".to_owned())
    );
}

#[cfg(target_os = "macos")]
#[test]
fn macos_sound_outcome_maps_loaded_and_started_to_truthful_result() {
    assert_eq!(
        macos_sound_outcome(false, true),
        NativeAttentionSoundOutcome::Failed
    );
    assert_eq!(
        macos_sound_outcome(true, false),
        NativeAttentionSoundOutcome::Failed
    );
    assert_eq!(
        macos_sound_outcome(true, true),
        NativeAttentionSoundOutcome::Played
    );
}

#[cfg(target_os = "macos")]
#[test]
fn macos_started_sound_is_retained_for_asynchronous_playback() {
    let mut slot = None;
    assert_eq!(
        retain_started_macos_sound(&mut slot, "rejected", false),
        NativeAttentionSoundOutcome::Failed
    );
    assert_eq!(slot, None);

    assert_eq!(
        retain_started_macos_sound(&mut slot, "accepted", true),
        NativeAttentionSoundOutcome::Played
    );
    assert_eq!(slot, Some("accepted"));
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn actual_linux_platform_adapter_is_explicitly_unsupported() {
    assert_eq!(
        PlatformNativeAttentionSoundBackend.play().await,
        NativeAttentionSoundOutcome::Unsupported
    );
}

#[tokio::test]
async fn command_helper_crosses_core_runtime_and_settles_the_matching_dispatch() {
    let runtime = CoreRuntime::start();
    let mut observer = runtime.attach();
    seed_ready(&runtime, &mut observer).await;
    let seed_request = observer.next_request_id();
    observer
        .command(CoreCommand::App(AppCommand::UpdateNativeAttentionState {
            request_id: seed_request,
            attention: NativeAttentionState {
                summary: NativeAttentionSummary {
                    unread_count: 1,
                    highlight_count: 0,
                    badge_count: 1,
                    candidate: Some(NativeAttentionCandidate {
                        room_display_name: "Room".to_owned(),
                        kind: RoomAttentionKind::Message,
                        unread_count: 1,
                        highlight_count: 0,
                    }),
                    capabilities: NativeAttentionCapabilities::default(),
                },
                dispatch: NativeAttentionDispatchState::Idle,
                notification: None,
            },
        }))
        .await
        .expect("seed native attention badge through core command");
    executor::timeout(Duration::from_secs(1), async {
        loop {
            if observer.snapshot().native_attention.summary.badge_count == 1 {
                break;
            }
            observer
                .next_versioned_snapshot()
                .await
                .expect("snapshot stream");
        }
    })
    .await
    .expect("seed badge must reach reducer before dispatch");

    let backend = FakeBackend {
        calls: Cell::new(0),
        outcome: NativeAttentionSoundOutcome::Played,
    };
    let (outcome, dispatch_id) = dispatch_native_attention_sound(&runtime, &backend).await;
    assert_eq!(outcome, NativeAttentionSoundOutcome::Played);
    let dispatch_id = dispatch_id.expect("submitted dispatch id");

    let snapshot = executor::timeout(Duration::from_secs(1), async {
        loop {
            let snapshot = observer.snapshot();
            if matches!(
                snapshot.native_attention.dispatch,
                NativeAttentionDispatchState::Delivered { .. }
            ) {
                return snapshot;
            }
            observer
                .next_versioned_snapshot()
                .await
                .expect("snapshot stream");
        }
    })
    .await
    .expect("matching dispatch should settle through the runtime reducer");

    assert_eq!(
        snapshot.native_attention.dispatch,
        NativeAttentionDispatchState::Delivered { dispatch_id }
    );
    assert_eq!(backend.calls.get(), 1);
}

#[tokio::test]
async fn concurrent_command_helpers_admit_only_one_native_backend_call() {
    let runtime = CoreRuntime::start();
    let mut seeder = runtime.attach();
    seed_ready(&runtime, &mut seeder).await;
    let request_id = seeder.next_request_id();
    seeder
        .command(CoreCommand::App(AppCommand::UpdateNativeAttentionState {
            request_id,
            attention: NativeAttentionState {
                summary: NativeAttentionSummary {
                    unread_count: 1,
                    highlight_count: 0,
                    badge_count: 1,
                    candidate: Some(NativeAttentionCandidate {
                        room_display_name: "Room".to_owned(),
                        kind: RoomAttentionKind::Message,
                        unread_count: 1,
                        highlight_count: 0,
                    }),
                    capabilities: NativeAttentionCapabilities::default(),
                },
                dispatch: NativeAttentionDispatchState::Idle,
                notification: None,
            },
        }))
        .await
        .expect("seed candidate");
    executor::timeout(Duration::from_secs(1), async {
        loop {
            if seeder
                .snapshot()
                .native_attention
                .summary
                .candidate
                .is_some()
            {
                break;
            }
            seeder
                .next_versioned_snapshot()
                .await
                .expect("snapshot stream");
        }
    })
    .await
    .expect("seed candidate must reach reducer before concurrent dispatch");
    let backend = ControlledBackend {
        calls: AtomicU32::new(0),
        entered: Notify::new(),
        release: Notify::new(),
    };

    let lock = tokio::sync::Mutex::new(());
    let first = dispatch_native_attention_sound_with_lock(&runtime, &backend, &lock);
    let second = dispatch_native_attention_sound_with_lock(&runtime, &backend, &lock);
    let release = async {
        backend.entered.notified().await;
        backend.release.notify_one();
    };
    let (first, second, ()) = tokio::join!(first, second, release);

    assert_eq!(first.0, NativeAttentionSoundOutcome::Played);
    assert_eq!(second, (NativeAttentionSoundOutcome::Skipped, None));
    assert_eq!(backend.calls.load(Ordering::SeqCst), 1);
}
