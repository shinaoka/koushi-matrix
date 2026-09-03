//! Tray icon ownership for the desktop shell.
//!
//! The tray is a platform capability owned by this adapter (overview.md,
//! "Desktop Window Lifecycle And Tray"). Exactly one tray icon is created
//! during `setup`, it carries no Matrix data — a static tooltip plus Show and
//! Quit — and its creation outcome is recorded so
//! `NativeAttentionCapabilities.tray` can be resolved truthfully instead of
//! claiming a tray that does not exist.
//!
//! Tray creation failing is a NORMAL outcome: a Linux session without a
//! status-notifier host has no tray to attach to. The failure is recorded as a
//! diagnostic, the capability becomes `Unavailable`, close-to-hide stays off,
//! and the app keeps running as an ordinary windowed application.

use std::sync::atomic::{AtomicU8, Ordering};

use koushi_diagnostics::{DiagnosticEvent, DiagnosticField, DiagnosticLevel};
use koushi_state::NativeAttentionCapability;
use tauri::{
    Manager,
    menu::{Menu, MenuItem},
    tray::TrayIconBuilder,
};

pub(crate) const TRAY_ID: &str = "koushi-main-tray";
pub(crate) const TRAY_TOOLTIP: &str = "Koushi";
pub(crate) const TRAY_MENU_ID_SHOW: &str = "tray_show_main_window";
pub(crate) const TRAY_MENU_ID_QUIT: &str = "tray_quit";
pub(crate) const TRAY_MENU_LABEL_SHOW: &str = "Show Koushi";
pub(crate) const TRAY_MENU_LABEL_QUIT: &str = "Quit Koushi";

const TRAY_UNKNOWN: u8 = 0;
const TRAY_AVAILABLE: u8 = 1;
const TRAY_UNAVAILABLE: u8 = 2;

/// Process-wide tray outcome.
///
/// The tray is a process singleton, exactly like the display platform reported
/// by `dto::frontend_display_platform`, so it is observed once and read from
/// the DTO projection without a new transport. It stays `Unknown` until
/// `setup` has attempted the build.
static TRAY_AVAILABILITY: AtomicU8 = AtomicU8::new(TRAY_UNKNOWN);

fn record_tray_availability(available: bool) {
    TRAY_AVAILABILITY.store(
        if available {
            TRAY_AVAILABLE
        } else {
            TRAY_UNAVAILABLE
        },
        Ordering::Release,
    );
}

/// Tray capability as actually observed by this adapter.
pub(crate) fn observed_tray_capability() -> NativeAttentionCapability {
    tray_capability_from_observation(TRAY_AVAILABILITY.load(Ordering::Acquire))
}

fn tray_capability_from_observation(observation: u8) -> NativeAttentionCapability {
    match observation {
        TRAY_AVAILABLE => NativeAttentionCapability::Available,
        TRAY_UNAVAILABLE => NativeAttentionCapability::Unavailable,
        _ => NativeAttentionCapability::Unknown,
    }
}

/// Whether close-to-hide has a tray to hide into.
///
/// Only a confirmed tray counts: hiding the last window with no tray and no
/// dock presence would leave the process unreachable, so `Unknown` and
/// `Unavailable` both answer `false`.
///
/// Only the non-macOS close-to-hide gate consults this; macOS hides
/// unconditionally, so on a macOS build it is exercised only by tests.
#[cfg_attr(all(target_os = "macos", not(test)), allow(dead_code))]
pub(crate) fn tray_is_available() -> bool {
    matches!(
        observed_tray_capability(),
        NativeAttentionCapability::Available
    )
}

/// Create the process-wide tray icon. Best-effort by contract.
pub(crate) fn install_tray_icon(app: &tauri::App) {
    match build_tray_icon(app) {
        Ok(()) => {
            record_tray_availability(true);
            koushi_diagnostics::record(
                DiagnosticEvent::new(DiagnosticLevel::Info, "desktop.lifecycle", "tray_installed")
                    .field(DiagnosticField::boolean("available", true)),
            );
        }
        Err(error) => {
            record_tray_availability(false);
            // The error text comes from the windowing toolkit and carries no
            // Matrix data; only its coarse kind is recorded.
            koushi_diagnostics::record(
                DiagnosticEvent::new(
                    DiagnosticLevel::Warn,
                    "desktop.lifecycle",
                    "tray_unavailable",
                )
                .field(DiagnosticField::boolean("available", false))
                .field(DiagnosticField::token("reason", tray_failure_token(&error))),
            );
        }
    }
}

fn tray_failure_token(error: &tauri::Error) -> &'static str {
    match error {
        tauri::Error::Menu(_) => "menu_build_failed",
        _ => "tray_build_failed",
    }
}

fn build_tray_icon(app: &tauri::App) -> tauri::Result<()> {
    let show = MenuItem::with_id(
        app,
        TRAY_MENU_ID_SHOW,
        TRAY_MENU_LABEL_SHOW,
        true,
        None::<&str>,
    )?;
    let quit = MenuItem::with_id(
        app,
        TRAY_MENU_ID_QUIT,
        TRAY_MENU_LABEL_QUIT,
        true,
        None::<&str>,
    )?;
    let menu = Menu::with_items(app, &[&show, &quit])?;

    let mut builder = TrayIconBuilder::with_id(TRAY_ID)
        .tooltip(TRAY_TOOLTIP)
        .menu(&menu)
        // Left-click activates the window; the menu belongs to the
        // right-click/secondary gesture on every platform that has one.
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| handle_tray_menu_event(app, event.id().as_ref()))
        .on_tray_icon_event(|tray, event| {
            if tray_icon_event_activates_window(&event) {
                crate::ensure_main_window_visible_for_handle(tray.app_handle());
            }
        });

    if let Some(icon) = app.default_window_icon().cloned() {
        builder = builder.icon(icon);
    }

    builder.build(app)?;
    Ok(())
}

fn handle_tray_menu_event<R: tauri::Runtime>(app: &tauri::AppHandle<R>, menu_id: &str) {
    match menu_id {
        TRAY_MENU_ID_SHOW => crate::ensure_main_window_visible_for_handle(app),
        // Quit routes through the ordinary exit request so the graceful
        // `AppCommand::Shutdown` barrier in `run()` owns shutdown exactly once.
        TRAY_MENU_ID_QUIT => crate::request_application_exit(app),
        _ => {}
    }
}

/// Only a completed primary click activates the window; press-down, secondary
/// clicks (which open the menu), and hover/move events must not.
fn tray_icon_event_activates_window(event: &tauri::tray::TrayIconEvent) -> bool {
    matches!(
        event,
        tauri::tray::TrayIconEvent::Click {
            button: tauri::tray::MouseButton::Left,
            button_state: tauri::tray::MouseButtonState::Up,
            ..
        }
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn click_event(
        button: tauri::tray::MouseButton,
        button_state: tauri::tray::MouseButtonState,
    ) -> tauri::tray::TrayIconEvent {
        tauri::tray::TrayIconEvent::Click {
            id: tauri::tray::TrayIconId::new(TRAY_ID),
            position: tauri::PhysicalPosition::new(0.0, 0.0),
            rect: tauri::Rect {
                position: tauri::PhysicalPosition::new(0.0, 0.0).into(),
                size: tauri::PhysicalSize::new(16.0, 16.0).into(),
            },
            button,
            button_state,
        }
    }

    #[test]
    fn tray_capability_reports_only_what_was_observed() {
        assert_eq!(
            tray_capability_from_observation(TRAY_UNKNOWN),
            NativeAttentionCapability::Unknown,
            "before setup attempts the build the capability is not yet known"
        );
        assert_eq!(
            tray_capability_from_observation(TRAY_AVAILABLE),
            NativeAttentionCapability::Available
        );
        assert_eq!(
            tray_capability_from_observation(TRAY_UNAVAILABLE),
            NativeAttentionCapability::Unavailable
        );
    }

    #[test]
    fn only_a_completed_primary_click_activates_the_window() {
        assert!(tray_icon_event_activates_window(&click_event(
            tauri::tray::MouseButton::Left,
            tauri::tray::MouseButtonState::Up
        )));
        assert!(!tray_icon_event_activates_window(&click_event(
            tauri::tray::MouseButton::Left,
            tauri::tray::MouseButtonState::Down
        )));
        assert!(!tray_icon_event_activates_window(&click_event(
            tauri::tray::MouseButton::Right,
            tauri::tray::MouseButtonState::Up
        )));
        assert!(!tray_icon_event_activates_window(&click_event(
            tauri::tray::MouseButton::Middle,
            tauri::tray::MouseButtonState::Up
        )));
    }
}
