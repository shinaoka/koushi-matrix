use super::*;
use koushi_state::{NativeAttentionDispatchId, NativeAttentionSoundOutcome};

trait NativeAttentionSoundBackend {
    async fn play(&self) -> NativeAttentionSoundOutcome;
}

struct PlatformNativeAttentionSoundBackend;
static NATIVE_ATTENTION_SOUND_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

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
    use koushi_core::{CoreEvent, CoreRuntime, executor};
    use koushi_state::{
        NativeAttentionCandidate, NativeAttentionCapabilities, NativeAttentionDispatchState,
        NativeAttentionState, NativeAttentionSummary, RoomAttentionKind,
    };
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
                },
            }))
            .await
            .expect("seed native attention candidate through core command");

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
        let seeder = runtime.attach();
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
