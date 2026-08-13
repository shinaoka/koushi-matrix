use super::*;
use koushi_state::{NativeAttentionDispatchId, NativeAttentionSoundOutcome};

const NATIVE_BADGE_APPLY_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum NativeAttentionBadgeOutcome {
    Applied,
    Unsupported,
    Mismatch,
}

trait NativeAttentionSoundBackend {
    async fn play(&self) -> NativeAttentionSoundOutcome;
}

struct PlatformNativeAttentionSoundBackend;
static NATIVE_ATTENTION_SOUND_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

pub(crate) fn build_observe_native_window_focus_command(
    request_id: RequestId,
    focused: bool,
    observation_generation: u64,
) -> CoreCommand {
    CoreCommand::App(AppCommand::ObserveNativeWindowFocus {
        request_id,
        focused,
        observation_generation,
    })
}

#[tauri::command]
pub(crate) async fn play_native_attention_sound(
    state: State<'_, CoreRuntimeState>,
) -> Result<NativeAttentionSoundOutcome, &'static str> {
    Ok(
        dispatch_native_attention_sound(&state.runtime, &PlatformNativeAttentionSoundBackend)
            .await
            .0,
    )
}

/// Apply the Rust-owned unread count at the native application boundary.
///
/// On macOS this bypasses the webview window bridge and updates `NSDockTile`
/// on the AppKit main thread. The value is read back before the command settles,
/// so `Applied` means the native backend accepted the expected label; it does
/// not claim that the user's system badge preference made it visually visible.
#[tauri::command]
pub(crate) async fn set_native_attention_badge(
    app: AppHandle,
    count: Option<u64>,
) -> Result<NativeAttentionBadgeOutcome, &'static str> {
    let count = count.filter(|count| *count > 0);
    record(
        DiagnosticEvent::new(
            DiagnosticLevel::Info,
            "desktop.native_badge",
            "apply_requested",
        )
        .field(DiagnosticField::count("count", count.unwrap_or(0))),
    );

    let outcome = apply_native_attention_badge(&app, count).await?;
    record(
        DiagnosticEvent::new(
            if outcome == NativeAttentionBadgeOutcome::Applied {
                DiagnosticLevel::Info
            } else {
                DiagnosticLevel::Warn
            },
            "desktop.native_badge",
            "apply_settled",
        )
        .field(DiagnosticField::token(
            "outcome",
            native_attention_badge_outcome_token(outcome),
        ))
        .field(DiagnosticField::count("count", count.unwrap_or(0))),
    );
    Ok(outcome)
}

#[cfg(target_os = "macos")]
async fn apply_native_attention_badge(
    app: &AppHandle,
    count: Option<u64>,
) -> Result<NativeAttentionBadgeOutcome, &'static str> {
    let expected_label = native_attention_badge_label(count);
    let label_for_main_thread = expected_label.clone();
    let (sender, receiver) = tokio::sync::oneshot::channel();
    app.run_on_main_thread(move || {
        let _ = sender.send(apply_macos_dock_badge_now(label_for_main_thread));
    })
    .map_err(|_| "native badge main-thread dispatch failed")?;

    tokio::time::timeout(NATIVE_BADGE_APPLY_TIMEOUT, receiver)
        .await
        .map_err(|_| "native badge main-thread dispatch timed out")?
        .map_err(|_| "native badge main-thread result was dropped")
}

#[cfg(target_os = "macos")]
fn apply_macos_dock_badge_now(expected_label: Option<String>) -> NativeAttentionBadgeOutcome {
    use objc2_foundation::NSString;

    let Some(main_thread_marker) = objc2::MainThreadMarker::new() else {
        return NativeAttentionBadgeOutcome::Unsupported;
    };
    let application = objc2_app_kit::NSApplication::sharedApplication(main_thread_marker);
    let dock_tile = application.dockTile();
    let native_label = expected_label.as_deref().map(NSString::from_str);

    dock_tile.setShowsApplicationBadge(true);
    dock_tile.setBadgeLabel(native_label.as_deref());
    dock_tile.display();

    let observed_label = dock_tile.badgeLabel().map(|label| label.to_string());
    if observed_label == expected_label {
        NativeAttentionBadgeOutcome::Applied
    } else {
        NativeAttentionBadgeOutcome::Mismatch
    }
}

#[cfg(not(target_os = "macos"))]
async fn apply_native_attention_badge(
    app: &AppHandle,
    count: Option<u64>,
) -> Result<NativeAttentionBadgeOutcome, &'static str> {
    let Some(window) = app.get_webview_window("main") else {
        return Err("native badge main window unavailable");
    };
    let count = count.map(|count| i64::try_from(count).unwrap_or(i64::MAX));
    window
        .set_badge_count(count)
        .map_err(|_| "native badge backend failed")?;
    Ok(NativeAttentionBadgeOutcome::Applied)
}

fn native_attention_badge_label(count: Option<u64>) -> Option<String> {
    count
        .filter(|count| *count > 0)
        .map(|count| count.to_string())
}

fn native_attention_badge_outcome_token(outcome: NativeAttentionBadgeOutcome) -> &'static str {
    match outcome {
        NativeAttentionBadgeOutcome::Applied => "applied",
        NativeAttentionBadgeOutcome::Unsupported => "unsupported",
        NativeAttentionBadgeOutcome::Mismatch => "mismatch",
    }
}

async fn dispatch_native_attention_sound(
    runtime: &koushi_core::CoreRuntime,
    backend: &impl NativeAttentionSoundBackend,
) -> (
    NativeAttentionSoundOutcome,
    Option<NativeAttentionDispatchId>,
) {
    dispatch_native_attention_sound_with_lock(runtime, backend, &NATIVE_ATTENTION_SOUND_LOCK).await
}

async fn dispatch_native_attention_sound_with_lock(
    runtime: &koushi_core::CoreRuntime,
    backend: &impl NativeAttentionSoundBackend,
    lock: &tokio::sync::Mutex<()>,
) -> (
    NativeAttentionSoundOutcome,
    Option<NativeAttentionDispatchId>,
) {
    let Ok(_guard) = lock.try_lock() else {
        return (NativeAttentionSoundOutcome::Skipped, None);
    };
    let mut connection = runtime.attach();
    let start_request = connection.next_request_id();
    let dispatch_id =
        NativeAttentionDispatchId::new(start_request.connection_id.0, start_request.sequence);
    if connection
        .command(CoreCommand::App(AppCommand::StartNativeAttentionDispatch {
            request_id: start_request,
            dispatch_id,
        }))
        .await
        .is_err()
    {
        return (NativeAttentionSoundOutcome::Failed, None);
    }
    let admitted = koushi_core::executor::timeout(std::time::Duration::from_secs(2), async {
        loop {
            if let CoreEvent::NativeAttention(
                koushi_core::NativeAttentionEvent::DispatchAdmission {
                    dispatch_id: observed,
                    accepted,
                },
            ) = connection.recv_event().await.ok()?
                && observed == dispatch_id
            {
                return Some(accepted);
            }
        }
    })
    .await
    .ok()
    .flatten()
    .unwrap_or(false);
    if !admitted {
        return (NativeAttentionSoundOutcome::Skipped, Some(dispatch_id));
    }
    let outcome = backend.play().await;
    let settle_request = connection.next_request_id();
    let _ = connection
        .command(CoreCommand::App(
            AppCommand::SettleNativeAttentionDispatch {
                request_id: settle_request,
                dispatch_id,
                outcome,
            },
        ))
        .await;
    (outcome, Some(dispatch_id))
}

#[cfg(target_os = "macos")]
impl NativeAttentionSoundBackend for PlatformNativeAttentionSoundBackend {
    async fn play(&self) -> NativeAttentionSoundOutcome {
        #[link(name = "AudioToolbox", kind = "framework")]
        unsafe extern "C" {
            fn AudioServicesPlaySystemSound(sound_id: u32);
        }
        // The system alert is an OS-owned native sound; no third-party asset is bundled.
        unsafe { AudioServicesPlaySystemSound(1007) };
        NativeAttentionSoundOutcome::Played
    }
}

#[cfg(target_os = "windows")]
impl NativeAttentionSoundBackend for PlatformNativeAttentionSoundBackend {
    async fn play(&self) -> NativeAttentionSoundOutcome {
        #[link(name = "user32")]
        unsafe extern "system" {
            fn MessageBeep(kind: u32) -> i32;
        }
        if unsafe { MessageBeep(u32::MAX) } == 0 {
            NativeAttentionSoundOutcome::Failed
        } else {
            NativeAttentionSoundOutcome::Played
        }
    }
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
impl NativeAttentionSoundBackend for PlatformNativeAttentionSoundBackend {
    async fn play(&self) -> NativeAttentionSoundOutcome {
        NativeAttentionSoundOutcome::Unsupported
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use koushi_core::{CoreConnection, CoreEvent, CoreRuntime, executor};
    use koushi_state::{
        AppAction, CurrentDeviceTrustState, NativeAttentionCandidate, NativeAttentionCapabilities,
        NativeAttentionDispatchState, NativeAttentionState, NativeAttentionSummary,
        RoomAttentionKind, SessionInfo,
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
                if matches!(connection.recv_event().await, Ok(CoreEvent::StateChanged(snapshot)) if matches!(snapshot.session, koushi_state::SessionState::Ready(_))) {
                    break;
                }
            }
        }).await.expect("canonical Ready fixture must reach reducer");
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
                        candidate: None,
                        capabilities: NativeAttentionCapabilities::default(),
                    },
                    dispatch: NativeAttentionDispatchState::Idle,
                },
            }))
            .await
            .expect("seed native attention badge through core command");
        executor::timeout(Duration::from_secs(1), async {
            loop {
                if matches!(observer.recv_event().await, Ok(CoreEvent::StateChanged(snapshot)) if snapshot.native_attention.summary.badge_count == 1) {
                    break;
                }
            }
        }).await.expect("seed badge must reach reducer before dispatch");

        let backend = FakeBackend {
            calls: Cell::new(0),
            outcome: NativeAttentionSoundOutcome::Played,
        };
        let (outcome, dispatch_id) = dispatch_native_attention_sound(&runtime, &backend).await;
        assert_eq!(outcome, NativeAttentionSoundOutcome::Played);
        let dispatch_id = dispatch_id.expect("submitted dispatch id");

        let snapshot = executor::timeout(Duration::from_secs(1), async {
            loop {
                match observer.recv_event().await.expect("core event") {
                    CoreEvent::StateChanged(snapshot)
                        if matches!(
                            snapshot.native_attention.dispatch,
                            NativeAttentionDispatchState::Delivered { .. }
                        ) =>
                    {
                        return snapshot;
                    }
                    _ => continue,
                }
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
                },
            }))
            .await
            .expect("seed candidate");
        executor::timeout(Duration::from_secs(1), async {
            loop {
                if matches!(seeder.recv_event().await, Ok(CoreEvent::StateChanged(snapshot)) if snapshot.native_attention.summary.candidate.is_some()) {
                    break;
                }
            }
        }).await.expect("seed candidate must reach reducer before concurrent dispatch");
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
}
