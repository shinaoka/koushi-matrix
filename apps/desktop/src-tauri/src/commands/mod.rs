//! Tauri command handlers: transport adapter only.
//!
//! Each handler allocates a `RequestId` and submits a `CoreCommand`.
//! Handlers return the current `FrontendDesktopSnapshot` only when the React
//! caller actually applies it; high-frequency fire-and-forget commands return
//! a tiny acknowledgement. Side-effects (state changes, timeline diffs) flow
//! back to the webview as Tauri events — not as command return values.
//!
//! No Matrix semantics live here. No SDK types. No `koushi_sdk` calls.
//! (Secret-bearing QA helpers remain behind `#[cfg(any(debug_assertions, test))]`.)

use std::{
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};

use koushi_core::{
    AccountCommand, AccountEvent, AccountKey, AppCommand, CoreCommand, CoreConnection, CoreEvent,
    CreateRoomOptions, ImageUploadCompressionPolicy, ImageUploadCompressionState,
    ImageUploadDimensions, ImageUploadVariantKind, IntentNoOpReason, IntentOutcome,
    MediaDownloadSelection, PaginationDirection, RequestId, RoomCommand, RoomEvent,
    RoomKeyExportRequest, RoomKeyImportRequest, SearchCommand, SearchEvent, SearchScope,
    SecureBackupPassphraseChangeRequest, SecureBackupSetupRequest, SetAvatarRequest, SyncCommand,
    TimelineCommand, TimelineKey, TimelineKind, TimelineViewportObservation, UploadMediaKind,
    UploadMediaRequest, UploadMediaThumbnail,
};
use koushi_state::{
    ActivityMarkReadTarget, ActivityTab, AttachmentFilter, AttachmentSort, AuthSecret,
    ComposerKeyEvent, ComposerResolvedAction, ComposerResolverContext, ComposerSurface,
    DirectoryQuery, FilesViewScope, FocusedContextState, IdentityResetAuthRequest,
    ImageUploadCompressionMode, InviteScopeSelection, LoginRequest, MentionIntent, PresenceKind,
    RecoveryRequest, RoomListFilter, RoomModerationAction, RoomNotificationMode, RoomSettingChange,
    RoomTagKind, SessionInfo, SettingsPatch, StagedUploadCompressionChoice, StagedUploadItem,
    StagedUploadKind, TimelineScrollAnchor, VerificationCancelReason,
    build_formatted_message_draft,
};
use serde::Deserialize;
#[cfg(any(debug_assertions, test))]
use tauri::Emitter;
use tauri::{AppHandle, Manager, State};

use crate::{
    CoreRuntimeState,
    dto::{FrontendDesktopSnapshot, SearchScopeKind},
};

static NEXT_TRANSACTION_ID: AtomicU64 = AtomicU64::new(1);

#[cfg(any(debug_assertions, test))]
const QA_RECOVERY_PROMPT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);
const QA_TITLE_ENV: &str = "KOUSHI_QA_TITLE";
const TIMELINE_BACKWARDS_PAGE_EVENT_COUNT: u16 = 100;
#[cfg(test)]
const TIMELINE_RESTORE_ANCHOR_MAX_BATCHES: u16 = 6;

pub(crate) mod account;
pub(crate) mod activity;
pub(crate) mod directory;
pub(crate) mod e2ee;
pub(crate) mod live_signals;
pub(crate) mod local_encryption;
pub(crate) mod navigation;
pub(crate) mod profile;
pub(crate) mod room;
pub(crate) mod search;
pub(crate) mod session;
pub(crate) mod settings;
pub(crate) mod timeline;
pub(crate) mod views;

// ---- Core command dispatch helpers ----

/// Submit a `CoreCommand` over the command-dispatch connection.
///
/// This is the ONLY way commands leave the Tauri adapter.
/// Uses `tokio::sync::Mutex` so the guard may be held across `.await`.
pub(crate) async fn submit_core_command(
    state: &CoreRuntimeState,
    command: CoreCommand,
) -> Result<(), String> {
    state
        .connection
        .lock()
        .await
        .command(command)
        .await
        .map_err(|e| format!("command submit failed: {e}"))
}

/// Allocate a `RequestId` from the command-dispatch connection.
async fn next_request_id(state: &CoreRuntimeState) -> koushi_core::RequestId {
    state.connection.lock().await.next_request_id()
}

pub(crate) fn trace_tauri_timeline_command(stage: &str, kind: &str, request_id: RequestId) {
    if std::env::var_os("KOUSHI_SUBSCRIBE_TRACE").is_some() {
        eprintln!(
            "koushi.desktop stage={stage} kind={kind} request_id={}/{}",
            request_id.connection_id.0, request_id.sequence
        );
    }
}

/// Read the latest `AppStateSnapshot` and convert to `FrontendDesktopSnapshot`.
async fn current_snapshot(state: &CoreRuntimeState) -> Result<FrontendDesktopSnapshot, String> {
    let snapshot = state.connection.lock().await.versioned_snapshot();
    Ok(FrontendDesktopSnapshot::from_versioned(
        snapshot.state,
        snapshot.generation,
    ))
}

// ---- QA window title ----

fn qa_window_title_enabled() -> bool {
    matches!(
        std::env::var(QA_TITLE_ENV)
            .ok()
            .as_deref()
            .map(str::trim)
            .map(str::to_ascii_lowercase)
            .as_deref(),
        Some("1" | "true" | "yes")
    )
}

async fn update_qa_window_title_from_state(app: &AppHandle, state: &CoreRuntimeState) {
    if !qa_window_title_enabled() {
        return;
    }
    let snapshot = state.connection.lock().await.snapshot();
    let timeline_items = state.timeline_items_count.load(Ordering::Relaxed);
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.set_title(&qa_window_title_string(&snapshot, timeline_items));
    }
}

pub(crate) fn qa_window_title_string(
    snapshot: &koushi_state::AppState,
    timeline_items: usize,
) -> String {
    [
        "koushi-desktop qa".to_owned(),
        format!("session={}", qa_session_label(&snapshot.session)),
        format!("sync={}", qa_sync_label(&snapshot.sync)),
        format!("rooms={}", snapshot.rooms.len()),
        format!("spaces={}", snapshot.spaces.len()),
        format!(
            "active_room={}",
            snapshot.navigation.active_room_id.is_some()
        ),
        format!("timeline_subscribed={}", snapshot.timeline.is_subscribed),
        format!("timeline_items={timeline_items}"),
        format!("errors={}", snapshot.errors.len()),
    ]
    .join(" ")
}

fn qa_session_label(session: &koushi_state::SessionState) -> &'static str {
    use koushi_state::SessionState;
    match session {
        SessionState::SignedOut => "signedOut",
        SessionState::Restoring => "restoring",
        SessionState::SwitchingAccount { .. } => "switchingAccount",
        SessionState::Authenticating { .. } => "authenticating",
        SessionState::NeedsRecovery { .. } => "needsRecovery",
        SessionState::Recovering { .. } => "recovering",
        SessionState::Ready(_) => "ready",
        SessionState::Locked(_) => "locked",
        SessionState::LoggingOut => "loggingOut",
    }
}

fn qa_sync_label(sync: &koushi_state::SyncState) -> &'static str {
    match sync {
        koushi_state::SyncState::Stopped => "stopped",
        koushi_state::SyncState::Starting => "starting",
        koushi_state::SyncState::Running => "running",
        koushi_state::SyncState::Failed { .. } => "failed",
        koushi_state::SyncState::Reconnecting { .. } => "reconnecting",
    }
}

// ---- Tauri commands ----

pub(crate) async fn submit_login_request(
    app: AppHandle,
    state: &CoreRuntimeState,
    login_request: LoginRequest,
) -> Result<(), String> {
    submit_login_and_wait_for_authenticated(app, state, login_request).await?;
    Ok(())
}

const LOGIN_EVENT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);
const SELECT_ROOM_EVENT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);
const FOCUSED_CONTEXT_EVENT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(15);
const SEARCH_EVENT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);
const ROOM_OPERATION_EVENT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);
const UPLOAD_STAGING_EVENT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

async fn submit_login_and_wait_for_authenticated(
    app: AppHandle,
    state: &CoreRuntimeState,
    login_request: LoginRequest,
) -> Result<(), String> {
    // Use a dedicated connection so the event cursor is attached before the
    // login command is submitted and the correlated LoggedIn event cannot be
    // missed by this product path.
    let mut event_conn = state.runtime.attach();
    let login_request_id = event_conn.next_request_id();
    event_conn
        .command(build_submit_login_command(login_request_id, login_request))
        .await
        .map_err(|e| format!("command submit failed: {e}"))?;

    wait_for_logged_in_authenticated(&mut event_conn, login_request_id, LOGIN_EVENT_TIMEOUT)
        .await?;
    update_qa_window_title_from_state(&app, state).await;
    Ok(())
}

async fn wait_for_logged_in_authenticated(
    event_conn: &mut CoreConnection,
    login_request_id: RequestId,
    timeout: std::time::Duration,
) -> Result<(), String> {
    let deadline = tokio::time::Instant::now() + timeout;
    let mut logged_in = false;

    loop {
        if logged_in && snapshot_has_authenticated_session(&event_conn.snapshot()) {
            return Ok(());
        }

        let event = tokio::time::timeout_at(deadline, event_conn.recv_event())
            .await
            .map_err(|_| "login did not complete".to_owned())?;
        match event {
            Ok(CoreEvent::Account(AccountEvent::LoggedIn { request_id, .. }))
                if request_id == login_request_id =>
            {
                logged_in = true;
            }
            Ok(CoreEvent::OperationFailed { request_id, .. }) if request_id == login_request_id => {
                return Err("login failed".to_owned());
            }
            Ok(_) => {}
            Err(_) => return Err("login event stream lagged".to_owned()),
        }
    }
}

async fn wait_for_auth_changed(
    event_conn: &mut CoreConnection,
    timeout: std::time::Duration,
) -> Result<(), String> {
    let deadline = tokio::time::Instant::now() + timeout;

    loop {
        let event = tokio::time::timeout_at(deadline, event_conn.recv_event())
            .await
            .map_err(|_| "login discovery did not complete".to_owned())?;
        match event {
            Ok(CoreEvent::StateChanged(snapshot))
                if matches!(
                    snapshot.auth,
                    koushi_state::AuthDiscoveryState::Ready { .. }
                        | koushi_state::AuthDiscoveryState::Failed { .. }
                ) =>
            {
                return Ok(());
            }
            Ok(_) => {}
            Err(_) => return Err("login discovery event stream lagged".to_owned()),
        }
    }
}

fn snapshot_has_authenticated_session(snapshot: &koushi_state::AppState) -> bool {
    matches!(
        snapshot.session,
        koushi_state::SessionState::Ready(_)
            | koushi_state::SessionState::NeedsRecovery { .. }
            | koushi_state::SessionState::Recovering { .. }
    )
}

fn snapshot_has_focused_context(snapshot: &koushi_state::AppState, room_id: &str) -> bool {
    match &snapshot.focused_context {
        FocusedContextState::Opening {
            room_id: focused_room_id,
            ..
        }
        | FocusedContextState::Open {
            room_id: focused_room_id,
            ..
        } => focused_room_id == room_id,
        FocusedContextState::Closed => false,
    }
}

fn snapshot_has_no_focused_context(snapshot: &koushi_state::AppState) -> bool {
    snapshot.focused_context == FocusedContextState::Closed
        && snapshot.navigation.main_timeline_anchor.is_none()
}

fn snapshot_has_main_timeline_anchor(
    snapshot: &koushi_state::AppState,
    room_id: &str,
    event_id: &str,
) -> bool {
    snapshot.navigation.active_room_id.as_deref() == Some(room_id)
        && snapshot
            .navigation
            .main_timeline_anchor
            .as_ref()
            .is_some_and(|anchor| anchor.event_id == event_id)
}

async fn wait_for_focused_context_closed(
    event_conn: &mut CoreConnection,
    request_id: RequestId,
    timeout: std::time::Duration,
) -> Result<(), String> {
    let deadline = tokio::time::Instant::now() + timeout;

    loop {
        if snapshot_has_no_focused_context(&event_conn.snapshot()) {
            return Ok(());
        }

        let event = tokio::time::timeout_at(deadline, event_conn.recv_event())
            .await
            .map_err(|_| "focused context did not close".to_owned())?;
        match event {
            Ok(CoreEvent::StateChanged(snapshot)) if snapshot_has_no_focused_context(&snapshot) => {
                return Ok(());
            }
            Ok(CoreEvent::OperationFailed {
                request_id: failed_request_id,
                ..
            }) if failed_request_id == request_id => {
                return Err("focused context close failed".to_owned());
            }
            Ok(_) => {}
            Err(_) if snapshot_has_no_focused_context(&event_conn.snapshot()) => {
                return Ok(());
            }
            Err(_) => return Err("focused context close event stream lagged".to_owned()),
        }
    }
}

async fn wait_for_focused_context(
    event_conn: &mut CoreConnection,
    request_id: RequestId,
    room_id: &str,
    timeout: std::time::Duration,
) -> Result<(), String> {
    let deadline = tokio::time::Instant::now() + timeout;

    loop {
        if snapshot_has_focused_context(&event_conn.snapshot(), room_id) {
            return Ok(());
        }

        let event = tokio::time::timeout_at(deadline, event_conn.recv_event())
            .await
            .map_err(|_| "focused context did not open".to_owned())?;
        match event {
            Ok(CoreEvent::StateChanged(snapshot))
                if snapshot_has_focused_context(&snapshot, room_id) =>
            {
                return Ok(());
            }
            Ok(CoreEvent::OperationFailed {
                request_id: failed_request_id,
                ..
            }) if failed_request_id == request_id => {
                return Err("focused context open failed".to_owned());
            }
            Ok(_) => {}
            Err(_) if snapshot_has_focused_context(&event_conn.snapshot(), room_id) => {
                return Ok(());
            }
            Err(_) => return Err("focused context event stream lagged".to_owned()),
        }
    }
}

async fn wait_for_main_timeline_anchor(
    event_conn: &mut CoreConnection,
    request_id: RequestId,
    room_id: &str,
    event_id: &str,
    timeout: std::time::Duration,
) -> Result<(), String> {
    let deadline = tokio::time::Instant::now() + timeout;

    loop {
        if snapshot_has_main_timeline_anchor(&event_conn.snapshot(), room_id, event_id) {
            return Ok(());
        }

        let event = tokio::time::timeout_at(deadline, event_conn.recv_event())
            .await
            .map_err(|_| "main timeline anchor did not open".to_owned())?;
        match event {
            Ok(CoreEvent::StateChanged(snapshot))
                if snapshot_has_main_timeline_anchor(&snapshot, room_id, event_id) =>
            {
                return Ok(());
            }
            Ok(CoreEvent::OperationFailed {
                request_id: failed_request_id,
                ..
            }) if failed_request_id == request_id => {
                return Err("main timeline anchor open failed".to_owned());
            }
            Ok(_) => {}
            Err(_)
                if snapshot_has_main_timeline_anchor(&event_conn.snapshot(), room_id, event_id) =>
            {
                return Ok(());
            }
            Err(_) => return Err("main timeline anchor event stream lagged".to_owned()),
        }
    }
}

async fn wait_for_selected_room(
    event_conn: &mut CoreConnection,
    select_request_id: RequestId,
    selected_room_id: &str,
    timeout: std::time::Duration,
) -> Result<(), String> {
    // Diagnostic-only, private-data-free probe (no room ids). Enable with
    // KOUSHI_SUBSCRIBE_TRACE=1 to see WHY select_room times out:
    //   events=0                      -> runtime/AppActor delivered nothing (hung)
    //   events>0, active=none         -> deltas flow but the reducer never set
    //                                    the active room (command unprocessed/rejected)
    //   events>0, active=other        -> a different room got selected
    let trace = std::env::var_os("KOUSHI_SUBSCRIBE_TRACE").is_some();
    let deadline = tokio::time::Instant::now() + timeout;
    let mut events: u32 = 0;
    let mut state_changed: u32 = 0;
    let mut state_delta: u32 = 0;

    loop {
        if snapshot_has_active_room(&event_conn.snapshot(), selected_room_id) {
            if trace {
                eprintln!(
                    "koushi.select stage=ok_watch events={events} state_changed={state_changed} state_delta={state_delta}"
                );
            }
            return Ok(());
        }

        let event = match tokio::time::timeout_at(deadline, event_conn.recv_event()).await {
            Ok(event) => event,
            Err(_) => {
                if trace {
                    let active =
                        select_active_room_trace_label(&event_conn.snapshot(), selected_room_id);
                    eprintln!(
                        "koushi.select stage=timeout events={events} state_changed={state_changed} state_delta={state_delta} active={active}"
                    );
                }
                return Err("room selection did not complete".to_owned());
            }
        };
        events += 1;
        match event {
            Ok(CoreEvent::StateChanged(snapshot)) => {
                state_changed += 1;
                if snapshot_has_active_room(&snapshot, selected_room_id) {
                    if trace {
                        eprintln!("koushi.select stage=ok_statechanged events={events}");
                    }
                    return Ok(());
                }
            }
            Ok(CoreEvent::StateDelta(_)) => {
                state_delta += 1;
            }
            Ok(CoreEvent::OperationFailed { request_id, .. })
                if request_id == select_request_id =>
            {
                if trace {
                    eprintln!("koushi.select stage=op_failed events={events}");
                }
                return Err("room selection failed".to_owned());
            }
            // Telemetry-lane fast path: IntentLifecycle lets us fail fast with
            // a specific reason instead of waiting the full 10s timeout.
            Ok(CoreEvent::IntentLifecycle {
                request_id,
                outcome,
            }) if request_id == select_request_id => {
                match outcome {
                    IntentOutcome::Committed | IntentOutcome::BenignNoOp(_) => {
                        if trace {
                            eprintln!(
                                "koushi.select stage=ok_intent events={events} outcome={outcome:?}"
                            );
                        }
                        return Ok(());
                    }
                    IntentOutcome::FailedNoOp(IntentNoOpReason::RoomNotInState) => {
                        if trace {
                            eprintln!("koushi.select stage=failed_not_in_state events={events}");
                        }
                        return Err("room not yet loaded".to_owned());
                    }
                    IntentOutcome::FailedNoOp(IntentNoOpReason::SessionNotReady) => {
                        if trace {
                            eprintln!(
                                "koushi.select stage=failed_session_not_ready events={events}"
                            );
                        }
                        return Err("session not ready".to_owned());
                    }
                    IntentOutcome::FailedNoOp(IntentNoOpReason::AlreadyActive) => {
                        // AlreadyActive is benign; this arm is unreachable per
                        // the classification logic but handle it defensively.
                        if trace {
                            eprintln!("koushi.select stage=ok_already_active events={events}");
                        }
                        return Ok(());
                    }
                }
            }
            Ok(_) => {}
            Err(_) if snapshot_has_active_room(&event_conn.snapshot(), selected_room_id) => {
                if trace {
                    eprintln!("koushi.select stage=ok_after_lag events={events}");
                }
                return Ok(());
            }
            Err(_) => {
                if trace {
                    eprintln!("koushi.select stage=lag events={events}");
                }
                return Err("room selection event stream lagged".to_owned());
            }
        }
    }
}

fn snapshot_has_active_room(snapshot: &koushi_state::AppState, room_id: &str) -> bool {
    snapshot.navigation.active_room_id.as_deref() == Some(room_id)
}

fn snapshot_has_completed_search(snapshot: &koushi_state::AppState, request_id: RequestId) -> bool {
    match &snapshot.search {
        koushi_state::SearchState::Results {
            request_id: state_request_id,
            ..
        }
        | koushi_state::SearchState::Failed {
            request_id: state_request_id,
            ..
        } => *state_request_id == request_id.sequence,
        _ => false,
    }
}

fn snapshot_has_closed_search(snapshot: &koushi_state::AppState) -> bool {
    snapshot.search == koushi_state::SearchState::Closed
}

async fn wait_for_search_completed(
    event_conn: &mut CoreConnection,
    request_id: RequestId,
    timeout: std::time::Duration,
) -> Result<(), String> {
    let deadline = tokio::time::Instant::now() + timeout;

    loop {
        if snapshot_has_completed_search(&event_conn.snapshot(), request_id) {
            return Ok(());
        }

        let event = tokio::time::timeout_at(deadline, event_conn.recv_event())
            .await
            .map_err(|_| "search did not complete".to_owned())?;
        match event {
            Ok(CoreEvent::Search(SearchEvent::Results {
                request_id: result_request_id,
                ..
            })) if result_request_id == request_id => {}
            Ok(CoreEvent::OperationFailed {
                request_id: failed_request_id,
                ..
            }) if failed_request_id == request_id => return Err("search failed".to_owned()),
            Ok(CoreEvent::StateChanged(snapshot))
                if snapshot_has_completed_search(&snapshot, request_id) =>
            {
                return Ok(());
            }
            Ok(_) => {}
            Err(_) if snapshot_has_completed_search(&event_conn.snapshot(), request_id) => {
                return Ok(());
            }
            Err(_) => return Err("search event stream lagged".to_owned()),
        }
    }
}

async fn wait_for_search_closed(
    event_conn: &mut CoreConnection,
    request_id: RequestId,
    timeout: std::time::Duration,
) -> Result<(), String> {
    let deadline = tokio::time::Instant::now() + timeout;

    loop {
        if snapshot_has_closed_search(&event_conn.snapshot()) {
            return Ok(());
        }

        let event = tokio::time::timeout_at(deadline, event_conn.recv_event())
            .await
            .map_err(|_| "search did not close".to_owned())?;
        match event {
            Ok(CoreEvent::StateChanged(snapshot)) if snapshot_has_closed_search(&snapshot) => {
                return Ok(());
            }
            Ok(CoreEvent::OperationFailed {
                request_id: failed_request_id,
                ..
            }) if failed_request_id == request_id => return Err("search close failed".to_owned()),
            Ok(_) => {}
            Err(_) if snapshot_has_closed_search(&event_conn.snapshot()) => return Ok(()),
            Err(_) => return Err("search close event stream lagged".to_owned()),
        }
    }
}

fn select_active_room_trace_label(
    snapshot: &koushi_state::AppState,
    selected_room_id: &str,
) -> &'static str {
    match snapshot.navigation.active_room_id.as_deref() {
        None => "none",
        Some(id) if id == selected_room_id => "match",
        Some(_) => "other",
    }
}

async fn wait_for_upload_staging_snapshot(
    event_conn: &mut CoreConnection,
    request_id: RequestId,
    predicate: impl Fn(&koushi_state::AppState) -> bool,
    description: &str,
) -> Result<(), String> {
    let deadline = tokio::time::Instant::now() + UPLOAD_STAGING_EVENT_TIMEOUT;

    loop {
        if predicate(&event_conn.snapshot()) {
            return Ok(());
        }

        let event = tokio::time::timeout_at(deadline, event_conn.recv_event())
            .await
            .map_err(|_| description.to_owned())?;
        match event {
            Ok(CoreEvent::StateChanged(snapshot)) if predicate(&snapshot) => return Ok(()),
            Ok(CoreEvent::OperationFailed {
                request_id: failed_request_id,
                ..
            }) if failed_request_id == request_id => return Err(description.to_owned()),
            Ok(_) => {}
            Err(_) if predicate(&event_conn.snapshot()) => return Ok(()),
            Err(_) => return Err("upload staging event stream lagged".to_owned()),
        }
    }
}

/// How long the adapter waits for the `SavedSessionsListed` answer before
/// reporting a transport error. The query is a local credential-store read in
/// core, so 5 seconds is generous.
const SAVED_SESSIONS_EVENT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

pub(crate) async fn submit_recovery_request(
    app: AppHandle,
    state: &CoreRuntimeState,
    secret: AuthSecret,
) -> Result<(), String> {
    let request_id = next_request_id(state).await;
    submit_core_command(state, build_submit_recovery_command(request_id, secret)).await?;
    update_qa_window_title_from_state(&app, state).await;
    Ok(())
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StageUploadInputItem {
    staged_id: String,
    position: u64,
    filename: String,
    mime_type: String,
    byte_count: u64,
    kind: StagedUploadKind,
    compression_choice: StagedUploadCompressionChoice,
}

const CREATE_EVENT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(15);

async fn wait_for_room_created(
    event_conn: &mut CoreConnection,
    create_request_id: RequestId,
    timeout: std::time::Duration,
) -> Result<(), String> {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        let event = tokio::time::timeout_at(deadline, event_conn.recv_event())
            .await
            .map_err(|_| "room creation did not complete".to_owned())?;
        match event {
            Ok(CoreEvent::Room(RoomEvent::RoomCreated { request_id, .. }))
                if request_id == create_request_id =>
            {
                return Ok(());
            }
            Ok(CoreEvent::OperationFailed { request_id, .. })
                if request_id == create_request_id =>
            {
                return Err("room creation failed".to_owned());
            }
            Ok(_) => {}
            Err(_) => return Err("room creation event stream lagged".to_owned()),
        }
    }
}

async fn wait_for_space_created(
    event_conn: &mut CoreConnection,
    create_request_id: RequestId,
    timeout: std::time::Duration,
) -> Result<(), String> {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        let event = tokio::time::timeout_at(deadline, event_conn.recv_event())
            .await
            .map_err(|_| "space creation did not complete".to_owned())?;
        match event {
            Ok(CoreEvent::Room(RoomEvent::SpaceCreated { request_id, .. }))
                if request_id == create_request_id =>
            {
                return Ok(());
            }
            Ok(CoreEvent::OperationFailed { request_id, .. })
                if request_id == create_request_id =>
            {
                return Err("space creation failed".to_owned());
            }
            Ok(_) => {}
            Err(_) => return Err("space creation event stream lagged".to_owned()),
        }
    }
}

async fn wait_for_room_operation<F>(
    event_conn: &mut CoreConnection,
    operation_request_id: RequestId,
    timeout: std::time::Duration,
    is_success: F,
    timeout_message: &'static str,
    failure_message: &'static str,
) -> Result<(), String>
where
    F: Fn(&RoomEvent, RequestId) -> bool,
{
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        let event = tokio::time::timeout_at(deadline, event_conn.recv_event())
            .await
            .map_err(|_| timeout_message.to_owned())?;
        match event {
            Ok(CoreEvent::Room(room_event)) if is_success(&room_event, operation_request_id) => {
                return Ok(());
            }
            Ok(CoreEvent::OperationFailed { request_id, .. })
                if request_id == operation_request_id =>
            {
                return Err(failure_message.to_owned());
            }
            Ok(_) => {}
            Err(_) => return Err("room operation event stream lagged".to_owned()),
        }
    }
}

async fn wait_for_room_joined(
    event_conn: &mut CoreConnection,
    operation_request_id: RequestId,
    timeout: std::time::Duration,
) -> Result<String, String> {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        let event = tokio::time::timeout_at(deadline, event_conn.recv_event())
            .await
            .map_err(|_| "room join did not complete".to_owned())?;
        match event {
            Ok(CoreEvent::Room(RoomEvent::RoomJoined {
                request_id,
                room_id,
            })) if request_id == operation_request_id => {
                return Ok(room_id);
            }
            Ok(CoreEvent::OperationFailed { request_id, .. })
                if request_id == operation_request_id =>
            {
                return Err("room join failed".to_owned());
            }
            Ok(_) => {}
            Err(_) => return Err("room operation event stream lagged".to_owned()),
        }
    }
}

// ---- Helpers ----

pub(crate) fn build_submit_login_command(
    request_id: koushi_core::RequestId,
    login_request: LoginRequest,
) -> CoreCommand {
    CoreCommand::Account(AccountCommand::LoginPassword {
        request_id,
        request: login_request,
    })
}

pub(crate) fn build_discover_login_command(
    request_id: koushi_core::RequestId,
    homeserver: String,
) -> CoreCommand {
    CoreCommand::Account(AccountCommand::DiscoverLogin {
        request_id,
        homeserver,
    })
}

pub(crate) fn build_start_oidc_login_command(
    request_id: koushi_core::RequestId,
    homeserver: String,
) -> CoreCommand {
    CoreCommand::Account(AccountCommand::StartOidcLogin {
        request_id,
        homeserver,
    })
}

pub(crate) fn build_complete_oidc_login_command(
    request_id: koushi_core::RequestId,
    callback_url: String,
) -> CoreCommand {
    CoreCommand::Account(AccountCommand::CompleteOidcLogin {
        request_id,
        callback_url,
    })
}

pub(crate) fn build_switch_account_command(
    request_id: koushi_core::RequestId,
    user_id: String,
) -> CoreCommand {
    CoreCommand::Account(AccountCommand::SwitchAccount {
        request_id,
        account_key: AccountKey(user_id),
    })
}

pub(crate) fn build_submit_recovery_command(
    request_id: koushi_core::RequestId,
    secret: AuthSecret,
) -> CoreCommand {
    CoreCommand::Account(AccountCommand::SubmitRecovery {
        request_id,
        request: RecoveryRequest { secret },
    })
}

pub(crate) fn build_logout_command(request_id: koushi_core::RequestId) -> CoreCommand {
    CoreCommand::Account(AccountCommand::Logout { request_id })
}

pub(crate) fn build_restart_sync_command(request_id: koushi_core::RequestId) -> CoreCommand {
    CoreCommand::Sync(SyncCommand::Restart { request_id })
}

pub(crate) fn build_update_settings_command(
    request_id: koushi_core::RequestId,
    patch: SettingsPatch,
) -> CoreCommand {
    CoreCommand::App(AppCommand::UpdateSettings { request_id, patch })
}

pub(crate) fn build_rebuild_search_index_command(
    request_id: koushi_core::RequestId,
) -> CoreCommand {
    CoreCommand::App(AppCommand::RebuildSearchIndex { request_id })
}

pub(crate) fn build_set_room_url_preview_override_command(
    request_id: koushi_core::RequestId,
    room_id: String,
    enabled: bool,
) -> CoreCommand {
    CoreCommand::App(AppCommand::SetRoomUrlPreviewOverride {
        request_id,
        room_id,
        enabled,
    })
}

pub(crate) fn build_probe_local_encryption_health_command(
    request_id: koushi_core::RequestId,
) -> CoreCommand {
    CoreCommand::Account(AccountCommand::ProbeLocalEncryptionHealth { request_id })
}

pub(crate) fn build_reset_local_data_command(request_id: koushi_core::RequestId) -> CoreCommand {
    CoreCommand::Account(AccountCommand::ResetLocalData { request_id })
}

pub(crate) fn build_open_activity_command(request_id: koushi_core::RequestId) -> CoreCommand {
    CoreCommand::App(AppCommand::OpenActivity { request_id })
}

pub(crate) fn build_close_activity_command(request_id: koushi_core::RequestId) -> CoreCommand {
    CoreCommand::App(AppCommand::CloseActivity { request_id })
}

pub(crate) fn build_set_activity_tab_command(
    request_id: koushi_core::RequestId,
    tab: ActivityTab,
) -> CoreCommand {
    CoreCommand::App(AppCommand::SetActivityTab { request_id, tab })
}

pub(crate) fn build_paginate_activity_command(
    request_id: koushi_core::RequestId,
    tab: ActivityTab,
    cursor: Option<String>,
) -> CoreCommand {
    CoreCommand::App(AppCommand::PaginateActivity {
        request_id,
        tab,
        cursor: optional_non_blank(cursor),
    })
}

pub(crate) fn build_mark_activity_read_command(
    request_id: koushi_core::RequestId,
    target: ActivityMarkReadTarget,
) -> CoreCommand {
    CoreCommand::App(AppCommand::MarkActivityRead { request_id, target })
}

pub(crate) fn build_open_files_view_command(
    request_id: koushi_core::RequestId,
    scope: FilesViewScope,
    filter: AttachmentFilter,
    sort: AttachmentSort,
) -> CoreCommand {
    CoreCommand::App(AppCommand::OpenFilesView {
        request_id,
        scope,
        filter,
        sort,
    })
}

pub(crate) fn build_close_files_view_command(request_id: koushi_core::RequestId) -> CoreCommand {
    CoreCommand::App(AppCommand::CloseFilesView { request_id })
}

pub(crate) fn build_open_threads_list_command(
    request_id: koushi_core::RequestId,
    room_id: String,
) -> CoreCommand {
    CoreCommand::App(AppCommand::OpenThreadsList {
        request_id,
        room_id,
    })
}

pub(crate) fn build_close_threads_list_command(request_id: koushi_core::RequestId) -> CoreCommand {
    CoreCommand::App(AppCommand::CloseThreadsList { request_id })
}

pub(crate) fn build_paginate_threads_list_command(
    request_id: koushi_core::RequestId,
    room_id: String,
) -> CoreCommand {
    CoreCommand::App(AppCommand::PaginateThreadsList {
        request_id,
        room_id,
    })
}

pub(crate) fn build_bootstrap_cross_signing_command(
    request_id: koushi_core::RequestId,
    auth: Option<AuthSecret>,
) -> CoreCommand {
    CoreCommand::Account(AccountCommand::BootstrapCrossSigning { request_id, auth })
}

pub(crate) fn build_enable_key_backup_command(request_id: koushi_core::RequestId) -> CoreCommand {
    CoreCommand::Account(AccountCommand::EnableKeyBackup {
        request_id,
        passphrase: None,
    })
}

pub(crate) fn build_bootstrap_secure_backup_command(
    request_id: koushi_core::RequestId,
    passphrase: Option<AuthSecret>,
    recovery_key_destination_path: Option<String>,
) -> CoreCommand {
    CoreCommand::Account(AccountCommand::BootstrapSecureBackup {
        request_id,
        request: SecureBackupSetupRequest {
            passphrase,
            recovery_key_destination_path: recovery_key_destination_path.map(PathBuf::from),
        },
    })
}

pub(crate) fn build_change_secure_backup_passphrase_command(
    request_id: koushi_core::RequestId,
    old_secret: AuthSecret,
    new_passphrase: AuthSecret,
    recovery_key_destination_path: Option<String>,
) -> CoreCommand {
    CoreCommand::Account(AccountCommand::ChangeSecureBackupPassphrase {
        request_id,
        request: SecureBackupPassphraseChangeRequest {
            old_secret,
            new_passphrase,
            recovery_key_destination_path: recovery_key_destination_path.map(PathBuf::from),
        },
    })
}

pub(crate) fn build_export_room_keys_command(
    request_id: koushi_core::RequestId,
    destination_path: String,
    passphrase: AuthSecret,
) -> CoreCommand {
    CoreCommand::Account(AccountCommand::ExportRoomKeys {
        request_id,
        request: RoomKeyExportRequest {
            destination_path: PathBuf::from(destination_path),
            passphrase,
        },
    })
}

pub(crate) fn build_import_room_keys_command(
    request_id: koushi_core::RequestId,
    source_path: String,
    passphrase: AuthSecret,
) -> CoreCommand {
    CoreCommand::Account(AccountCommand::ImportRoomKeys {
        request_id,
        request: RoomKeyImportRequest {
            source_path: PathBuf::from(source_path),
            passphrase,
        },
    })
}

pub(crate) fn build_accept_verification_command(
    request_id: koushi_core::RequestId,
    flow_id: u64,
) -> CoreCommand {
    CoreCommand::Account(AccountCommand::AcceptVerification {
        request_id,
        flow_id,
    })
}

pub(crate) fn build_confirm_sas_verification_command(
    request_id: koushi_core::RequestId,
    flow_id: u64,
) -> CoreCommand {
    CoreCommand::Account(AccountCommand::ConfirmSasVerification {
        request_id,
        flow_id,
    })
}

pub(crate) fn build_cancel_verification_command(
    request_id: koushi_core::RequestId,
    flow_id: u64,
    reason: VerificationCancelReason,
) -> CoreCommand {
    CoreCommand::Account(AccountCommand::CancelVerification {
        request_id,
        flow_id,
        reason,
    })
}

pub(crate) fn build_reset_identity_command(request_id: koushi_core::RequestId) -> CoreCommand {
    CoreCommand::Account(AccountCommand::ResetIdentity { request_id })
}

pub(crate) fn build_submit_identity_reset_password_command(
    request_id: koushi_core::RequestId,
    flow_id: u64,
    password: AuthSecret,
) -> CoreCommand {
    CoreCommand::Account(AccountCommand::SubmitIdentityResetAuth {
        request_id,
        flow_id,
        request: IdentityResetAuthRequest::UiaaPassword { password },
    })
}

pub(crate) fn build_submit_identity_reset_oauth_command(
    request_id: koushi_core::RequestId,
    flow_id: u64,
) -> CoreCommand {
    CoreCommand::Account(AccountCommand::SubmitIdentityResetAuth {
        request_id,
        flow_id,
        request: IdentityResetAuthRequest::OAuthApproved,
    })
}

pub(crate) fn build_submit_account_management_uia_command(
    request_id: koushi_core::RequestId,
    flow_id: u64,
    password: AuthSecret,
) -> CoreCommand {
    CoreCommand::Account(AccountCommand::SubmitAccountManagementUia {
        request_id,
        flow_id,
        auth: IdentityResetAuthRequest::UiaaPassword { password },
    })
}

pub(crate) fn build_select_space_command(
    request_id: koushi_core::RequestId,
    space_id: Option<String>,
) -> CoreCommand {
    CoreCommand::Room(RoomCommand::SelectSpace {
        request_id,
        space_id,
    })
}

pub(crate) fn build_reorder_spaces_command(
    request_id: koushi_core::RequestId,
    space_ids: Vec<String>,
) -> CoreCommand {
    CoreCommand::Room(RoomCommand::ReorderSpaces {
        request_id,
        space_ids,
    })
}

pub(crate) fn build_select_room_command(
    request_id: koushi_core::RequestId,
    room_id: String,
) -> CoreCommand {
    CoreCommand::Room(RoomCommand::SelectRoom {
        request_id,
        room_id,
    })
}

fn build_timeline_key(account_key: AccountKey, room_id: String) -> TimelineKey {
    TimelineKey {
        account_key,
        kind: TimelineKind::Room { room_id },
    }
}

#[cfg(test)]
pub(crate) fn build_subscribe_focused_timeline_command(
    request_id: koushi_core::RequestId,
    account_key: AccountKey,
    room_id: String,
    event_id: String,
) -> CoreCommand {
    CoreCommand::Timeline(TimelineCommand::Subscribe {
        request_id,
        key: TimelineKey {
            account_key,
            kind: TimelineKind::Focused { room_id, event_id },
        },
    })
}

pub(crate) fn build_paginate_timeline_backwards_command(
    request_id: koushi_core::RequestId,
    account_key: AccountKey,
    room_id: String,
) -> CoreCommand {
    CoreCommand::Timeline(TimelineCommand::Paginate {
        request_id,
        key: build_timeline_key(account_key, room_id),
        direction: PaginationDirection::Backward,
        event_count: TIMELINE_BACKWARDS_PAGE_EVENT_COUNT,
    })
}

pub(crate) fn build_paginate_thread_timeline_backwards_command(
    request_id: koushi_core::RequestId,
    account_key: AccountKey,
    room_id: String,
    root_event_id: String,
) -> CoreCommand {
    CoreCommand::Timeline(TimelineCommand::Paginate {
        request_id,
        key: TimelineKey {
            account_key,
            kind: TimelineKind::Thread {
                room_id,
                root_event_id,
            },
        },
        direction: PaginationDirection::Backward,
        event_count: TIMELINE_BACKWARDS_PAGE_EVENT_COUNT,
    })
}

pub(crate) fn build_restore_timeline_anchor_command(
    request_id: koushi_core::RequestId,
    account_key: AccountKey,
    timeline_key: TimelineKey,
    event_id: String,
    max_batches: u16,
    event_count: u16,
) -> CoreCommand {
    CoreCommand::Timeline(TimelineCommand::RestoreTimelineAnchor {
        request_id,
        key: TimelineKey {
            account_key,
            kind: timeline_key.kind,
        },
        event_id,
        max_batches,
        event_count,
    })
}

pub(crate) fn build_open_timeline_at_timestamp_command(
    request_id: koushi_core::RequestId,
    room_id: String,
    timestamp_ms: u64,
) -> CoreCommand {
    CoreCommand::App(AppCommand::OpenTimelineAtTimestamp {
        request_id,
        room_id,
        timestamp_ms,
    })
}

pub(crate) fn build_update_navigation_scroll_anchor_command(
    request_id: koushi_core::RequestId,
    room_id: String,
    anchor: TimelineScrollAnchor,
) -> CoreCommand {
    CoreCommand::App(AppCommand::TimelineScrollAnchorUpdated {
        request_id,
        room_id,
        anchor,
    })
}

pub(crate) fn build_observe_timeline_viewport_command(
    request_id: koushi_core::RequestId,
    account_key: AccountKey,
    room_id: String,
    first_visible_event_id: Option<String>,
    last_visible_event_id: Option<String>,
    at_bottom: bool,
) -> CoreCommand {
    CoreCommand::Timeline(TimelineCommand::ObserveViewport {
        request_id,
        key: build_timeline_key(account_key, room_id),
        observation: TimelineViewportObservation {
            first_visible_event_id,
            last_visible_event_id,
            at_bottom,
        },
    })
}

pub(crate) fn build_send_text_command(
    request_id: koushi_core::RequestId,
    account_key: AccountKey,
    room_id: String,
    transaction_id: String,
    body: String,
    mentions: koushi_state::MentionIntent,
) -> Option<CoreCommand> {
    if body.trim().is_empty() {
        return None;
    }
    Some(CoreCommand::Timeline(TimelineCommand::SendText {
        request_id,
        key: build_timeline_key(account_key, room_id),
        transaction_id,
        body,
        mentions,
    }))
}

pub(crate) fn build_schedule_send_command(
    request_id: koushi_core::RequestId,
    room_id: String,
    body: String,
    send_at_ms: u64,
) -> Option<CoreCommand> {
    if body.trim().is_empty() {
        return None;
    }
    Some(CoreCommand::App(AppCommand::ScheduleSend {
        request_id,
        room_id,
        body,
        send_at_ms,
    }))
}

pub(crate) fn build_set_upload_staging_command(
    request_id: koushi_core::RequestId,
    room_id: String,
    items: Vec<StageUploadInputItem>,
) -> CoreCommand {
    let room_id = room_id.trim().to_owned();
    let staged_items = items
        .into_iter()
        .filter(|item| !item.staged_id.trim().is_empty())
        .map(|item| StagedUploadItem {
            staged_id: item.staged_id,
            room_id: room_id.clone(),
            position: item.position,
            filename: match item.filename.trim() {
                "" => "attachment".to_owned(),
                value => value.to_owned(),
            },
            mime_type: match item.mime_type.trim() {
                "" => "application/octet-stream".to_owned(),
                value => value.to_owned(),
            },
            byte_count: item.byte_count,
            kind: item.kind,
            caption: None,
            compression_choice: item.compression_choice,
        })
        .collect();
    CoreCommand::App(AppCommand::SetUploadStaging {
        request_id,
        room_id,
        items: staged_items,
    })
}

pub(crate) fn build_cancel_scheduled_send_command(
    request_id: koushi_core::RequestId,
    scheduled_id: String,
) -> Option<CoreCommand> {
    if scheduled_id.trim().is_empty() {
        return None;
    }
    Some(CoreCommand::App(AppCommand::CancelScheduledSend {
        request_id,
        scheduled_id,
    }))
}

pub(crate) fn build_reschedule_scheduled_send_command(
    request_id: koushi_core::RequestId,
    scheduled_id: String,
    send_at_ms: u64,
) -> Option<CoreCommand> {
    if scheduled_id.trim().is_empty() {
        return None;
    }
    Some(CoreCommand::App(AppCommand::RescheduleScheduledSend {
        request_id,
        scheduled_id,
        send_at_ms,
    }))
}

pub(crate) fn build_retry_send_command(
    request_id: koushi_core::RequestId,
    account_key: AccountKey,
    room_id: String,
    transaction_id: String,
) -> Option<CoreCommand> {
    if transaction_id.trim().is_empty() {
        return None;
    }
    Some(CoreCommand::Timeline(TimelineCommand::RetrySend {
        request_id,
        key: build_timeline_key(account_key, room_id),
        transaction_id,
    }))
}

pub(crate) fn build_cancel_send_command(
    request_id: koushi_core::RequestId,
    account_key: AccountKey,
    room_id: String,
    transaction_id: String,
) -> Option<CoreCommand> {
    if transaction_id.trim().is_empty() {
        return None;
    }
    Some(CoreCommand::Timeline(TimelineCommand::CancelSend {
        request_id,
        key: build_timeline_key(account_key, room_id),
        transaction_id,
    }))
}

pub(crate) fn build_upload_media_command(
    request_id: koushi_core::RequestId,
    account_key: AccountKey,
    room_id: String,
    transaction_id: String,
    filename: String,
    mime_type: String,
    bytes: Vec<u8>,
    caption: Option<String>,
    image_compression_mode: ImageUploadCompressionMode,
    image_compression_policy: ImageUploadCompressionPolicy,
    image_dimensions: Option<ImageUploadDimensions>,
    image_compression: Option<ImageUploadCompressionState>,
    thumbnail: Option<UploadMediaThumbnail>,
) -> Option<CoreCommand> {
    if bytes.is_empty() {
        return None;
    }
    let filename = match filename.trim() {
        "" => "attachment".to_owned(),
        value => value.to_owned(),
    };
    let mime_type = match mime_type.trim() {
        "" => "application/octet-stream".to_owned(),
        value => value.to_owned(),
    };
    let is_image = mime_type.to_ascii_lowercase().starts_with("image/");
    let selected_byte_count = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
    let image_compression = if is_image {
        Some(normalize_image_upload_compression(
            image_compression_mode,
            image_compression_policy,
            mime_type.clone(),
            selected_byte_count,
            image_dimensions,
            image_compression,
            thumbnail.is_some(),
        ))
    } else {
        None
    };
    let selected_dimensions = image_compression
        .as_ref()
        .and_then(|compression| compression.selected.dimensions)
        .or(image_dimensions);
    let kind = if is_image {
        UploadMediaKind::Image {
            width: selected_dimensions.map(|dimensions| dimensions.width),
            height: selected_dimensions.map(|dimensions| dimensions.height),
        }
    } else {
        UploadMediaKind::File
    };

    Some(CoreCommand::Timeline(TimelineCommand::UploadAndSendMedia {
        request_id,
        key: build_timeline_key(account_key, room_id),
        transaction_id,
        request: UploadMediaRequest {
            filename,
            mime_type,
            bytes,
            kind,
            compression: image_compression,
            thumbnail: if is_image { thumbnail } else { None },
            caption: media_caption_from_composer_body(caption),
        },
    }))
}

fn normalize_image_upload_compression(
    mode: ImageUploadCompressionMode,
    policy: ImageUploadCompressionPolicy,
    mime_type: String,
    selected_byte_count: u64,
    image_dimensions: Option<ImageUploadDimensions>,
    image_compression: Option<ImageUploadCompressionState>,
    thumbnail_present: bool,
) -> ImageUploadCompressionState {
    match image_compression {
        Some(mut compression) => {
            compression.mode = mode;
            compression.policy = policy;
            if compression.original.mime_type.trim().is_empty() {
                compression.original.mime_type = mime_type.clone();
            }
            if compression.selected.mime_type.trim().is_empty() {
                compression.selected.mime_type = mime_type;
            }
            compression.selected.byte_count = selected_byte_count;
            if compression.selected.dimensions.is_none() {
                compression.selected.dimensions = image_dimensions;
            }
            if compression.selected_variant == ImageUploadVariantKind::Original {
                compression.metadata_stripped = false;
            }
            if thumbnail_present {
                compression.thumbnail_refreshed = true;
            }
            compression
        }
        None => {
            let mut compression = ImageUploadCompressionState::original(
                mode,
                mime_type,
                selected_byte_count,
                image_dimensions,
            );
            compression.policy = policy;
            compression.skipped_small_image = policy.should_skip(&compression.original);
            compression
        }
    }
}

fn media_caption_from_composer_body(
    caption: Option<String>,
) -> Option<koushi_state::FormattedMessageDraft> {
    let caption = caption?.trim().to_owned();
    if caption.is_empty() {
        return None;
    }
    Some(build_formatted_message_draft(
        caption,
        MentionIntent::default(),
    ))
}

pub(crate) fn build_download_media_command(
    request_id: koushi_core::RequestId,
    account_key: AccountKey,
    room_id: String,
    event_id: String,
) -> Option<CoreCommand> {
    if event_id.trim().is_empty() {
        return None;
    }
    Some(CoreCommand::Timeline(TimelineCommand::DownloadMedia {
        request_id,
        key: build_timeline_key(account_key, room_id),
        event_id,
        selection: MediaDownloadSelection::File,
    }))
}

pub(crate) fn build_load_message_source_command(
    request_id: koushi_core::RequestId,
    account_key: AccountKey,
    room_id: String,
    event_id: String,
) -> Option<CoreCommand> {
    if event_id.trim().is_empty() {
        return None;
    }
    Some(CoreCommand::Timeline(TimelineCommand::LoadMessageSource {
        request_id,
        key: build_timeline_key(account_key, room_id),
        event_id,
    }))
}

pub(crate) fn build_request_room_key_command(
    request_id: koushi_core::RequestId,
    account_key: AccountKey,
    room_id: String,
    event_id: String,
) -> Option<CoreCommand> {
    if event_id.trim().is_empty() {
        return None;
    }
    Some(CoreCommand::Timeline(TimelineCommand::RequestRoomKey {
        request_id,
        key: build_timeline_key(account_key, room_id),
        event_id,
    }))
}

pub(crate) fn build_load_link_previews_command(
    request_id: koushi_core::RequestId,
    account_key: AccountKey,
    room_id: String,
    event_id: String,
) -> Option<CoreCommand> {
    if event_id.trim().is_empty() {
        return None;
    }
    Some(CoreCommand::Timeline(TimelineCommand::LoadLinkPreviews {
        request_id,
        key: build_timeline_key(account_key, room_id),
        event_id,
    }))
}

pub(crate) fn build_hide_link_preview_command(
    request_id: koushi_core::RequestId,
    account_key: AccountKey,
    room_id: String,
    event_id: String,
) -> Option<CoreCommand> {
    if event_id.trim().is_empty() {
        return None;
    }
    Some(CoreCommand::Timeline(TimelineCommand::HideLinkPreview {
        request_id,
        key: build_timeline_key(account_key, room_id),
        event_id,
    }))
}

pub(crate) fn build_forward_message_command(
    request_id: koushi_core::RequestId,
    account_key: AccountKey,
    room_id: String,
    source_event_id: String,
    destination_room_id: String,
    transaction_id: String,
) -> Option<CoreCommand> {
    if source_event_id.trim().is_empty()
        || destination_room_id.trim().is_empty()
        || transaction_id.trim().is_empty()
    {
        return None;
    }
    Some(CoreCommand::Timeline(TimelineCommand::ForwardMessage {
        request_id,
        key: build_timeline_key(account_key, room_id),
        source_event_id,
        destination_room_id,
        transaction_id,
    }))
}

pub(crate) fn build_edit_message_command(
    request_id: koushi_core::RequestId,
    account_key: AccountKey,
    room_id: String,
    event_id: String,
    body: String,
) -> Option<CoreCommand> {
    if body.trim().is_empty() {
        return None;
    }
    Some(CoreCommand::Timeline(TimelineCommand::EditText {
        request_id,
        key: build_timeline_key(account_key, room_id),
        event_id,
        body,
    }))
}

pub(crate) fn build_redact_message_command(
    request_id: koushi_core::RequestId,
    account_key: AccountKey,
    room_id: String,
    event_id: String,
) -> CoreCommand {
    CoreCommand::Timeline(TimelineCommand::Redact {
        request_id,
        key: build_timeline_key(account_key, room_id),
        event_id,
    })
}

pub(crate) fn build_toggle_reaction_command(
    request_id: koushi_core::RequestId,
    account_key: AccountKey,
    room_id: String,
    event_id: String,
    reaction_key: String,
) -> Option<CoreCommand> {
    if reaction_key.is_empty() {
        return None;
    }
    Some(CoreCommand::Timeline(TimelineCommand::ToggleReaction {
        request_id,
        key: build_timeline_key(account_key, room_id),
        event_id,
        reaction_key,
    }))
}

pub(crate) fn build_send_reaction_command(
    request_id: koushi_core::RequestId,
    account_key: AccountKey,
    room_id: String,
    event_id: String,
    reaction_key: String,
) -> Option<CoreCommand> {
    if event_id.trim().is_empty() || reaction_key.trim().is_empty() {
        return None;
    }
    Some(CoreCommand::Timeline(TimelineCommand::SendReaction {
        request_id,
        key: build_timeline_key(account_key, room_id),
        event_id,
        reaction_key,
    }))
}

pub(crate) fn build_redact_reaction_command(
    request_id: koushi_core::RequestId,
    account_key: AccountKey,
    room_id: String,
    event_id: String,
    reaction_key: String,
    reaction_event_id: String,
) -> Option<CoreCommand> {
    if event_id.trim().is_empty()
        || reaction_key.trim().is_empty()
        || reaction_event_id.trim().is_empty()
    {
        return None;
    }
    Some(CoreCommand::Timeline(TimelineCommand::RedactReaction {
        request_id,
        key: build_timeline_key(account_key, room_id),
        event_id,
        reaction_key,
        reaction_event_id,
    }))
}

pub(crate) fn build_send_read_receipt_command(
    request_id: koushi_core::RequestId,
    account_key: AccountKey,
    room_id: String,
    event_id: String,
) -> Option<CoreCommand> {
    if event_id.trim().is_empty() {
        return None;
    }
    Some(CoreCommand::Timeline(TimelineCommand::SendReadReceipt {
        request_id,
        key: build_timeline_key(account_key, room_id),
        event_id,
    }))
}

pub(crate) fn build_set_fully_read_command(
    request_id: koushi_core::RequestId,
    account_key: AccountKey,
    room_id: String,
    event_id: String,
) -> Option<CoreCommand> {
    if event_id.trim().is_empty() {
        return None;
    }
    Some(CoreCommand::Timeline(TimelineCommand::SetFullyRead {
        request_id,
        key: build_timeline_key(account_key, room_id),
        event_id,
    }))
}

pub(crate) fn build_set_typing_command(
    request_id: koushi_core::RequestId,
    account_key: AccountKey,
    room_id: String,
    is_typing: bool,
) -> CoreCommand {
    CoreCommand::Timeline(TimelineCommand::SetTyping {
        request_id,
        key: build_timeline_key(account_key, room_id),
        is_typing,
    })
}

pub(crate) fn build_set_presence_command(
    request_id: koushi_core::RequestId,
    presence: PresenceKind,
) -> CoreCommand {
    CoreCommand::Account(AccountCommand::SetPresence {
        request_id,
        presence,
    })
}

pub(crate) fn build_set_display_name_command(
    request_id: koushi_core::RequestId,
    display_name: Option<String>,
) -> CoreCommand {
    CoreCommand::Account(AccountCommand::SetDisplayName {
        request_id,
        display_name,
    })
}

pub(crate) fn build_set_local_user_alias_command(
    request_id: koushi_core::RequestId,
    user_id: String,
    alias: Option<String>,
) -> CoreCommand {
    CoreCommand::Account(AccountCommand::SetLocalUserAlias {
        request_id,
        user_id,
        alias,
    })
}

pub(crate) fn build_ignore_user_command(
    request_id: koushi_core::RequestId,
    user_id: String,
) -> CoreCommand {
    CoreCommand::Account(AccountCommand::IgnoreUser {
        request_id,
        user_id,
    })
}

pub(crate) fn build_unignore_user_command(
    request_id: koushi_core::RequestId,
    user_id: String,
) -> CoreCommand {
    CoreCommand::Account(AccountCommand::UnignoreUser {
        request_id,
        user_id,
    })
}

pub(crate) fn build_report_user_command(
    request_id: koushi_core::RequestId,
    user_id: String,
    reason: Option<String>,
) -> CoreCommand {
    CoreCommand::Account(AccountCommand::ReportUser {
        request_id,
        user_id,
        reason: reason.unwrap_or_default(),
    })
}

pub(crate) fn build_report_content_command(
    request_id: koushi_core::RequestId,
    room_id: String,
    event_id: String,
    reason: Option<String>,
) -> CoreCommand {
    CoreCommand::Room(RoomCommand::ReportContent {
        request_id,
        room_id,
        event_id,
        reason,
    })
}

pub(crate) fn build_report_room_command(
    request_id: koushi_core::RequestId,
    room_id: String,
    reason: Option<String>,
) -> CoreCommand {
    CoreCommand::Room(RoomCommand::ReportRoom {
        request_id,
        room_id,
        reason: reason.unwrap_or_default(),
    })
}

pub(crate) fn build_set_avatar_command(
    request_id: koushi_core::RequestId,
    mime_type: String,
    bytes: Vec<u8>,
) -> CoreCommand {
    CoreCommand::Account(AccountCommand::SetAvatar {
        request_id,
        request: SetAvatarRequest { mime_type, bytes },
    })
}

pub(crate) fn build_download_avatar_thumbnail_command(
    request_id: koushi_core::RequestId,
    mxc_uri: String,
) -> CoreCommand {
    CoreCommand::Account(AccountCommand::DownloadAvatarThumbnail {
        request_id,
        mxc_uri,
    })
}

pub(crate) fn build_leave_room_command(
    request_id: koushi_core::RequestId,
    room_id: String,
) -> CoreCommand {
    CoreCommand::Room(RoomCommand::LeaveRoom {
        request_id,
        room_id,
    })
}

pub(crate) fn build_forget_room_command(
    request_id: koushi_core::RequestId,
    room_id: String,
) -> CoreCommand {
    CoreCommand::Room(RoomCommand::ForgetRoom {
        request_id,
        room_id,
    })
}

pub(crate) fn build_set_room_tag_command(
    request_id: koushi_core::RequestId,
    room_id: String,
    tag: RoomTagKind,
    order: Option<f64>,
) -> CoreCommand {
    CoreCommand::Room(RoomCommand::SetTag {
        request_id,
        room_id,
        tag,
        order,
    })
}

pub(crate) fn build_remove_room_tag_command(
    request_id: koushi_core::RequestId,
    room_id: String,
    tag: RoomTagKind,
) -> CoreCommand {
    CoreCommand::Room(RoomCommand::RemoveTag {
        request_id,
        room_id,
        tag,
    })
}

pub(crate) fn build_pin_event_command(
    request_id: koushi_core::RequestId,
    room_id: String,
    event_id: String,
) -> CoreCommand {
    CoreCommand::Room(RoomCommand::PinEvent {
        request_id,
        room_id,
        event_id,
    })
}

pub(crate) fn build_unpin_event_command(
    request_id: koushi_core::RequestId,
    room_id: String,
    event_id: String,
) -> CoreCommand {
    CoreCommand::Room(RoomCommand::UnpinEvent {
        request_id,
        room_id,
        event_id,
    })
}

pub(crate) fn build_load_room_settings_command(
    request_id: koushi_core::RequestId,
    room_id: String,
) -> CoreCommand {
    CoreCommand::Room(RoomCommand::LoadRoomSettings {
        request_id,
        room_id,
    })
}

pub(crate) fn build_reshare_room_key_command(
    request_id: koushi_core::RequestId,
    room_id: String,
) -> CoreCommand {
    CoreCommand::Room(RoomCommand::ReshareRoomKey {
        request_id,
        room_id,
    })
}

pub(crate) fn build_update_room_setting_command(
    request_id: koushi_core::RequestId,
    room_id: String,
    change: RoomSettingChange,
) -> CoreCommand {
    CoreCommand::Room(RoomCommand::UpdateRoomSetting {
        request_id,
        room_id,
        change,
    })
}

pub(crate) fn build_moderate_room_member_command(
    request_id: koushi_core::RequestId,
    room_id: String,
    target_user_id: String,
    action: RoomModerationAction,
    reason: Option<String>,
) -> CoreCommand {
    CoreCommand::Room(RoomCommand::ModerateRoomMember {
        request_id,
        room_id,
        target_user_id,
        action,
        reason,
    })
}

pub(crate) fn build_update_room_member_role_command(
    request_id: koushi_core::RequestId,
    room_id: String,
    target_user_id: String,
    power_level: i64,
) -> CoreCommand {
    CoreCommand::Room(RoomCommand::UpdateRoomMemberRole {
        request_id,
        room_id,
        target_user_id,
        power_level,
    })
}

pub(crate) fn build_submit_search_command(
    request_id: koushi_core::RequestId,
    query: String,
    scope: SearchScope,
) -> CoreCommand {
    CoreCommand::Search(SearchCommand::Query {
        request_id,
        query,
        scope,
    })
}

pub(crate) fn build_close_search_command(request_id: koushi_core::RequestId) -> CoreCommand {
    CoreCommand::App(AppCommand::CloseSearch { request_id })
}

pub(crate) fn build_create_room_command(
    request_id: koushi_core::RequestId,
    options: CreateRoomOptions,
) -> CoreCommand {
    CoreCommand::Room(RoomCommand::CreateRoom {
        request_id,
        options,
    })
}

pub(crate) fn build_create_space_command(
    request_id: koushi_core::RequestId,
    name: String,
) -> CoreCommand {
    CoreCommand::Room(RoomCommand::CreateSpace { request_id, name })
}

pub(crate) fn build_query_directory_command(
    request_id: koushi_core::RequestId,
    term: Option<String>,
    server_name: Option<String>,
    limit: Option<u32>,
    since: Option<String>,
) -> CoreCommand {
    CoreCommand::Room(RoomCommand::QueryDirectory {
        request_id,
        query: DirectoryQuery {
            term: optional_non_blank(term),
            server_name: optional_non_blank(server_name),
            limit,
            since: optional_non_blank(since),
        },
    })
}

pub(crate) fn build_join_directory_room_command(
    request_id: koushi_core::RequestId,
    alias: String,
    via_server: Option<String>,
) -> Option<CoreCommand> {
    let alias = alias.trim().to_owned();
    if alias.is_empty() {
        return None;
    }
    Some(CoreCommand::Room(RoomCommand::JoinDirectoryRoom {
        request_id,
        alias,
        via_server: optional_non_blank(via_server),
    }))
}

pub(crate) fn build_join_room_command(
    request_id: koushi_core::RequestId,
    room_id: String,
) -> Option<CoreCommand> {
    let room_id = room_id.trim().to_owned();
    if room_id.is_empty() {
        return None;
    }
    Some(CoreCommand::Room(RoomCommand::JoinRoom {
        request_id,
        room_id,
    }))
}

fn optional_non_blank(value: Option<String>) -> Option<String> {
    value.and_then(|value| {
        let trimmed = value.trim();
        (!trimmed.is_empty()).then(|| trimmed.to_owned())
    })
}

pub(crate) fn build_set_space_child_command(
    request_id: koushi_core::RequestId,
    space_id: String,
    child_room_id: String,
    via_server: String,
) -> CoreCommand {
    CoreCommand::Room(RoomCommand::SetSpaceChild {
        request_id,
        space_id,
        child_room_id,
        via_server,
    })
}

pub(crate) fn build_accept_invite_command(
    request_id: koushi_core::RequestId,
    room_id: String,
) -> CoreCommand {
    CoreCommand::Room(RoomCommand::AcceptInvite {
        request_id,
        room_id,
    })
}

pub(crate) fn build_decline_invite_command(
    request_id: koushi_core::RequestId,
    room_id: String,
) -> CoreCommand {
    CoreCommand::Room(RoomCommand::DeclineInvite {
        request_id,
        room_id,
    })
}

pub(crate) fn build_start_direct_message_command(
    request_id: koushi_core::RequestId,
    user_id: String,
) -> CoreCommand {
    CoreCommand::Room(RoomCommand::StartDirectMessage {
        request_id,
        user_id,
    })
}

pub(crate) fn build_invite_user_command(
    request_id: koushi_core::RequestId,
    room_id: String,
    user_id: String,
) -> CoreCommand {
    CoreCommand::Room(RoomCommand::InviteUser {
        request_id,
        room_id,
        user_id,
    })
}

pub(crate) fn build_open_invite_workflow_command(
    request_id: koushi_core::RequestId,
    room_id: String,
) -> CoreCommand {
    CoreCommand::App(AppCommand::OpenInviteWorkflow {
        request_id,
        room_id,
    })
}

pub(crate) fn build_close_invite_workflow_command(
    request_id: koushi_core::RequestId,
) -> CoreCommand {
    CoreCommand::App(AppCommand::CloseInviteWorkflow { request_id })
}

pub(crate) fn build_search_invite_targets_command(
    request_id: koushi_core::RequestId,
    room_id: String,
    query: String,
) -> CoreCommand {
    CoreCommand::App(AppCommand::SearchInviteTargets {
        request_id,
        room_id,
        query,
    })
}

pub(crate) fn build_select_invite_target_command(
    request_id: koushi_core::RequestId,
    room_id: String,
    user_id: String,
) -> CoreCommand {
    CoreCommand::App(AppCommand::SelectInviteTarget {
        request_id,
        room_id,
        user_id,
    })
}

pub(crate) fn build_remove_invite_target_command(
    request_id: koushi_core::RequestId,
    user_id: String,
) -> CoreCommand {
    CoreCommand::App(AppCommand::RemoveInviteTarget {
        request_id,
        user_id,
    })
}

pub(crate) fn build_invite_targets_command(
    request_id: koushi_core::RequestId,
    room_id: String,
    user_ids: Vec<String>,
    scope: InviteScopeSelection,
) -> CoreCommand {
    CoreCommand::Room(RoomCommand::InviteTargets {
        request_id,
        room_id,
        user_ids,
        scope,
    })
}

pub(crate) fn build_send_reply_command(
    request_id: koushi_core::RequestId,
    account_key: AccountKey,
    room_id: String,
    transaction_id: String,
    in_reply_to_event_id: String,
    body: String,
    mentions: koushi_state::MentionIntent,
) -> Option<CoreCommand> {
    if body.trim().is_empty() {
        return None;
    }
    Some(CoreCommand::Timeline(TimelineCommand::SendReply {
        request_id,
        key: build_timeline_key(account_key, room_id),
        transaction_id,
        in_reply_to_event_id,
        body,
        mentions,
    }))
}

pub(crate) fn build_set_thread_composer_draft_command(
    request_id: koushi_core::RequestId,
    room_id: String,
    root_event_id: String,
    draft: String,
) -> CoreCommand {
    CoreCommand::App(AppCommand::SetThreadComposerDraft {
        request_id,
        room_id,
        root_event_id,
        draft,
    })
}

pub(crate) fn build_set_composer_draft_command(
    request_id: koushi_core::RequestId,
    room_id: String,
    draft: String,
) -> CoreCommand {
    CoreCommand::App(AppCommand::SetComposerDraft {
        request_id,
        room_id,
        draft,
    })
}

pub(crate) fn build_send_thread_reply_command(
    request_id: koushi_core::RequestId,
    account_key: AccountKey,
    room_id: String,
    root_event_id: String,
    transaction_id: String,
    body: String,
) -> Option<CoreCommand> {
    if body.trim().is_empty() {
        return None;
    }
    Some(CoreCommand::Timeline(TimelineCommand::SendReply {
        request_id,
        key: TimelineKey {
            account_key,
            kind: TimelineKind::Thread {
                room_id,
                root_event_id: root_event_id.clone(),
            },
        },
        transaction_id,
        in_reply_to_event_id: root_event_id,
        body,
        mentions: koushi_state::MentionIntent::default(),
    }))
}

/// Derive the `AccountKey` for the currently active session from the snapshot.
///
/// Returns an empty key if no session is active (commands that require a Ready
/// session will be rejected by `AppActor::requires_ready_session`).
async fn account_key_from_snapshot(state: &CoreRuntimeState) -> AccountKey {
    let snapshot = state.connection.lock().await.snapshot();
    match &snapshot.session {
        koushi_state::SessionState::Ready(info)
        | koushi_state::SessionState::NeedsRecovery { info, .. }
        | koushi_state::SessionState::Recovering { info, .. }
        | koushi_state::SessionState::Locked(info)
        | koushi_state::SessionState::SwitchingAccount { info } => AccountKey(info.user_id.clone()),
        _ => AccountKey(String::new()),
    }
}

async fn image_upload_compression_contract_from_snapshot(
    state: &CoreRuntimeState,
) -> (ImageUploadCompressionMode, ImageUploadCompressionPolicy) {
    let media = state
        .connection
        .lock()
        .await
        .snapshot()
        .settings
        .values
        .media;
    (
        media.image_upload_compression,
        ImageUploadCompressionPolicy {
            threshold_bytes: media.image_upload_compression_policy.threshold_bytes,
            threshold_long_edge: media.image_upload_compression_policy.threshold_long_edge,
            target_long_edge: media.image_upload_compression_policy.target_long_edge,
            quality_percent: media.image_upload_compression_policy.quality_percent,
        },
    )
}

fn resolve_search_scope_from_active_room(
    scope: SearchScopeKind,
    active_room_id: Option<String>,
) -> SearchScope {
    match scope {
        SearchScopeKind::CurrentRoom => active_room_id
            .map(|room_id| SearchScope::Room { room_id })
            .unwrap_or(SearchScope::Global),
        SearchScopeKind::CurrentSpace | SearchScopeKind::AllRooms => SearchScope::Global,
        SearchScopeKind::Dms => SearchScope::Global,
    }
}

async fn resolve_search_scope(
    scope: SearchScopeKind,
    state: &CoreRuntimeState,
) -> koushi_core::SearchScope {
    let snapshot = state.connection.lock().await.snapshot();
    resolve_search_scope_from_active_room(scope, snapshot.navigation.active_room_id)
}

// ---- QA login pipe (debug/test only) ----

#[cfg(any(debug_assertions, test))]
#[derive(Deserialize)]
struct QaLoginPipePayload {
    homeserver: String,
    username: String,
    password: String,
    device_display_name: Option<String>,
    recovery_secret: Option<String>,
}

#[cfg(any(debug_assertions, test))]
#[derive(Debug)]
pub(crate) struct QaLoginPipeRequest {
    pub login: LoginRequest,
    pub recovery_secret: Option<AuthSecret>,
}

#[cfg(any(debug_assertions, test))]
pub(crate) fn parse_qa_login_pipe_payload(payload: &str) -> Result<QaLoginPipeRequest, String> {
    let payload: QaLoginPipePayload =
        serde_json::from_str(payload).map_err(|_| "QA login payload was invalid".to_owned())?;
    if payload.homeserver.trim().is_empty()
        || payload.username.trim().is_empty()
        || payload.password.is_empty()
    {
        return Err("QA login payload was incomplete".to_owned());
    }

    Ok(QaLoginPipeRequest {
        login: LoginRequest {
            homeserver: payload.homeserver,
            username: payload.username,
            password: AuthSecret::new(payload.password),
            device_display_name: payload.device_display_name,
        },
        recovery_secret: payload
            .recovery_secret
            .filter(|secret| !secret.trim().is_empty())
            .map(AuthSecret::new),
    })
}

#[cfg(any(debug_assertions, test))]
pub(crate) fn spawn_qa_login_pipe_reader(app: AppHandle, pipe_path: PathBuf) {
    tauri::async_runtime::spawn(async move {
        let payload = match read_qa_login_pipe(pipe_path).await {
            Ok(payload) => payload,
            Err(message) => {
                record_qa_login_failure(&app, &message).await;
                return;
            }
        };
        let request = match parse_qa_login_pipe_payload(&payload) {
            Ok(request) => request,
            Err(message) => {
                record_qa_login_failure(&app, &message).await;
                return;
            }
        };
        let state = app.state::<CoreRuntimeState>();
        if let Err(message) = submit_login_request(app.clone(), state.inner(), request.login).await
        {
            record_qa_login_failure(&app, &message).await;
            return;
        }
        if let Some(recovery_secret) = request.recovery_secret {
            let state = app.state::<CoreRuntimeState>();
            if let Err(message) =
                wait_for_qa_recovery_prompt(&app, state.inner(), QA_RECOVERY_PROMPT_TIMEOUT).await
            {
                record_qa_login_failure(&app, &message).await;
                return;
            }
            let state = app.state::<CoreRuntimeState>();
            if let Err(message) =
                submit_recovery_request(app.clone(), state.inner(), recovery_secret).await
            {
                record_qa_login_failure(&app, &message).await;
            }
        }
    });
}

#[cfg(any(debug_assertions, test))]
async fn read_qa_login_pipe(pipe_path: PathBuf) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        std::fs::read_to_string(pipe_path).map_err(|_| "QA login pipe could not be read".to_owned())
    })
    .await
    .map_err(|_| "QA login pipe reader failed".to_owned())?
}

#[cfg(any(debug_assertions, test))]
async fn record_qa_login_failure(app: &AppHandle, message: &str) {
    // Emit a QA title update so the harness sees `session=signedOut`.
    let state = app.state::<CoreRuntimeState>();
    update_qa_window_title_from_state(app, state.inner()).await;
    // Also emit a discrete error event.
    let _ = app.emit(
        crate::CORE_EVENT_NAME,
        serde_json::json!({
            "kind": "OperationFailed",
            "request_id": null,
            "failure": { "kind": "LoginFailed", "message": message },
        }),
    );
}

#[cfg(any(debug_assertions, test))]
async fn wait_for_qa_recovery_prompt(
    app: &AppHandle,
    state: &CoreRuntimeState,
    timeout: std::time::Duration,
) -> Result<(), String> {
    let started_at = std::time::Instant::now();
    while started_at.elapsed() < timeout {
        let snapshot = state.connection.lock().await.snapshot();
        if qa_recovery_prompt_is_available(&snapshot) {
            update_qa_window_title_from_state(app, state).await;
            return Ok(());
        }
        update_qa_window_title_from_state(app, state).await;
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }
    Err("QA recovery prompt did not become available".to_owned())
}

#[cfg(any(debug_assertions, test))]
pub(crate) fn qa_recovery_prompt_is_available(state: &koushi_state::AppState) -> bool {
    matches!(
        state.session,
        koushi_state::SessionState::NeedsRecovery { .. }
    )
}

// ---- QA control pipe (debug/test only) ----
//
// A newline-delimited JSON control channel that lets unattended GUI smoke drive
// a clean logout after a real login, so no stale device survives the run. This
// mirrors the QA login pipe: it carries no secrets, only control commands, and
// is gated to debug/test builds (release builds never read the env var).

#[cfg(any(debug_assertions, test))]
#[derive(Deserialize)]
struct QaControlPipeCommand {
    command: String,
}

/// Parsed QA control command. Only logout is supported today; unknown commands
/// are ignored by the reader rather than treated as failures.
#[cfg(any(debug_assertions, test))]
#[derive(Debug, Eq, PartialEq)]
pub(crate) enum QaControlCommand {
    Logout,
    Unknown(String),
}

#[cfg(any(debug_assertions, test))]
pub(crate) fn parse_qa_control_pipe_line(line: &str) -> Result<QaControlCommand, String> {
    let parsed: QaControlPipeCommand =
        serde_json::from_str(line).map_err(|_| "QA control command was invalid".to_owned())?;
    Ok(match parsed.command.as_str() {
        "logout" => QaControlCommand::Logout,
        other => QaControlCommand::Unknown(other.to_owned()),
    })
}

#[cfg(any(debug_assertions, test))]
pub(crate) fn spawn_qa_control_pipe_reader(app: AppHandle, pipe_path: PathBuf) {
    tauri::async_runtime::spawn(async move {
        let contents = match read_qa_control_pipe(pipe_path).await {
            Ok(contents) => contents,
            Err(message) => {
                record_qa_login_failure(&app, &message).await;
                return;
            }
        };
        for line in contents.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            match parse_qa_control_pipe_line(line) {
                Ok(QaControlCommand::Logout) => {
                    let state = app.state::<CoreRuntimeState>();
                    let request_id = next_request_id(state.inner()).await;
                    if let Err(message) =
                        submit_core_command(state.inner(), build_logout_command(request_id)).await
                    {
                        record_qa_login_failure(&app, &message).await;
                        continue;
                    }
                    // Surface the post-logout state in the QA window title so the
                    // smoke harness can wait for `session=signedOut`.
                    update_qa_window_title_from_state(&app, state.inner()).await;
                }
                Ok(QaControlCommand::Unknown(_)) => {
                    // Forward-compatible: ignore commands we do not recognise.
                }
                Err(message) => {
                    record_qa_login_failure(&app, &message).await;
                }
            }
        }
    });
}

#[cfg(any(debug_assertions, test))]
async fn read_qa_control_pipe(pipe_path: PathBuf) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        std::fs::read_to_string(pipe_path)
            .map_err(|_| "QA control pipe could not be read".to_owned())
    })
    .await
    .map_err(|_| "QA control pipe reader failed".to_owned())?
}

#[cfg(test)]
mod tests {

    // Phase 2c: handlers were split into commands/<feature>.rs submodules. The
    // source-characterization tests below read the combined command source so handler
    // bodies/signatures stay findable after the move.
    //
    // Ordering matters and mirrors the pre-split layout (production code, then this
    // test module): the per-feature handler files come first and `mod.rs` (which holds
    // the builder/helper defs AND this test module's own string literals) comes LAST.
    // That way `source.find("pub async fn X")` and the `"pub async fn Y"` end-markers
    // resolve to the real handlers, not to literals in this test module. The cross-file
    // end-markers require: navigation before timeline (select_*/close_focused_context
    // bound by `paginate_timeline_backwards`) and directory before room
    // (`join_directory_room` bound by `set_space_child`).
    fn commands_source() -> String {
        [
            include_str!("session.rs"),
            include_str!("settings.rs"),
            include_str!("account.rs"),
            include_str!("local_encryption.rs"),
            include_str!("e2ee.rs"),
            include_str!("navigation.rs"),
            include_str!("timeline.rs"),
            include_str!("live_signals.rs"),
            include_str!("profile.rs"),
            include_str!("directory.rs"),
            include_str!("room.rs"),
            include_str!("activity.rs"),
            include_str!("views.rs"),
            include_str!("search.rs"),
            include_str!("mod.rs"),
        ]
        .concat()
    }
    use crate::commands::{
        TIMELINE_BACKWARDS_PAGE_EVENT_COUNT, TIMELINE_RESTORE_ANCHOR_MAX_BATCHES,
    };
    use koushi_core::AccountKey;
    use koushi_core::{
        AccountCommand, AppCommand, CoreCommand, CreateRoomOptions, CreateRoomParentSpace,
        CreateRoomVisibility, ImageUploadCompressionPolicy, ImageUploadCompressionState,
        ImageUploadDimensions, ImageUploadVariantInfo, ImageUploadVariantKind,
        MediaDownloadSelection, PaginationDirection, RoomCommand, SearchCommand, SearchScope,
        SyncCommand, TimelineCommand, UploadMediaKind, UploadMediaThumbnail,
    };
    use koushi_state::{
        ActivityMarkReadTarget, ActivityTab, AppearanceSettings, ImageUploadCompressionMode,
        LocaleSettings, SettingsPatch, TextDirectionPreference, ThemePreference,
    };
    use koushi_state::{
        AppState, AuthSecret, IdentityResetAuthRequest, LoginRequest, MentionIntent, MentionTarget,
        SessionInfo, SessionState, VerificationCancelReason,
    };

    use super::QaControlCommand;
    use super::SearchScopeKind;
    use super::{
        build_accept_invite_command, build_accept_verification_command,
        build_bootstrap_cross_signing_command, build_bootstrap_secure_backup_command,
        build_cancel_scheduled_send_command, build_cancel_send_command,
        build_cancel_verification_command, build_change_secure_backup_passphrase_command,
        build_close_activity_command, build_close_files_view_command, build_close_search_command,
        build_confirm_sas_verification_command, build_create_room_command,
        build_create_space_command, build_decline_invite_command, build_discover_login_command,
        build_download_media_command, build_edit_message_command, build_enable_key_backup_command,
        build_export_room_keys_command, build_forget_room_command, build_forward_message_command,
        build_hide_link_preview_command, build_ignore_user_command, build_import_room_keys_command,
        build_invite_user_command, build_join_directory_room_command, build_join_room_command,
        build_leave_room_command, build_load_link_previews_command, build_load_message_source_command,
        build_load_room_settings_command, build_logout_command, build_mark_activity_read_command,
        build_moderate_room_member_command, build_observe_timeline_viewport_command,
        build_open_activity_command, build_open_files_view_command,
        build_open_timeline_at_timestamp_command, build_paginate_activity_command,
        build_paginate_thread_timeline_backwards_command,
        build_paginate_timeline_backwards_command, build_pin_event_command,
        build_probe_local_encryption_health_command, build_query_directory_command,
        build_redact_message_command, build_redact_reaction_command, build_remove_room_tag_command,
        build_reorder_spaces_command, build_report_content_command, build_report_room_command,
        build_report_user_command, build_reschedule_scheduled_send_command,
        build_reset_identity_command, build_reset_local_data_command, build_restart_sync_command,
        build_restore_timeline_anchor_command, build_retry_send_command,
        build_schedule_send_command, build_select_room_command, build_select_space_command,
        build_send_reaction_command, build_send_read_receipt_command, build_send_reply_command,
        build_send_text_command, build_send_thread_reply_command, build_set_activity_tab_command,
        build_set_avatar_command, build_set_composer_draft_command, build_set_display_name_command,
        build_set_fully_read_command, build_set_local_user_alias_command,
        build_set_presence_command, build_set_room_tag_command,
        build_set_room_url_preview_override_command, build_set_space_child_command,
        build_set_thread_composer_draft_command, build_set_typing_command,
        build_start_direct_message_command, build_submit_identity_reset_oauth_command,
        build_submit_identity_reset_password_command, build_submit_login_command,
        build_submit_recovery_command, build_submit_search_command,
        build_subscribe_focused_timeline_command, build_switch_account_command,
        build_toggle_reaction_command, build_unignore_user_command, build_unpin_event_command,
        build_update_room_member_role_command, build_update_room_setting_command,
        build_update_settings_command, build_upload_media_command, parse_qa_control_pipe_line,
        parse_qa_login_pipe_payload, qa_recovery_prompt_is_available, qa_window_title_string,
        resolve_search_scope_from_active_room,
    };
    use koushi_state::{
        PresenceKind, RoomHistoryVisibility, RoomJoinRule, RoomModerationAction, RoomSettingChange,
        RoomSummary, RoomTagKind, RoomTags,
    };

    #[test]
    fn qa_login_pipe_payload_maps_to_login_request_without_debugging_secret() {
        let request = parse_qa_login_pipe_payload(
            r#"{"homeserver":"https://matrix.example.org","username":"fixture-user","password":"synthetic-password","device_display_name":"Koushi GUI Smoke","recovery_secret":"synthetic-recovery-secret"}"#,
        )
        .expect("payload should parse");

        assert_eq!(request.login.homeserver, "https://matrix.example.org");
        assert_eq!(request.login.username, "fixture-user");
        assert_eq!(request.login.password.expose_secret(), "synthetic-password");
        assert_eq!(
            request.login.device_display_name.as_deref(),
            Some("Koushi GUI Smoke")
        );
        assert_eq!(
            request
                .recovery_secret
                .as_ref()
                .map(|secret| secret.expose_secret()),
            Some("synthetic-recovery-secret")
        );
        assert!(!format!("{request:?}").contains("synthetic-password"));
        assert!(!format!("{request:?}").contains("synthetic-recovery-secret"));
    }

    #[test]
    fn qa_control_pipe_line_parses_logout_and_ignores_unknown_commands() {
        assert_eq!(
            parse_qa_control_pipe_line(r#"{"command":"logout"}"#).expect("logout should parse"),
            QaControlCommand::Logout
        );
        assert_eq!(
            parse_qa_control_pipe_line(r#"{"command":"focus"}"#).expect("unknown should parse"),
            QaControlCommand::Unknown("focus".to_owned())
        );
        assert!(parse_qa_control_pipe_line("not json").is_err());
    }

    #[test]
    fn qa_control_logout_builds_account_logout_command() {
        // The control pipe must reuse the same logout core command the manual
        // logout button submits — no bespoke logout path.
        match build_logout_command(fake_request_id(99)) {
            CoreCommand::Account(AccountCommand::Logout { request_id }) => {
                assert_eq!(request_id, fake_request_id(99));
            }
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn qa_recovery_prompt_available_iff_needs_recovery() {
        let mut state = AppState::default();
        assert!(!qa_recovery_prompt_is_available(&state));

        state.session = SessionState::NeedsRecovery {
            info: SessionInfo {
                homeserver: "https://matrix.example.org".to_owned(),
                user_id: "@user:example.org".to_owned(),
                device_id: "DEVICE".to_owned(),
            },
            methods: vec![],
        };
        assert!(qa_recovery_prompt_is_available(&state));
    }

    #[test]
    fn qa_window_title_reflects_session_sync_room_and_timeline_counts() {
        let mut snapshot = AppState::default();
        snapshot.rooms = vec![
            RoomSummary {
                room_id: "!room1:example.org".to_owned(),
                display_name: "Room 1".to_owned(),
                display_label: "Room 1".to_owned(),
                original_display_label: "Room 1".to_owned(),
                avatar: None,
                is_dm: false,
                dm_user_ids: Vec::new(),
                tags: RoomTags::default(),
                unread_count: 0,
                notification_count: 0,
                highlight_count: 0,
                marked_unread: false,
                last_activity_ms: 0,
                latest_event: None,
                parent_space_ids: vec![],
                dm_space_ids: vec![],
                is_encrypted: false,
                joined_members: 0,
            },
            RoomSummary {
                room_id: "!room2:example.org".to_owned(),
                display_name: "Room 2".to_owned(),
                display_label: "Room 2".to_owned(),
                original_display_label: "Room 2".to_owned(),
                avatar: None,
                is_dm: false,
                dm_user_ids: Vec::new(),
                tags: RoomTags::default(),
                unread_count: 0,
                notification_count: 0,
                highlight_count: 0,
                marked_unread: false,
                last_activity_ms: 0,
                latest_event: None,
                parent_space_ids: vec![],
                dm_space_ids: vec![],
                is_encrypted: false,
                joined_members: 0,
            },
        ];

        let title = qa_window_title_string(&snapshot, 42);

        assert!(title.contains("session=signedOut"));
        assert!(title.contains("sync=stopped"));
        assert!(title.contains("rooms=2"));
        assert!(title.contains("timeline_items=42"));
    }

    fn fake_request_id(sequence: u64) -> koushi_core::RequestId {
        koushi_core::RequestId {
            connection_id: koushi_core::RuntimeConnectionId(7),
            sequence,
        }
    }

    #[test]
    fn tauri_command_routes_build_expected_core_commands() {
        let active_account_key = AccountKey("@alice:example.org".to_owned());
        let room_id = "!room:example.org".to_owned();
        let transaction_id = "desktop-1".to_owned();
        let body = "body with visible content".to_owned();
        let edit_body = "updated body".to_owned();
        let query = "search terms".to_owned();

        match build_submit_login_command(
            fake_request_id(1),
            LoginRequest {
                homeserver: "https://matrix.example.org".to_owned(),
                username: "alice".to_owned(),
                password: AuthSecret::new("password-123"),
                device_display_name: Some("Laptop".to_owned()),
            },
        ) {
            CoreCommand::Account(AccountCommand::LoginPassword {
                request_id,
                request,
            }) => {
                assert_eq!(request_id, fake_request_id(1));
                assert_eq!(request.homeserver, "https://matrix.example.org");
                assert_eq!(request.username, "alice");
                assert_eq!(request.password.expose_secret(), "password-123");
                assert_eq!(request.device_display_name.as_deref(), Some("Laptop"));
            }
            other => panic!("unexpected command: {other:?}"),
        }

        match build_discover_login_command(
            fake_request_id(101),
            "https://matrix.example.org".to_owned(),
        ) {
            CoreCommand::Account(AccountCommand::DiscoverLogin {
                request_id,
                homeserver,
            }) => {
                assert_eq!(request_id, fake_request_id(101));
                assert_eq!(homeserver, "https://matrix.example.org");
            }
            other => panic!("unexpected command: {other:?}"),
        }

        match build_switch_account_command(fake_request_id(2), "@bob:example.org".to_owned()) {
            CoreCommand::Account(AccountCommand::SwitchAccount {
                request_id,
                account_key,
            }) => {
                assert_eq!(request_id, fake_request_id(2));
                assert_eq!(
                    account_key,
                    koushi_core::AccountKey("@bob:example.org".to_owned())
                );
            }
            other => panic!("unexpected command: {other:?}"),
        }

        match build_submit_recovery_command(fake_request_id(3), AuthSecret::new("recovery-123")) {
            CoreCommand::Account(AccountCommand::SubmitRecovery {
                request_id,
                request,
            }) => {
                assert_eq!(request_id, fake_request_id(3));
                assert_eq!(request.secret.expose_secret(), "recovery-123");
            }
            other => panic!("unexpected command: {other:?}"),
        }

        match build_export_room_keys_command(
            fake_request_id(33),
            "/tmp/element-compatible-export.txt".to_owned(),
            AuthSecret::new("room-key-transfer-phrase"),
        ) {
            CoreCommand::Account(AccountCommand::ExportRoomKeys {
                request_id,
                request,
            }) => {
                assert_eq!(request_id, fake_request_id(33));
                assert_eq!(
                    request.destination_path,
                    std::path::PathBuf::from("/tmp/element-compatible-export.txt")
                );
                assert_eq!(
                    request.passphrase.expose_secret(),
                    "room-key-transfer-phrase"
                );
            }
            other => panic!("unexpected command: {other:?}"),
        }

        match build_import_room_keys_command(
            fake_request_id(34),
            "/tmp/element-compatible-import.txt".to_owned(),
            AuthSecret::new("room-key-transfer-phrase"),
        ) {
            CoreCommand::Account(AccountCommand::ImportRoomKeys {
                request_id,
                request,
            }) => {
                assert_eq!(request_id, fake_request_id(34));
                assert_eq!(
                    request.source_path,
                    std::path::PathBuf::from("/tmp/element-compatible-import.txt")
                );
                assert_eq!(
                    request.passphrase.expose_secret(),
                    "room-key-transfer-phrase"
                );
            }
            other => panic!("unexpected command: {other:?}"),
        }

        match build_bootstrap_secure_backup_command(
            fake_request_id(35),
            Some(AuthSecret::new("backup-setup-phrase")),
            Some("/tmp/recovery-artifact.txt".to_owned()),
        ) {
            CoreCommand::Account(AccountCommand::BootstrapSecureBackup {
                request_id,
                request,
            }) => {
                assert_eq!(request_id, fake_request_id(35));
                assert_eq!(
                    request
                        .passphrase
                        .as_ref()
                        .expect("passphrase")
                        .expose_secret(),
                    "backup-setup-phrase"
                );
                assert_eq!(
                    request.recovery_key_destination_path,
                    Some(std::path::PathBuf::from("/tmp/recovery-artifact.txt"))
                );
            }
            other => panic!("unexpected command: {other:?}"),
        }

        match build_change_secure_backup_passphrase_command(
            fake_request_id(36),
            AuthSecret::new("old-backup-phrase"),
            AuthSecret::new("new-backup-phrase"),
            Some("/tmp/recovery-artifact.txt".to_owned()),
        ) {
            CoreCommand::Account(AccountCommand::ChangeSecureBackupPassphrase {
                request_id,
                request,
            }) => {
                assert_eq!(request_id, fake_request_id(36));
                assert_eq!(request.old_secret.expose_secret(), "old-backup-phrase");
                assert_eq!(request.new_passphrase.expose_secret(), "new-backup-phrase");
                assert_eq!(
                    request.recovery_key_destination_path,
                    Some(std::path::PathBuf::from("/tmp/recovery-artifact.txt"))
                );
            }
            other => panic!("unexpected command: {other:?}"),
        }

        match build_logout_command(fake_request_id(4)) {
            CoreCommand::Account(AccountCommand::Logout { request_id }) => {
                assert_eq!(request_id, fake_request_id(4));
            }
            other => panic!("unexpected command: {other:?}"),
        }

        match build_restart_sync_command(fake_request_id(5)) {
            CoreCommand::Sync(SyncCommand::Restart { request_id }) => {
                assert_eq!(request_id, fake_request_id(5));
            }
            other => panic!("unexpected command: {other:?}"),
        }

        match build_select_space_command(fake_request_id(6), Some("!space:example.org".to_owned()))
        {
            CoreCommand::Room(RoomCommand::SelectSpace {
                request_id,
                space_id,
            }) => {
                assert_eq!(request_id, fake_request_id(6));
                assert_eq!(space_id.as_deref(), Some("!space:example.org"));
            }
            other => panic!("unexpected command: {other:?}"),
        }

        match build_reorder_spaces_command(
            fake_request_id(37),
            vec![
                "!space-b:example.org".to_owned(),
                "!space-a:example.org".to_owned(),
            ],
        ) {
            CoreCommand::Room(RoomCommand::ReorderSpaces {
                request_id,
                space_ids,
            }) => {
                assert_eq!(request_id, fake_request_id(37));
                assert_eq!(
                    space_ids,
                    vec!["!space-b:example.org", "!space-a:example.org"]
                );
            }
            other => panic!("unexpected command: {other:?}"),
        }

        match build_select_room_command(fake_request_id(7), room_id.clone()) {
            CoreCommand::Room(RoomCommand::SelectRoom {
                request_id,
                room_id: route_room_id,
            }) => {
                assert_eq!(request_id, fake_request_id(7));
                assert_eq!(route_room_id, room_id);
            }
            other => panic!("unexpected command: {other:?}"),
        }

        match build_paginate_timeline_backwards_command(
            fake_request_id(9),
            active_account_key.clone(),
            room_id.clone(),
        ) {
            CoreCommand::Timeline(TimelineCommand::Paginate {
                request_id,
                key,
                direction,
                event_count,
            }) => {
                assert_eq!(request_id, fake_request_id(9));
                assert_eq!(key.account_key, active_account_key);
                assert_eq!(
                    key.kind,
                    koushi_core::TimelineKind::Room {
                        room_id: room_id.clone()
                    }
                );
                assert_eq!(direction, PaginationDirection::Backward);
                assert_eq!(event_count, 100);
            }
            other => panic!("unexpected command: {other:?}"),
        }

        match build_restore_timeline_anchor_command(
            fake_request_id(10),
            active_account_key.clone(),
            koushi_core::TimelineKey::room(active_account_key.clone(), room_id.clone()),
            "$anchor:example.invalid".to_owned(),
            TIMELINE_RESTORE_ANCHOR_MAX_BATCHES,
            TIMELINE_BACKWARDS_PAGE_EVENT_COUNT,
        ) {
            CoreCommand::Timeline(TimelineCommand::RestoreTimelineAnchor {
                request_id,
                key,
                event_id,
                max_batches,
                event_count,
            }) => {
                assert_eq!(request_id, fake_request_id(10));
                assert_eq!(key.account_key, active_account_key);
                assert_eq!(
                    key.kind,
                    koushi_core::TimelineKind::Room {
                        room_id: room_id.clone()
                    }
                );
                assert_eq!(event_id, "$anchor:example.invalid");
                assert_eq!(max_batches, TIMELINE_RESTORE_ANCHOR_MAX_BATCHES);
                assert_eq!(event_count, TIMELINE_BACKWARDS_PAGE_EVENT_COUNT);
            }
            other => panic!("unexpected command: {other:?}"),
        }

        match build_send_text_command(
            fake_request_id(11),
            active_account_key.clone(),
            room_id.clone(),
            transaction_id.clone(),
            body.clone(),
            MentionIntent {
                targets: vec![MentionTarget::User {
                    user_id: "@alice:example.invalid".to_owned(),
                    display_label: "Alice".to_owned(),
                }],
            },
        )
        .expect("send_text should build a command")
        {
            CoreCommand::Timeline(TimelineCommand::SendText {
                request_id,
                key,
                transaction_id: route_transaction_id,
                body: route_body,
                mentions,
            }) => {
                assert_eq!(request_id, fake_request_id(11));
                assert_eq!(
                    mentions,
                    MentionIntent {
                        targets: vec![MentionTarget::User {
                            user_id: "@alice:example.invalid".to_owned(),
                            display_label: "Alice".to_owned(),
                        }],
                    }
                );
                assert_eq!(key.account_key, active_account_key);
                assert_eq!(
                    key.kind,
                    koushi_core::TimelineKind::Room {
                        room_id: room_id.clone()
                    }
                );
                assert_eq!(route_transaction_id, transaction_id);
                assert_eq!(route_body, body);
            }
            other => panic!("unexpected command: {other:?}"),
        }

        match build_schedule_send_command(
            fake_request_id(33),
            room_id.clone(),
            "send later body".to_owned(),
            1_900_000_000_000,
        )
        .expect("schedule_send should build a command")
        {
            CoreCommand::App(AppCommand::ScheduleSend {
                request_id,
                room_id: route_room_id,
                body,
                send_at_ms,
            }) => {
                assert_eq!(request_id, fake_request_id(33));
                assert_eq!(route_room_id, room_id);
                assert_eq!(body, "send later body");
                assert_eq!(send_at_ms, 1_900_000_000_000);
            }
            other => panic!("unexpected command: {other:?}"),
        }

        match build_cancel_scheduled_send_command(fake_request_id(34), "scheduled-1".to_owned())
            .expect("cancel_scheduled_send should build a command")
        {
            CoreCommand::App(AppCommand::CancelScheduledSend {
                request_id,
                scheduled_id,
            }) => {
                assert_eq!(request_id, fake_request_id(34));
                assert_eq!(scheduled_id, "scheduled-1");
            }
            other => panic!("unexpected command: {other:?}"),
        }

        match build_reschedule_scheduled_send_command(
            fake_request_id(35),
            "scheduled-1".to_owned(),
            1_900_000_060_000,
        )
        .expect("reschedule_scheduled_send should build a command")
        {
            CoreCommand::App(AppCommand::RescheduleScheduledSend {
                request_id,
                scheduled_id,
                send_at_ms,
            }) => {
                assert_eq!(request_id, fake_request_id(35));
                assert_eq!(scheduled_id, "scheduled-1");
                assert_eq!(send_at_ms, 1_900_000_060_000);
            }
            other => panic!("unexpected command: {other:?}"),
        }

        match build_retry_send_command(
            fake_request_id(31),
            active_account_key.clone(),
            room_id.clone(),
            "sdk-txn-1".to_owned(),
        )
        .expect("retry_send should build a command")
        {
            CoreCommand::Timeline(TimelineCommand::RetrySend {
                request_id,
                key,
                transaction_id,
            }) => {
                assert_eq!(request_id, fake_request_id(31));
                assert_eq!(key.account_key, active_account_key);
                assert_eq!(
                    key.kind,
                    koushi_core::TimelineKind::Room {
                        room_id: room_id.clone()
                    }
                );
                assert_eq!(transaction_id, "sdk-txn-1");
            }
            other => panic!("unexpected command: {other:?}"),
        }

        match build_cancel_send_command(
            fake_request_id(32),
            active_account_key.clone(),
            room_id.clone(),
            "sdk-txn-2".to_owned(),
        )
        .expect("cancel_send should build a command")
        {
            CoreCommand::Timeline(TimelineCommand::CancelSend {
                request_id,
                key,
                transaction_id,
            }) => {
                assert_eq!(request_id, fake_request_id(32));
                assert_eq!(key.account_key, active_account_key);
                assert_eq!(
                    key.kind,
                    koushi_core::TimelineKind::Room {
                        room_id: room_id.clone()
                    }
                );
                assert_eq!(transaction_id, "sdk-txn-2");
            }
            other => panic!("unexpected command: {other:?}"),
        }

        match build_forward_message_command(
            fake_request_id(33),
            active_account_key.clone(),
            room_id.clone(),
            "$source-event".to_owned(),
            "!destination:example.invalid".to_owned(),
            "desktop-forward-1".to_owned(),
        )
        .expect("forward_message should build a command")
        {
            CoreCommand::Timeline(TimelineCommand::ForwardMessage {
                request_id,
                key,
                source_event_id,
                destination_room_id,
                transaction_id,
            }) => {
                assert_eq!(request_id, fake_request_id(33));
                assert_eq!(key.account_key, active_account_key);
                assert_eq!(
                    key.kind,
                    koushi_core::TimelineKind::Room {
                        room_id: room_id.clone()
                    }
                );
                assert_eq!(source_event_id, "$source-event");
                assert_eq!(destination_room_id, "!destination:example.invalid");
                assert_eq!(transaction_id, "desktop-forward-1");
            }
            other => panic!("unexpected command: {other:?}"),
        }

        match build_load_message_source_command(
            fake_request_id(34),
            active_account_key.clone(),
            room_id.clone(),
            "$source-event".to_owned(),
        )
        .expect("load_message_source should build a command")
        {
            CoreCommand::Timeline(TimelineCommand::LoadMessageSource {
                request_id,
                key,
                event_id,
            }) => {
                assert_eq!(request_id, fake_request_id(34));
                assert_eq!(key.account_key, active_account_key);
                assert_eq!(
                    key.kind,
                    koushi_core::TimelineKind::Room {
                        room_id: room_id.clone()
                    }
                );
                assert_eq!(event_id, "$source-event");
            }
            other => panic!("unexpected command: {other:?}"),
        }

        assert!(
            build_retry_send_command(
                fake_request_id(35),
                active_account_key.clone(),
                room_id.clone(),
                " \t".to_owned()
            )
            .is_none()
        );
        assert!(
            build_cancel_send_command(
                fake_request_id(36),
                active_account_key.clone(),
                room_id.clone(),
                "\n".to_owned()
            )
            .is_none()
        );

        match build_upload_media_command(
            fake_request_id(25),
            active_account_key.clone(),
            room_id.clone(),
            "desktop-media-1".to_owned(),
            "report.pdf".to_owned(),
            "application/pdf".to_owned(),
            vec![1, 2, 3, 4],
            None,
            ImageUploadCompressionMode::Never,
            ImageUploadCompressionPolicy::default(),
            None,
            None,
            None,
        )
        .expect("upload_media should build a command")
        {
            CoreCommand::Timeline(TimelineCommand::UploadAndSendMedia {
                request_id,
                key,
                transaction_id,
                request,
            }) => {
                assert_eq!(request_id, fake_request_id(25));
                assert_eq!(key.account_key, active_account_key);
                assert_eq!(
                    key.kind,
                    koushi_core::TimelineKind::Room {
                        room_id: room_id.clone()
                    }
                );
                assert_eq!(transaction_id, "desktop-media-1");
                assert_eq!(request.filename, "report.pdf");
                assert_eq!(request.mime_type, "application/pdf");
                assert_eq!(request.bytes, vec![1, 2, 3, 4]);
                assert_eq!(request.kind, UploadMediaKind::File);
                assert_eq!(request.caption, None);
            }
            other => panic!("unexpected command: {other:?}"),
        }

        match build_upload_media_command(
            fake_request_id(26),
            active_account_key.clone(),
            room_id.clone(),
            "desktop-media-2".to_owned(),
            "photo.png".to_owned(),
            "image/png".to_owned(),
            vec![9],
            Some("single **event** caption".to_owned()),
            ImageUploadCompressionMode::Never,
            ImageUploadCompressionPolicy::default(),
            None,
            None,
            None,
        )
        .expect("image upload_media should build a command")
        {
            CoreCommand::Timeline(TimelineCommand::UploadAndSendMedia { request, .. }) => {
                assert_eq!(
                    request.kind,
                    UploadMediaKind::Image {
                        width: None,
                        height: None
                    }
                );
                let caption = request.caption.expect("caption should be preserved");
                assert_eq!(caption.plain_body, "single **event** caption");
                assert_eq!(
                    caption.formatted_body.as_deref(),
                    Some("single <strong>event</strong> caption")
                );
            }
            other => panic!("unexpected command: {other:?}"),
        }

        match build_upload_media_command(
            fake_request_id(37),
            active_account_key.clone(),
            room_id.clone(),
            "desktop-media-3".to_owned(),
            "screenshot.jpg".to_owned(),
            "image/jpeg".to_owned(),
            vec![7, 8, 9, 10],
            None,
            ImageUploadCompressionMode::Always,
            ImageUploadCompressionPolicy::default(),
            Some(ImageUploadDimensions {
                width: 1200,
                height: 900,
            }),
            Some(ImageUploadCompressionState {
                mode: koushi_state::ImageUploadCompressionMode::Always,
                policy: ImageUploadCompressionPolicy::default(),
                original: ImageUploadVariantInfo {
                    mime_type: "image/jpeg".to_owned(),
                    byte_count: 3_200_000,
                    dimensions: Some(ImageUploadDimensions {
                        width: 4032,
                        height: 3024,
                    }),
                },
                selected: ImageUploadVariantInfo {
                    mime_type: "image/jpeg".to_owned(),
                    byte_count: 999,
                    dimensions: Some(ImageUploadDimensions {
                        width: 1200,
                        height: 900,
                    }),
                },
                selected_variant: ImageUploadVariantKind::Compressed,
                skipped_small_image: false,
                metadata_stripped: true,
                thumbnail_refreshed: true,
            }),
            Some(UploadMediaThumbnail {
                mime_type: "image/jpeg".to_owned(),
                bytes: vec![1, 1, 1],
                width: 320,
                height: 240,
            }),
        )
        .expect("compressed image upload_media should build a command")
        {
            CoreCommand::Timeline(TimelineCommand::UploadAndSendMedia { request, .. }) => {
                assert_eq!(
                    request.kind,
                    UploadMediaKind::Image {
                        width: Some(1200),
                        height: Some(900)
                    }
                );
                let compression = request
                    .compression
                    .expect("image compression contract should be preserved");
                assert_eq!(
                    compression.selected_variant,
                    ImageUploadVariantKind::Compressed
                );
                assert_eq!(compression.selected.byte_count, 4);
                assert!(compression.metadata_stripped);
                assert!(compression.thumbnail_refreshed);
                assert_eq!(
                    request.thumbnail.as_ref().map(|thumbnail| {
                        (
                            thumbnail.mime_type.as_str(),
                            thumbnail.bytes.len(),
                            thumbnail.width,
                            thumbnail.height,
                        )
                    }),
                    Some(("image/jpeg", 3, 320, 240))
                );
            }
            other => panic!("unexpected command: {other:?}"),
        }

        match build_download_media_command(
            fake_request_id(27),
            active_account_key.clone(),
            room_id.clone(),
            "$media-event".to_owned(),
        )
        .expect("download_media should build a command")
        {
            CoreCommand::Timeline(TimelineCommand::DownloadMedia {
                request_id,
                key,
                event_id,
                selection,
            }) => {
                assert_eq!(request_id, fake_request_id(27));
                assert_eq!(key.account_key, active_account_key);
                assert_eq!(
                    key.kind,
                    koushi_core::TimelineKind::Room {
                        room_id: room_id.clone()
                    }
                );
                assert_eq!(event_id, "$media-event");
                assert_eq!(selection, MediaDownloadSelection::File);
            }
            other => panic!("unexpected command: {other:?}"),
        }

        match build_edit_message_command(
            fake_request_id(11),
            active_account_key.clone(),
            room_id.clone(),
            "$event".to_owned(),
            edit_body.clone(),
        )
        .expect("edit_message should build a command")
        {
            CoreCommand::Timeline(TimelineCommand::EditText {
                request_id,
                key,
                event_id,
                body: route_body,
            }) => {
                assert_eq!(request_id, fake_request_id(11));
                assert_eq!(key.account_key, active_account_key);
                assert_eq!(
                    key.kind,
                    koushi_core::TimelineKind::Room {
                        room_id: room_id.clone()
                    }
                );
                assert_eq!(event_id, "$event");
                assert_eq!(route_body, edit_body);
            }
            other => panic!("unexpected command: {other:?}"),
        }

        match build_redact_message_command(
            fake_request_id(12),
            active_account_key.clone(),
            room_id.clone(),
            "$event".to_owned(),
        ) {
            CoreCommand::Timeline(TimelineCommand::Redact {
                request_id,
                key,
                event_id,
            }) => {
                assert_eq!(request_id, fake_request_id(12));
                assert_eq!(key.account_key, active_account_key);
                assert_eq!(
                    key.kind,
                    koushi_core::TimelineKind::Room {
                        room_id: room_id.clone()
                    }
                );
                assert_eq!(event_id, "$event");
            }
            other => panic!("unexpected command: {other:?}"),
        }

        match build_toggle_reaction_command(
            fake_request_id(13),
            active_account_key.clone(),
            room_id.clone(),
            "$event".to_owned(),
            "👍".to_owned(),
        )
        .expect("toggle_reaction should build a command")
        {
            CoreCommand::Timeline(TimelineCommand::ToggleReaction {
                request_id,
                key,
                event_id,
                reaction_key,
            }) => {
                assert_eq!(request_id, fake_request_id(13));
                assert_eq!(key.account_key, active_account_key);
                assert_eq!(
                    key.kind,
                    koushi_core::TimelineKind::Room {
                        room_id: room_id.clone()
                    }
                );
                assert_eq!(event_id, "$event");
                assert_eq!(reaction_key, "👍");
                let debug = format!(
                    "{:?}",
                    CoreCommand::Timeline(TimelineCommand::ToggleReaction {
                        request_id: fake_request_id(13),
                        key,
                        event_id,
                        reaction_key,
                    })
                );
                assert!(!debug.contains("👍"), "{debug}");
                assert!(!debug.contains("$event"), "{debug}");
            }
            other => panic!("unexpected command: {other:?}"),
        }

        match build_send_reaction_command(
            fake_request_id(25),
            active_account_key.clone(),
            room_id.clone(),
            "$event".to_owned(),
            "👍".to_owned(),
        )
        .expect("send_reaction should build a command")
        {
            CoreCommand::Timeline(TimelineCommand::SendReaction {
                request_id,
                key,
                event_id,
                reaction_key,
            }) => {
                assert_eq!(request_id, fake_request_id(25));
                assert_eq!(key.account_key, active_account_key);
                assert_eq!(
                    key.kind,
                    koushi_core::TimelineKind::Room {
                        room_id: room_id.clone()
                    }
                );
                assert_eq!(event_id, "$event");
                assert_eq!(reaction_key, "👍");
            }
            other => panic!("unexpected command: {other:?}"),
        }

        match build_redact_reaction_command(
            fake_request_id(26),
            active_account_key.clone(),
            room_id.clone(),
            "$event".to_owned(),
            "👍".to_owned(),
            "$reaction".to_owned(),
        )
        .expect("redact_reaction should build a command")
        {
            CoreCommand::Timeline(TimelineCommand::RedactReaction {
                request_id,
                key,
                event_id,
                reaction_key,
                reaction_event_id,
            }) => {
                assert_eq!(request_id, fake_request_id(26));
                assert_eq!(key.account_key, active_account_key);
                assert_eq!(
                    key.kind,
                    koushi_core::TimelineKind::Room {
                        room_id: room_id.clone()
                    }
                );
                assert_eq!(event_id, "$event");
                assert_eq!(reaction_key, "👍");
                assert_eq!(reaction_event_id, "$reaction");
            }
            other => panic!("unexpected command: {other:?}"),
        }

        match build_send_read_receipt_command(
            fake_request_id(28),
            active_account_key.clone(),
            room_id.clone(),
            "$receipt-event".to_owned(),
        )
        .expect("send_read_receipt should build a command")
        {
            CoreCommand::Timeline(TimelineCommand::SendReadReceipt {
                request_id,
                key,
                event_id,
            }) => {
                assert_eq!(request_id, fake_request_id(28));
                assert_eq!(key.account_key, active_account_key);
                assert_eq!(
                    key.kind,
                    koushi_core::TimelineKind::Room {
                        room_id: room_id.clone()
                    }
                );
                assert_eq!(event_id, "$receipt-event");
            }
            other => panic!("unexpected command: {other:?}"),
        }

        match build_set_fully_read_command(
            fake_request_id(29),
            active_account_key.clone(),
            room_id.clone(),
            "$fully-read-event".to_owned(),
        )
        .expect("set_fully_read should build a command")
        {
            CoreCommand::Timeline(TimelineCommand::SetFullyRead {
                request_id,
                key,
                event_id,
            }) => {
                assert_eq!(request_id, fake_request_id(29));
                assert_eq!(key.account_key, active_account_key);
                assert_eq!(
                    key.kind,
                    koushi_core::TimelineKind::Room {
                        room_id: room_id.clone()
                    }
                );
                assert_eq!(event_id, "$fully-read-event");
            }
            other => panic!("unexpected command: {other:?}"),
        }

        match build_observe_timeline_viewport_command(
            fake_request_id(31),
            active_account_key.clone(),
            room_id.clone(),
            Some("$first-visible".to_owned()),
            Some("$last-visible".to_owned()),
            false,
        ) {
            CoreCommand::Timeline(TimelineCommand::ObserveViewport {
                request_id,
                key,
                observation,
            }) => {
                assert_eq!(request_id, fake_request_id(31));
                assert_eq!(key.account_key, active_account_key);
                assert_eq!(
                    key.kind,
                    koushi_core::TimelineKind::Room {
                        room_id: room_id.clone()
                    }
                );
                assert_eq!(
                    observation.first_visible_event_id.as_deref(),
                    Some("$first-visible")
                );
                assert_eq!(
                    observation.last_visible_event_id.as_deref(),
                    Some("$last-visible")
                );
                assert!(!observation.at_bottom);
            }
            other => panic!("unexpected command: {other:?}"),
        }

        match build_open_timeline_at_timestamp_command(
            fake_request_id(32),
            room_id.clone(),
            1_718_000_000_000,
        ) {
            CoreCommand::App(AppCommand::OpenTimelineAtTimestamp {
                request_id,
                room_id: command_room_id,
                timestamp_ms,
            }) => {
                assert_eq!(request_id, fake_request_id(32));
                assert_eq!(command_room_id, room_id);
                assert_eq!(timestamp_ms, 1_718_000_000_000);
            }
            other => panic!("unexpected command: {other:?}"),
        }

        match build_set_typing_command(
            fake_request_id(30),
            active_account_key.clone(),
            room_id.clone(),
            true,
        ) {
            CoreCommand::Timeline(TimelineCommand::SetTyping {
                request_id,
                key,
                is_typing,
            }) => {
                assert_eq!(request_id, fake_request_id(30));
                assert_eq!(key.account_key, active_account_key);
                assert_eq!(
                    key.kind,
                    koushi_core::TimelineKind::Room {
                        room_id: room_id.clone()
                    }
                );
                assert!(is_typing);
            }
            other => panic!("unexpected command: {other:?}"),
        }

        match build_set_presence_command(fake_request_id(31), PresenceKind::Away) {
            CoreCommand::Account(AccountCommand::SetPresence {
                request_id,
                presence,
            }) => {
                assert_eq!(request_id, fake_request_id(31));
                assert_eq!(presence, PresenceKind::Away);
            }
            other => panic!("unexpected command: {other:?}"),
        }

        match build_set_display_name_command(
            fake_request_id(32),
            Some("Private Display".to_owned()),
        ) {
            CoreCommand::Account(AccountCommand::SetDisplayName {
                request_id,
                display_name,
            }) => {
                assert_eq!(request_id, fake_request_id(32));
                assert_eq!(display_name.as_deref(), Some("Private Display"));
                let debug = format!(
                    "{:?}",
                    CoreCommand::Account(AccountCommand::SetDisplayName {
                        request_id,
                        display_name,
                    })
                );
                assert!(!debug.contains("Private Display"), "{debug}");
            }
            other => panic!("unexpected command: {other:?}"),
        }

        match build_set_local_user_alias_command(
            fake_request_id(34),
            "@target:example.invalid".to_owned(),
            Some("Desk Alias".to_owned()),
        ) {
            CoreCommand::Account(AccountCommand::SetLocalUserAlias {
                request_id,
                user_id,
                alias,
            }) => {
                assert_eq!(request_id, fake_request_id(34));
                assert_eq!(user_id, "@target:example.invalid");
                assert_eq!(alias.as_deref(), Some("Desk Alias"));
                let debug = format!(
                    "{:?}",
                    CoreCommand::Account(AccountCommand::SetLocalUserAlias {
                        request_id,
                        user_id,
                        alias,
                    })
                );
                assert!(!debug.contains("@target:example.invalid"), "{debug}");
                assert!(!debug.contains("Desk Alias"), "{debug}");
            }
            other => panic!("unexpected command: {other:?}"),
        }

        match build_set_local_user_alias_command(
            fake_request_id(35),
            "@target:example.invalid".to_owned(),
            None,
        ) {
            CoreCommand::Account(AccountCommand::SetLocalUserAlias {
                request_id,
                user_id,
                alias,
            }) => {
                assert_eq!(request_id, fake_request_id(35));
                assert_eq!(user_id, "@target:example.invalid");
                assert_eq!(alias, None);
            }
            other => panic!("unexpected command: {other:?}"),
        }

        match build_ignore_user_command(fake_request_id(60), "@ignored:example.invalid".to_owned())
        {
            CoreCommand::Account(AccountCommand::IgnoreUser {
                request_id,
                user_id,
            }) => {
                assert_eq!(request_id, fake_request_id(60));
                assert_eq!(user_id, "@ignored:example.invalid");
                let debug = format!(
                    "{:?}",
                    CoreCommand::Account(AccountCommand::IgnoreUser {
                        request_id,
                        user_id
                    })
                );
                assert!(!debug.contains("@ignored:example.invalid"), "{debug}");
            }
            other => panic!("unexpected command: {other:?}"),
        }

        match build_unignore_user_command(
            fake_request_id(61),
            "@ignored:example.invalid".to_owned(),
        ) {
            CoreCommand::Account(AccountCommand::UnignoreUser {
                request_id,
                user_id,
            }) => {
                assert_eq!(request_id, fake_request_id(61));
                assert_eq!(user_id, "@ignored:example.invalid");
                let debug = format!(
                    "{:?}",
                    CoreCommand::Account(AccountCommand::UnignoreUser {
                        request_id,
                        user_id
                    })
                );
                assert!(!debug.contains("@ignored:example.invalid"), "{debug}");
            }
            other => panic!("unexpected command: {other:?}"),
        }

        match build_report_user_command(
            fake_request_id(62),
            "@reported:example.invalid".to_owned(),
            Some("spam".to_owned()),
        ) {
            CoreCommand::Account(AccountCommand::ReportUser {
                request_id,
                user_id,
                reason,
            }) => {
                assert_eq!(request_id, fake_request_id(62));
                assert_eq!(user_id, "@reported:example.invalid");
                assert_eq!(reason, "spam");
                let debug = format!(
                    "{:?}",
                    CoreCommand::Account(AccountCommand::ReportUser {
                        request_id,
                        user_id,
                        reason: reason.clone(),
                    })
                );
                assert!(!debug.contains("@reported:example.invalid"), "{debug}");
                assert!(!debug.contains("spam"), "{debug}");
            }
            other => panic!("unexpected command: {other:?}"),
        }

        match build_report_content_command(
            fake_request_id(63),
            room_id.clone(),
            "$reported-event".to_owned(),
            Some("abuse".to_owned()),
        ) {
            CoreCommand::Room(RoomCommand::ReportContent {
                request_id,
                room_id: route_room_id,
                event_id,
                reason,
            }) => {
                assert_eq!(request_id, fake_request_id(63));
                assert_eq!(route_room_id, room_id);
                assert_eq!(event_id, "$reported-event");
                assert_eq!(reason.as_deref(), Some("abuse"));
                let debug = format!(
                    "{:?}",
                    CoreCommand::Room(RoomCommand::ReportContent {
                        request_id,
                        room_id: route_room_id.clone(),
                        event_id: event_id.clone(),
                        reason: reason.clone(),
                    })
                );
                assert!(!debug.contains("$reported-event"), "{debug}");
                assert!(!debug.contains("abuse"), "{debug}");
            }
            other => panic!("unexpected command: {other:?}"),
        }

        match build_report_room_command(
            fake_request_id(64),
            room_id.clone(),
            Some("spam room".to_owned()),
        ) {
            CoreCommand::Room(RoomCommand::ReportRoom {
                request_id,
                room_id: route_room_id,
                reason,
            }) => {
                assert_eq!(request_id, fake_request_id(64));
                assert_eq!(route_room_id, room_id);
                assert_eq!(reason, "spam room");
                let debug = format!(
                    "{:?}",
                    CoreCommand::Room(RoomCommand::ReportRoom {
                        request_id,
                        room_id: route_room_id.clone(),
                        reason: reason.clone(),
                    })
                );
                assert!(!debug.contains("spam room"), "{debug}");
            }
            other => panic!("unexpected command: {other:?}"),
        }

        match build_set_avatar_command(
            fake_request_id(33),
            "image/png".to_owned(),
            vec![9, 8, 7, 6],
        ) {
            CoreCommand::Account(AccountCommand::SetAvatar {
                request_id,
                request,
            }) => {
                assert_eq!(request_id, fake_request_id(33));
                assert_eq!(request.mime_type, "image/png");
                assert_eq!(request.bytes, vec![9, 8, 7, 6]);
                let debug = format!(
                    "{:?}",
                    CoreCommand::Account(AccountCommand::SetAvatar {
                        request_id,
                        request,
                    })
                );
                assert!(debug.contains("image/png"), "{debug}");
                assert!(!debug.contains("9, 8, 7, 6"), "{debug}");
            }
            other => panic!("unexpected command: {other:?}"),
        }

        match build_leave_room_command(fake_request_id(13), room_id.clone()) {
            CoreCommand::Room(RoomCommand::LeaveRoom {
                request_id,
                room_id: route_room_id,
            }) => {
                assert_eq!(request_id, fake_request_id(13));
                assert_eq!(route_room_id, room_id);
            }
            other => panic!("unexpected command: {other:?}"),
        }

        match build_forget_room_command(fake_request_id(14), room_id.clone()) {
            CoreCommand::Room(RoomCommand::ForgetRoom {
                request_id,
                room_id: route_room_id,
            }) => {
                assert_eq!(request_id, fake_request_id(14));
                assert_eq!(route_room_id, room_id);
            }
            other => panic!("unexpected command: {other:?}"),
        }

        match build_submit_search_command(
            fake_request_id(15),
            query.clone(),
            resolve_search_scope_from_active_room(
                SearchScopeKind::CurrentRoom,
                Some(room_id.clone()),
            ),
        ) {
            CoreCommand::Search(SearchCommand::Query {
                request_id,
                query: route_query,
                scope,
            }) => {
                assert_eq!(request_id, fake_request_id(15));
                assert_eq!(route_query, query);
                assert_eq!(
                    scope,
                    SearchScope::Room {
                        room_id: room_id.clone()
                    }
                );
            }
            other => panic!("unexpected command: {other:?}"),
        }

        assert_eq!(
            resolve_search_scope_from_active_room(SearchScopeKind::CurrentRoom, None,),
            SearchScope::Global
        );

        match build_close_search_command(fake_request_id(16)) {
            CoreCommand::App(AppCommand::CloseSearch { request_id }) => {
                assert_eq!(request_id, fake_request_id(16));
            }
            other => panic!("unexpected command: {other:?}"),
        }

        match build_create_room_command(
            fake_request_id(17),
            CreateRoomOptions {
                name: "Local QA Room".to_owned(),
                topic: Some("Local topic".to_owned()),
                alias_localpart: Some("local-qa-room".to_owned()),
                encrypted: false,
                visibility: CreateRoomVisibility::Public,
                parent_space: Some(CreateRoomParentSpace {
                    space_id: "!space:example.org".to_owned(),
                    via_server: "example.org".to_owned(),
                }),
            },
        ) {
            CoreCommand::Room(RoomCommand::CreateRoom {
                request_id,
                options,
            }) => {
                assert_eq!(request_id, fake_request_id(17));
                assert_eq!(options.name, "Local QA Room");
                assert_eq!(options.topic.as_deref(), Some("Local topic"));
                assert_eq!(options.alias_localpart.as_deref(), Some("local-qa-room"));
                assert!(!options.encrypted);
                assert_eq!(options.visibility, CreateRoomVisibility::Public);
                assert_eq!(
                    options.parent_space.as_ref().map(|parent| parent.space_id.as_str()),
                    Some("!space:example.org")
                );
                assert_eq!(
                    options.parent_space.as_ref().map(|parent| parent.via_server.as_str()),
                    Some("example.org")
                );
            }
            other => panic!("unexpected command: {other:?}"),
        }

        match build_create_space_command(fake_request_id(17), "Local QA Space".to_owned()) {
            CoreCommand::Room(RoomCommand::CreateSpace { request_id, name }) => {
                assert_eq!(request_id, fake_request_id(17));
                assert_eq!(name, "Local QA Space");
            }
            other => panic!("unexpected command: {other:?}"),
        }

        match build_set_space_child_command(
            fake_request_id(18),
            "!space:example.org".to_owned(),
            "!room:example.org".to_owned(),
            "example.org".to_owned(),
        ) {
            CoreCommand::Room(RoomCommand::SetSpaceChild {
                request_id,
                space_id,
                child_room_id,
                via_server,
            }) => {
                assert_eq!(request_id, fake_request_id(18));
                assert_eq!(space_id, "!space:example.org");
                assert_eq!(child_room_id, "!room:example.org");
                assert_eq!(via_server, "example.org");
            }
            other => panic!("unexpected command: {other:?}"),
        }

        match build_accept_invite_command(fake_request_id(19), "!invite:example.org".to_owned()) {
            CoreCommand::Room(RoomCommand::AcceptInvite {
                request_id,
                room_id,
            }) => {
                assert_eq!(request_id, fake_request_id(19));
                assert_eq!(room_id, "!invite:example.org");
            }
            other => panic!("unexpected command: {other:?}"),
        }

        match build_decline_invite_command(fake_request_id(20), "!decline:example.org".to_owned()) {
            CoreCommand::Room(RoomCommand::DeclineInvite {
                request_id,
                room_id,
            }) => {
                assert_eq!(request_id, fake_request_id(20));
                assert_eq!(room_id, "!decline:example.org");
            }
            other => panic!("unexpected command: {other:?}"),
        }

        match build_start_direct_message_command(
            fake_request_id(21),
            "@target:example.org".to_owned(),
        ) {
            CoreCommand::Room(RoomCommand::StartDirectMessage {
                request_id,
                user_id,
            }) => {
                assert_eq!(request_id, fake_request_id(21));
                assert_eq!(user_id, "@target:example.org");
            }
            other => panic!("unexpected command: {other:?}"),
        }

        match build_invite_user_command(
            fake_request_id(22),
            "!room:example.org".to_owned(),
            "@target:example.org".to_owned(),
        ) {
            CoreCommand::Room(RoomCommand::InviteUser {
                request_id,
                room_id,
                user_id,
            }) => {
                assert_eq!(request_id, fake_request_id(22));
                assert_eq!(room_id, "!room:example.org");
                assert_eq!(user_id, "@target:example.org");
            }
            other => panic!("unexpected command: {other:?}"),
        }

        match build_set_room_tag_command(
            fake_request_id(23),
            "!room:example.org".to_owned(),
            RoomTagKind::Favourite,
            Some(0.25),
        ) {
            CoreCommand::Room(RoomCommand::SetTag {
                request_id,
                room_id,
                tag,
                order,
            }) => {
                assert_eq!(request_id, fake_request_id(23));
                assert_eq!(room_id, "!room:example.org");
                assert_eq!(tag, RoomTagKind::Favourite);
                assert_eq!(order, Some(0.25));
            }
            other => panic!("unexpected command: {other:?}"),
        }

        match build_remove_room_tag_command(
            fake_request_id(24),
            "!room:example.org".to_owned(),
            RoomTagKind::LowPriority,
        ) {
            CoreCommand::Room(RoomCommand::RemoveTag {
                request_id,
                room_id,
                tag,
            }) => {
                assert_eq!(request_id, fake_request_id(24));
                assert_eq!(room_id, "!room:example.org");
                assert_eq!(tag, RoomTagKind::LowPriority);
            }
            other => panic!("unexpected command: {other:?}"),
        }

        match build_pin_event_command(
            fake_request_id(25),
            "!room:example.org".to_owned(),
            "$event:example.org".to_owned(),
        ) {
            CoreCommand::Room(RoomCommand::PinEvent {
                request_id,
                room_id,
                event_id,
            }) => {
                assert_eq!(request_id, fake_request_id(25));
                assert_eq!(room_id, "!room:example.org");
                assert_eq!(event_id, "$event:example.org");
            }
            other => panic!("unexpected command: {other:?}"),
        }

        match build_unpin_event_command(
            fake_request_id(26),
            "!room:example.org".to_owned(),
            "$event:example.org".to_owned(),
        ) {
            CoreCommand::Room(RoomCommand::UnpinEvent {
                request_id,
                room_id,
                event_id,
            }) => {
                assert_eq!(request_id, fake_request_id(26));
                assert_eq!(room_id, "!room:example.org");
                assert_eq!(event_id, "$event:example.org");
            }
            other => panic!("unexpected command: {other:?}"),
        }

        match build_query_directory_command(
            fake_request_id(27),
            Some("public rooms".to_owned()),
            Some("example.org".to_owned()),
            Some(20),
            Some("page-2".to_owned()),
        ) {
            CoreCommand::Room(RoomCommand::QueryDirectory { request_id, query }) => {
                assert_eq!(request_id, fake_request_id(27));
                assert_eq!(query.term.as_deref(), Some("public rooms"));
                assert_eq!(query.server_name.as_deref(), Some("example.org"));
                assert_eq!(query.limit, Some(20));
                assert_eq!(query.since.as_deref(), Some("page-2"));
            }
            other => panic!("unexpected command: {other:?}"),
        }

        match build_join_directory_room_command(
            fake_request_id(28),
            "#public:example.org".to_owned(),
            Some("example.org".to_owned()),
        )
        .expect("directory join should build a command")
        {
            CoreCommand::Room(RoomCommand::JoinDirectoryRoom {
                request_id,
                alias,
                via_server,
            }) => {
                assert_eq!(request_id, fake_request_id(28));
                assert_eq!(alias, "#public:example.org");
                assert_eq!(via_server.as_deref(), Some("example.org"));
            }
            other => panic!("unexpected command: {other:?}"),
        }

        assert!(
            build_join_directory_room_command(fake_request_id(29), "   ".to_owned(), None,)
                .is_none()
        );

        match build_join_room_command(fake_request_id(290), " !child:example.org ".to_owned())
            .expect("room join should build a command")
        {
            CoreCommand::Room(RoomCommand::JoinRoom {
                request_id,
                room_id,
            }) => {
                assert_eq!(request_id, fake_request_id(290));
                assert_eq!(room_id, "!child:example.org");
            }
            other => panic!("unexpected command: {other:?}"),
        }

        assert!(build_join_room_command(fake_request_id(291), "   ".to_owned()).is_none());

        match build_load_room_settings_command(fake_request_id(30), "!room:example.org".to_owned())
        {
            CoreCommand::Room(RoomCommand::LoadRoomSettings {
                request_id,
                room_id,
            }) => {
                assert_eq!(request_id, fake_request_id(30));
                assert_eq!(room_id, "!room:example.org");
            }
            other => panic!("unexpected command: {other:?}"),
        }

        for (offset, change) in [
            (
                31,
                RoomSettingChange::Name(Some("Private room name".to_owned())),
            ),
            (
                32,
                RoomSettingChange::Topic(Some("Private room topic".to_owned())),
            ),
            (
                33,
                RoomSettingChange::AvatarUrl(Some("mxc://example.org/private".to_owned())),
            ),
            (34, RoomSettingChange::JoinRule(RoomJoinRule::Invite)),
            (
                35,
                RoomSettingChange::HistoryVisibility(RoomHistoryVisibility::Shared),
            ),
        ] {
            match build_update_room_setting_command(
                fake_request_id(offset),
                "!room:example.org".to_owned(),
                change.clone(),
            ) {
                CoreCommand::Room(RoomCommand::UpdateRoomSetting {
                    request_id,
                    room_id,
                    change: routed_change,
                }) => {
                    assert_eq!(request_id, fake_request_id(offset));
                    assert_eq!(room_id, "!room:example.org");
                    assert_eq!(routed_change, change);
                }
                other => panic!("unexpected command: {other:?}"),
            }
        }

        match build_moderate_room_member_command(
            fake_request_id(36),
            "!room:example.org".to_owned(),
            "@target:example.org".to_owned(),
            RoomModerationAction::Kick,
            Some("private reason".to_owned()),
        ) {
            CoreCommand::Room(RoomCommand::ModerateRoomMember {
                request_id,
                room_id,
                target_user_id,
                action,
                reason,
            }) => {
                assert_eq!(request_id, fake_request_id(36));
                assert_eq!(room_id, "!room:example.org");
                assert_eq!(target_user_id, "@target:example.org");
                assert_eq!(action, RoomModerationAction::Kick);
                assert_eq!(reason.as_deref(), Some("private reason"));
            }
            other => panic!("unexpected command: {other:?}"),
        }

        match build_send_reply_command(
            fake_request_id(23),
            active_account_key.clone(),
            room_id.clone(),
            "desktop-reply-1".to_owned(),
            "$root".to_owned(),
            "reply body".to_owned(),
            MentionIntent::default(),
        )
        .expect("send_reply should build a command")
        {
            CoreCommand::Timeline(TimelineCommand::SendReply {
                request_id,
                key,
                transaction_id,
                in_reply_to_event_id,
                body,
                mentions,
            }) => {
                assert_eq!(request_id, fake_request_id(23));
                assert_eq!(mentions, koushi_state::MentionIntent::default());
                assert_eq!(key.account_key, active_account_key);
                assert_eq!(
                    key.kind,
                    koushi_core::TimelineKind::Room {
                        room_id: room_id.clone()
                    }
                );
                assert_eq!(transaction_id, "desktop-reply-1");
                assert_eq!(in_reply_to_event_id, "$root");
                assert_eq!(body, "reply body");
            }
            other => panic!("unexpected command: {other:?}"),
        }

        match build_send_thread_reply_command(
            fake_request_id(24),
            active_account_key.clone(),
            room_id.clone(),
            "$root".to_owned(),
            "desktop-thread-reply-1".to_owned(),
            "thread reply body".to_owned(),
        )
        .expect("send_thread_reply should build a command")
        {
            CoreCommand::Timeline(TimelineCommand::SendReply {
                request_id,
                key,
                transaction_id,
                in_reply_to_event_id,
                body,
                mentions,
            }) => {
                assert_eq!(request_id, fake_request_id(24));
                assert_eq!(mentions, koushi_state::MentionIntent::default());
                assert_eq!(key.account_key, active_account_key);
                assert_eq!(
                    key.kind,
                    koushi_core::TimelineKind::Thread {
                        room_id: room_id.clone(),
                        root_event_id: "$root".to_owned(),
                    }
                );
                assert_eq!(transaction_id, "desktop-thread-reply-1");
                assert_eq!(in_reply_to_event_id, "$root");
                assert_eq!(body, "thread reply body");
            }
            other => panic!("unexpected command: {other:?}"),
        }

        match build_set_thread_composer_draft_command(
            fake_request_id(21),
            room_id.clone(),
            "$root".to_owned(),
            "thread draft".to_owned(),
        ) {
            CoreCommand::App(AppCommand::SetThreadComposerDraft {
                request_id,
                room_id: command_room_id,
                root_event_id,
                draft,
            }) => {
                assert_eq!(request_id, fake_request_id(21));
                assert_eq!(command_room_id, room_id);
                assert_eq!(root_event_id, "$root");
                assert_eq!(draft, "thread draft");
            }
            other => panic!("unexpected command: {other:?}"),
        }

        match build_set_composer_draft_command(
            fake_request_id(22),
            room_id.clone(),
            "room draft".to_owned(),
        ) {
            CoreCommand::App(AppCommand::SetComposerDraft {
                request_id,
                room_id: command_room_id,
                draft,
            }) => {
                assert_eq!(request_id, fake_request_id(22));
                assert_eq!(command_room_id, room_id);
                assert_eq!(draft, "room draft");
            }
            other => panic!("unexpected command: {other:?}"),
        }

        match build_open_activity_command(fake_request_id(37)) {
            CoreCommand::App(AppCommand::OpenActivity { request_id }) => {
                assert_eq!(request_id, fake_request_id(37));
            }
            other => panic!("unexpected command: {other:?}"),
        }

        match build_set_activity_tab_command(fake_request_id(38), ActivityTab::Unread) {
            CoreCommand::App(AppCommand::SetActivityTab { request_id, tab }) => {
                assert_eq!(request_id, fake_request_id(38));
                assert_eq!(tab, ActivityTab::Unread);
            }
            other => panic!("unexpected command: {other:?}"),
        }

        match build_paginate_activity_command(
            fake_request_id(39),
            ActivityTab::Recent,
            Some("page-2".to_owned()),
        ) {
            CoreCommand::App(AppCommand::PaginateActivity {
                request_id,
                tab,
                cursor,
            }) => {
                assert_eq!(request_id, fake_request_id(39));
                assert_eq!(tab, ActivityTab::Recent);
                assert_eq!(cursor.as_deref(), Some("page-2"));
            }
            other => panic!("unexpected command: {other:?}"),
        }

        assert!(matches!(
            build_paginate_activity_command(
                fake_request_id(40),
                ActivityTab::Unread,
                Some("  ".to_owned())
            ),
            CoreCommand::App(AppCommand::PaginateActivity { cursor: None, .. })
        ));

        let target = ActivityMarkReadTarget::Room {
            room_id: "!room:example.org".to_owned(),
            up_to_event_id: "$event:example.org".to_owned(),
        };
        match build_mark_activity_read_command(fake_request_id(41), target.clone()) {
            CoreCommand::App(AppCommand::MarkActivityRead {
                request_id,
                target: routed_target,
            }) => {
                assert_eq!(request_id, fake_request_id(41));
                assert_eq!(routed_target, target);
            }
            other => panic!("unexpected command: {other:?}"),
        }

        match build_close_activity_command(fake_request_id(42)) {
            CoreCommand::App(AppCommand::CloseActivity { request_id }) => {
                assert_eq!(request_id, fake_request_id(42));
            }
            other => panic!("unexpected command: {other:?}"),
        }

        let files_scope = koushi_state::FilesViewScope::Room {
            room_id: "!room:example.org".to_owned(),
        };
        let files_filter = koushi_state::AttachmentFilter {
            kinds: vec![koushi_state::AttachmentKind::Image],
            filename_query: Some("cat".to_owned()),
        };
        match build_open_files_view_command(
            fake_request_id(65),
            files_scope.clone(),
            files_filter.clone(),
            koushi_state::AttachmentSort::Filename,
        ) {
            CoreCommand::App(AppCommand::OpenFilesView {
                request_id,
                scope,
                filter,
                sort,
            }) => {
                assert_eq!(request_id, fake_request_id(65));
                assert_eq!(scope, files_scope);
                assert_eq!(filter, files_filter);
                assert!(matches!(sort, koushi_state::AttachmentSort::Filename));
            }
            other => panic!("unexpected command: {other:?}"),
        }

        match build_close_files_view_command(fake_request_id(66)) {
            CoreCommand::App(AppCommand::CloseFilesView { request_id }) => {
                assert_eq!(request_id, fake_request_id(66));
            }
            other => panic!("unexpected command: {other:?}"),
        }

        match build_update_room_member_role_command(
            fake_request_id(37),
            "!room:example.org".to_owned(),
            "@target:example.org".to_owned(),
            50,
        ) {
            CoreCommand::Room(RoomCommand::UpdateRoomMemberRole {
                request_id,
                room_id,
                target_user_id,
                power_level,
            }) => {
                assert_eq!(request_id, fake_request_id(37));
                assert_eq!(room_id, "!room:example.org");
                assert_eq!(target_user_id, "@target:example.org");
                assert_eq!(power_level, 50);
            }
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn load_link_previews_tauri_command_contract_is_present() {
        let request_id = koushi_core::RequestId {
            connection_id: koushi_core::RuntimeConnectionId(1),
            sequence: 1,
        };
        let command = build_load_link_previews_command(
            request_id,
            AccountKey("@u:example.test".to_owned()),
            "!room:example.test".to_owned(),
            "$event:example.test".to_owned(),
        );
        assert!(matches!(
            command,
            Some(CoreCommand::Timeline(
                TimelineCommand::LoadLinkPreviews { .. }
            ))
        ));
    }

    #[test]
    fn hide_link_preview_tauri_command_contract_is_present() {
        let request_id = koushi_core::RequestId {
            connection_id: koushi_core::RuntimeConnectionId(1),
            sequence: 1,
        };
        let command = build_hide_link_preview_command(
            request_id,
            AccountKey("@u:example.test".to_owned()),
            "!room:example.test".to_owned(),
            "$event:example.test".to_owned(),
        );
        assert!(matches!(
            command,
            Some(CoreCommand::Timeline(
                TimelineCommand::HideLinkPreview { .. }
            ))
        ));
    }

    #[test]
    fn tauri_command_routes_blank_message_bodies_return_no_command() {
        let account_key = AccountKey("@alice:example.org".to_owned());
        let room_id = "!room:example.org".to_owned();

        assert!(
            build_send_text_command(
                fake_request_id(14),
                account_key.clone(),
                room_id.clone(),
                "desktop-14".to_owned(),
                "   ".to_owned(),
                MentionIntent::default(),
            )
            .is_none()
        );
        assert!(
            build_edit_message_command(
                fake_request_id(15),
                account_key,
                room_id,
                "$event".to_owned(),
                "\n\t ".to_owned(),
            )
            .is_none()
        );
        assert!(
            build_upload_media_command(
                fake_request_id(17),
                AccountKey("@alice:example.org".to_owned()),
                "!room:example.org".to_owned(),
                "desktop-media-empty".to_owned(),
                "empty.bin".to_owned(),
                "application/octet-stream".to_owned(),
                vec![],
                None,
                ImageUploadCompressionMode::Never,
                ImageUploadCompressionPolicy::default(),
                None,
                None,
                None,
            )
            .is_none()
        );
        assert!(
            build_download_media_command(
                fake_request_id(18),
                AccountKey("@alice:example.org".to_owned()),
                "!room:example.org".to_owned(),
                "\n\t ".to_owned(),
            )
            .is_none()
        );
        assert!(
            build_send_thread_reply_command(
                fake_request_id(16),
                AccountKey("@alice:example.org".to_owned()),
                "!room:example.org".to_owned(),
                "$root".to_owned(),
                "desktop-16".to_owned(),
                "\n\t ".to_owned(),
            )
            .is_none()
        );
    }

    #[test]
    fn thread_timeline_backwards_pagination_builder_targets_thread_key() {
        let account_key = AccountKey("@alice:example.org".to_owned());
        let room_id = "!room:example.org".to_owned();
        let root_event_id = "$thread-root".to_owned();

        match build_paginate_thread_timeline_backwards_command(
            fake_request_id(22),
            account_key.clone(),
            room_id.clone(),
            root_event_id.clone(),
        ) {
            CoreCommand::Timeline(TimelineCommand::Paginate {
                request_id,
                key,
                direction,
                event_count,
            }) => {
                assert_eq!(request_id, fake_request_id(22));
                assert_eq!(key.account_key, account_key);
                assert_eq!(
                    key.kind,
                    koushi_core::TimelineKind::Thread {
                        room_id,
                        root_event_id,
                    }
                );
                assert_eq!(direction, PaginationDirection::Backward);
                assert_eq!(event_count, 100);
            }
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn thread_timeline_backwards_pagination_contract_is_present() {
        let commands_source = commands_source();
        let lib_source = include_str!("../lib.rs");
        let helper_name = "build_paginate_thread_timeline_backwards_command";
        let command_name = "pub async fn paginate_thread_timeline_backwards";
        let registration_name = "commands::timeline::paginate_thread_timeline_backwards";

        let helper_offset = commands_source
            .find(helper_name)
            .expect("thread pagination builder helper should exist");
        let helper_source = &commands_source[helper_offset..];
        let helper_end = helper_source
            .find("pub(crate) fn build_send_text_command")
            .expect("thread pagination builder should live before send_text builder");
        let helper_source = &helper_source[..helper_end];

        assert!(
            commands_source.contains(command_name),
            "Tauri command should expose thread pagination"
        );
        assert!(
            lib_source.contains(registration_name),
            "Tauri command should be registered in generate_handler"
        );
        assert!(
            helper_source.contains("TimelineKind::Thread"),
            "thread pagination builder should use a thread timeline key"
        );
        assert!(
            helper_source.contains("PaginationDirection::Backward"),
            "thread pagination builder should request backward pagination"
        );
        assert!(
            helper_source.contains("event_count: TIMELINE_BACKWARDS_PAGE_EVENT_COUNT"),
            "thread pagination should keep the shared room pagination event count"
        );
    }

    #[test]
    fn reaction_tauri_command_contracts_are_present() {
        let commands_source = commands_source();
        let lib_source = include_str!("../lib.rs");
        for (command_name, route_name, registration_name) in [
            (
                "pub async fn send_reaction",
                "build_send_reaction_command",
                "commands::timeline::send_reaction",
            ),
            (
                "pub async fn redact_reaction",
                "build_redact_reaction_command",
                "commands::timeline::redact_reaction",
            ),
        ] {
            assert!(
                commands_source.contains(command_name),
                "Tauri command should expose {command_name}"
            );
            assert!(
                commands_source.contains(route_name),
                "Tauri command should route through {route_name}"
            );
            assert!(
                lib_source.contains(registration_name),
                "Tauri command should register {registration_name}"
            );
        }
    }

    #[test]
    fn send_queue_tauri_command_contracts_are_present() {
        let commands_source = commands_source();
        let lib_source = include_str!("../lib.rs");
        for (command_name, route_name, registration_name) in [
            (
                "pub async fn retry_send",
                "build_retry_send_command",
                "commands::timeline::retry_send",
            ),
            (
                "pub async fn cancel_send",
                "build_cancel_send_command",
                "commands::timeline::cancel_send",
            ),
        ] {
            assert!(
                commands_source.contains(command_name),
                "Tauri command should expose {command_name}"
            );
            assert!(
                commands_source.contains(route_name),
                "Tauri command should route through {route_name}"
            );
            assert!(
                lib_source.contains(registration_name),
                "Tauri command should register {registration_name}"
            );
        }
    }

    #[test]
    fn scheduled_send_tauri_command_contracts_are_present() {
        let commands_source = commands_source();
        let lib_source = include_str!("../lib.rs");
        for (command_name, route_name, registration_name) in [
            (
                "pub async fn schedule_send",
                "build_schedule_send_command",
                "commands::timeline::schedule_send",
            ),
            (
                "pub async fn cancel_scheduled_send",
                "build_cancel_scheduled_send_command",
                "commands::timeline::cancel_scheduled_send",
            ),
            (
                "pub async fn reschedule_scheduled_send",
                "build_reschedule_scheduled_send_command",
                "commands::timeline::reschedule_scheduled_send",
            ),
        ] {
            assert!(
                commands_source.contains(command_name),
                "Tauri command should expose {command_name}"
            );
            assert!(
                commands_source.contains(route_name),
                "Tauri command should route through {route_name}"
            );
            assert!(
                lib_source.contains(registration_name),
                "Tauri command should register {registration_name}"
            );
        }
    }

    #[test]
    fn activity_tauri_command_contracts_are_present() {
        let commands_source = commands_source();
        let lib_source = include_str!("../lib.rs");
        for (command_name, route_name, registration_name) in [
            (
                "pub async fn open_activity",
                "build_open_activity_command",
                "commands::activity::open_activity",
            ),
            (
                "pub async fn close_activity",
                "build_close_activity_command",
                "commands::activity::close_activity",
            ),
            (
                "pub async fn set_activity_tab",
                "build_set_activity_tab_command",
                "commands::activity::set_activity_tab",
            ),
            (
                "pub async fn paginate_activity",
                "build_paginate_activity_command",
                "commands::activity::paginate_activity",
            ),
            (
                "pub async fn mark_activity_read",
                "build_mark_activity_read_command",
                "commands::activity::mark_activity_read",
            ),
            (
                "pub async fn open_files_view",
                "build_open_files_view_command",
                "commands::views::open_files_view",
            ),
            (
                "pub async fn close_files_view",
                "build_close_files_view_command",
                "commands::views::close_files_view",
            ),
        ] {
            assert!(
                commands_source.contains(command_name),
                "Tauri command should expose {command_name}"
            );
            assert!(
                commands_source.contains(route_name),
                "Tauri command should route through {route_name}"
            );
            assert!(
                lib_source.contains(registration_name),
                "Tauri command should register {registration_name}"
            );
        }
    }

    #[test]
    fn update_settings_command_routes_patch_to_app_update_settings() {
        let command = build_update_settings_command(
            fake_request_id(23),
            SettingsPatch {
                appearance: Some(AppearanceSettings {
                    theme: ThemePreference::Dark,
                }),
                ..SettingsPatch::default()
            },
        );

        match command {
            CoreCommand::App(AppCommand::UpdateSettings { request_id, patch }) => {
                assert_eq!(request_id, fake_request_id(23));
                assert_eq!(
                    patch.appearance.expect("appearance patch").theme,
                    ThemePreference::Dark
                );
            }
            other => panic!("unexpected command: {other:?}"),
        }

        let debug = format!(
            "{:?}",
            build_update_settings_command(
                fake_request_id(24),
                SettingsPatch {
                    locale: Some(LocaleSettings {
                        language_tag: Some("ja-JP-private".to_owned()),
                        text_direction: TextDirectionPreference::Auto,
                    }),
                    ..SettingsPatch::default()
                },
            )
        );
        assert!(debug.contains("locale"), "{debug}");
        assert!(!debug.contains("ja-JP-private"), "{debug}");
    }

    #[test]
    fn set_room_url_preview_override_command_routes_to_app_state() {
        let command = build_set_room_url_preview_override_command(
            fake_request_id(24),
            "!room:example.invalid".to_owned(),
            false,
        );

        match command {
            CoreCommand::App(AppCommand::SetRoomUrlPreviewOverride {
                request_id,
                room_id,
                enabled,
            }) => {
                assert_eq!(request_id, fake_request_id(24));
                assert_eq!(room_id, "!room:example.invalid");
                assert!(!enabled);
            }
            other => panic!("unexpected command: {other:?}"),
        }

        let debug = format!(
            "{:?}",
            build_set_room_url_preview_override_command(
                fake_request_id(25),
                "!private-room:example.invalid".to_owned(),
                true,
            )
        );
        assert!(debug.contains("SetRoomUrlPreviewOverride"), "{debug}");
        assert!(debug.contains("RoomId(..)"), "{debug}");
        assert!(!debug.contains("!private-room:example.invalid"), "{debug}");
    }

    #[test]
    fn e2ee_trust_commands_route_to_account_state_machine() {
        match build_bootstrap_cross_signing_command(
            fake_request_id(25),
            Some(AuthSecret::new("cross-signing-password")),
        ) {
            CoreCommand::Account(AccountCommand::BootstrapCrossSigning { request_id, auth }) => {
                assert_eq!(request_id, fake_request_id(25));
                assert_eq!(
                    auth.expect("auth secret").expose_secret(),
                    "cross-signing-password"
                );
            }
            other => panic!("unexpected command: {other:?}"),
        }

        match build_enable_key_backup_command(fake_request_id(26)) {
            CoreCommand::Account(AccountCommand::EnableKeyBackup {
                request_id,
                passphrase,
            }) => {
                assert_eq!(request_id, fake_request_id(26));
                assert!(passphrase.is_none());
            }
            other => panic!("unexpected command: {other:?}"),
        }

        match build_accept_verification_command(fake_request_id(27), 72) {
            CoreCommand::Account(AccountCommand::AcceptVerification {
                request_id,
                flow_id,
            }) => {
                assert_eq!(request_id, fake_request_id(27));
                assert_eq!(flow_id, 72);
            }
            other => panic!("unexpected command: {other:?}"),
        }

        match build_confirm_sas_verification_command(fake_request_id(28), 73) {
            CoreCommand::Account(AccountCommand::ConfirmSasVerification {
                request_id,
                flow_id,
            }) => {
                assert_eq!(request_id, fake_request_id(28));
                assert_eq!(flow_id, 73);
            }
            other => panic!("unexpected command: {other:?}"),
        }

        match build_cancel_verification_command(
            fake_request_id(29),
            74,
            VerificationCancelReason::User,
        ) {
            CoreCommand::Account(AccountCommand::CancelVerification {
                request_id,
                flow_id,
                reason,
            }) => {
                assert_eq!(request_id, fake_request_id(29));
                assert_eq!(flow_id, 74);
                assert_eq!(reason, VerificationCancelReason::User);
            }
            other => panic!("unexpected command: {other:?}"),
        }

        match build_reset_identity_command(fake_request_id(30)) {
            CoreCommand::Account(AccountCommand::ResetIdentity { request_id }) => {
                assert_eq!(request_id, fake_request_id(30));
            }
            other => panic!("unexpected command: {other:?}"),
        }

        let password_command = build_submit_identity_reset_password_command(
            fake_request_id(31),
            75,
            AuthSecret::new("identity-reset-password"),
        );
        match &password_command {
            CoreCommand::Account(AccountCommand::SubmitIdentityResetAuth {
                request_id,
                flow_id,
                request: IdentityResetAuthRequest::UiaaPassword { password },
            }) => {
                assert_eq!(*request_id, fake_request_id(31));
                assert_eq!(*flow_id, 75);
                assert_eq!(password.expose_secret(), "identity-reset-password");
            }
            other => panic!("unexpected command: {other:?}"),
        }
        let debug = format!("{password_command:?}");
        assert!(!debug.contains("identity-reset-password"), "{debug}");

        match build_submit_identity_reset_oauth_command(fake_request_id(32), 76) {
            CoreCommand::Account(AccountCommand::SubmitIdentityResetAuth {
                request_id,
                flow_id,
                request: IdentityResetAuthRequest::OAuthApproved,
            }) => {
                assert_eq!(request_id, fake_request_id(32));
                assert_eq!(flow_id, 76);
            }
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn update_settings_tauri_command_contract_is_present() {
        let commands_source = commands_source();
        let lib_source = include_str!("../lib.rs");
        for (command_name, builder_name, route_name, registration_name) in [
            (
                "pub async fn update_settings",
                "build_update_settings_command",
                "AppCommand::UpdateSettings",
                "commands::settings::update_settings",
            ),
            (
                "pub async fn set_room_url_preview_override",
                "build_set_room_url_preview_override_command",
                "AppCommand::SetRoomUrlPreviewOverride",
                "commands::settings::set_room_url_preview_override",
            ),
        ] {
            assert!(
                commands_source.contains(command_name),
                "Tauri command should expose {command_name}"
            );
            assert!(
                commands_source.contains(builder_name),
                "Tauri command should keep a testable builder {builder_name}"
            );
            assert!(
                commands_source.contains(route_name),
                "Tauri command should route through {route_name}"
            );
            assert!(
                lib_source.contains(registration_name),
                "Tauri command should register {registration_name}"
            );
        }
    }

    #[test]
    fn rebuild_search_index_tauri_command_contract_is_present() {
        let commands_source = commands_source();
        let lib_source = include_str!("../lib.rs");

        assert!(
            commands_source.contains("pub async fn rebuild_search_index"),
            "Tauri command should expose search index rebuild"
        );
        assert!(
            commands_source.contains("build_rebuild_search_index_command"),
            "Tauri command should route through a testable builder"
        );
        assert!(
            commands_source.contains("AppCommand::RebuildSearchIndex"),
            "Tauri command should route through app state"
        );
        assert!(
            lib_source.contains("commands::settings::rebuild_search_index"),
            "Tauri command should be registered in generate_handler"
        );
    }

    #[test]
    fn credential_health_command_routes_to_account_state_machine() {
        match build_probe_local_encryption_health_command(fake_request_id(47)) {
            CoreCommand::Account(AccountCommand::ProbeLocalEncryptionHealth { request_id }) => {
                assert_eq!(request_id, fake_request_id(47));
            }
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn reset_local_data_command_routes_to_account_state_machine() {
        match build_reset_local_data_command(fake_request_id(48)) {
            CoreCommand::Account(AccountCommand::ResetLocalData { request_id }) => {
                assert_eq!(request_id, fake_request_id(48));
            }
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn credential_health_tauri_command_contract_is_present() {
        let commands_source = commands_source();
        let lib_source = include_str!("../lib.rs");
        let command_name = "pub async fn probe_local_encryption_health";
        let builder_name = "build_probe_local_encryption_health_command";
        let route_name = "AccountCommand::ProbeLocalEncryptionHealth";
        let registration_name = "commands::local_encryption::probe_local_encryption_health";

        assert!(
            commands_source.contains(command_name),
            "Tauri command should expose probe_local_encryption_health"
        );
        assert!(
            commands_source.contains(builder_name),
            "Tauri command should keep a testable local encryption probe builder"
        );
        assert!(
            commands_source.contains(route_name),
            "Tauri command should route through the Rust credential health state machine"
        );
        assert!(
            lib_source.contains(registration_name),
            "Tauri command should be registered in generate_handler"
        );
    }

    #[test]
    fn reset_local_data_tauri_command_contract_is_present() {
        let commands_source = commands_source();
        let lib_source = include_str!("../lib.rs");
        let command_name = "pub async fn reset_local_data";
        let builder_name = "build_reset_local_data_command";
        let route_name = "AccountCommand::ResetLocalData";
        let registration_name = "commands::local_encryption::reset_local_data";

        assert!(
            commands_source.contains(command_name),
            "Tauri command should expose reset_local_data"
        );
        assert!(
            commands_source.contains(builder_name),
            "Tauri command should keep a testable local data reset builder"
        );
        assert!(
            commands_source.contains(route_name),
            "Tauri command should route through the Rust local-encryption state machine"
        );
        assert!(
            lib_source.contains(registration_name),
            "Tauri command should be registered in generate_handler"
        );
    }

    #[test]
    fn e2ee_trust_tauri_command_contracts_are_present() {
        let commands_source = commands_source();
        let lib_source = include_str!("../lib.rs");
        for (command_name, route_name, registration_name) in [
            (
                "pub async fn bootstrap_cross_signing",
                "build_bootstrap_cross_signing_command",
                "commands::e2ee::bootstrap_cross_signing",
            ),
            (
                "pub async fn enable_key_backup",
                "build_enable_key_backup_command",
                "commands::e2ee::enable_key_backup",
            ),
            (
                "pub async fn export_room_keys",
                "build_export_room_keys_command",
                "commands::e2ee::export_room_keys",
            ),
            (
                "pub async fn import_room_keys",
                "build_import_room_keys_command",
                "commands::e2ee::import_room_keys",
            ),
            (
                "pub async fn bootstrap_secure_backup",
                "build_bootstrap_secure_backup_command",
                "commands::e2ee::bootstrap_secure_backup",
            ),
            (
                "pub async fn change_secure_backup_passphrase",
                "build_change_secure_backup_passphrase_command",
                "commands::e2ee::change_secure_backup_passphrase",
            ),
            (
                "pub async fn accept_verification",
                "build_accept_verification_command",
                "commands::e2ee::accept_verification",
            ),
            (
                "pub async fn confirm_sas_verification",
                "build_confirm_sas_verification_command",
                "commands::e2ee::confirm_sas_verification",
            ),
            (
                "pub async fn cancel_verification",
                "build_cancel_verification_command",
                "commands::e2ee::cancel_verification",
            ),
            (
                "pub async fn reset_identity",
                "build_reset_identity_command",
                "commands::e2ee::reset_identity",
            ),
            (
                "pub async fn submit_identity_reset_password",
                "build_submit_identity_reset_password_command",
                "commands::e2ee::submit_identity_reset_password",
            ),
            (
                "pub async fn submit_identity_reset_oauth",
                "build_submit_identity_reset_oauth_command",
                "commands::e2ee::submit_identity_reset_oauth",
            ),
        ] {
            assert!(
                commands_source.contains(command_name),
                "Tauri command should expose {command_name}"
            );
            assert!(
                commands_source.contains(route_name),
                "Tauri command should route through {route_name}"
            );
            assert!(
                lib_source.contains(registration_name),
                "Tauri command should register {registration_name}"
            );
        }
    }

    #[test]
    fn profile_tauri_command_contracts_are_present() {
        let commands_source = commands_source();
        let lib_source = include_str!("../lib.rs");
        for (command_name, route_name, registration_name) in [
            (
                "pub async fn set_display_name",
                "build_set_display_name_command",
                "commands::profile::set_display_name",
            ),
            (
                "pub async fn set_local_user_alias",
                "build_set_local_user_alias_command",
                "commands::profile::set_local_user_alias",
            ),
            (
                "pub async fn set_avatar",
                "build_set_avatar_command",
                "commands::profile::set_avatar",
            ),
        ] {
            assert!(
                commands_source.contains(command_name),
                "Tauri command should expose {command_name}"
            );
            assert!(
                commands_source.contains(route_name),
                "Tauri command should route through {route_name}"
            );
            assert!(
                lib_source.contains(registration_name),
                "Tauri command should register {registration_name}"
            );
        }
    }

    #[test]
    fn composer_key_resolver_command_contract_is_present() {
        let commands_source = commands_source();
        let lib_source = include_str!("../lib.rs");
        let command_name = "pub async fn resolve_composer_key_action";
        let route_name = "koushi_state::resolve_composer_key_action";
        let settings_token = "settings.values.keyboard.composer_send_shortcut";
        let registration_name = "commands::timeline::resolve_composer_key_action";

        assert!(
            commands_source.contains(command_name),
            "Tauri command should expose resolve_composer_key_action"
        );
        assert!(
            commands_source.contains(route_name),
            "Tauri command should route through the Rust-owned resolver"
        );
        assert!(
            commands_source.contains(settings_token),
            "resolver should derive the send shortcut from Rust-owned settings"
        );
        assert!(
            lib_source.contains(registration_name),
            "Tauri command should be registered in generate_handler"
        );
    }

    #[test]
    fn select_room_waits_for_core_selection_without_resubscribing_timeline() {
        let source = commands_source();
        let fn_name = concat!("pub async fn select", "_room");
        let select_token = concat!("build_select", "_room_command");
        let attach_token = concat!("state.runtime.", "attach");
        let wait_token = concat!("wait_for_selected", "_room");
        let subscribe_token = concat!("build_subscribe", "_timeline_command");
        let account_key_token = concat!("account_key", "_from_snapshot");
        let timeout_token = concat!("SELECT_ROOM", "_EVENT_TIMEOUT");
        let fn_offset = source
            .find(fn_name)
            .expect("select_room command should exist");
        let rest = &source[fn_offset..];
        let end = rest
            .find("pub async fn open_activity_event")
            .expect("next command should exist");
        let select_room_source = &rest[..end];
        let attach_offset = select_room_source
            .find(attach_token)
            .expect("select_room should attach an event connection before selecting");
        let select_offset = select_room_source
            .find(select_token)
            .expect("select_room should submit room selection");
        let wait_offset = select_room_source
            .find(wait_token)
            .expect("select_room should wait for selected-room state");

        assert!(
            attach_offset < select_offset,
            "event connection should be attached before room selection"
        );
        assert!(
            select_offset < wait_offset,
            "room selection state should be observed after submitting selection"
        );
        assert!(
            !select_room_source.contains(subscribe_token),
            "room selection reducers already emit the canonical timeline subscription"
        );
        assert!(
            !select_room_source.contains(account_key_token),
            "select_room should not derive an account key just to duplicate timeline subscription"
        );
        assert!(
            select_room_source.contains(timeout_token),
            "selected-room wait should be bounded"
        );
    }

    #[test]
    fn room_transition_and_backfill_commands_emit_submit_trace_tokens() {
        let source = commands_source();
        assert!(
            source.contains("fn trace_tauri_timeline_command"),
            "Tauri command layer must expose a private-data-free timeline trace helper"
        );
        assert!(
            source.contains("koushi.desktop"),
            "Tauri command traces must share a stable koushi.desktop prefix"
        );
        let select_start = source
            .find("pub async fn select_room")
            .expect("select_room command should exist");
        let paginate_start = source
            .find("pub async fn paginate_timeline_backwards")
            .expect("paginate command should exist");
        let load_link_previews_start = source
            .find("pub async fn load_link_previews")
            .expect("load_link_previews command should exist");
        let select_source = &source[select_start..paginate_start];
        let paginate_source = &source[paginate_start..load_link_previews_start];
        let load_link_previews_source = &source[load_link_previews_start..];
        assert!(
            select_source.contains("trace_tauri_timeline_command(\"submit\", \"select_room\""),
            "select_room should trace the submitted room transition command"
        );
        assert!(
            paginate_source
                .contains("trace_tauri_timeline_command(\"submit\", \"paginate_backwards\""),
            "paginate_timeline_backwards should trace submitted backfill requests"
        );
        assert!(
            load_link_previews_source
                .contains("trace_tauri_timeline_command(\"submit\", \"load_link_previews\""),
            "load_link_previews should trace submitted preview expansion requests"
        );
    }

    #[test]
    fn select_search_result_selects_room_then_enters_anchored_timeline_without_room_resubscribe() {
        let source = commands_source();
        let fn_name = "pub async fn select_search_result";
        let select_token = "select_search_result";
        let close_token = "CloseFocusedContext";
        let open_token = "OpenFocusedContext";
        let anchor_token = "EnterAnchoredTimeline";
        let select_room_token = concat!("build_select", "_room_command");
        let subscribe_room_token = "build_subscribe_timeline_command";

        let fn_offset = source
            .find(fn_name)
            .expect("select_search_result command should exist");
        let rest = &source[fn_offset..];
        let end = rest
            .find("pub async fn close_focused_context")
            .expect("next command should exist");
        let select_source = &rest[..end];

        assert!(
            select_source.contains(select_token),
            "select_search_result should name the command path"
        );
        assert!(
            select_source.contains(close_token),
            "select_search_result should close the previous focused context"
        );
        assert!(
            select_source.contains(open_token),
            "select_search_result should subscribe the focused event timeline"
        );
        assert!(
            select_source.contains(anchor_token),
            "select_search_result should route the selected result into the main anchored timeline"
        );
        assert!(
            select_source.contains(select_room_token),
            "select_search_result should select the room before opening the focused context"
        );
        assert!(
            !select_source.contains(subscribe_room_token),
            "select_search_result should rely on room selection reducers for room timeline subscription"
        );
        assert!(
            select_source.contains("wait_for_selected_room"),
            "select_search_result should wait for the selected room state"
        );
        assert!(
            select_source.contains("state.runtime.attach"),
            "select_search_result should attach a fresh core connection"
        );

        let select_offset = select_source
            .find(select_room_token)
            .expect("search result command should select the room");
        let wait_offset = select_source
            .find("wait_for_selected_room")
            .expect("search result command should wait for the selected room");
        let open_offset = select_source
            .find(open_token)
            .expect("search result command should open focused context");
        let anchor_offset = select_source
            .find(anchor_token)
            .expect("search result command should enter anchored timeline");
        assert!(
            select_offset < wait_offset && wait_offset < open_offset && open_offset < anchor_offset,
            "focused event timeline should open and become the main anchored timeline only after the selected room state is observed"
        );
    }

    #[test]
    fn submit_search_waits_for_correlated_result_before_returning_snapshot() {
        let source = commands_source();
        let fn_name = "pub async fn submit_search";

        let fn_offset = source
            .find(fn_name)
            .expect("submit_search command should exist");
        let rest = &source[fn_offset..];
        let end = rest
            .find("pub async fn start_room_crawl")
            .expect("start_room_crawl command should follow submit_search");
        let command_source = &rest[..end];

        let submit_offset = command_source
            .find("build_submit_search_command")
            .expect("submit_search should submit the query command");
        let wait_offset = command_source
            .find("wait_for_search_completed")
            .expect("submit_search must wait for the correlated search result");
        let snapshot_offset = command_source
            .find("current_snapshot")
            .expect("submit_search should return a snapshot");
        assert!(
            submit_offset < wait_offset && wait_offset < snapshot_offset,
            "submit_search must not return a pre-result searching snapshot"
        );
    }

    #[test]
    fn open_activity_event_opens_anchored_main_timeline_without_room_resubscribe() {
        let source = commands_source();
        let fn_name = "pub async fn open_activity_event";
        let open_token = "OpenFocusedContext";
        let anchor_token = "EnterAnchoredTimeline";

        let fn_offset = source
            .find(fn_name)
            .expect("open_activity_event command should exist");
        let rest = &source[fn_offset..];
        let end = rest
            .find("pub async fn select_search_result")
            .expect("select_search_result command should follow open_activity_event");
        let command_source = &rest[..end];

        assert!(
            command_source.contains("build_select_room_command"),
            "activity event navigation should select the destination room"
        );
        assert!(
            !command_source.contains("build_subscribe_timeline_command"),
            "activity event navigation should rely on room selection reducers for timeline subscription"
        );
        assert!(
            command_source.contains(open_token),
            "activity event navigation should subscribe the focused event timeline"
        );
        assert!(
            command_source.contains(anchor_token),
            "activity event navigation should route the activity event into the main anchored timeline"
        );
        assert!(
            command_source.contains("wait_for_focused_context"),
            "activity event navigation should wait for the focused event timeline"
        );
        assert!(
            command_source.contains("wait_for_main_timeline_anchor"),
            "activity event navigation should wait for the main anchored timeline"
        );
        assert!(
            !command_source.contains("build_update_navigation_scroll_anchor_command"),
            "activity event navigation must not anchor an event that may be absent from the live timeline"
        );
    }

    #[test]
    fn open_activity_event_waits_before_opening_anchored_event_timeline() {
        let source = commands_source();
        let fn_name = "pub async fn open_activity_event";
        let open_token = "OpenFocusedContext";
        let anchor_token = "EnterAnchoredTimeline";

        let fn_offset = source
            .find(fn_name)
            .expect("open_activity_event command should exist");
        let rest = &source[fn_offset..];
        let end = rest
            .find("pub async fn select_search_result")
            .expect("select_search_result command should follow open_activity_event");
        let command_source = &rest[..end];

        let close_offset = command_source
            .find("CloseFocusedContext")
            .expect("activity event navigation should close any focused main timeline first");
        let wait_close_offset = command_source
            .find("wait_for_focused_context_closed")
            .expect(
                "activity event navigation must wait until focused context/main anchor is closed",
            );
        let select_offset = command_source
            .find("build_select_room_command")
            .expect("activity event navigation should select the destination room");
        let wait_select_offset = command_source
            .find("wait_for_selected_room")
            .expect("activity event navigation should wait for the selected room state");
        let open_offset = command_source
            .find(open_token)
            .expect("activity event navigation should open the focused event timeline");
        let wait_open_offset = command_source[open_offset..]
            .find("wait_for_focused_context(")
            .map(|offset| open_offset + offset)
            .expect("activity event navigation should wait for focused event timeline state");
        let anchor_offset = command_source
            .find(anchor_token)
            .expect("activity event navigation should enter the main anchored timeline");

        assert!(
            close_offset < wait_close_offset
                && wait_close_offset < select_offset
                && select_offset < wait_select_offset
                && wait_select_offset < open_offset
                && open_offset < wait_open_offset
                && wait_open_offset < anchor_offset,
            "activity event navigation must clear the previous anchor, select the room, open the focused event timeline, then enter the main anchored timeline"
        );
    }

    #[test]
    fn close_focused_context_command_routes_to_app_close_focused_context() {
        let source = commands_source();
        let fn_name = concat!("pub async fn close", "_focused_context");
        let command_token = concat!("Close", "FocusedContext");
        let submit_token = "submit_core_command";
        let title_token = "update_qa_window_title_from_state";
        let snapshot_token = "current_snapshot";

        let fn_offset = source
            .find(fn_name)
            .expect("close_focused_context command should exist");
        let rest = &source[fn_offset..];
        let end = rest
            .find("pub async fn paginate_timeline_backwards")
            .expect("next command should exist");
        let close_source = &rest[..end];

        assert!(
            close_source.contains(command_token),
            "close_focused_context should route through AppCommand::CloseFocusedContext"
        );
        assert!(
            close_source.contains(submit_token),
            "close_focused_context should submit the core command"
        );
        assert!(
            close_source.contains(title_token),
            "close_focused_context should refresh the QA title after state changes"
        );
        assert!(
            close_source.contains(snapshot_token),
            "close_focused_context should return the current snapshot"
        );
    }

    #[test]
    fn close_focused_context_command_waits_until_main_timeline_is_live() {
        let source = commands_source();
        let fn_name = concat!("pub async fn close", "_focused_context");
        let command_token = concat!("Close", "FocusedContext");

        let fn_offset = source
            .find(fn_name)
            .expect("close_focused_context command should exist");
        let rest = &source[fn_offset..];
        let end = rest
            .find("pub async fn paginate_timeline_backwards")
            .expect("next command should exist");
        let command_source = &rest[..end];

        let close_offset = command_source
            .find(command_token)
            .expect("close_focused_context should submit the close command");
        let wait_offset = command_source
            .find("wait_for_focused_context_closed")
            .expect("close_focused_context must wait before returning its snapshot");
        let snapshot_offset = command_source
            .find("current_snapshot")
            .expect("close_focused_context should return a snapshot");

        assert!(
            close_offset < wait_offset && wait_offset < snapshot_offset,
            "close_focused_context must return only after focused_context is closed and main_timeline_anchor is cleared"
        );
    }

    #[test]
    fn open_timeline_at_timestamp_command_routes_through_app_command() {
        let command = build_open_timeline_at_timestamp_command(
            fake_request_id(40),
            "!room:example.org".to_owned(),
            1_718_000_000_000,
        );

        match command {
            CoreCommand::App(AppCommand::OpenTimelineAtTimestamp {
                request_id,
                room_id,
                timestamp_ms,
            }) => {
                assert_eq!(request_id, fake_request_id(40));
                assert_eq!(room_id, "!room:example.org");
                assert_eq!(timestamp_ms, 1_718_000_000_000);
                let debug = format!(
                    "{:?}",
                    AppCommand::OpenTimelineAtTimestamp {
                        request_id,
                        room_id,
                        timestamp_ms,
                    }
                );
                assert!(!debug.contains("!room:example.org"), "{debug}");
                assert!(!debug.contains("1718000000000"), "{debug}");
            }
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn observe_timeline_viewport_command_routes_viewport_facts_only() {
        let account_key = AccountKey("@alice:example.org".to_owned());
        let command = build_observe_timeline_viewport_command(
            fake_request_id(41),
            account_key.clone(),
            "!room:example.org".to_owned(),
            Some("$first".to_owned()),
            Some("$last".to_owned()),
            false,
        );
        let debug = format!("{command:?}");
        assert!(!debug.contains("!room:example.org"), "{debug}");
        assert!(!debug.contains("$first"), "{debug}");
        assert!(!debug.contains("$last"), "{debug}");

        match command {
            CoreCommand::Timeline(TimelineCommand::ObserveViewport {
                request_id,
                key,
                observation,
            }) => {
                assert_eq!(request_id, fake_request_id(41));
                assert_eq!(key.account_key, account_key);
                assert_eq!(
                    key.kind,
                    koushi_core::TimelineKind::Room {
                        room_id: "!room:example.org".to_owned()
                    }
                );
                assert_eq!(
                    observation.first_visible_event_id.as_deref(),
                    Some("$first")
                );
                assert_eq!(observation.last_visible_event_id.as_deref(), Some("$last"));
                assert!(!observation.at_bottom);
            }
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn room_management_tauri_commands_wait_for_correlated_core_events() {
        let source = commands_source();

        for (fn_name, event_token) in [
            ("pub async fn load_room_settings", "RoomSettingsLoaded"),
            ("pub async fn update_room_setting", "RoomSettingUpdated"),
            ("pub async fn moderate_room_member", "RoomMemberModerated"),
            (
                "pub async fn update_room_member_role",
                "RoomMemberRoleUpdated",
            ),
        ] {
            let fn_offset = source
                .find(fn_name)
                .unwrap_or_else(|| panic!("{fn_name} command should exist"));
            let rest = &source[fn_offset..];
            let end = rest.find("\n#[tauri::command]").unwrap_or(rest.len());
            let command_source = &rest[..end];

            assert!(
                command_source.contains("wait_for_room_operation"),
                "{fn_name} should wait for the correlated RoomEvent before returning a snapshot"
            );
            assert!(
                command_source.contains(event_token),
                "{fn_name} should wait for {event_token}"
            );
            assert!(
                command_source.contains("update_qa_window_title_from_state"),
                "{fn_name} should refresh the QA title after state changes"
            );
            assert!(
                command_source.contains("current_snapshot"),
                "{fn_name} should return the current snapshot"
            );
        }
    }

    #[test]
    fn build_subscribe_focused_timeline_command_routes_to_focused_timeline_kind() {
        let account_key = AccountKey("@alice:example.org".to_owned());
        let command = build_subscribe_focused_timeline_command(
            fake_request_id(21),
            account_key.clone(),
            "!room:example.org".to_owned(),
            "$event".to_owned(),
        );

        match command {
            CoreCommand::Timeline(TimelineCommand::Subscribe { request_id, key }) => {
                assert_eq!(request_id, fake_request_id(21));
                assert_eq!(key.account_key, account_key);
                assert_eq!(
                    key.kind,
                    koushi_core::TimelineKind::Focused {
                        room_id: "!room:example.org".to_owned(),
                        event_id: "$event".to_owned(),
                    }
                );
            }
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn wait_for_selected_room_observes_state_changed_failures_and_timeout() {
        let source = commands_source();
        let helper_name = concat!("async fn wait_for_selected", "_room");
        let helper_offset = source
            .find(helper_name)
            .expect("selected-room wait helper should exist");
        let rest = &source[helper_offset..];
        let end = rest
            .find("fn snapshot_has_active_room")
            .expect("active-room snapshot helper should follow selected-room wait");
        let helper_source = &rest[..end];

        assert!(helper_source.contains("timeout_at"));
        assert!(helper_source.contains("CoreEvent::StateChanged"));
        assert!(helper_source.contains(concat!("Operation", "Failed")));
        assert!(helper_source.contains(concat!("snapshot_has_active", "_room")));
    }

    #[test]
    fn join_directory_room_waits_for_backend_selected_room() {
        let source = commands_source();
        let fn_name = "pub async fn join_directory_room";
        let fn_offset = source
            .find(fn_name)
            .expect("join_directory_room command should exist");
        let rest = &source[fn_offset..];
        let end = rest
            .find("pub async fn set_space_child")
            .expect("next command should exist");
        let join_source = &rest[..end];
        let joined_offset = join_source
            .find("wait_for_room_joined")
            .expect("directory join should wait for RoomJoined");
        let selected_offset = join_source
            .find("wait_for_selected_room")
            .expect("directory join should wait for selected-room state");

        assert!(
            joined_offset < selected_offset,
            "join should learn the joined room id before waiting for selection"
        );
        assert!(
            join_source.contains("joined_room_id"),
            "joined room id should be carried into selected-room wait"
        );
        assert!(
            join_source.contains("SELECT_ROOM_EVENT_TIMEOUT"),
            "selected-room wait should be bounded"
        );
    }

    #[test]
    fn tauri_command_routes_redact_secret_bearing_values_from_debug() {
        let account_key = AccountKey("@alice:example.org".to_owned());
        let room_id = "!room:example.org".to_owned();
        let login = build_submit_login_command(
            fake_request_id(16),
            LoginRequest {
                homeserver: "https://matrix.example.org".to_owned(),
                username: "alice".to_owned(),
                password: AuthSecret::new("password-123"),
                device_display_name: Some("Laptop".to_owned()),
            },
        );
        let recovery =
            build_submit_recovery_command(fake_request_id(17), AuthSecret::new("recovery-123"));
        let send = build_send_text_command(
            fake_request_id(18),
            account_key.clone(),
            room_id.clone(),
            "desktop-18".to_owned(),
            "sensitive body".to_owned(),
            MentionIntent::default(),
        )
        .expect("send_text should build a command");
        let edit = build_edit_message_command(
            fake_request_id(19),
            account_key,
            room_id,
            "$event".to_owned(),
            "sensitive edit body".to_owned(),
        )
        .expect("edit_message should build a command");
        let upload = build_upload_media_command(
            fake_request_id(21),
            AccountKey("@alice:example.org".to_owned()),
            "!room:example.org".to_owned(),
            "desktop-media-sensitive".to_owned(),
            "secret-filename.pdf".to_owned(),
            "application/pdf".to_owned(),
            b"secret media bytes".to_vec(),
            Some("secret media caption".to_owned()),
            ImageUploadCompressionMode::Never,
            ImageUploadCompressionPolicy::default(),
            None,
            None,
            None,
        )
        .expect("upload_media should build a command");
        let download = build_download_media_command(
            fake_request_id(22),
            AccountKey("@alice:example.org".to_owned()),
            "!room:example.org".to_owned(),
            "$secret-media-event".to_owned(),
        )
        .expect("download_media should build a command");
        let search = build_submit_search_command(
            fake_request_id(20),
            "secret search terms".to_owned(),
            resolve_search_scope_from_active_room(
                SearchScopeKind::CurrentRoom,
                Some("!room:example.org".to_owned()),
            ),
        );
        let room_key_export = build_export_room_keys_command(
            fake_request_id(23),
            "/tmp/private-room-key-export.txt".to_owned(),
            AuthSecret::new("room-key-transfer-phrase"),
        );
        let room_key_import = build_import_room_keys_command(
            fake_request_id(24),
            "/tmp/private-room-key-import.txt".to_owned(),
            AuthSecret::new("room-key-transfer-phrase"),
        );
        let secure_backup_setup = build_bootstrap_secure_backup_command(
            fake_request_id(25),
            Some(AuthSecret::new("backup-setup-phrase")),
            Some("/tmp/private-recovery-artifact.txt".to_owned()),
        );
        let secure_backup_change = build_change_secure_backup_passphrase_command(
            fake_request_id(26),
            AuthSecret::new("old-backup-phrase"),
            AuthSecret::new("new-backup-phrase"),
            Some("/tmp/private-recovery-artifact.txt".to_owned()),
        );

        for (command, secret) in [
            (&login, "password-123"),
            (&recovery, "recovery-123"),
            (&send, "sensitive body"),
            (&edit, "sensitive edit body"),
            (&upload, "secret-filename.pdf"),
            (&upload, "secret media bytes"),
            (&download, "$secret-media-event"),
            (&search, "secret search terms"),
            (&room_key_export, "/tmp/private-room-key-export.txt"),
            (&room_key_export, "room-key-transfer-phrase"),
            (&room_key_import, "/tmp/private-room-key-import.txt"),
            (&room_key_import, "room-key-transfer-phrase"),
            (&secure_backup_setup, "backup-setup-phrase"),
            (&secure_backup_setup, "/tmp/private-recovery-artifact.txt"),
            (&secure_backup_change, "old-backup-phrase"),
            (&secure_backup_change, "new-backup-phrase"),
            (&secure_backup_change, "/tmp/private-recovery-artifact.txt"),
        ] {
            let debug = format!("{command:?}");
            assert!(
                !debug.contains(secret),
                "Debug output leaked a secret: {debug}"
            );
        }
    }

    #[test]
    fn submit_login_request_waits_for_authenticated_session_and_leaves_sync_to_runtime_effects() {
        let source = commands_source();
        let helper_name = concat!("async fn submit_login", "_and_wait_for_authenticated");
        let wait_call_token = concat!("wait_for_logged", "_in_authenticated");
        let logged_in_token = concat!("AccountEvent::", "LoggedIn");
        let start_sync_token = concat!("build_start", "_sync_command");
        let failed_token = concat!("Operation", "Failed");
        let timeout_token = concat!("LOGIN_EVENT", "_TIMEOUT");
        let helper_offset = source
            .find(helper_name)
            .expect("shared login helper should exist");
        let helper_source = &source[helper_offset..];
        let helper_source = helper_source
            .split(concat!("async fn wait_for_logged", "_in_authenticated"))
            .next()
            .expect("login wait helper should follow shared helper");
        let wait_call_offset = helper_source
            .find(wait_call_token)
            .expect("helper should wait for an authenticated session");

        assert!(wait_call_offset > 0);
        assert!(
            !helper_source.contains(start_sync_token),
            "sync startup belongs to AppEffect::StartSync in core runtime, not the Tauri adapter"
        );
        assert!(helper_source.contains(timeout_token));
        let wait_helper_offset = source
            .find(concat!("async fn wait_for_logged", "_in_authenticated"))
            .expect("login wait helper should exist");
        let wait_helper_source = &source[wait_helper_offset..];
        assert!(wait_helper_source.contains(logged_in_token));
        assert!(wait_helper_source.contains(failed_token));
        assert!(wait_helper_source.contains("timeout_at"));
    }

    #[test]
    fn login_wait_treats_recovery_states_as_authenticated_sessions() {
        let mut state = AppState::default();
        assert!(!super::snapshot_has_authenticated_session(&state));

        let info = SessionInfo {
            homeserver: "https://matrix.example.org".to_owned(),
            user_id: "@user:example.org".to_owned(),
            device_id: "DEVICE".to_owned(),
        };

        state.session = SessionState::NeedsRecovery {
            info: info.clone(),
            methods: vec![],
        };
        assert!(super::snapshot_has_authenticated_session(&state));

        state.session = SessionState::Recovering {
            info,
            methods: vec![],
        };
        assert!(super::snapshot_has_authenticated_session(&state));
    }
}
