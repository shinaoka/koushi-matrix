//! Desktop OS notification display and activation.
//!
//! This adapter is the only owner of the platform notification surface
//! (overview.md, "Desktop Attention Surfaces"). The banner text and the
//! navigation target come from the Rust-owned
//! [`NativeNotificationPayload`] that the attention projection built for the
//! selected candidate; the webview never receives the preview text, and only a
//! completed click hands the target back for presentation.
//!
//! Display goes through `notify-rust` instead of `tauri-plugin-notification`
//! because the plugin has no desktop activation path: on macOS, Linux, and
//! Windows it shows a banner but never reports the user's click back to the
//! app. Linux and Windows use `notify-rust`, which is the same backend the
//! plugin already uses there and can wait for the response.
//!
//! macOS does not: `notify-rust` 4.18's macOS backend never attaches a click
//! waiter (its `wait_for_click(true)` call is commented out inside the crate)
//! and our banner carries no action button, so the banner is fire-and-forget
//! and a click can never reach Koushi (#1128). The macOS path therefore sends
//! through `mac-notification-sys` directly with `wait_for_click(true)`, which is
//! the same backend `notify-rust` uses underneath.

use std::sync::atomic::{AtomicUsize, Ordering};

use koushi_core::account_runtime_manager::{AccountRuntimeManager, AccountTabId};
use koushi_diagnostics::{DiagnosticEvent, DiagnosticField, DiagnosticLevel, record};
use koushi_protocol::AccountKey;
use koushi_state::{AppState, NativeNotificationPayload, NativeNotificationTarget, SessionState};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};

use crate::CoreRuntimeState;
use crate::commands::account_key_from_app_state;

/// Tauri event carrying one completed notification click.
pub(crate) const NOTIFICATION_ACTIVATED_EVENT_NAME: &str =
    "koushi-desktop://notification-activated";

/// Upper bound on banners whose click is still being awaited.
///
/// Each waiter occupies one OS thread until the banner is activated, dismissed,
/// or withdrawn. Past this bound the banner is still shown, but its click
/// cannot navigate; the user keeps the pointer to the message in the app
/// itself.
const MAX_PENDING_ACTIVATION_WAITERS: usize = 4;

/// Action token `notify-rust` reports when the banner went away without the
/// user activating it. macOS does not use `notify-rust`'s handle (#1128).
#[cfg(not(target_os = "macos"))]
const NOTIFICATION_CLOSED_ACTION: &str = "__closed";

static PENDING_ACTIVATION_WAITERS: AtomicUsize = AtomicUsize::new(0);

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum NativeNotificationOutcome {
    /// The banner was handed to the platform with a click wait configured.
    ///
    /// This is dispatch acceptance, not proof that the platform displayed the
    /// banner or armed a click. An explicit backend send error is reported
    /// afterwards by the `show_failed` diagnostic and a completed click by
    /// `activation_settled`; a banner the platform silently drops or never
    /// confirms is reported by neither (the pinned macOS backend turns a
    /// missing delivery confirmation into an auto-dismissal).
    Delivered,
    /// Handed to the platform, but no click waiter was available.
    DisplayOnly,
    /// Nothing to show, or the current state must not raise a banner.
    Skipped,
    /// The platform notification backend refused the banner.
    Failed,
}

impl NativeNotificationOutcome {
    fn token(self) -> &'static str {
        match self {
            Self::Delivered => "delivered",
            Self::DisplayOnly => "display_only",
            Self::Skipped => "skipped",
            Self::Failed => "failed",
        }
    }

    /// Whether this dispatch configured a click waiter for the banner.
    ///
    /// Distinguishes a banner the user can activate from a display-only one, so
    /// the diagnostic log cannot report a clickable banner that never armed a
    /// waiter (#1128).
    fn click_waiter(self) -> &'static str {
        match self {
            Self::Delivered => "configured",
            Self::DisplayOnly | Self::Skipped | Self::Failed => "unattached",
        }
    }
}

/// Target handed to the webview after a completed notification click.
///
/// Carries only identifiers the webview already holds; the notification body
/// and its preview stay in Rust.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub(crate) struct NativeNotificationActivation {
    pub account_tab_id: String,
    pub room_id: String,
    pub event_id: Option<String>,
    pub thread_root_event_id: Option<String>,
}

impl NativeNotificationActivation {
    fn new(account_tab_id: &AccountTabId, target: NativeNotificationTarget) -> Self {
        Self {
            account_tab_id: account_tab_id.as_str().to_owned(),
            room_id: target.room_id,
            event_id: target.event_id,
            thread_root_event_id: target.thread_root_event_id,
        }
    }
}

/// Account fence captured when the banner was shown.
///
/// A click that arrives after a logout, account switch, or session change must
/// not navigate into the wrong session, so the fence is re-checked against the
/// live state before the activation is published.
#[derive(Clone, Eq, PartialEq)]
struct ActivationFence {
    account_key: AccountKey,
    account_tab_id: AccountTabId,
}

/// One eligible banner: the Rust-owned text/target plus its account fence.
struct PendingNotification {
    payload: NativeNotificationPayload,
    fence: ActivationFence,
}

/// Decide whether the live state still admits a desktop notification.
///
/// Returns the reason token instead of raising a banner, so the caller records
/// a private-data-free diagnostic and the webview keeps a single trigger path.
fn pending_notification(
    state: &AppState,
    account_tab_id: AccountTabId,
) -> Result<PendingNotification, &'static str> {
    if !matches!(state.session, SessionState::Ready(_)) {
        return Err("session_unavailable");
    }
    if !state.settings.values.notifications.desktop_notifications {
        return Err("desktop_notifications_off");
    }
    let Some(payload) = state.native_attention.notification.clone() else {
        return Err("no_candidate");
    };
    let account_key = account_key_from_app_state(state);
    if account_key.0.is_empty() {
        return Err("session_unavailable");
    }
    Ok(PendingNotification {
        payload,
        fence: ActivationFence {
            account_key,
            account_tab_id,
        },
    })
}

pub(crate) fn dispatch_notification_for_tab(
    app: &AppHandle,
    account_tab_id: &AccountTabId,
    state: &AppState,
) -> NativeNotificationOutcome {
    let pending = match pending_notification(state, account_tab_id.clone()) {
        Ok(pending) => pending,
        Err(reason) => {
            return record_notification_outcome(NativeNotificationOutcome::Skipped, reason);
        }
    };
    let outcome = dispatch_native_notification(app, &pending.payload, pending.fence);
    record_notification_outcome(outcome, "candidate")
}

/// Show the Rust-owned desktop notification for the selected attention
/// candidate.
///
/// The webview only triggers this when the projection published a new
/// candidate; the text, the preview gate, and the navigation target are read
/// from the live snapshot here.
#[tauri::command]
pub(crate) async fn show_native_attention_notification(
    account_tab_id: Option<String>,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<NativeNotificationOutcome, &'static str> {
    let (account_tab_id, connection) = match account_tab_id {
        Some(id) => {
            let tab_id = AccountTabId::from_string(id.clone());
            let connection = super::super::account_connection(state.inner(), Some(&id))
                .await
                .map_err(|_| "account tab does not exist")?;
            (tab_id, connection)
        }
        None => state.runtime.selected_binding(),
    };
    let snapshot = connection.versioned_snapshot();
    match pending_notification(&snapshot.state, account_tab_id) {
        Err(reason) => Ok(record_notification_outcome(
            NativeNotificationOutcome::Skipped,
            reason,
        )),
        Ok(pending) => {
            let outcome = dispatch_native_notification(&app, &pending.payload, pending.fence);
            Ok(record_notification_outcome(outcome, "candidate"))
        }
    }
}

fn record_notification_outcome(
    outcome: NativeNotificationOutcome,
    reason: &'static str,
) -> NativeNotificationOutcome {
    record(
        DiagnosticEvent::new(
            if outcome == NativeNotificationOutcome::Failed {
                DiagnosticLevel::Warn
            } else {
                DiagnosticLevel::Info
            },
            "desktop.native_notification",
            "dispatch_settled",
        )
        .field(DiagnosticField::token("outcome", outcome.token()))
        .field(DiagnosticField::token("reason", reason))
        .field(DiagnosticField::token(
            "click_waiter",
            outcome.click_waiter(),
        ))
        .field(DiagnosticField::count(
            "pending_waiters",
            PENDING_ACTIVATION_WAITERS.load(Ordering::Relaxed) as u64,
        )),
    );
    outcome
}

/// Hand one banner to the platform and keep a click waiter on it when the
/// bounded waiter budget allows.
fn dispatch_native_notification(
    app: &AppHandle,
    payload: &NativeNotificationPayload,
    fence: ActivationFence,
) -> NativeNotificationOutcome {
    set_macos_application_identity(app);

    let Some(reservation) = ActivationWaiterReservation::acquire() else {
        // ponytail: bounded waiter pool; past the bound the banner is shown
        // without activation. Raise the bound, or move activation to a platform
        // delegate, only if users actually hit it.
        return if show_without_activation(payload) {
            NativeNotificationOutcome::DisplayOnly
        } else {
            NativeNotificationOutcome::Failed
        };
    };

    let title = payload.title.clone();
    let body = payload.body.clone();
    let target = payload.target.clone();
    let app = app.clone();
    let spawned = std::thread::Builder::new()
        .name("koushi-notification-activation".to_owned())
        .spawn(move || {
            let _reservation = reservation;
            show_banner_and_wait_for_activation(&app, &fence, &title, &body, target);
        });

    match spawned {
        Ok(_) => NativeNotificationOutcome::Delivered,
        // A failed spawn dropped the closure, so the reservation is released.
        Err(_) => {
            if show_without_activation(payload) {
                NativeNotificationOutcome::DisplayOnly
            } else {
                NativeNotificationOutcome::Failed
            }
        }
    }
}

/// Show one banner and block this activation thread until the user activates
/// or dismisses it.
///
/// macOS sends through `mac-notification-sys` itself: the pinned `notify-rust`
/// macOS backend never attaches a click waiter, so a click could never activate
/// a banner it showed (#1128). Linux and Windows keep `notify-rust`'s action
/// waiter, which reports `__closed` when the banner goes away unactivated.
fn show_banner_and_wait_for_activation(
    app: &AppHandle,
    fence: &ActivationFence,
    title: &str,
    body: &str,
    target: NativeNotificationTarget,
) {
    #[cfg(target_os = "macos")]
    {
        match send_macos_banner_awaiting_click(title, body) {
            Ok(response) => {
                if macos_banner_activates(&response) {
                    activate_notification(app, fence, target);
                }
            }
            Err(()) => record_show_failed(),
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        let mut notification = notify_rust::Notification::new();
        notification.summary(title).body(body);
        let Ok(handle) = notification.show() else {
            record_show_failed();
            return;
        };
        handle.wait_for_action(|action| {
            if action == NOTIFICATION_CLOSED_ACTION {
                return;
            }
            activate_notification(app, fence, target);
        });
    }
}

fn record_show_failed() {
    record(
        DiagnosticEvent::new(
            DiagnosticLevel::Warn,
            "desktop.native_notification",
            "show_failed",
        )
        .field(DiagnosticField::token("reason", "platform_backend")),
    );
}

/// Whether a macOS banner must be shown with a click waiter attached.
///
/// A macOS banner with no action button, no close button, and no click wait is
/// fire-and-forget: `mac-notification-sys` returns after its delivery
/// confirmation and no click reaches the process, which is exactly the #1128
/// defect. This constant is the structural guard for that request; the live
/// smoke in this module's tests is the behavioural evidence.
#[cfg(target_os = "macos")]
const MACOS_BANNER_WAITS_FOR_CLICK: bool = true;

/// The click-wait request the macOS sender passes to `mac-notification-sys`.
///
/// Kept as the single decision point so the request can be asserted without
/// showing a banner; the live smoke in this module's tests is the behavioural
/// evidence.
#[cfg(target_os = "macos")]
fn macos_click_wait_requested() -> bool {
    MACOS_BANNER_WAITS_FOR_CLICK
}

/// Show one macOS banner and block until the user activates or dismisses it.
#[cfg(target_os = "macos")]
fn send_macos_banner_awaiting_click(
    title: &str,
    body: &str,
) -> Result<mac_notification_sys::NotificationResponse, ()> {
    let mut notification = mac_notification_sys::Notification::new();
    notification
        .title(title)
        .message(body)
        .wait_for_click(macos_click_wait_requested());
    notification.send().map_err(|_| ())
}

/// Whether a macOS banner response means the user activated the banner.
///
/// Only a direct click activates. A dismissal, a close button, an action
/// button, or a text reply must not navigate: the target is only shown when the
/// user opened the banner itself.
#[cfg(target_os = "macos")]
fn macos_banner_activates(response: &mac_notification_sys::NotificationResponse) -> bool {
    matches!(response, mac_notification_sys::NotificationResponse::Click)
}

fn show_without_activation(payload: &NativeNotificationPayload) -> bool {
    let mut notification = notify_rust::Notification::new();
    notification.summary(&payload.title).body(&payload.body);
    notification.show().is_ok()
}

/// macOS delivers notifications under the bundle identifier of the sending
/// process; without this the banner belongs to the development host and a click
/// cannot reach Koushi.
#[cfg(target_os = "macos")]
fn set_macos_application_identity(app: &AppHandle) {
    let identifier = if tauri::is_dev() {
        "com.apple.Terminal".to_owned()
    } else {
        app.config().identifier.clone()
    };
    let _ = notify_rust::set_application(&identifier);
}

#[cfg(not(target_os = "macos"))]
fn set_macos_application_identity(_app: &AppHandle) {}

async fn select_activation_tab(
    runtime: &AccountRuntimeManager,
    generation: &std::sync::atomic::AtomicU64,
    account_tab_id: &AccountTabId,
    window_focused: bool,
) -> bool {
    let previous_tab = runtime.selected_tab_id();
    if !matches!(runtime.select_tab(account_tab_id).await, Ok(true)) {
        return false;
    }
    crate::commands::native_attention::transfer_native_window_focus(
        runtime,
        generation,
        &previous_tab,
        account_tab_id,
        window_focused,
    )
    .await;
    true
}

/// Bring Koushi forward, select the owning account, and publish the click target.
fn activate_notification(
    app: &AppHandle,
    fence: &ActivationFence,
    target: NativeNotificationTarget,
) {
    let app = app.clone();
    let fence = fence.clone();
    tauri::async_runtime::spawn(async move {
        crate::ensure_main_window_visible_for_handle(&app);

        let state = app.state::<CoreRuntimeState>();
        let Some(connection) = state.runtime.tab_connection(&fence.account_tab_id) else {
            record_activation("stale_account");
            return;
        };
        let snapshot = connection.versioned_snapshot();
        if !matches!(snapshot.state.session, SessionState::Ready(_)) {
            record_activation("session_unavailable");
            return;
        }
        if account_key_from_app_state(&snapshot.state) != fence.account_key {
            record_activation("stale_account");
            return;
        }
        drop(connection);

        let changed = state.runtime.selected_tab_id() != fence.account_tab_id;
        if changed {
            state.close_reader_subscriptions().await;
            state.stop_selected_forwarder().await;
        }
        if !select_activation_tab(
            &state.runtime,
            &state.native_window_focus_generation,
            &fence.account_tab_id,
            state.native_window_focused.load(Ordering::Relaxed),
        )
        .await
        {
            if changed {
                state.restart_selected_forwarder(app.clone());
            }
            record_activation("stale_account");
            return;
        }
        if changed {
            state.restart_selected_forwarder(app.clone());
            crate::emit_account_tabs_changed(&app, &state.runtime);
            crate::allow_account_media_cache_dirs(&app, &state.runtime);
        }

        if app
            .emit(
                NOTIFICATION_ACTIVATED_EVENT_NAME,
                NativeNotificationActivation::new(&fence.account_tab_id, target),
            )
            .is_err()
        {
            record_activation("delivery_failed");
            return;
        }
        record_activation("delivered");
    });
}

fn record_activation(reason: &'static str) {
    record(
        DiagnosticEvent::new(
            DiagnosticLevel::Info,
            "desktop.native_notification",
            "activation_settled",
        )
        .field(DiagnosticField::token("outcome", reason)),
    );
}

/// Bounded budget for threads blocked on a notification click.
struct ActivationWaiterReservation;

impl ActivationWaiterReservation {
    fn acquire() -> Option<Self> {
        let mut current = PENDING_ACTIVATION_WAITERS.load(Ordering::Acquire);
        loop {
            if current >= MAX_PENDING_ACTIVATION_WAITERS {
                return None;
            }
            match PENDING_ACTIVATION_WAITERS.compare_exchange_weak(
                current,
                current + 1,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => return Some(Self),
                Err(observed) => current = observed,
            }
        }
    }
}

impl Drop for ActivationWaiterReservation {
    fn drop(&mut self) {
        PENDING_ACTIVATION_WAITERS.fetch_sub(1, Ordering::AcqRel);
    }
}

#[cfg(test)]
mod tests;
