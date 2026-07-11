use super::*;
use koushi_state::NativeAttentionSoundOutcome;

trait NativeAttentionSoundBackend {
    fn play(&self) -> NativeAttentionSoundOutcome;
}

struct PlatformNativeAttentionSoundBackend;

#[tauri::command]
pub(crate) async fn play_native_attention_sound(
    state: State<'_, CoreRuntimeState>,
) -> Result<NativeAttentionSoundOutcome, &'static str> {
    let connection = state.runtime.attach();
    let start_request = connection.next_request_id();
    let dispatch_id = start_request.sequence;
    if connection
        .command(CoreCommand::App(AppCommand::StartNativeAttentionDispatch {
            request_id: start_request,
            dispatch_id,
        }))
        .await
        .is_err()
    {
        return Ok(NativeAttentionSoundOutcome::Failed);
    }
    let outcome = PlatformNativeAttentionSoundBackend.play();
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
    Ok(outcome)
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
    use std::cell::Cell;

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
}
