use super::*;
use koushi_state::{NativeAttentionDispatchId, NativeAttentionSoundOutcome};

pub(crate) mod notification;

#[cfg(target_os = "macos")]
const NATIVE_BADGE_APPLY_TIMEOUT: Duration = Duration::from_secs(2);
#[cfg(target_os = "macos")]
const MACOS_ALERT_SOUND_DEFAULTS_KEY: &str = "com.apple.sound.beep.sound";
#[cfg(target_os = "macos")]
const MACOS_DEFAULT_ALERT_SOUND_NAME: &str = "Funk";
#[cfg(target_os = "macos")]
const MACOS_SOUND_DISPATCH_TIMEOUT: Duration = Duration::from_secs(2);

#[cfg(target_os = "macos")]
thread_local! {
    // `NSSound::play` starts asynchronous playback. Keep the object alive on
    // the AppKit thread after `play` accepts it; dropping the final retain as
    // this function returns can cut playback off before it becomes audible.
    static ACTIVE_MACOS_ALERT_SOUND: std::cell::RefCell<
        Option<objc2::rc::Retained<objc2_app_kit::NSSound>>,
    > = const { std::cell::RefCell::new(None) };
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum NativeAttentionBadgeOutcome {
    Applied,
    // Shared IPC contract (mirrored by `desktopAttention.ts`); only the macOS
    // Dock backend can observe these outcomes today.
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    Unsupported,
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    Mismatch,
}

trait NativeAttentionSoundBackend {
    async fn play(&self) -> NativeAttentionSoundOutcome;
}

#[cfg(target_os = "macos")]
struct PlatformNativeAttentionSoundBackend {
    app: AppHandle,
}

#[cfg(not(target_os = "macos"))]
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
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<NativeAttentionSoundOutcome, &'static str> {
    #[cfg(target_os = "macos")]
    let backend = PlatformNativeAttentionSoundBackend { app };
    #[cfg(not(target_os = "macos"))]
    let backend = PlatformNativeAttentionSoundBackend;
    #[cfg(not(target_os = "macos"))]
    let _ = &app;
    Ok(dispatch_native_attention_sound(&state.runtime, &backend)
        .await
        .0)
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

// Dock badge label conversion for the macOS backend, kept under test on every
// platform.
#[cfg(any(target_os = "macos", test))]
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
                koushi_protocol::NativeAttentionEvent::DispatchAdmission {
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
        play_macos_alert_sound(&self.app).await
    }
}

/// The macOS alert-sound setting (NSGlobalDomain `com.apple.sound.beep.sound`)
/// interpreted as the sound to play. The system stores an empty string when
/// the user chose "None" for the alert sound, an absolute path for custom
/// sounds, and a plain name for system sounds; the key is absent when the
/// user kept the macOS default.
#[cfg(target_os = "macos")]
#[derive(Clone, Debug, Eq, PartialEq)]
enum MacosAlertSoundSource {
    /// The user set the alert sound to "None" (intentionally muted).
    Muted,
    /// A named system sound (resolved by `NSSound::soundNamed`).
    Named(String),
    /// An absolute path to a sound file.
    Path(String),
}

#[cfg(target_os = "macos")]
fn resolve_macos_alert_sound_source(setting: Option<&str>) -> MacosAlertSoundSource {
    match setting {
        None => MacosAlertSoundSource::Named(MACOS_DEFAULT_ALERT_SOUND_NAME.to_owned()),
        Some("") => MacosAlertSoundSource::Muted,
        Some(value) if value.starts_with('/') => MacosAlertSoundSource::Path(value.to_owned()),
        Some(value) => MacosAlertSoundSource::Named(value.to_owned()),
    }
}

/// Truthful outcome mapping for the macOS adapter: `Played` only when a sound
/// source was loaded and playback started; everything else is `Failed`.
#[cfg(target_os = "macos")]
fn macos_sound_outcome(loaded: bool, started: bool) -> NativeAttentionSoundOutcome {
    if loaded && started {
        NativeAttentionSoundOutcome::Played
    } else {
        NativeAttentionSoundOutcome::Failed
    }
}

#[cfg(target_os = "macos")]
fn retain_started_macos_sound<T>(
    slot: &mut Option<T>,
    sound: T,
    started: bool,
) -> NativeAttentionSoundOutcome {
    let outcome = macos_sound_outcome(true, started);
    if outcome == NativeAttentionSoundOutcome::Played {
        *slot = Some(sound);
    }
    outcome
}

#[cfg(target_os = "macos")]
async fn play_macos_alert_sound(app: &AppHandle) -> NativeAttentionSoundOutcome {
    use objc2_foundation::{NSString, NSUserDefaults};

    let setting = NSUserDefaults::standardUserDefaults()
        .stringForKey(&NSString::from_str(MACOS_ALERT_SOUND_DEFAULTS_KEY))
        .map(|value| value.to_string());
    match resolve_macos_alert_sound_source(setting.as_deref()) {
        MacosAlertSoundSource::Muted => NativeAttentionSoundOutcome::Failed,
        source => {
            // NSSound is an AppKit object; create and play it on the main
            // thread (same pattern as the Dock-badge path).
            let (sender, receiver) = tokio::sync::oneshot::channel();
            if app
                .run_on_main_thread(move || {
                    let _ = sender.send(play_macos_alert_sound_now(source));
                })
                .is_err()
            {
                return NativeAttentionSoundOutcome::Failed;
            }
            match tokio::time::timeout(MACOS_SOUND_DISPATCH_TIMEOUT, receiver).await {
                Ok(Ok(outcome)) => outcome,
                // The callback was dropped or the main thread never answered.
                Ok(Err(_)) | Err(_) => NativeAttentionSoundOutcome::Failed,
            }
        }
    }
}

#[cfg(target_os = "macos")]
fn play_macos_alert_sound_now(source: MacosAlertSoundSource) -> NativeAttentionSoundOutcome {
    use objc2::AnyThread;
    use objc2_app_kit::NSSound;
    use objc2_foundation::NSString;

    let sound = match source {
        MacosAlertSoundSource::Named(name) => NSSound::soundNamed(&NSString::from_str(&name)),
        MacosAlertSoundSource::Path(path) => NSSound::initWithContentsOfFile_byReference(
            NSSound::alloc(),
            &NSString::from_str(&path),
            true,
        ),
        MacosAlertSoundSource::Muted => {
            unreachable!("muted is handled before main-thread dispatch")
        }
    };
    let Some(sound) = sound else {
        return NativeAttentionSoundOutcome::Failed;
    };
    let started = sound.play();
    ACTIVE_MACOS_ALERT_SOUND
        .with(|slot| retain_started_macos_sound(&mut slot.borrow_mut(), sound, started))
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
mod tests;
