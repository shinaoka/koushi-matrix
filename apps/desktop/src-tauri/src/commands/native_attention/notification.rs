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
//! macOS uses `UNUserNotificationCenter` through `mac-usernotifications` so
//! its delegate can report body clicks without a blocked OS thread (#1128).

#[cfg(not(target_os = "macos"))]
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;

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

/// Upper bound on Linux/Windows banners whose click is still being awaited.
///
/// Each waiter occupies one OS thread until the banner is activated or
/// dismissed. Past this bound the banner is still shown, but its click
/// cannot navigate.
#[cfg(not(target_os = "macos"))]
const MAX_PENDING_ACTIVATION_WAITERS: usize = 64;

#[cfg(not(target_os = "macos"))]
static PENDING_ACTIVATION_WAITERS: AtomicUsize = AtomicUsize::new(0);

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum NativeNotificationOutcome {
    /// The banner was handed to the platform with a click wait configured.
    ///
    /// This is dispatch acceptance, not proof that the platform displayed the
    /// banner or armed a click. An explicit backend send error is reported
    /// afterwards by the `show_failed` diagnostic and a completed click by
    /// `activation_settled`; a banner the platform silently drops is reported
    /// by neither.
    Delivered,
    /// Handed to the platform, but no click waiter was available.
    #[cfg_attr(target_os = "macos", allow(dead_code))]
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

/// OS-level authorization for this app's desktop banners.
///
/// The persisted Desktop notifications switch is the user's preference; this is
/// whether the operating system will actually show a banner for it. The wire
/// tokens are the same on every platform so the settings UI has one shape.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub(crate) enum NativeNotificationAuthorization {
    Authorized,
    Provisional,
    Ephemeral,
    Denied,
    NotDetermined,
    /// The platform has no OS-level notification grant to report.
    #[cfg_attr(target_os = "macos", allow(dead_code))]
    Unsupported,
    /// The OS could not answer: an unbundled dev build, or a failed query.
    Unavailable,
}

impl NativeNotificationAuthorization {
    /// Whether the OS will show a banner for this app.
    ///
    /// The delivery gate is macOS-only today, but the answer is the same on
    /// every platform, so the platforms that never call it keep the method for
    /// the shared tests instead of hiding it behind a cfg.
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    fn allows_delivery(self) -> bool {
        matches!(self, Self::Authorized | Self::Provisional | Self::Ephemeral)
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
    message_previews: bool,
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
        message_previews: state.settings.values.notifications.message_previews,
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
    let outcome = dispatch_native_notification(
        app,
        &pending.payload,
        pending.fence,
        pending.message_previews,
    );
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
            let outcome = dispatch_native_notification(
                &app,
                &pending.payload,
                pending.fence,
                pending.message_previews,
            );
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
            pending_activation_count() as u64,
        )),
    );
    outcome
}

/// Hand one banner to the platform with an activation response when the
/// platform's bounded activation budget allows.
fn dispatch_native_notification(
    app: &AppHandle,
    payload: &NativeNotificationPayload,
    fence: ActivationFence,
    message_previews: bool,
) -> NativeNotificationOutcome {
    #[cfg(target_os = "macos")]
    return macos::dispatch(app, payload, fence, message_previews);

    #[cfg(not(target_os = "macos"))]
    {
        let _ = message_previews;
        let Some(reservation) = ActivationWaiterReservation::acquire() else {
            // ponytail: bounded waiter pool; past the bound the banner is shown
            // without activation. Raise the bound, or move activation to a platform
            // delegate, only if users actually hit it.
            return if show_without_activation(app, payload) {
                NativeNotificationOutcome::DisplayOnly
            } else {
                NativeNotificationOutcome::Failed
            };
        };

        let title = payload.title.clone();
        let body = payload.body.clone();
        let target = payload.target.clone();
        let activation_app = app.clone();
        let spawned = std::thread::Builder::new()
            .name("koushi-notification-activation".to_owned())
            .spawn(move || {
                let _reservation = reservation;
                show_banner_and_wait_for_activation(&activation_app, &fence, &title, &body, target);
            });

        match spawned {
            Ok(_) => NativeNotificationOutcome::Delivered,
            // A failed spawn dropped the closure, so the reservation is released.
            Err(_) => {
                if show_without_activation(app, payload) {
                    NativeNotificationOutcome::DisplayOnly
                } else {
                    NativeNotificationOutcome::Failed
                }
            }
        }
    }
}

/// Show one banner and block this activation thread until the user activates
/// or dismisses it.
///
/// Linux and Windows use `notify-rust`'s typed response. Windows maps a body
/// click to `Default`, while its legacy action callback maps it to `__closed`.
#[cfg(not(target_os = "macos"))]
fn show_banner_and_wait_for_activation(
    app: &AppHandle,
    fence: &ActivationFence,
    title: &str,
    body: &str,
    target: NativeNotificationTarget,
) {
    {
        let notification = response_notification(title, body, notification_app_identity(app));
        let Ok(handle) = notification.show() else {
            record_show_failed();
            return;
        };
        let result = handle.wait_for_response(|response: &notify_rust::NotificationResponse| {
            if notification_response_activates(response) {
                activate_notification(app, fence, target.clone());
            }
        });
        if result.is_err() {
            record_show_failed();
        }
    }
}

#[cfg(any(not(target_os = "macos"), test))]
fn notification_response_activates(response: &notify_rust::NotificationResponse) -> bool {
    response.is_default_action()
}

#[cfg(not(target_os = "macos"))]
fn response_notification(
    title: &str,
    body: &str,
    app_id: Option<&str>,
) -> notify_rust::Notification {
    let mut notification = notify_rust::Notification::new();
    notification.summary(title).body(body);
    #[cfg(target_os = "linux")]
    notification.action("default", "Open");
    #[cfg(windows)]
    if let Some(app_id) = app_id {
        notification.app_id(app_id);
    }
    #[cfg(not(windows))]
    let _ = app_id;
    notification
}

#[cfg(windows)]
fn notification_app_id(is_dev: bool, identifier: &str) -> Option<&str> {
    (!is_dev).then_some(identifier)
}

#[cfg(windows)]
fn notification_app_identity(app: &AppHandle) -> Option<&str> {
    notification_app_id(tauri::is_dev(), &app.config().identifier)
}

#[cfg(target_os = "linux")]
fn notification_app_identity(_app: &AppHandle) -> Option<&str> {
    None
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

#[cfg(not(target_os = "macos"))]
fn show_without_activation(app: &AppHandle, payload: &NativeNotificationPayload) -> bool {
    let notification = response_notification(
        &payload.title,
        &payload.body,
        notification_app_identity(app),
    );
    notification.show().is_ok()
}

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
#[cfg(not(target_os = "macos"))]
struct ActivationWaiterReservation;

#[cfg(not(target_os = "macos"))]
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

#[cfg(not(target_os = "macos"))]
impl Drop for ActivationWaiterReservation {
    fn drop(&mut self) {
        PENDING_ACTIVATION_WAITERS.fetch_sub(1, Ordering::AcqRel);
    }
}

#[cfg(target_os = "macos")]
mod macos;

#[cfg(target_os = "macos")]
fn pending_activation_count() -> usize {
    macos::pending_count()
}

#[cfg(not(target_os = "macos"))]
fn pending_activation_count() -> usize {
    PENDING_ACTIVATION_WAITERS.load(Ordering::Relaxed)
}

/// Reconcile one account's banners from its Rust-owned attention and privacy policy.
pub(crate) async fn reconcile_notifications_for_tab(
    tab: &AccountTabId,
    snapshot: &AppState,
    preview_disabled: bool,
) {
    #[cfg(target_os = "macos")]
    {
        let values = &snapshot.settings.values.notifications;
        let immediate = !matches!(snapshot.session, SessionState::Ready(_))
            || !values.desktop_notifications
            || (!values.message_previews
                && (preview_disabled || macos::has_previews(tab.as_str())));
        if immediate || snapshot.native_attention.summary.unread_count == 0 {
            macos::clear(tab.as_str(), immediate).await;
        }
    }
    #[cfg(not(target_os = "macos"))]
    let _ = (tab, snapshot, preview_disabled);
}

/// Withdraw the owning account's banners; routine reads preserve five seconds.
#[tauri::command]
pub(crate) async fn clear_native_attention_notifications(
    account_tab_id: Option<String>,
    state: State<'_, CoreRuntimeState>,
) -> Result<(), &'static str> {
    #[cfg(target_os = "macos")]
    {
        let tab = account_tab_id
            .map(AccountTabId::from_string)
            .unwrap_or_else(|| state.runtime.selected_tab_id());
        if let Some(connection) = state.runtime.tab_connection(&tab) {
            let snapshot = connection.versioned_snapshot();
            reconcile_notifications_for_tab(&tab, &snapshot.state, false).await;
        } else {
            clear_removed_account_notifications(&tab).await;
        }
        Ok(())
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (account_tab_id, state);
        Err("native notification clear is not implemented on this platform")
    }
}

pub(crate) async fn clear_notifications_for_missing_tabs(known: &[AccountTabId]) {
    #[cfg(target_os = "macos")]
    macos::clear_missing_accounts(known).await;
    #[cfg(not(target_os = "macos"))]
    let _ = known;
}

pub(crate) async fn clear_removed_account_notifications(tab: &AccountTabId) {
    #[cfg(target_os = "macos")]
    macos::clear(tab.as_str(), true).await;
    #[cfg(not(target_os = "macos"))]
    let _ = tab;
}

/// Called only from the explicit Desktop notifications switch, never from a
/// passive banner dispatch.
#[tauri::command]
pub(crate) async fn request_native_attention_notification_permission() -> bool {
    #[cfg(target_os = "macos")]
    return mac_usernotifications::request_auth().await.unwrap_or(false);
    #[cfg(not(target_os = "macos"))]
    true
}

/// Report what the OS currently authorizes for this app's banners.
#[tauri::command]
pub(crate) async fn native_attention_notification_authorization() -> NativeNotificationAuthorization
{
    current_authorization().await
}

/// Open the OS settings page where the user can allow this app's banners.
#[tauri::command]
pub(crate) async fn open_native_attention_notification_settings() -> Result<(), &'static str> {
    open_notification_settings()
}

#[cfg(target_os = "macos")]
fn authorization_from_status(
    status: mac_usernotifications::AuthorizationStatus,
) -> NativeNotificationAuthorization {
    use mac_usernotifications::AuthorizationStatus;
    match status {
        AuthorizationStatus::Authorized => NativeNotificationAuthorization::Authorized,
        AuthorizationStatus::Provisional => NativeNotificationAuthorization::Provisional,
        AuthorizationStatus::Ephemeral => NativeNotificationAuthorization::Ephemeral,
        AuthorizationStatus::Denied => NativeNotificationAuthorization::Denied,
        AuthorizationStatus::NotDetermined => NativeNotificationAuthorization::NotDetermined,
        // A status this build does not know is not a grant.
        AuthorizationStatus::Unknown => NativeNotificationAuthorization::Unavailable,
    }
}

#[cfg(target_os = "macos")]
async fn current_authorization() -> NativeNotificationAuthorization {
    if mac_usernotifications::check_bundle().is_err() {
        return NativeNotificationAuthorization::Unavailable;
    }
    match mac_usernotifications::get_notification_settings().await {
        Ok(settings) => authorization_from_status(settings.authorization_status),
        Err(_) => NativeNotificationAuthorization::Unavailable,
    }
}

#[cfg(not(target_os = "macos"))]
async fn current_authorization() -> NativeNotificationAuthorization {
    NativeNotificationAuthorization::Unsupported
}

#[cfg(target_os = "macos")]
fn open_notification_settings() -> Result<(), &'static str> {
    // A fixed, user-input-free URL: the pane macOS uses for this app's banners.
    std::process::Command::new("open")
        .arg("x-apple.systempreferences:com.apple.Notifications-Settings.extension")
        .spawn()
        .map(|_| ())
        .map_err(|_| "could not open System Settings")
}

#[cfg(not(target_os = "macos"))]
fn open_notification_settings() -> Result<(), &'static str> {
    Err("opening OS notification settings is not supported on this platform")
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod platform_response_tests;

#[cfg(all(test, target_os = "macos"))]
mod macos_activation_tests;
