use super::*;
use koushi_state::{NativeAttentionDispatchId, NativeAttentionSoundOutcome};

trait NativeAttentionSoundBackend {
    fn play(&self) -> NativeAttentionSoundOutcome;
}

struct PlatformNativeAttentionSoundBackend;

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
    let connection = runtime.attach();
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
    let outcome = backend.play();
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
    fn play(&self) -> NativeAttentionSoundOutcome {
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
    fn play(&self) -> NativeAttentionSoundOutcome {
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
    fn play(&self) -> NativeAttentionSoundOutcome {
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
    use std::time::Duration;

    struct FakeBackend {
        calls: Cell<u32>,
        outcome: NativeAttentionSoundOutcome,
    }

    impl NativeAttentionSoundBackend for FakeBackend {
        fn play(&self) -> NativeAttentionSoundOutcome {
            self.calls.set(self.calls.get() + 1);
            self.outcome
        }
    }

    #[test]
    fn available_backend_is_invoked_once_and_returns_typed_outcome() {
        let backend = FakeBackend {
            calls: Cell::new(0),
            outcome: NativeAttentionSoundOutcome::Played,
        };
        assert_eq!(backend.play(), NativeAttentionSoundOutcome::Played);
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
    #[test]
    fn actual_linux_platform_adapter_is_explicitly_unsupported() {
        assert_eq!(
            PlatformNativeAttentionSoundBackend.play(),
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
}
