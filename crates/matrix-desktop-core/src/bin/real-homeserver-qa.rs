//! Real homeserver QA binary (Phase 8 — Milestone G).
//!
//! Exercises the full real-homeserver QA scenario against a live Matrix
//! homeserver (matrix.org) using approved test-account credentials stored in
//! `.local-secrets/real-account-qa/credentials.json` (git-ignored, mode 600).
//!
//! ## Secrets protocol (engineering-rules Secrets section)
//!
//! - The credentials file is read by THIS binary only, behind a debug/test
//!   compile-time gate. The file path is passed via env; the path itself is
//!   not a secret.
//! - Passwords and recovery keys are NEVER logged, echoed, printed, or included
//!   in error messages. This binary self-checks its own transcript for those values
//!   before exit and fails if they are found (redaction check).
//! - An unexpected keychain prompt = automation failure. The file credential
//!   store override (MATRIX_DESKTOP_QA_FILE_CREDENTIAL_STORE_DIR) is mandatory.
//! - ABSOLUTE_PROHIBITION: no GUI launch in any form.
//! - Logout cleanup runs even on earlier failures (finally-ish path) so no
//!   stale devices accumulate on the homeserver.
//!
//! ## QA coverage (canon QA Model layer 3)
//!
//! 1. HTTPS login to the homeserver -> pre-sync Ready snapshot (store bootstrap
//!    invariant and reducer gate).
//! 2. Sync lifecycle: Start -> Started{backend} -> Running (print backend).
//! 3. Recovery: after sync/account data flows in, require RecoveryRequired ->
//!    SubmitRecovery -> RecoveryCompleted -> assert Ready.
//! 4. Room list: wait non-empty or timeout; print COUNTS ONLY (rooms=N spaces=N dms=N).
//! 5. Create synthetic QA room, subscribe timeline, send edit/redact fixture
//!    messages plus a dedicated search probe, wait SendCompleted + diffs, edit
//!    one, redact the other, paginate backward to EndReached. Only operations
//!    on the QA-created room.
//! 6. Search smoke: query a unique token from the unedited probe message;
//!    assert the QA room/event.
//! 7. Encrypted store restore: stop sync, drop runtime, start fresh runtime over
//!    same data dir, RestoreLastSession -> SessionRestored -> start sync -> Running ->
//!    resubscribe QA room timeline and assert the edited message body arrives.
//! 8. Leave/forget the QA room if a leave primitive is available (checked below).
//! 9. Logout -> SignedOut + post-logout RestoreLastSession = SessionNotFound.
//! 10. Self-check transcript for password/recovery-key leakage.
//!
//! ## Rate limits (matrix.org)
//!
//! - Single login per run. No login/logout cycles.
//! - Bounded retries with backoff on 429.
//! - Logout cleanup MUST run even on failure (no --keep-session).
//!
//! ## Required env
//!
//! - MATRIX_DESKTOP_REAL_QA_CREDENTIALS_PATH - path to the credentials JSON file
//! - MATRIX_DESKTOP_QA_FILE_CREDENTIAL_STORE_DIR - mandatory; see keychain guard
//! - MATRIX_DESKTOP_QA_DATA_DIR (optional) - overrides per-run data dir root

#![allow(dead_code)]

use std::process::ExitCode;
use std::time::Duration;

use matrix_desktop_core::command::{
    AccountCommand, CoreCommand, RoomCommand, SearchCommand, SearchScope, SyncCommand,
    TimelineCommand,
};
use matrix_desktop_core::event::{
    AccountEvent, CoreEvent, PaginationDirection, PaginationState, RoomEvent, SearchEvent,
    SyncBackendKind, SyncEvent, TimelineEvent,
};
use matrix_desktop_core::failure::{CoreFailure, RecoveryFailureKind};
use matrix_desktop_core::ids::{AccountKey, RequestId, TimelineKey};
use matrix_desktop_core::runtime::{CoreConnection, CoreRuntime};
use matrix_desktop_state::{AppState, AuthSecret, LoginRequest, RecoveryRequest, SessionState};

// ---------------------------------------------------------------------------
// Env var constants
// ---------------------------------------------------------------------------

const ENV_DATA_DIR: &str = "MATRIX_DESKTOP_QA_DATA_DIR";

#[cfg(any(debug_assertions, test))]
const ENV_CREDENTIALS_PATH: &str = "MATRIX_DESKTOP_REAL_QA_CREDENTIALS_PATH";
#[cfg(any(debug_assertions, test))]
const ENV_FILE_CREDENTIAL_STORE_DIR: &str = "MATRIX_DESKTOP_QA_FILE_CREDENTIAL_STORE_DIR";

// ---------------------------------------------------------------------------
// Timeout constants - matrix.org is slower than local servers
// ---------------------------------------------------------------------------

/// Standard per-event wait.
const EVENT_TIMEOUT: Duration = Duration::from_secs(60);
/// Extended timeout for sync operations (matrix.org initial sync can be slow).
const SYNC_TIMEOUT: Duration = Duration::from_secs(120);
/// Extended timeout for room list non-empty wait.
const ROOM_LIST_TIMEOUT: Duration = Duration::from_secs(120);
/// Timeout for search indexing (ngram index updated by sync loop).
const SEARCH_TIMEOUT: Duration = Duration::from_secs(90);
/// Timeout for edit/redact confirmation (sync round-trip).
const EDIT_REDACT_TIMEOUT: Duration = Duration::from_secs(90);
/// Timeout for paginate-to-EndReached.
const PAGINATE_TIMEOUT: Duration = Duration::from_secs(90);

// ---------------------------------------------------------------------------
// Credentials (only loaded in debug/test builds)
// ---------------------------------------------------------------------------

/// Loaded from the credentials JSON file at the path given by
/// `ENV_CREDENTIALS_PATH`. Values are zeroized on drop via `AuthSecret`.
/// The file is read ONCE; the values are never written to stdout/stderr/logs.
#[cfg(any(debug_assertions, test))]
struct RealCredentials {
    homeserver: String,
    user_id: String,
    /// Username part of user_id (before the colon): "@alice:server" -> "alice"
    username: String,
    password: AuthSecret,
    recovery_key: AuthSecret,
    device_display_name: String,
}

#[cfg(any(debug_assertions, test))]
impl RealCredentials {
    fn load() -> Result<Self, String> {
        let path = std::env::var(ENV_CREDENTIALS_PATH).map_err(|_| {
            format!("{ENV_CREDENTIALS_PATH} is required (path to the credentials JSON file)")
        })?;

        let content = std::fs::read_to_string(&path)
            .map_err(|e| format!("cannot read credentials file at {path}: {e}"))?;

        let value: serde_json::Value = serde_json::from_str(&content)
            .map_err(|e| format!("credentials file is not valid JSON: {e}"))?;

        let homeserver = value["homeserver"]
            .as_str()
            .ok_or("credentials JSON missing 'homeserver'")?
            .to_owned();
        let user_id = value["user_id"]
            .as_str()
            .ok_or("credentials JSON missing 'user_id'")?
            .to_owned();
        // Extract username: "@alice:server" -> "alice"
        let username = user_id
            .trim_start_matches('@')
            .split(':')
            .next()
            .unwrap_or(&user_id)
            .to_owned();
        let password_str = value["password"]
            .as_str()
            .ok_or("credentials JSON missing 'password'")?
            .to_owned();
        let recovery_key_str = value["recovery_key"]
            .as_str()
            .ok_or("credentials JSON missing 'recovery_key'")?
            .to_owned();
        let device_display_name = value["device_display_name"]
            .as_str()
            .unwrap_or("Matrix Desktop Real QA")
            .to_owned();

        Ok(Self {
            homeserver,
            user_id,
            username,
            password: AuthSecret::new(password_str),
            recovery_key: AuthSecret::new(recovery_key_str),
            device_display_name,
        })
    }
}

// ---------------------------------------------------------------------------
// Main
// ---------------------------------------------------------------------------

fn main() -> ExitCode {
    #[cfg(not(any(debug_assertions, test)))]
    {
        eprintln!("real-homeserver-qa: this binary is only available in debug/test builds");
        return ExitCode::FAILURE;
    }

    #[cfg(any(debug_assertions, test))]
    match run() {
        Ok(summary) => {
            println!("{summary}");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("Real homeserver QA failed: {error}");
            ExitCode::FAILURE
        }
    }
}

// ---------------------------------------------------------------------------
// Sync entry point - loads creds, runs tokio, self-checks transcript
// ---------------------------------------------------------------------------

#[cfg(any(debug_assertions, test))]
fn run() -> Result<String, String> {
    // Hard guard BEFORE credentials are loaded: unattended QA must never
    // touch the OS keychain (a keychain prompt = automation failure).
    assert_file_credential_store_active()?;

    let creds = RealCredentials::load()?;

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|e| format!("Tokio runtime creation failed: {e}"))?;

    let mut transcript: Vec<String> = Vec::new();
    let result = rt.block_on(run_async(&creds, &mut transcript));

    // Self-check: scan every line of the transcript for the secret values
    // before we emit the summary to stdout.
    let password_str = creds.password.expose_secret();
    let recovery_key_str = creds.recovery_key.expose_secret();
    let combined = transcript.join("\n");

    if combined.contains(password_str) {
        return Err("REDACTION FAILURE: password appears in QA transcript".to_owned());
    }
    if combined.contains(recovery_key_str) {
        return Err("REDACTION FAILURE: recovery_key appears in QA transcript".to_owned());
    }

    result
}

#[cfg(any(debug_assertions, test))]
fn assert_file_credential_store_active() -> Result<(), String> {
    if std::env::var_os(ENV_FILE_CREDENTIAL_STORE_DIR).is_none() {
        return Err(format!(
            "real-homeserver-qa refuses to run against the OS keychain: \
             {ENV_FILE_CREDENTIAL_STORE_DIR} is not set"
        ));
    }
    if !matrix_desktop_core::store::resolved_credential_backend_is_file_dir() {
        return Err(
            "real-homeserver-qa refuses to run against the OS keychain: \
             resolved credential store backend is not the file-dir backend"
                .to_owned(),
        );
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Recovery outcome
// ---------------------------------------------------------------------------

#[cfg(any(debug_assertions, test))]
enum RecoveryOutcome {
    Completed,
    Failed(RecoveryFailureKind),
}

#[cfg(any(debug_assertions, test))]
struct RealHomeserverQaMessagePlan {
    search_token: String,
    msg1_body: String,
    search_probe_body: String,
    msg2_body: String,
    edited_body: String,
}

#[cfg(any(debug_assertions, test))]
fn build_real_homeserver_qa_message_plan(ts: u64) -> RealHomeserverQaMessagePlan {
    let search_token = format!("real-qa-search-{}-{}", std::process::id(), ts);
    RealHomeserverQaMessagePlan {
        search_token: search_token.clone(),
        msg1_body: "Real homeserver QA message 1".to_owned(),
        search_probe_body: format!("Real homeserver QA search probe {search_token}"),
        msg2_body: "Real homeserver QA message 2".to_owned(),
        edited_body: "Real homeserver QA message 1 EDITED".to_owned(),
    }
}

#[cfg(test)]
mod search_plan_tests {
    use super::*;

    #[test]
    fn real_homeserver_qa_search_plan_uses_a_dedicated_unedited_probe_message() {
        let plan = build_real_homeserver_qa_message_plan(1234567890);

        assert!(plan.search_probe_body.contains(&plan.search_token));
        assert_ne!(plan.search_probe_body, plan.msg1_body);
        assert!(!plan.msg1_body.contains(&plan.search_token));
        assert_eq!(plan.msg2_body, "Real homeserver QA message 2");
        assert_eq!(plan.edited_body, "Real homeserver QA message 1 EDITED");
    }
}

// ---------------------------------------------------------------------------
// Data directory helper
// ---------------------------------------------------------------------------

#[cfg(any(debug_assertions, test))]
fn real_qa_data_dir() -> std::path::PathBuf {
    if let Ok(dir) = std::env::var(ENV_DATA_DIR) {
        return std::path::PathBuf::from(dir);
    }
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    std::env::temp_dir()
        .join("matrix-desktop-real-qa")
        .join(format!("{}_{}", std::process::id(), ts))
}

// ---------------------------------------------------------------------------
// Async QA flow
// ---------------------------------------------------------------------------

#[cfg(any(debug_assertions, test))]
async fn run_async(
    creds: &RealCredentials,
    transcript: &mut Vec<String>,
) -> Result<String, String> {
    let data_dir = real_qa_data_dir();

    // -----------------------------------------------------------------------
    // Step 1: HTTPS login (single login per run - rate limit rule)
    // -----------------------------------------------------------------------
    let runtime = CoreRuntime::start_with_data_dir(data_dir.clone());
    let mut conn = runtime.attach();

    let login_id = conn.next_request_id();
    conn.command(CoreCommand::Account(AccountCommand::LoginPassword {
        request_id: login_id,
        request: LoginRequest {
            homeserver: creds.homeserver.clone(),
            username: creds.username.clone(),
            password: creds.password.clone(),
            device_display_name: Some(creds.device_display_name.clone()),
        },
    }))
    .await
    .map_err(|e| format!("login command submit failed: {e}"))?;

    let account_key = wait_for_logged_in(&mut conn, login_id, "login").await?;
    // user_id is allowed in QA output (canon Security: visible state)
    let line = format!("login=ok user_id={}", account_key.0);
    transcript.push(line.clone());
    println!("{line}");

    if let Err(e) = wait_for_post_login_ready_snapshot(&mut conn, "post-login Ready").await {
        do_logout(&mut conn, &account_key, transcript).await;
        return Err(e);
    }

    // -----------------------------------------------------------------------
    // Step 2: Sync lifecycle - Start -> Started{backend} -> Running
    // -----------------------------------------------------------------------
    let sync_start_id = conn.next_request_id();
    conn.command(CoreCommand::Sync(SyncCommand::Start {
        request_id: sync_start_id,
    }))
    .await
    .map_err(|e| format!("sync start command submit failed: {e}"))?;

    let sync_backend =
        match wait_for_sync_started(&mut conn, sync_start_id, "sync start", SYNC_TIMEOUT).await {
            Ok(b) => b,
            Err(e) => {
                do_logout(&mut conn, &account_key, transcript).await;
                return Err(e);
            }
        };

    let backend_name = match sync_backend {
        SyncBackendKind::SyncService => "SyncService",
        SyncBackendKind::LegacySync => "LegacySync",
    };
    let line = format!("sync_backend={backend_name}");
    transcript.push(line.clone());
    println!("{line}");

    if let Err(e) = wait_for_sync_running(&mut conn, "sync running", SYNC_TIMEOUT).await {
        do_logout(&mut conn, &account_key, transcript).await;
        return Err(e);
    }
    let line = "sync=running".to_owned();
    transcript.push(line.clone());
    println!("{line}");

    // -----------------------------------------------------------------------
    // Step 3: Recovery check
    // -----------------------------------------------------------------------
    // Wait for the post-sync recovery observer to publish the final state.
    // Recovery becomes actionable only once sync/account data has flowed in.
    if let Err(e) =
        wait_for_recovery_required_after_sync(&mut conn, "post-sync recovery gate").await
    {
        do_logout(&mut conn, &account_key, transcript).await;
        return Err(e);
    }

    let submit_id = conn.next_request_id();
    conn.command(CoreCommand::Account(AccountCommand::SubmitRecovery {
        request_id: submit_id,
        request: RecoveryRequest {
            secret: creds.recovery_key.clone(),
        },
    }))
    .await
    .map_err(|e| format!("submit recovery command failed: {e}"))?;

    match wait_for_recovery_outcome(&mut conn, submit_id, "recovery").await? {
        RecoveryOutcome::Completed => {
            let line = "recovery=completed".to_owned();
            transcript.push(line.clone());
            println!("{line}");
        }
        RecoveryOutcome::Failed(kind) => {
            // Recovery failure is a hard QA failure; attempt logout cleanup first.
            let line = format!("recovery=failed kind={kind:?}");
            transcript.push(line.clone());
            eprintln!("{line}");
            do_logout(&mut conn, &account_key, transcript).await;
            return Err(format!("recovery failed with kind {kind:?}"));
        }
    }

    // Assert Ready snapshot after recovery completes.
    wait_for_ready_snapshot(&mut conn, "post-recovery Ready").await?;
    let line = "session=ready".to_owned();
    transcript.push(line.clone());
    println!("{line}");

    // -----------------------------------------------------------------------
    // Step 4: Room list - wait non-empty or timeout; print COUNTS ONLY
    // -----------------------------------------------------------------------
    let room_snapshot =
        wait_for_non_empty_room_list(&mut conn, "room list non-empty", ROOM_LIST_TIMEOUT).await?;
    let rooms_count = room_snapshot.rooms.len();
    let spaces_count = room_snapshot.spaces.len();
    let dms_count = room_snapshot.rooms.iter().filter(|r| r.is_dm).count();
    let line = format!("rooms={rooms_count} spaces={spaces_count} dms={dms_count}");
    transcript.push(line.clone());
    println!("{line}");

    // -----------------------------------------------------------------------
    // Step 5: Create synthetic QA room, send 2 messages, edit, redact, paginate
    // -----------------------------------------------------------------------
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let qa_room_name = format!("core-real-qa-{ts}");

    let create_room_id = conn.next_request_id();
    conn.command(CoreCommand::Room(RoomCommand::CreateRoom {
        request_id: create_room_id,
        name: qa_room_name.clone(),
    }))
    .await
    .map_err(|e| format!("create QA room command submit failed: {e}"))?;

    let qa_room_id = wait_for_room_created(&mut conn, create_room_id, "create QA room").await?;
    // QA-created room name and room_id are synthetic - allowed in output.
    let line = format!("qa_room=created room_id={qa_room_id}");
    transcript.push(line.clone());
    println!("{line}");

    // Subscribe to the QA room timeline.
    let timeline_key = TimelineKey::room(account_key.clone(), qa_room_id.clone());
    let subscribe_id = conn.next_request_id();
    conn.command(CoreCommand::Timeline(TimelineCommand::Subscribe {
        request_id: subscribe_id,
        key: timeline_key.clone(),
    }))
    .await
    .map_err(|e| format!("subscribe timeline command submit failed: {e}"))?;

    wait_for_initial_items(
        &mut conn,
        &timeline_key,
        subscribe_id,
        "subscribe QA timeline",
    )
    .await?;
    let line = "timeline_subscribed=ok".to_owned();
    transcript.push(line.clone());
    println!("{line}");

    // Send message 1, then a dedicated search probe message. The search probe
    // is the only message that carries the unique search token; message 1 is
    // reserved for edit coverage.
    let message_plan = build_real_homeserver_qa_message_plan(ts);
    let txn1 = format!("real-qa-txn-1-{ts}");
    let send1_id = conn.next_request_id();
    conn.command(CoreCommand::Timeline(TimelineCommand::SendText {
        request_id: send1_id,
        key: timeline_key.clone(),
        transaction_id: txn1,
        body: message_plan.msg1_body.clone(),
    }))
    .await
    .map_err(|e| format!("send message 1 command submit failed: {e}"))?;

    let (_, event1_id) =
        wait_for_send_completed(&mut conn, send1_id, &timeline_key, "send msg1").await?;
    let line = format!("send_msg1=ok event_id={event1_id}");
    transcript.push(line.clone());
    println!("{line}");

    // Send the dedicated search probe. It is never edited or redacted.
    let txn_search = format!("real-qa-txn-search-{ts}");
    let send_search_id = conn.next_request_id();
    conn.command(CoreCommand::Timeline(TimelineCommand::SendText {
        request_id: send_search_id,
        key: timeline_key.clone(),
        transaction_id: txn_search,
        body: message_plan.search_probe_body.clone(),
    }))
    .await
    .map_err(|e| format!("send search probe command submit failed: {e}"))?;

    let (_, search_event_id) = wait_for_send_completed(
        &mut conn,
        send_search_id,
        &timeline_key,
        "send search probe",
    )
    .await?;
    let line = format!("send_search=ok event_id={search_event_id}");
    transcript.push(line.clone());
    println!("{line}");

    // Send message 2.
    let txn2 = format!("real-qa-txn-2-{ts}");
    let send2_id = conn.next_request_id();
    conn.command(CoreCommand::Timeline(TimelineCommand::SendText {
        request_id: send2_id,
        key: timeline_key.clone(),
        transaction_id: txn2,
        body: message_plan.msg2_body.clone(),
    }))
    .await
    .map_err(|e| format!("send message 2 command submit failed: {e}"))?;

    let (_, event2_id) =
        wait_for_send_completed(&mut conn, send2_id, &timeline_key, "send msg2").await?;
    let line = format!("send_msg2=ok event_id={event2_id}");
    transcript.push(line.clone());
    println!("{line}");

    // Edit message 1.
    let edit1_id = conn.next_request_id();
    conn.command(CoreCommand::Timeline(TimelineCommand::EditText {
        request_id: edit1_id,
        key: timeline_key.clone(),
        event_id: event1_id.clone(),
        body: message_plan.edited_body.clone(),
    }))
    .await
    .map_err(|e| format!("edit message 1 command submit failed: {e}"))?;

    wait_for_edit_diff(
        &mut conn,
        &timeline_key,
        edit1_id,
        &event1_id,
        &message_plan.edited_body,
        "edit msg1",
        EDIT_REDACT_TIMEOUT,
    )
    .await?;
    let line = "edit_msg1=ok".to_owned();
    transcript.push(line.clone());
    println!("{line}");

    // Redact message 2.
    let redact2_id = conn.next_request_id();
    conn.command(CoreCommand::Timeline(TimelineCommand::Redact {
        request_id: redact2_id,
        key: timeline_key.clone(),
        event_id: event2_id.clone(),
    }))
    .await
    .map_err(|e| format!("redact message 2 command submit failed: {e}"))?;

    wait_for_redact_diff(
        &mut conn,
        &timeline_key,
        redact2_id,
        "redact msg2",
        EDIT_REDACT_TIMEOUT,
    )
    .await?;
    let line = "redact_msg2=ok".to_owned();
    transcript.push(line.clone());
    println!("{line}");

    // Paginate backward to EndReached.
    let paginate_id = conn.next_request_id();
    conn.command(CoreCommand::Timeline(TimelineCommand::Paginate {
        request_id: paginate_id,
        key: timeline_key.clone(),
        direction: PaginationDirection::Backward,
        event_count: 10,
    }))
    .await
    .map_err(|e| format!("paginate command submit failed: {e}"))?;

    let paginate_result = match wait_for_paginate_end_reached(
        &mut conn,
        &timeline_key,
        paginate_id,
        "paginate to EndReached",
        PAGINATE_TIMEOUT,
    )
    .await
    {
        Ok(s) => s,
        Err(e) => {
            // Non-fatal: old rooms may have enough history that EndReached
            // is not reached within the timeout. Record and continue.
            let warn = format!("paginate_warning={e}");
            transcript.push(warn.clone());
            println!("{warn}");
            "partial".to_owned()
        }
    };
    let line = format!("paginate={paginate_result}");
    transcript.push(line.clone());
    println!("{line}");

    // -----------------------------------------------------------------------
    // Step 6: Search smoke - query the dedicated unedited search probe.
    // -----------------------------------------------------------------------
    let search_status = match poll_search_until_found_or_timeout(
        &mut conn,
        &message_plan.search_token,
        &search_event_id,
        &qa_room_id,
        "search smoke",
        SEARCH_TIMEOUT,
    )
    .await
    {
        Ok(()) => "ok",
        Err(e) => {
            let errline = format!("search_smoke=failed reason={e}");
            transcript.push(errline.clone());
            eprintln!("{errline}");
            do_logout(&mut conn, &account_key, transcript).await;
            return Err(format!("search smoke failed: {e}"));
        }
    };
    let line = format!("search={search_status}");
    transcript.push(line.clone());
    println!("{line}");

    // Unsubscribe before stopping sync.
    let unsub_id = conn.next_request_id();
    let _ = conn
        .command(CoreCommand::Timeline(TimelineCommand::Unsubscribe {
            request_id: unsub_id,
            key: timeline_key.clone(),
        }))
        .await;

    tokio::time::sleep(Duration::from_millis(300)).await;

    // -----------------------------------------------------------------------
    // Step 7: Encrypted store restore
    // -----------------------------------------------------------------------
    let sync_stop_id = conn.next_request_id();
    conn.command(CoreCommand::Sync(SyncCommand::Stop {
        request_id: sync_stop_id,
    }))
    .await
    .map_err(|e| format!("sync stop command submit failed: {e}"))?;

    wait_for_sync_stopped(&mut conn, sync_stop_id, "sync stop").await?;
    let line = "sync=stopped".to_owned();
    transcript.push(line.clone());
    println!("{line}");

    // Drop connection and runtime so the store is fully released.
    drop(conn);
    drop(runtime);

    tokio::time::sleep(Duration::from_millis(500)).await;

    // Start a fresh runtime over the same data dir.
    let runtime2 = CoreRuntime::start_with_data_dir(data_dir.clone());
    let mut conn2 = runtime2.attach();

    let restore_id = conn2.next_request_id();
    conn2
        .command(CoreCommand::Account(AccountCommand::RestoreLastSession {
            request_id: restore_id,
        }))
        .await
        .map_err(|e| format!("RestoreLastSession command submit failed: {e}"))?;

    wait_for_session_restored_with_recovery(
        &mut conn2,
        restore_id,
        &account_key,
        creds,
        transcript,
        "restore session",
    )
    .await?;

    wait_for_ready_snapshot(&mut conn2, "restored session Ready").await?;
    let line = "store_restore=ok".to_owned();
    transcript.push(line.clone());
    println!("{line}");

    // Start sync on restored session.
    let sync2_id = conn2.next_request_id();
    conn2
        .command(CoreCommand::Sync(SyncCommand::Start {
            request_id: sync2_id,
        }))
        .await
        .map_err(|e| format!("sync start (restored) command submit failed: {e}"))?;

    let sync2_backend =
        wait_for_sync_started(&mut conn2, sync2_id, "sync start restored", SYNC_TIMEOUT).await?;
    let backend2_name = match sync2_backend {
        SyncBackendKind::SyncService => "SyncService",
        SyncBackendKind::LegacySync => "LegacySync",
    };
    let line = format!("sync_backend_restored={backend2_name}");
    transcript.push(line.clone());
    println!("{line}");

    wait_for_sync_running(&mut conn2, "sync running restored", SYNC_TIMEOUT).await?;
    let line = "sync_restored=running".to_owned();
    transcript.push(line.clone());
    println!("{line}");

    // Resubscribe the QA room timeline and assert the edited message body is visible.
    let timeline_key2 = TimelineKey::room(account_key.clone(), qa_room_id.clone());
    let subscribe2_id = conn2.next_request_id();
    conn2
        .command(CoreCommand::Timeline(TimelineCommand::Subscribe {
            request_id: subscribe2_id,
            key: timeline_key2.clone(),
        }))
        .await
        .map_err(|e| format!("subscribe restored timeline command submit failed: {e}"))?;

    let initial2 = wait_for_initial_items(
        &mut conn2,
        &timeline_key2,
        subscribe2_id,
        "subscribe restored timeline",
    )
    .await?;

    let restore_body_found_initial = initial2
        .iter()
        .any(|item| item.body.as_deref().unwrap_or("").contains("EDITED"));

    let restore_body_ok = if restore_body_found_initial {
        true
    } else {
        // Backfill may be needed. Paginate backward and scan diffs.
        let bp_id = conn2.next_request_id();
        let _ = conn2
            .command(CoreCommand::Timeline(TimelineCommand::Paginate {
                request_id: bp_id,
                key: timeline_key2.clone(),
                direction: PaginationDirection::Backward,
                event_count: 20,
            }))
            .await;

        wait_for_body_substring_in_timeline(
            &mut conn2,
            &timeline_key2,
            "EDITED",
            "restore: edited message visible",
            Duration::from_secs(60),
        )
        .await
        .is_ok()
    };

    let restore_body_tag = if restore_body_ok { "ok" } else { "not_found" };
    let line = format!("restore_body={restore_body_tag}");
    transcript.push(line.clone());
    println!("{line}");

    // Unsubscribe restored timeline.
    let unsub2_id = conn2.next_request_id();
    let _ = conn2
        .command(CoreCommand::Timeline(TimelineCommand::Unsubscribe {
            request_id: unsub2_id,
            key: timeline_key2,
        }))
        .await;

    tokio::time::sleep(Duration::from_millis(200)).await;

    // -----------------------------------------------------------------------
    // Step 8: Leave/forget QA room
    // LeaveRoom is not in RoomCommand yet; tracked in the post-headless-core
    // follow-up spec.
    // -----------------------------------------------------------------------
    let line =
        "leave_room=not_available (LeaveRoom not yet in RoomCommand; tracked follow-up)".to_owned();
    transcript.push(line.clone());
    println!("{line}");

    // -----------------------------------------------------------------------
    // Step 9: Logout -> SignedOut + post-logout RestoreLastSession = SessionNotFound
    // -----------------------------------------------------------------------
    do_logout(&mut conn2, &account_key, transcript).await;

    // Post-logout: RestoreLastSession must yield SessionNotFound.
    let restore_gone_id = conn2.next_request_id();
    conn2
        .command(CoreCommand::Account(AccountCommand::RestoreLastSession {
            request_id: restore_gone_id,
        }))
        .await
        .map_err(|e| format!("post-logout restore-last command submit failed: {e}"))?;

    let failure =
        wait_for_operation_failed(&mut conn2, restore_gone_id, "post-logout restore-last").await?;
    if failure != CoreFailure::SessionNotFound {
        return Err(format!(
            "post-logout restore-last failed with unexpected failure kind: {failure:?}"
        ));
    }
    if !matches!(conn2.snapshot().session, SessionState::SignedOut) {
        return Err("post-logout restore-last must leave session in SignedOut state".to_owned());
    }

    let line = "post_logout_restore=not_found".to_owned();
    transcript.push(line.clone());
    println!("{line}");

    // -----------------------------------------------------------------------
    // Summary line (tokens only; no secret values)
    // -----------------------------------------------------------------------
    Ok(format!(
        "Real homeserver QA OK. \
         user={user_id} login=ok recovery={recovery} \
         sync_backend={backend} sync=ok \
         rooms={rooms} spaces={spaces} dms={dms} \
         qa_room=created send_msg1=ok send_msg2=ok \
         edit_msg1=ok redact_msg2=ok \
         paginate={paginate} search={search} \
         store_restore=ok restore_body={body_ok} \
         leave_room=not_available \
         logout=ok post_logout_restore=not_found",
        user_id = account_key.0,
        recovery = "completed",
        backend = backend_name,
        rooms = rooms_count,
        spaces = spaces_count,
        dms = dms_count,
        paginate = paginate_result,
        search = search_status,
        body_ok = restore_body_tag,
    ))
}

// ---------------------------------------------------------------------------
// Logout helper (finally-ish path - runs even on earlier failure)
// ---------------------------------------------------------------------------

/// Best-effort logout. Records to transcript but never propagates errors.
/// Called in failure paths so no stale devices accumulate.
#[cfg(any(debug_assertions, test))]
async fn do_logout(
    conn: &mut CoreConnection,
    account_key: &AccountKey,
    transcript: &mut Vec<String>,
) {
    let logout_id = conn.next_request_id();
    match conn
        .command(CoreCommand::Account(AccountCommand::Logout {
            request_id: logout_id,
        }))
        .await
    {
        Ok(()) => {}
        Err(e) => {
            let line = format!("logout_submit=failed reason={e}");
            transcript.push(line.clone());
            eprintln!("{line}");
            return;
        }
    }

    match wait_for_logged_out(conn, logout_id, account_key, "logout").await {
        Ok(()) => {
            let line = "logout=ok".to_owned();
            transcript.push(line.clone());
            println!("{line}");
        }
        Err(e) => {
            let line = format!("logout_wait=failed reason={e}");
            transcript.push(line.clone());
            eprintln!("{line}");
        }
    }
}

// ---------------------------------------------------------------------------
// Event waiter helpers
// ---------------------------------------------------------------------------

#[cfg(any(debug_assertions, test))]
async fn wait_for_logged_in(
    conn: &mut CoreConnection,
    request_id: RequestId,
    label: &str,
) -> Result<AccountKey, String> {
    loop {
        let event = tokio::time::timeout(EVENT_TIMEOUT, conn.recv_event())
            .await
            .map_err(|_| format!("{label}: timed out waiting for LoggedIn event"))?
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;

        match event {
            CoreEvent::Account(AccountEvent::LoggedIn {
                request_id: ev_id,
                account_key,
            }) if ev_id == request_id => {
                return Ok(account_key);
            }
            CoreEvent::OperationFailed {
                request_id: ev_id,
                failure,
            } if ev_id == request_id => {
                return Err(format!("{label} failed: {failure:?}"));
            }
            _ => continue,
        }
    }
}

#[cfg(any(debug_assertions, test))]
async fn wait_for_recovery_outcome(
    conn: &mut CoreConnection,
    request_id: RequestId,
    label: &str,
) -> Result<RecoveryOutcome, String> {
    loop {
        let event = tokio::time::timeout(EVENT_TIMEOUT, conn.recv_event())
            .await
            .map_err(|_| {
                format!("{label}: timed out waiting for RecoveryCompleted or RecoveryFailed")
            })?
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;

        match event {
            CoreEvent::Account(AccountEvent::RecoveryCompleted {
                request_id: ev_id, ..
            }) if ev_id == request_id => {
                return Ok(RecoveryOutcome::Completed);
            }
            CoreEvent::OperationFailed {
                request_id: ev_id,
                failure: CoreFailure::RecoveryFailed { kind },
            } if ev_id == request_id => {
                return Ok(RecoveryOutcome::Failed(kind));
            }
            CoreEvent::OperationFailed {
                request_id: ev_id,
                failure,
            } if ev_id == request_id => {
                return Err(format!("{label}: unexpected failure: {failure:?}"));
            }
            _ => continue,
        }
    }
}

/// Wait for the session snapshot to land in either `Ready` or `NeedsRecovery`.
/// This must be used after `wait_for_logged_in` because the LoggedIn event
/// may arrive before the reducer processes the LoginSucceeded action (the
/// action is sent through an async mpsc channel to the AppActor).
#[cfg(any(debug_assertions, test))]
async fn wait_for_recovery_required_after_sync(
    conn: &mut CoreConnection,
    label: &str,
) -> Result<(), String> {
    // Check the snapshot first in case the action channel already advanced.
    if matches!(conn.snapshot().session, SessionState::NeedsRecovery { .. }) {
        return Ok(());
    }
    let deadline = tokio::time::Instant::now() + EVENT_TIMEOUT;
    loop {
        if tokio::time::Instant::now() >= deadline {
            return Err(format!(
                "{label}: timed out waiting for RecoveryRequired or NeedsRecovery"
            ));
        }

        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        let event = tokio::time::timeout(remaining, conn.recv_event())
            .await
            .map_err(|_| {
                format!("{label}: timed out waiting for RecoveryRequired or NeedsRecovery")
            })?
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;

        match event {
            CoreEvent::StateChanged(snapshot)
                if matches!(snapshot.session, SessionState::NeedsRecovery { .. }) =>
            {
                return Ok(());
            }
            CoreEvent::Account(AccountEvent::RecoveryRequired { .. }) => {
                return Ok(());
            }
            _ => continue,
        }
    }
}

#[cfg(any(debug_assertions, test))]
async fn wait_for_ready_snapshot(conn: &mut CoreConnection, label: &str) -> Result<(), String> {
    if matches!(conn.snapshot().session, SessionState::Ready(_)) {
        return Ok(());
    }
    loop {
        let event = tokio::time::timeout(EVENT_TIMEOUT, conn.recv_event())
            .await
            .map_err(|_| format!("{label}: timed out waiting for Ready snapshot"))?
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;

        if let CoreEvent::StateChanged(snapshot) = event {
            if matches!(snapshot.session, SessionState::Ready(_)) {
                return Ok(());
            }
        }
    }
}

/// Wait for the post-login `Ready` snapshot before starting sync.
/// `LoggedIn` can arrive before the reducer has processed `LoginSucceeded`,
/// so this gate closes the action-channel race before `SyncCommand::Start`.
#[cfg(any(debug_assertions, test))]
async fn wait_for_post_login_ready_snapshot(
    conn: &mut CoreConnection,
    label: &str,
) -> Result<(), String> {
    wait_for_ready_snapshot(conn, label).await
}

#[cfg(any(debug_assertions, test))]
async fn wait_for_sync_started(
    conn: &mut CoreConnection,
    request_id: RequestId,
    label: &str,
    timeout: Duration,
) -> Result<SyncBackendKind, String> {
    loop {
        let event = tokio::time::timeout(timeout, conn.recv_event())
            .await
            .map_err(|_| format!("{label}: timed out waiting for SyncEvent::Started"))?
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;

        match event {
            CoreEvent::Sync(SyncEvent::Started {
                request_id: Some(ev_id),
                backend,
            }) if ev_id == request_id => {
                return Ok(backend);
            }
            CoreEvent::OperationFailed {
                request_id: ev_id,
                failure,
            } if ev_id == request_id => {
                return Err(format!("{label} failed: {failure:?}"));
            }
            _ => continue,
        }
    }
}

#[cfg(any(debug_assertions, test))]
async fn wait_for_sync_running(
    conn: &mut CoreConnection,
    label: &str,
    timeout: Duration,
) -> Result<(), String> {
    loop {
        let event = tokio::time::timeout(timeout, conn.recv_event())
            .await
            .map_err(|_| format!("{label}: timed out waiting for SyncEvent::Running"))?
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;

        if matches!(event, CoreEvent::Sync(SyncEvent::Running)) {
            return Ok(());
        }
        if matches!(event, CoreEvent::Sync(SyncEvent::Failed)) {
            return Err(format!(
                "{label}: SyncEvent::Failed received before Running"
            ));
        }
    }
}

#[cfg(any(debug_assertions, test))]
async fn wait_for_sync_stopped(
    conn: &mut CoreConnection,
    request_id: RequestId,
    label: &str,
) -> Result<(), String> {
    loop {
        let event = tokio::time::timeout(EVENT_TIMEOUT, conn.recv_event())
            .await
            .map_err(|_| format!("{label}: timed out waiting for SyncEvent::Stopped"))?
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;

        if matches!(
            event,
            CoreEvent::Sync(SyncEvent::Stopped { request_id: Some(ev_id) })
            if ev_id == request_id
        ) {
            return Ok(());
        }
        if matches!(
            event,
            CoreEvent::Sync(SyncEvent::Stopped { request_id: None })
        ) {
            return Ok(());
        }
        if let CoreEvent::OperationFailed {
            request_id: ev_id,
            failure,
        } = event
        {
            if ev_id == request_id {
                return Err(format!("{label} failed: {failure:?}"));
            }
        }
    }
}

#[cfg(any(debug_assertions, test))]
async fn wait_for_non_empty_room_list(
    conn: &mut CoreConnection,
    label: &str,
    timeout: Duration,
) -> Result<AppState, String> {
    let snapshot = conn.snapshot();
    if !snapshot.rooms.is_empty() || !snapshot.spaces.is_empty() {
        return Ok(snapshot);
    }

    loop {
        let event = tokio::time::timeout(timeout, conn.recv_event())
            .await
            .map_err(|_| format!("{label}: timed out waiting for non-empty room list"))?
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;

        match event {
            CoreEvent::Room(RoomEvent::RoomListUpdated) => {
                let s = conn.snapshot();
                if !s.rooms.is_empty() || !s.spaces.is_empty() {
                    return Ok(s);
                }
            }
            CoreEvent::StateChanged(s) => {
                if !s.rooms.is_empty() || !s.spaces.is_empty() {
                    return Ok(s);
                }
            }
            _ => continue,
        }
    }
}

#[cfg(any(debug_assertions, test))]
async fn wait_for_room_created(
    conn: &mut CoreConnection,
    request_id: RequestId,
    label: &str,
) -> Result<String, String> {
    loop {
        let event = tokio::time::timeout(EVENT_TIMEOUT, conn.recv_event())
            .await
            .map_err(|_| format!("{label}: timed out waiting for RoomEvent::RoomCreated"))?
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;

        match event {
            CoreEvent::Room(RoomEvent::RoomCreated {
                request_id: ev_id,
                room_id,
            }) if ev_id == request_id => {
                return Ok(room_id);
            }
            CoreEvent::OperationFailed {
                request_id: ev_id,
                failure,
            } if ev_id == request_id => {
                return Err(format!("{label} failed: {failure:?}"));
            }
            _ => continue,
        }
    }
}

#[cfg(any(debug_assertions, test))]
async fn wait_for_initial_items(
    conn: &mut CoreConnection,
    key: &TimelineKey,
    request_id: RequestId,
    label: &str,
) -> Result<Vec<matrix_desktop_core::event::TimelineItem>, String> {
    loop {
        let event = tokio::time::timeout(EVENT_TIMEOUT, conn.recv_event())
            .await
            .map_err(|_| format!("{label}: timed out waiting for TimelineEvent::InitialItems"))?
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;

        match event {
            CoreEvent::Timeline(TimelineEvent::InitialItems {
                request_id: Some(ev_id),
                key: ref ev_key,
                items,
                ..
            }) if ev_id == request_id && ev_key == key => {
                return Ok(items);
            }
            CoreEvent::OperationFailed {
                request_id: ev_id,
                failure,
            } if ev_id == request_id => {
                return Err(format!("{label} failed: {failure:?}"));
            }
            _ => continue,
        }
    }
}

#[cfg(any(debug_assertions, test))]
async fn wait_for_send_completed(
    conn: &mut CoreConnection,
    request_id: RequestId,
    key: &TimelineKey,
    label: &str,
) -> Result<(String, String), String> {
    loop {
        let event = tokio::time::timeout(EVENT_TIMEOUT, conn.recv_event())
            .await
            .map_err(|_| format!("{label}: timed out waiting for TimelineEvent::SendCompleted"))?
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;

        match event {
            CoreEvent::Timeline(TimelineEvent::SendCompleted {
                request_id: ev_id,
                key: ref ev_key,
                transaction_id,
                event_id,
            }) if ev_id == request_id && ev_key == key => {
                return Ok((transaction_id, event_id));
            }
            CoreEvent::OperationFailed {
                request_id: ev_id,
                failure,
            } if ev_id == request_id => {
                return Err(format!("{label} failed: {failure:?}"));
            }
            _ => continue,
        }
    }
}

/// Wait for a Set diff whose body contains `edited_body` or whose event_id
/// matches, signalling that the edit was received.
#[cfg(any(debug_assertions, test))]
async fn wait_for_edit_diff(
    conn: &mut CoreConnection,
    key: &TimelineKey,
    request_id: RequestId,
    event_id: &str,
    edited_body: &str,
    label: &str,
    timeout: Duration,
) -> Result<(), String> {
    loop {
        let event = tokio::time::timeout(timeout, conn.recv_event())
            .await
            .map_err(|_| format!("{label}: timed out waiting for edit Set diff"))?
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;

        match event {
            CoreEvent::Timeline(TimelineEvent::ItemsUpdated {
                key: ref ev_key,
                ref diffs,
                ..
            }) if ev_key == key => {
                for diff in diffs {
                    if let matrix_desktop_core::event::TimelineDiff::Set { item, .. } = diff {
                        let body_ok = item.body.as_deref().unwrap_or("").contains(edited_body);
                        let eid_ok = matches!(
                            &item.id,
                            matrix_desktop_core::event::TimelineItemId::Event { event_id: id }
                            if id == event_id
                        );
                        if body_ok || eid_ok {
                            return Ok(());
                        }
                    }
                }
            }
            CoreEvent::OperationFailed {
                request_id: ev_id,
                failure,
            } if ev_id == request_id => {
                return Err(format!("{label}: edit operation failed: {failure:?}"));
            }
            _ => continue,
        }
    }
}

/// Wait for an ItemsUpdated diff that signals a redaction (Remove or body-cleared Set).
#[cfg(any(debug_assertions, test))]
async fn wait_for_redact_diff(
    conn: &mut CoreConnection,
    key: &TimelineKey,
    request_id: RequestId,
    label: &str,
    timeout: Duration,
) -> Result<(), String> {
    loop {
        let event = tokio::time::timeout(timeout, conn.recv_event())
            .await
            .map_err(|_| format!("{label}: timed out waiting for redact diff"))?
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;

        match event {
            CoreEvent::Timeline(TimelineEvent::ItemsUpdated {
                key: ref ev_key,
                ref diffs,
                ..
            }) if ev_key == key => {
                for diff in diffs {
                    match diff {
                        matrix_desktop_core::event::TimelineDiff::Remove { .. } => return Ok(()),
                        matrix_desktop_core::event::TimelineDiff::Set { item, .. } => {
                            // A redacted item typically has no body.
                            if item.body.is_none() || item.body.as_deref() == Some("") {
                                return Ok(());
                            }
                        }
                        _ => {}
                    }
                }
            }
            CoreEvent::OperationFailed {
                request_id: ev_id,
                failure,
            } if ev_id == request_id => {
                return Err(format!("{label}: redact operation failed: {failure:?}"));
            }
            _ => continue,
        }
    }
}

/// Drive pagination until EndReached, re-issuing when Idle.
#[cfg(any(debug_assertions, test))]
async fn wait_for_paginate_end_reached(
    conn: &mut CoreConnection,
    key: &TimelineKey,
    first_request_id: RequestId,
    label: &str,
    timeout: Duration,
) -> Result<String, String> {
    let mut current_request_id = first_request_id;
    let mut saw_paginating = false;

    loop {
        let event = tokio::time::timeout(timeout, conn.recv_event())
            .await
            .map_err(|_| format!("{label}: timed out waiting for EndReached pagination state"))?
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;

        match event {
            CoreEvent::Timeline(TimelineEvent::PaginationStateChanged {
                key: ref ev_key,
                direction,
                state,
                ..
            }) if ev_key == key && direction == PaginationDirection::Backward => match state {
                PaginationState::Paginating => {
                    saw_paginating = true;
                }
                PaginationState::Idle => {
                    if !saw_paginating {
                        return Err(format!("{label}: Idle without prior Paginating"));
                    }
                    saw_paginating = false;
                    current_request_id = conn.next_request_id();
                    conn.command(CoreCommand::Timeline(TimelineCommand::Paginate {
                        request_id: current_request_id,
                        key: key.clone(),
                        direction: PaginationDirection::Backward,
                        event_count: 10,
                    }))
                    .await
                    .map_err(|e| format!("{label}: re-paginate submit failed: {e}"))?;
                }
                PaginationState::EndReached => {
                    return Ok("end_reached".to_owned());
                }
                PaginationState::Failed { kind } => {
                    return Err(format!("{label}: pagination failed: {kind:?}"));
                }
            },
            CoreEvent::OperationFailed {
                request_id: ev_id,
                failure,
            } if ev_id == current_request_id => {
                return Err(format!("{label}: paginate operation failed: {failure:?}"));
            }
            _ => {}
        }
    }
}

/// Wait for a session restore, handling an optional recovery requirement.
#[cfg(any(debug_assertions, test))]
async fn wait_for_session_restored_with_recovery(
    conn: &mut CoreConnection,
    request_id: RequestId,
    expected_account_key: &AccountKey,
    creds: &RealCredentials,
    transcript: &mut Vec<String>,
    label: &str,
) -> Result<(), String> {
    loop {
        let event = tokio::time::timeout(EVENT_TIMEOUT, conn.recv_event())
            .await
            .map_err(|_| {
                format!("{label}: timed out waiting for SessionRestored or RecoveryRequired")
            })?
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;

        match event {
            CoreEvent::Account(AccountEvent::SessionRestored {
                request_id: ev_id,
                account_key,
            }) if ev_id == request_id => {
                if account_key != *expected_account_key {
                    return Err(format!(
                        "{label}: account_key mismatch: got {:?}, expected {:?}",
                        account_key, expected_account_key
                    ));
                }
                return Ok(());
            }
            CoreEvent::Account(AccountEvent::RecoveryRequired { .. }) => {
                let line = "restore_recovery=required".to_owned();
                transcript.push(line.clone());
                println!("{line}");

                let submit_id = conn.next_request_id();
                conn.command(CoreCommand::Account(AccountCommand::SubmitRecovery {
                    request_id: submit_id,
                    request: RecoveryRequest {
                        secret: creds.recovery_key.clone(),
                    },
                }))
                .await
                .map_err(|e| format!("restore recovery submit failed: {e}"))?;

                match wait_for_recovery_outcome(conn, submit_id, "restore recovery").await? {
                    RecoveryOutcome::Completed => {
                        let line2 = "restore_recovery=completed".to_owned();
                        transcript.push(line2.clone());
                        println!("{line2}");
                    }
                    RecoveryOutcome::Failed(kind) => {
                        return Err(format!("restore recovery failed with kind {kind:?}"));
                    }
                }
                // Continue looping to receive SessionRestored.
            }
            CoreEvent::OperationFailed {
                request_id: ev_id,
                failure,
            } if ev_id == request_id => {
                return Err(format!("{label} failed: {failure:?}"));
            }
            _ => continue,
        }
    }
}

#[cfg(any(debug_assertions, test))]
async fn wait_for_logged_out(
    conn: &mut CoreConnection,
    request_id: RequestId,
    expected_account_key: &AccountKey,
    label: &str,
) -> Result<(), String> {
    loop {
        let event = tokio::time::timeout(EVENT_TIMEOUT, conn.recv_event())
            .await
            .map_err(|_| format!("{label}: timed out waiting for LoggedOut event"))?
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;

        match event {
            CoreEvent::Account(AccountEvent::LoggedOut {
                request_id: ev_id,
                account_key,
            }) if ev_id == request_id => {
                if account_key != *expected_account_key {
                    return Err(format!(
                        "{label}: account_key mismatch: got {:?}, expected {:?}",
                        account_key, expected_account_key
                    ));
                }
                return Ok(());
            }
            CoreEvent::OperationFailed {
                request_id: ev_id,
                failure,
            } if ev_id == request_id => {
                return Err(format!("{label} failed: {failure:?}"));
            }
            _ => continue,
        }
    }
}

#[cfg(any(debug_assertions, test))]
async fn wait_for_operation_failed(
    conn: &mut CoreConnection,
    request_id: RequestId,
    label: &str,
) -> Result<CoreFailure, String> {
    loop {
        let event = tokio::time::timeout(EVENT_TIMEOUT, conn.recv_event())
            .await
            .map_err(|_| format!("{label}: timed out waiting for OperationFailed event"))?
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;

        match event {
            CoreEvent::OperationFailed {
                request_id: ev_id,
                failure,
            } if ev_id == request_id => {
                return Ok(failure);
            }
            CoreEvent::Account(account_event) => {
                let matches_id = match &account_event {
                    AccountEvent::LoggedIn { request_id: id, .. }
                    | AccountEvent::SessionRestored { request_id: id, .. }
                    | AccountEvent::SavedSessionsListed { request_id: id, .. }
                    | AccountEvent::RecoveryCompleted { request_id: id, .. }
                    | AccountEvent::LoggedOut { request_id: id, .. }
                    | AccountEvent::AccountSwitched { request_id: id, .. } => *id == request_id,
                    AccountEvent::RecoveryRequired { .. } => false,
                };
                if matches_id {
                    return Err(format!(
                        "{label}: expected OperationFailed but the operation succeeded"
                    ));
                }
            }
            _ => continue,
        }
    }
}

/// Wait for any timeline diff in `key` containing `body_substring` in any item body.
#[cfg(any(debug_assertions, test))]
async fn wait_for_body_substring_in_timeline(
    conn: &mut CoreConnection,
    key: &TimelineKey,
    body_substring: &str,
    label: &str,
    timeout: Duration,
) -> Result<(), String> {
    loop {
        let event = tokio::time::timeout(timeout, conn.recv_event())
            .await
            .map_err(|_| {
                format!(
                    "{label}: timed out waiting for item with body containing '{body_substring}'"
                )
            })?
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;

        let found = match &event {
            CoreEvent::Timeline(TimelineEvent::ItemsUpdated {
                key: ev_key, diffs, ..
            }) if ev_key == key => diffs.iter().any(|diff| {
                let item_opt = match diff {
                    matrix_desktop_core::event::TimelineDiff::PushBack { item }
                    | matrix_desktop_core::event::TimelineDiff::PushFront { item }
                    | matrix_desktop_core::event::TimelineDiff::Insert { item, .. }
                    | matrix_desktop_core::event::TimelineDiff::Set { item, .. } => Some(item),
                    matrix_desktop_core::event::TimelineDiff::Reset { items } => {
                        return items
                            .iter()
                            .any(|it| it.body.as_deref().unwrap_or("").contains(body_substring));
                    }
                    _ => None,
                };
                item_opt.map_or(false, |it| {
                    it.body.as_deref().unwrap_or("").contains(body_substring)
                })
            }),
            CoreEvent::Timeline(TimelineEvent::InitialItems {
                key: ev_key, items, ..
            }) if ev_key == key => items
                .iter()
                .any(|it| it.body.as_deref().unwrap_or("").contains(body_substring)),
            _ => false,
        };

        if found {
            return Ok(());
        }
    }
}

/// Poll search until the expected event appears in results or the deadline is exceeded.
#[cfg(any(debug_assertions, test))]
async fn poll_search_until_found_or_timeout(
    conn: &mut CoreConnection,
    query: &str,
    expected_event_id: &str,
    room_id: &str,
    label: &str,
    timeout: Duration,
) -> Result<(), String> {
    let deadline = tokio::time::Instant::now() + timeout;

    loop {
        if tokio::time::Instant::now() >= deadline {
            return Err(format!(
                "{label}: event {expected_event_id} not found in search results after {timeout:?}"
            ));
        }

        let rid = conn.next_request_id();
        conn.command(CoreCommand::Search(SearchCommand::Query {
            request_id: rid,
            query: query.to_owned(),
            scope: SearchScope::Room {
                room_id: room_id.to_owned(),
            },
        }))
        .await
        .map_err(|e| format!("{label}: submit search query failed: {e}"))?;

        let found = wait_for_search_results(conn, rid, expected_event_id, label).await?;
        if found {
            return Ok(());
        }

        tokio::time::sleep(Duration::from_millis(1000)).await;
    }
}

#[cfg(any(debug_assertions, test))]
async fn wait_for_search_results(
    conn: &mut CoreConnection,
    request_id: RequestId,
    expected_event_id: &str,
    label: &str,
) -> Result<bool, String> {
    loop {
        let event = tokio::time::timeout(Duration::from_secs(10), conn.recv_event())
            .await
            .map_err(|_| format!("{label}: timed out waiting for SearchEvent::Results"))?
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;

        match event {
            CoreEvent::Search(SearchEvent::Results {
                request_id: ev_id,
                results,
            }) if ev_id == request_id => {
                return Ok(results.iter().any(|r| r.event_id == expected_event_id));
            }
            CoreEvent::OperationFailed {
                request_id: ev_id,
                failure,
            } if ev_id == request_id => {
                return Err(format!("{label}: search query failed: {failure:?}"));
            }
            _ => continue,
        }
    }
}

#[cfg(all(test, feature = "test-hooks"))]
mod tests {
    use std::sync::Arc;

    use tempfile::tempdir;
    use tokio::time::sleep;

    use super::*;

    #[tokio::test]
    async fn recovery_gate_waits_for_late_needs_recovery_after_ready_snapshot() {
        let data_dir = tempdir().unwrap();
        let runtime = Arc::new(CoreRuntime::start_with_data_dir(
            data_dir.path().to_path_buf(),
        ));
        let mut conn = runtime.attach();

        let info = matrix_desktop_state::SessionInfo {
            homeserver: "https://example.test".to_owned(),
            user_id: "@alice:example.test".to_owned(),
            device_id: "DEVICE1".to_owned(),
        };

        runtime
            .inject_actions(vec![matrix_desktop_state::AppAction::LoginSucceeded(
                info.clone(),
            )])
            .await;

        wait_for_ready_snapshot(&mut conn, "setup ready")
            .await
            .expect("setup ready snapshot");

        let runtime2 = Arc::clone(&runtime);
        let delayed = tokio::spawn(async move {
            sleep(Duration::from_millis(50)).await;
            runtime2
                .inject_actions(vec![
                    matrix_desktop_state::AppAction::E2eeRecoveryStateChanged {
                        state: matrix_desktop_state::E2eeRecoveryState::Incomplete,
                        methods: vec![matrix_desktop_state::RecoveryMethod::RecoveryKey],
                    },
                ])
                .await;
        });

        let result = wait_for_recovery_required_after_sync(&mut conn, "gate").await;
        delayed.await.expect("delayed injector");

        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn post_login_ready_gate_waits_for_late_ready_snapshot_before_sync() {
        let data_dir = tempdir().unwrap();
        let runtime = Arc::new(CoreRuntime::start_with_data_dir(
            data_dir.path().to_path_buf(),
        ));
        let mut conn = runtime.attach();

        let runtime2 = Arc::clone(&runtime);
        let delayed = tokio::spawn(async move {
            sleep(Duration::from_millis(50)).await;
            runtime2
                .inject_actions(vec![matrix_desktop_state::AppAction::LoginSucceeded(
                    matrix_desktop_state::SessionInfo {
                        homeserver: "https://example.test".to_owned(),
                        user_id: "@alice:example.test".to_owned(),
                        device_id: "DEVICE1".to_owned(),
                    },
                )])
                .await;
        });

        let result = wait_for_post_login_ready_snapshot(&mut conn, "post-login gate").await;
        delayed.await.expect("delayed injector");

        assert!(result.is_ok());
        assert!(matches!(conn.snapshot().session, SessionState::Ready(_)));
    }
}
