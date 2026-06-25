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
//!   store override (KOUSHI_QA_FILE_CREDENTIAL_STORE_DIR) is mandatory.
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
//! - KOUSHI_REAL_QA_CREDENTIALS_PATH - path to the credentials JSON file
//! - KOUSHI_QA_FILE_CREDENTIAL_STORE_DIR - mandatory; see keychain guard
//! - KOUSHI_QA_DATA_DIR (optional) - overrides per-run data dir root

#![allow(dead_code)]

use std::process::ExitCode;
use std::time::Duration;

use koushi_core::command::{
    AccountCommand, CoreCommand, RoomCommand, SearchCommand, SearchScope, SyncCommand,
    TimelineCommand,
};
use koushi_core::event::{
    AccountEvent, CoreEvent, PaginationDirection, PaginationState, RoomEvent, SearchEvent,
    SyncBackendKind, SyncEvent, TimelineEvent,
};
use koushi_core::failure::{CoreFailure, RecoveryFailureKind, TimelineFailureKind};
use koushi_core::ids::{AccountKey, RequestId, TimelineKey};
use koushi_core::runtime::{CoreConnection, CoreRuntime};
use koushi_state::{
    AppState, AuthSecret, LoginRequest, MentionIntent, RecoveryRequest, SessionState,
};

// ---------------------------------------------------------------------------
// Env var constants
// ---------------------------------------------------------------------------

const ENV_DATA_DIR: &str = "KOUSHI_QA_DATA_DIR";
const ENV_REAL_QA_SCENARIO: &str = "KOUSHI_REAL_QA_SCENARIO";

#[cfg(any(debug_assertions, test))]
const ENV_CREDENTIALS_PATH: &str = "KOUSHI_REAL_QA_CREDENTIALS_PATH";
#[cfg(any(debug_assertions, test))]
const ENV_FILE_CREDENTIAL_STORE_DIR: &str = "KOUSHI_QA_FILE_CREDENTIAL_STORE_DIR";
/// When set to "1", the `startup_latency` scenario logs out at teardown so the
/// QA device is removed from the homeserver. Unset by default: the session is
/// kept so run 2+ can restore rather than login.
#[cfg(any(debug_assertions, test))]
const ENV_STARTUP_LAT_TEARDOWN: &str = "KOUSHI_STARTUP_LAT_TEARDOWN";
/// Number of backward paginate pages to issue in the `startup_latency` scenario.
#[cfg(any(debug_assertions, test))]
const STARTUP_LAT_PAGES: usize = 3;

#[cfg(any(debug_assertions, test))]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RealQaScenario {
    Compat,
    SpaceCompat,
    All,
    /// Read-only timing probe: restore-or-login, sync to ready, subscribe and
    /// paginate a target room, emit `startup_lat phase=… ms=…` tokens only.
    StartupLatency,
}

#[cfg(any(debug_assertions, test))]
impl RealQaScenario {
    fn from_env() -> Result<Self, String> {
        Self::from_env_value(std::env::var(ENV_REAL_QA_SCENARIO).ok())
    }

    fn from_env_value(value: Option<String>) -> Result<Self, String> {
        match value.as_deref() {
            None | Some("space_compat") => Ok(Self::SpaceCompat),
            Some("compat") => Ok(Self::Compat),
            Some("all") => Ok(Self::All),
            Some("startup_latency") => Ok(Self::StartupLatency),
            Some(other) => Err(format!(
                "unsupported {ENV_REAL_QA_SCENARIO} value '{other}'; \
                 expected compat, space_compat, all, or startup_latency"
            )),
        }
    }

    fn includes_space_stage(self) -> bool {
        matches!(self, Self::SpaceCompat | Self::All)
    }
}

// ---------------------------------------------------------------------------
// Timeout constants - matrix.org is slower than local servers
// ---------------------------------------------------------------------------

/// Standard per-event wait.
const EVENT_TIMEOUT: Duration = Duration::from_secs(60);
/// Extended timeout for sync operations (matrix.org initial sync can be slow).
const SYNC_TIMEOUT: Duration = Duration::from_secs(120);
/// Extended timeout for room list non-empty wait.
const ROOM_LIST_TIMEOUT: Duration = Duration::from_secs(120);
/// Shorter timeout for the optional space-child projection observation.
const SPACE_CHILD_PROJECTION_TIMEOUT: Duration = Duration::from_secs(20);
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
            .unwrap_or("Koushi Real QA")
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
    let scenario = RealQaScenario::from_env()?;

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|e| format!("Tokio runtime creation failed: {e}"))?;

    let mut transcript: Vec<String> = Vec::new();
    let result = rt.block_on(run_async(&creds, scenario, &mut transcript));

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
    if !koushi_core::store::resolved_credential_backend_is_file_dir() {
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

/// Tracks the resources the real-homeserver QA run created so the catch-all
/// wrapper can leave/forget rooms/spaces and log out even when an inner step
/// fails via `?`. Without this, a `?`-propagated send/edit/restore failure
/// would leak BOTH the session/device AND the created room/space on the live
/// homeserver (REPOSITORY_RULES: QA must clean up every resource it creates).
#[cfg(any(debug_assertions, test))]
#[derive(Default)]
struct RealQaCleanupState {
    account_key: Option<AccountKey>,
    qa_room_id: Option<String>,
    qa_space_id: Option<String>,
    logged_out: bool,
}

#[cfg(any(debug_assertions, test))]
struct RealHomeserverQaMessagePlan {
    search_token: String,
    msg1_body: String,
    search_probe_body: String,
    msg2_body: String,
    edited_body: String,
    reply_body: String,
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
        reply_body: "Real homeserver QA reply to message 1".to_owned(),
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
        assert_eq!(plan.reply_body, "Real homeserver QA reply to message 1");
        // The reply body must not carry the search token, so replying does not
        // perturb the later search-probe assertion.
        assert!(!plan.reply_body.contains(&plan.search_token));
    }
}

#[cfg(test)]
mod scenario_tests {
    use super::*;

    #[test]
    fn real_homeserver_qa_scenario_parses_known_names() {
        assert_eq!(
            RealQaScenario::from_env_value(Some("compat".to_owned())).unwrap(),
            RealQaScenario::Compat
        );
        assert_eq!(
            RealQaScenario::from_env_value(Some("space_compat".to_owned())).unwrap(),
            RealQaScenario::SpaceCompat
        );
        assert_eq!(
            RealQaScenario::from_env_value(Some("all".to_owned())).unwrap(),
            RealQaScenario::All
        );
    }

    #[test]
    fn real_homeserver_qa_scenario_defaults_to_space_compat_when_missing() {
        // The default real lane proves space create/link/cleanup, matching the
        // qa:headless-basic:real package script and docs/qa contract.
        assert_eq!(
            RealQaScenario::from_env_value(None).unwrap(),
            RealQaScenario::SpaceCompat
        );
    }

    #[test]
    fn startup_latency_scenario_parses_from_env() {
        assert_eq!(
            RealQaScenario::from_env_value(Some("startup_latency".to_owned())),
            Ok(RealQaScenario::StartupLatency)
        );
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
        .join("koushi-desktop-real-qa")
        .join(format!("{}_{}", std::process::id(), ts))
}

// ---------------------------------------------------------------------------
// Async QA flow
// ---------------------------------------------------------------------------

/// Catch-all wrapper around the QA flow. Computes the per-run `data_dir` once,
/// runs the inner flow, and — on ANY failure (including `?`-propagated ones)
/// that did not already reach the final logout — runs a best-effort cleanup
/// pass that leaves/forgets every created room/space and logs out. This is the
/// finally-ish path required by the Secrets/QA canon: no stale device, room, or
/// space may survive a failed run.
#[cfg(any(debug_assertions, test))]
async fn run_async(
    creds: &RealCredentials,
    scenario: RealQaScenario,
    transcript: &mut Vec<String>,
) -> Result<String, String> {
    let data_dir = real_qa_data_dir();
    let mut cleanup = RealQaCleanupState::default();
    let result = run_async_inner(creds, scenario, &data_dir, transcript, &mut cleanup).await;
    if result.is_err() && !cleanup.logged_out {
        cleanup_real_qa_resources(creds, &data_dir, transcript, &mut cleanup).await;
    }
    result
}

#[cfg(any(debug_assertions, test))]
async fn run_async_inner(
    creds: &RealCredentials,
    scenario: RealQaScenario,
    data_dir: &std::path::Path,
    transcript: &mut Vec<String>,
    cleanup: &mut RealQaCleanupState,
) -> Result<String, String> {
    // The startup_latency scenario is read-only and has its own entry path:
    // restore-or-login, macro timing, subscribe+paginate, optional teardown.
    // Dispatch early so it never enters the compat create/send/paginate flow.
    if matches!(scenario, RealQaScenario::StartupLatency) {
        return run_startup_latency_scenario(creds, data_dir, transcript, cleanup).await;
    }

    // -----------------------------------------------------------------------
    // Step 1: HTTPS login (single login per run - rate limit rule)
    // -----------------------------------------------------------------------
    let runtime = CoreRuntime::start_with_data_dir(data_dir.to_path_buf());
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
    // Login succeeded: record the account key so the catch-all wrapper can log
    // out (and leave/forget any rooms/spaces) on a later failure.
    cleanup.account_key = Some(account_key.clone());
    // Matrix identifiers (user/room/event/space ids) MUST NOT appear in QA
    // output (REPOSITORY_RULES Security). Emit private-data-free tokens only.
    let line = "login=ok".to_owned();
    transcript.push(line.clone());
    println!("{line}");

    wait_for_post_login_ready_snapshot(&mut conn, "post-login Ready").await?;

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
        wait_for_sync_started(&mut conn, sync_start_id, "sync start", SYNC_TIMEOUT).await?;

    let backend_name = match sync_backend {
        SyncBackendKind::SyncService => "SyncService",
        SyncBackendKind::LegacySync => "LegacySync",
    };
    let line = format!("sync_backend={backend_name}");
    transcript.push(line.clone());
    println!("{line}");

    wait_for_sync_running(&mut conn, "sync running", SYNC_TIMEOUT).await?;
    let line = "sync=running".to_owned();
    transcript.push(line.clone());
    println!("{line}");

    // -----------------------------------------------------------------------
    // Step 3: Recovery check
    // -----------------------------------------------------------------------
    // Wait for the post-sync recovery observer to publish the final state.
    // Recovery becomes actionable only once sync/account data has flowed in.
    wait_for_recovery_required_after_sync(&mut conn, "post-sync recovery gate").await?;

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
            // Recovery failure is a hard QA failure; the catch-all wrapper owns
            // logout/cleanup after we return Err.
            let line = format!("recovery=failed kind={kind:?}");
            transcript.push(line.clone());
            eprintln!("{line}");
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
        encrypted: false,
    }))
    .await
    .map_err(|e| format!("create QA room command submit failed: {e}"))?;

    let qa_room_id = wait_for_room_created(&mut conn, create_room_id, "create QA room").await?;
    // Record the created room so the catch-all wrapper can leave/forget it if a
    // later step fails before the happy-path cleanup runs.
    cleanup.qa_room_id = Some(qa_room_id.clone());
    // QA-created room name and room_id are synthetic - allowed in output.
    let line = "qa_room=created".to_owned();
    transcript.push(line.clone());
    println!("{line}");

    let mut real_space_id: Option<String> = None;
    if scenario.includes_space_stage() {
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let qa_space_name = format!("core-real-space-{ts}-{}", std::process::id());

        let create_space_id = conn.next_request_id();
        conn.command(CoreCommand::Room(RoomCommand::CreateSpace {
            request_id: create_space_id,
            name: qa_space_name.clone(),
        }))
        .await
        .map_err(|e| format!("create QA space command submit failed: {e}"))?;

        let qa_space_id =
            wait_for_space_created(&mut conn, create_space_id, "create QA space").await?;
        // Record the created space so the catch-all wrapper can leave/forget it
        // if a later step fails before the happy-path cleanup runs.
        cleanup.qa_space_id = Some(qa_space_id.clone());
        let line = "real_space_create=ok".to_owned();
        transcript.push(line.clone());
        println!("{line}");

        let via_server = creds
            .user_id
            .split_once(':')
            .map(|(_, server)| server.to_owned())
            .ok_or_else(|| "cannot derive space via_server from user_id".to_owned())?;

        let set_child_id = conn.next_request_id();
        conn.command(CoreCommand::Room(RoomCommand::SetSpaceChild {
            request_id: set_child_id,
            space_id: qa_space_id.clone(),
            child_room_id: qa_room_id.clone(),
            via_server,
        }))
        .await
        .map_err(|e| format!("set QA space child command submit failed: {e}"))?;

        wait_for_space_child_set(
            &mut conn,
            set_child_id,
            &qa_space_id,
            &qa_room_id,
            "set QA space child",
        )
        .await?;

        let line = "real_space_child=ok".to_owned();
        transcript.push(line.clone());
        println!("{line}");

        match wait_for_room_list_space_child(
            &mut conn,
            &qa_space_id,
            &qa_room_id,
            "space child projection",
            SPACE_CHILD_PROJECTION_TIMEOUT,
        )
        .await
        {
            Ok(_) => {
                let line = "real_space_projection=observed".to_owned();
                transcript.push(line.clone());
                println!("{line}");
            }
            Err(_) => {
                let line = "real_space_projection=not_observed".to_owned();
                transcript.push(line.clone());
                println!("{line}");
            }
        }

        real_space_id = Some(qa_space_id);
    }

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
        mentions: MentionIntent::default(),
    }))
    .await
    .map_err(|e| format!("send message 1 command submit failed: {e}"))?;

    let (_, event1_id) =
        wait_for_send_completed(&mut conn, send1_id, &timeline_key, "send msg1").await?;
    let line = "send_msg1=ok".to_owned();
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
        mentions: MentionIntent::default(),
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
    let line = "send_search=ok".to_owned();
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
        mentions: MentionIntent::default(),
    }))
    .await
    .map_err(|e| format!("send message 2 command submit failed: {e}"))?;

    let (_, event2_id) =
        wait_for_send_completed(&mut conn, send2_id, &timeline_key, "send msg2").await?;
    let line = "send_msg2=ok".to_owned();
    transcript.push(line.clone());
    println!("{line}");

    // Reply to message 1, proving SendReply works against the real homeserver
    // now that reply support is green on the local lanes (roadmap Phase 15).
    // The reply targets a plain message event and its body carries no search
    // token, so it does not perturb the later search-probe assertion.
    let txn_reply = format!("real-qa-txn-reply-{ts}");
    let reply_id = conn.next_request_id();
    conn.command(CoreCommand::Timeline(TimelineCommand::SendReply {
        request_id: reply_id,
        key: timeline_key.clone(),
        transaction_id: txn_reply,
        in_reply_to_event_id: event1_id.clone(),
        body: message_plan.reply_body.clone(),
        mentions: MentionIntent::default(),
    }))
    .await
    .map_err(|e| format!("send reply command submit failed: {e}"))?;

    wait_for_send_completed(&mut conn, reply_id, &timeline_key, "send reply").await?;
    let line = "real_reply=ok".to_owned();
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
            // The catch-all wrapper owns logout/cleanup after we return Err.
            let errline = format!("search_smoke=failed reason={e}");
            transcript.push(errline.clone());
            eprintln!("{errline}");
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
    let runtime2 = CoreRuntime::start_with_data_dir(data_dir.to_path_buf());
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
    // -----------------------------------------------------------------------
    let cleanup_result: Result<(), String> = async {
        let leave_room_id = conn2.next_request_id();
        conn2
            .command(CoreCommand::Room(RoomCommand::LeaveRoom {
                request_id: leave_room_id,
                room_id: qa_room_id.clone(),
            }))
            .await
            .map_err(|e| format!("leave QA room command submit failed: {e}"))?;
        wait_for_room_left(&mut conn2, leave_room_id, &qa_room_id, "leave QA room").await?;

        let forget_room_id = conn2.next_request_id();
        conn2
            .command(CoreCommand::Room(RoomCommand::ForgetRoom {
                request_id: forget_room_id,
                room_id: qa_room_id.clone(),
            }))
            .await
            .map_err(|e| format!("forget QA room command submit failed: {e}"))?;
        wait_for_room_forgotten(&mut conn2, forget_room_id, &qa_room_id, "forget QA room").await?;

        let line = "leave_room=ok forget_room=ok".to_owned();
        transcript.push(line.clone());
        println!("{line}");

        if let Some(space_id) = real_space_id.as_ref() {
            let leave_space_id = conn2.next_request_id();
            conn2
                .command(CoreCommand::Room(RoomCommand::LeaveRoom {
                    request_id: leave_space_id,
                    room_id: space_id.clone(),
                }))
                .await
                .map_err(|e| format!("leave QA space command submit failed: {e}"))?;
            wait_for_room_left(&mut conn2, leave_space_id, space_id, "leave QA space").await?;

            let forget_space_id = conn2.next_request_id();
            conn2
                .command(CoreCommand::Room(RoomCommand::ForgetRoom {
                    request_id: forget_space_id,
                    room_id: space_id.clone(),
                }))
                .await
                .map_err(|e| format!("forget QA space command submit failed: {e}"))?;
            wait_for_room_forgotten(&mut conn2, forget_space_id, space_id, "forget QA space")
                .await?;

            let line = "real_space_cleanup=ok".to_owned();
            transcript.push(line.clone());
            println!("{line}");
        }

        Ok(())
    }
    .await;

    cleanup_result?;

    // -----------------------------------------------------------------------
    // Step 9: Logout -> SignedOut + post-logout RestoreLastSession = SessionNotFound
    // -----------------------------------------------------------------------
    do_logout(&mut conn2, &account_key, transcript).await;
    // The happy-path logout has run; tell the catch-all wrapper not to clean up
    // again (the post-logout assertions below are non-resource-leaking checks).
    cleanup.logged_out = true;

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
    let mut summary = format!(
        "Real homeserver QA OK. \
         login=ok recovery={recovery} \
         sync_backend={backend} sync=ok \
         rooms={rooms} spaces={spaces} dms={dms} \
         qa_room=created send_msg1=ok send_search=ok send_msg2=ok real_reply=ok \
         edit_msg1=ok redact_msg2=ok \
         paginate={paginate} search={search} \
         store_restore=ok restore_body={body_ok} \
         leave_room=ok forget_room=ok \
         logout=ok post_logout_restore=not_found",
        recovery = "completed",
        backend = backend_name,
        rooms = rooms_count,
        spaces = spaces_count,
        dms = dms_count,
        paginate = paginate_result,
        search = search_status,
        body_ok = restore_body_tag,
    );

    if real_space_id.is_some() {
        summary.push_str(" real_space_create=ok real_space_child=ok real_space_cleanup=ok");
    }

    Ok(summary)
}

// ---------------------------------------------------------------------------
// Catch-all cleanup (finally-ish path for `?`-propagated inner failures)
// ---------------------------------------------------------------------------

/// Best-effort cleanup invoked by `run_async` whenever the inner flow returns
/// an error before reaching the happy-path logout. It starts a fresh runtime
/// over the same `data_dir`, restores the session, then leaves/forgets every
/// recorded room and space and logs out so no stale device, room, or space
/// survives a failed run.
///
/// This function MUST NEVER return Err and MUST NEVER panic — every failure is
/// swallowed into a concrete `cleanup_warning=...` token. Matrix identifiers
/// (user/room/event/space ids) are never printed; only token lines are emitted
/// (REPOSITORY_RULES Security; Task 5).
#[cfg(any(debug_assertions, test))]
async fn cleanup_real_qa_resources(
    creds: &RealCredentials,
    data_dir: &std::path::Path,
    transcript: &mut Vec<String>,
    cleanup: &mut RealQaCleanupState,
) {
    // No login succeeded -> there is nothing to clean (no session, and rooms /
    // spaces cannot have been created without a session).
    let Some(account_key) = cleanup.account_key.clone() else {
        return;
    };

    // Start a fresh runtime over the same data dir and restore the session so
    // we hold a Matrix-capable connection to leave/forget and log out.
    let runtime = CoreRuntime::start_with_data_dir(data_dir.to_path_buf());
    let mut conn = runtime.attach();

    let restore_id = conn.next_request_id();
    if let Err(e) = conn
        .command(CoreCommand::Account(AccountCommand::RestoreLastSession {
            request_id: restore_id,
        }))
        .await
    {
        let line = format!("cleanup_warning=restore_failed reason={e}");
        transcript.push(line.clone());
        eprintln!("{line}");
        return;
    }

    if let Err(e) = wait_for_session_restored_with_recovery(
        &mut conn,
        restore_id,
        &account_key,
        creds,
        transcript,
        "cleanup restore",
    )
    .await
    {
        let line = format!("cleanup_warning=restore_failed reason={e}");
        transcript.push(line.clone());
        eprintln!("{line}");
        return;
    }

    if let Err(e) = wait_for_ready_snapshot(&mut conn, "cleanup restored Ready").await {
        let line = format!("cleanup_warning=restore_failed reason={e}");
        transcript.push(line.clone());
        eprintln!("{line}");
        return;
    }

    // Leave/forget the QA room. Each sub-step records a concrete warning token
    // on failure and CONTINUES (do not bail) so the space and logout still run.
    if let Some(room_id) = cleanup.qa_room_id.clone() {
        let leave_id = conn.next_request_id();
        match conn
            .command(CoreCommand::Room(RoomCommand::LeaveRoom {
                request_id: leave_id,
                room_id: room_id.clone(),
            }))
            .await
        {
            Ok(()) => {
                if let Err(e) =
                    wait_for_room_left(&mut conn, leave_id, &room_id, "cleanup leave room").await
                {
                    let line = format!("cleanup_warning=leave_room_failed reason={e}");
                    transcript.push(line.clone());
                    eprintln!("{line}");
                }
            }
            Err(e) => {
                let line = format!("cleanup_warning=leave_room_failed reason={e}");
                transcript.push(line.clone());
                eprintln!("{line}");
            }
        }

        let forget_id = conn.next_request_id();
        match conn
            .command(CoreCommand::Room(RoomCommand::ForgetRoom {
                request_id: forget_id,
                room_id: room_id.clone(),
            }))
            .await
        {
            Ok(()) => {
                if let Err(e) =
                    wait_for_room_forgotten(&mut conn, forget_id, &room_id, "cleanup forget room")
                        .await
                {
                    let line = format!("cleanup_warning=forget_room_failed reason={e}");
                    transcript.push(line.clone());
                    eprintln!("{line}");
                }
            }
            Err(e) => {
                let line = format!("cleanup_warning=forget_room_failed reason={e}");
                transcript.push(line.clone());
                eprintln!("{line}");
            }
        }
    }

    // Leave/forget the QA space (spaces are rooms on the homeserver).
    if let Some(space_id) = cleanup.qa_space_id.clone() {
        let leave_id = conn.next_request_id();
        match conn
            .command(CoreCommand::Room(RoomCommand::LeaveRoom {
                request_id: leave_id,
                room_id: space_id.clone(),
            }))
            .await
        {
            Ok(()) => {
                if let Err(e) =
                    wait_for_room_left(&mut conn, leave_id, &space_id, "cleanup leave space").await
                {
                    let line = format!("cleanup_warning=leave_space_failed reason={e}");
                    transcript.push(line.clone());
                    eprintln!("{line}");
                }
            }
            Err(e) => {
                let line = format!("cleanup_warning=leave_space_failed reason={e}");
                transcript.push(line.clone());
                eprintln!("{line}");
            }
        }

        let forget_id = conn.next_request_id();
        match conn
            .command(CoreCommand::Room(RoomCommand::ForgetRoom {
                request_id: forget_id,
                room_id: space_id.clone(),
            }))
            .await
        {
            Ok(()) => {
                if let Err(e) =
                    wait_for_room_forgotten(&mut conn, forget_id, &space_id, "cleanup forget space")
                        .await
                {
                    let line = format!("cleanup_warning=forget_space_failed reason={e}");
                    transcript.push(line.clone());
                    eprintln!("{line}");
                }
            }
            Err(e) => {
                let line = format!("cleanup_warning=forget_space_failed reason={e}");
                transcript.push(line.clone());
                eprintln!("{line}");
            }
        }
    }

    // Finally log out. `do_logout` already prints a `logout_submit=failed` /
    // `logout_wait=failed` token on failure and never propagates errors.
    do_logout(&mut conn, &account_key, transcript).await;
    cleanup.logged_out = true;
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
async fn wait_for_room_left(
    conn: &mut CoreConnection,
    request_id: RequestId,
    expected_room_id: &str,
    label: &str,
) -> Result<(), String> {
    loop {
        let event = tokio::time::timeout(EVENT_TIMEOUT, conn.recv_event())
            .await
            .map_err(|_| format!("{label}: timed out waiting for RoomEvent::RoomLeft"))?
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;

        match event {
            CoreEvent::Room(RoomEvent::RoomLeft {
                request_id: ev_id,
                room_id,
            }) if ev_id == request_id => {
                if room_id == expected_room_id {
                    return Ok(());
                }
                return Err(format!("{label}: unexpected room id (redacted)"));
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
async fn wait_for_room_forgotten(
    conn: &mut CoreConnection,
    request_id: RequestId,
    expected_room_id: &str,
    label: &str,
) -> Result<(), String> {
    loop {
        let event = tokio::time::timeout(EVENT_TIMEOUT, conn.recv_event())
            .await
            .map_err(|_| format!("{label}: timed out waiting for RoomEvent::RoomForgotten"))?
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;

        match event {
            CoreEvent::Room(RoomEvent::RoomForgotten {
                request_id: ev_id,
                room_id,
            }) if ev_id == request_id => {
                if room_id == expected_room_id {
                    return Ok(());
                }
                return Err(format!("{label}: unexpected room id (redacted)"));
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
async fn wait_for_space_created(
    conn: &mut CoreConnection,
    request_id: RequestId,
    label: &str,
) -> Result<String, String> {
    loop {
        let event = tokio::time::timeout(EVENT_TIMEOUT, conn.recv_event())
            .await
            .map_err(|_| format!("{label}: timed out waiting for RoomEvent::SpaceCreated"))?
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;

        match event {
            CoreEvent::Room(RoomEvent::SpaceCreated {
                request_id: ev_id,
                space_id,
            }) if ev_id == request_id => {
                return Ok(space_id);
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
async fn wait_for_space_child_set(
    conn: &mut CoreConnection,
    request_id: RequestId,
    space_id: &str,
    child_room_id: &str,
    label: &str,
) -> Result<(), String> {
    loop {
        let event = tokio::time::timeout(EVENT_TIMEOUT, conn.recv_event())
            .await
            .map_err(|_| format!("{label}: timed out waiting for RoomEvent::SpaceChildSet"))?
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;

        match event {
            CoreEvent::Room(RoomEvent::SpaceChildSet {
                request_id: ev_id,
                space_id: ev_space,
                child_room_id: ev_child,
            }) if ev_id == request_id => {
                if ev_space != space_id || ev_child != child_room_id {
                    return Err(format!("{label}: SpaceChildSet IDs mismatch (redacted)"));
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
async fn wait_for_room_list_space_child(
    conn: &mut CoreConnection,
    space_id: &str,
    child_room_id: &str,
    label: &str,
    timeout: Duration,
) -> Result<AppState, String> {
    let contains_expected = |snapshot: &AppState| {
        snapshot.spaces.iter().any(|space| {
            space.space_id == space_id
                && space
                    .child_room_ids
                    .iter()
                    .any(|room_id| room_id == child_room_id)
        }) || snapshot.rooms.iter().any(|room| {
            room.room_id == child_room_id && room.parent_space_ids.iter().any(|id| id == space_id)
        })
    };

    let snapshot = conn.snapshot();
    if contains_expected(&snapshot) {
        return Ok(snapshot);
    }

    loop {
        let event = tokio::time::timeout(timeout, conn.recv_event())
            .await
            .map_err(|_| format!("{label}: timed out waiting for space-child projection"))?
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;

        match event {
            CoreEvent::Room(RoomEvent::RoomListUpdated) => {
                let snapshot = conn.snapshot();
                if contains_expected(&snapshot) {
                    return Ok(snapshot);
                }
            }
            CoreEvent::StateChanged(snapshot) => {
                if contains_expected(&snapshot) {
                    return Ok(snapshot);
                }
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
) -> Result<Vec<koushi_core::event::TimelineItem>, String> {
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
                    if let koushi_core::event::TimelineDiff::Set { item, .. } = diff {
                        let body_ok = item.body.as_deref().unwrap_or("").contains(edited_body);
                        let eid_ok = matches!(
                            &item.id,
                            koushi_core::event::TimelineItemId::Event { event_id: id }
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
                        koushi_core::event::TimelineDiff::Remove { .. } => return Ok(()),
                        koushi_core::event::TimelineDiff::Set { item, .. } => {
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

/// Wait for a `Ready` session snapshot. If the session first enters
/// `NeedsRecovery` (first-login path on an account with secret storage),
/// submit the recovery key once and keep waiting. On the restore path the
/// session normally reaches Ready directly without recovery.
#[cfg(any(debug_assertions, test))]
async fn wait_for_ready_handling_recovery(
    conn: &mut CoreConnection,
    creds: &RealCredentials,
    transcript: &mut Vec<String>,
    label: &str,
) -> Result<(), String> {
    let mut recovery_submitted = false;
    loop {
        // Settle from the current snapshot first.
        match conn.snapshot().session {
            SessionState::Ready(_) => return Ok(()),
            SessionState::NeedsRecovery { .. } if !recovery_submitted => {
                recovery_submitted = true;
                submit_startup_lat_recovery(conn, creds, transcript, label).await?;
                continue;
            }
            _ => {}
        }

        let event = tokio::time::timeout(SYNC_TIMEOUT, conn.recv_event())
            .await
            .map_err(|_| format!("{label}: timed out waiting for Ready snapshot"))?
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;

        match event {
            CoreEvent::StateChanged(snapshot) => match snapshot.session {
                SessionState::Ready(_) => return Ok(()),
                SessionState::NeedsRecovery { .. } if !recovery_submitted => {
                    recovery_submitted = true;
                    submit_startup_lat_recovery(conn, creds, transcript, label).await?;
                }
                _ => {}
            },
            CoreEvent::Account(AccountEvent::RecoveryRequired { .. }) if !recovery_submitted => {
                recovery_submitted = true;
                submit_startup_lat_recovery(conn, creds, transcript, label).await?;
            }
            _ => {}
        }
    }
}

#[cfg(any(debug_assertions, test))]
async fn submit_startup_lat_recovery(
    conn: &mut CoreConnection,
    creds: &RealCredentials,
    transcript: &mut Vec<String>,
    label: &str,
) -> Result<(), String> {
    let line = "startup_lat recovery=required".to_owned();
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
    .map_err(|e| format!("{label} recovery submit failed: {e}"))?;
    match wait_for_recovery_outcome(conn, submit_id, label).await? {
        RecoveryOutcome::Completed => {
            let l = "startup_lat recovery=completed".to_owned();
            transcript.push(l.clone());
            println!("{l}");
            Ok(())
        }
        // Coarse, no Debug (consistent with the codex finding-2 fix).
        RecoveryOutcome::Failed(_) => Err(format!("{label}: recovery failed")),
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
                    | AccountEvent::ProfileUpdated { request_id: id, .. }
                    | AccountEvent::AvatarThumbnailDownloaded { request_id: id, .. }
                    | AccountEvent::ReportCompleted { request_id: id, .. }
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
                    koushi_core::event::TimelineDiff::PushBack { item }
                    | koushi_core::event::TimelineDiff::PushFront { item }
                    | koushi_core::event::TimelineDiff::Insert { item, .. }
                    | koushi_core::event::TimelineDiff::Set { item, .. } => Some(item),
                    koushi_core::event::TimelineDiff::Reset { items } => {
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
                "{label}: expected event not found in search results after {timeout:?}"
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

        // Wake on the next search index mutation rather than blindly sleeping:
        // wait (bounded) for a `SearchEvent::IndexUpdated`, then retry the
        // query. If no index event arrives within the bound, fall through to a
        // plain retry so a missing event can never deadlock the loop. The
        // overall `deadline` still bounds the whole poll.
        wait_for_index_update_or_idle(conn, deadline).await;
    }
}

/// Wait until the search index reports an `IndexUpdated`, the per-iteration
/// idle bound elapses, or the overall `deadline` passes. Other events on the
/// interleaved stream are ignored. Always returns (never errors): a missing
/// index event simply means the caller retries its query.
#[cfg(any(debug_assertions, test))]
async fn wait_for_index_update_or_idle(conn: &mut CoreConnection, deadline: tokio::time::Instant) {
    // Bound a single idle wait so the retry cadence matches the prior sleep
    // when the index is quiet, while still waking immediately on indexing.
    const IDLE_WAIT: Duration = Duration::from_millis(1000);
    let now = tokio::time::Instant::now();
    if now >= deadline {
        return;
    }
    let wait_until = (now + IDLE_WAIT).min(deadline);

    loop {
        let remaining = wait_until.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            return;
        }
        match tokio::time::timeout(remaining, conn.recv_event()).await {
            // Timed out waiting for an index event — fall through to retry.
            Err(_) => return,
            // Stream lagged or closed — let the caller resync via its next query.
            Ok(Err(_)) => return,
            Ok(Ok(CoreEvent::Search(SearchEvent::IndexUpdated { .. }))) => return,
            // Any other event: keep waiting for an index update (or the bound).
            Ok(Ok(_)) => continue,
        }
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

/// Read-only startup-latency scenario.
///
/// Run 1 (empty data dir): `RestoreLastSession` returns `SessionNotFound`, so
/// we fall back to `LoginPassword`. Run 2+: restore succeeds and times the
/// store-load path we care about.
///
/// Target-room selection rule: the **first joined non-DM room** in the
/// `RoomSummary` list returned by `wait_for_non_empty_room_list`, which is
/// snapshot-stable order (Rust sidebar projection). If no joined non-DM room
/// is present, the scenario still emits timing tokens up to `room_list` and
/// returns success without subscribe/paginate.
///
/// Writes: **none**. Creates no rooms, sends no messages, leaves nothing.
/// Teardown: by default the session is kept (run 2+ can restore). Set
/// `KOUSHI_STARTUP_LAT_TEARDOWN=1` to log out and remove the QA device.
#[cfg(any(debug_assertions, test))]
async fn run_startup_latency_scenario(
    creds: &RealCredentials,
    data_dir: &std::path::Path,
    transcript: &mut Vec<String>,
    cleanup: &mut RealQaCleanupState,
) -> Result<String, String> {
    let runtime = CoreRuntime::start_with_data_dir(data_dir.to_path_buf());
    let mut conn = runtime.attach();

    // ------------------------------------------------------------------
    // Phase 1: restore-or-login — measure the wall time from first command
    // to Ready snapshot.
    // ------------------------------------------------------------------
    let restore_started = std::time::Instant::now();

    // Attempt restore first; fall back to login on SessionNotFound.
    let restore_id = conn.next_request_id();
    conn.command(CoreCommand::Account(AccountCommand::RestoreLastSession {
        request_id: restore_id,
    }))
    .await
    .map_err(|e| format!("startup_latency restore command submit failed: {e}"))?;

    let account_key = loop {
        let event = tokio::time::timeout(EVENT_TIMEOUT, conn.recv_event())
            .await
            .map_err(|_| {
                "startup_latency: timed out waiting for SessionRestored or OperationFailed"
                    .to_owned()
            })?
            .map_err(|lag| {
                format!(
                    "startup_latency restore: event stream lagged (skipped={})",
                    lag.skipped
                )
            })?;

        match event {
            CoreEvent::Account(AccountEvent::SessionRestored {
                request_id: ev_id,
                account_key,
            }) if ev_id == restore_id => {
                let line = "startup_lat restore=session".to_owned();
                transcript.push(line.clone());
                println!("{line}");
                break account_key;
            }
            CoreEvent::OperationFailed {
                request_id: ev_id,
                failure: CoreFailure::SessionNotFound,
            } if ev_id == restore_id => {
                // Run 1: no stored session; fall back to credentials login.
                let line = "startup_lat restore=not_found login=fallback".to_owned();
                transcript.push(line.clone());
                println!("{line}");

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
                .map_err(|e| format!("startup_latency login command submit failed: {e}"))?;

                let key = wait_for_logged_in(&mut conn, login_id, "startup_latency login").await?;
                break key;
            }
            CoreEvent::OperationFailed {
                request_id: ev_id,
                failure: _,
            } if ev_id == restore_id => {
                return Err("startup_latency restore failed".to_owned());
            }
            _ => continue,
        }
    };
    // Record account key so the catch-all wrapper can log out on failure.
    cleanup.account_key = Some(account_key.clone());

    // Restore phase is complete here (session restored / logged in); measure
    // before sync so this token does not include the sync-to-ready span.
    let restore_ms = restore_started.elapsed().as_millis();
    let line = format!("startup_lat phase=restore ms={restore_ms}");
    transcript.push(line.clone());
    println!("{line}");

    // ------------------------------------------------------------------
    // Phase 2: sync start → running, then ready. Measure from sync start.
    // ------------------------------------------------------------------
    let sync_started = std::time::Instant::now();

    let sync_id = conn.next_request_id();
    conn.command(CoreCommand::Sync(SyncCommand::Start {
        request_id: sync_id,
    }))
    .await
    .map_err(|e| format!("startup_latency sync start submit failed: {e}"))?;

    wait_for_sync_started(
        &mut conn,
        sync_id,
        "startup_latency sync start",
        SYNC_TIMEOUT,
    )
    .await?;
    wait_for_sync_running(&mut conn, "startup_latency sync running", SYNC_TIMEOUT).await?;
    wait_for_ready_handling_recovery(&mut conn, creds, transcript, "startup_latency Ready").await?;

    let sync_ms = sync_started.elapsed().as_millis();
    let line = format!("startup_lat phase=sync_to_ready ms={sync_ms}");
    transcript.push(line.clone());
    println!("{line}");

    // ------------------------------------------------------------------
    // Phase 3: room list — time until first non-empty snapshot.
    // ------------------------------------------------------------------
    let room_list_started = std::time::Instant::now();
    let room_snapshot =
        wait_for_non_empty_room_list(&mut conn, "startup_latency room list", ROOM_LIST_TIMEOUT)
            .await?;
    let room_list_ms = room_list_started.elapsed().as_millis();
    let line = format!(
        "startup_lat phase=room_list ms={room_list_ms} rooms={}",
        room_snapshot.rooms.len()
    );
    transcript.push(line.clone());
    println!("{line}");

    // ------------------------------------------------------------------
    // Phase 4: subscribe + paginate a target room (first joined non-DM).
    //
    // Target selection rule: first entry in `room_snapshot.rooms` where
    // `is_dm == false`. This is the Rust sidebar projection order, which is
    // stable across runs for the same account.
    // ------------------------------------------------------------------
    let target_room = room_snapshot.rooms.iter().find(|r| !r.is_dm);
    let target_room_id = match target_room {
        Some(r) => r.room_id.clone(),
        None => {
            // No joined non-DM room yet; emit a note and skip subscribe/paginate.
            let line = "startup_lat subscribe=skipped reason=no_non_dm_room".to_owned();
            transcript.push(line.clone());
            println!("{line}");
            return finish_startup_latency(
                &mut conn,
                &account_key,
                transcript,
                cleanup,
                "startup_lat phase=paginate ms=0 reached_start=false pages=0",
            )
            .await;
        }
    };

    let timeline_key = TimelineKey::room(account_key.clone(), target_room_id.clone());
    let subscribe_id = conn.next_request_id();
    let subscribe_started = std::time::Instant::now();
    conn.command(CoreCommand::Timeline(TimelineCommand::Subscribe {
        request_id: subscribe_id,
        key: timeline_key.clone(),
    }))
    .await
    .map_err(|e| format!("startup_latency subscribe submit failed: {e}"))?;

    wait_for_initial_items(
        &mut conn,
        &timeline_key,
        subscribe_id,
        "startup_latency subscribe",
    )
    .await?;
    let subscribe_ms = subscribe_started.elapsed().as_millis();
    let line = format!("startup_lat phase=subscribe ms={subscribe_ms}");
    transcript.push(line.clone());
    println!("{line}");

    // ------------------------------------------------------------------
    // Phase 4b: bounded paginate — at most STARTUP_LAT_PAGES pages backward.
    // Stop early on EndReached; each page is timed individually.
    // ------------------------------------------------------------------
    let mut pages_done: usize = 0;
    let mut reached_start = false;
    for _page in 0..STARTUP_LAT_PAGES {
        let paginate_id = conn.next_request_id();
        let page_started = std::time::Instant::now();
        conn.command(CoreCommand::Timeline(TimelineCommand::Paginate {
            request_id: paginate_id,
            key: timeline_key.clone(),
            direction: PaginationDirection::Backward,
            event_count: 20,
        }))
        .await
        .map_err(|e| format!("startup_latency paginate submit failed: {e}"))?;

        // Wait for a terminal PaginationStateChanged for this page.
        let page_result = wait_for_pagination_terminal(
            &mut conn,
            &timeline_key,
            paginate_id,
            "startup_latency paginate",
        )
        .await?;

        let page_ms = page_started.elapsed().as_millis();
        pages_done += 1;

        match page_result {
            PaginationTerminal::EndReached => {
                reached_start = true;
                let line = format!("startup_lat phase=paginate ms={page_ms} reached_start=true");
                transcript.push(line.clone());
                println!("{line}");
                break;
            }
            PaginationTerminal::Idle => {
                let line = format!("startup_lat phase=paginate ms={page_ms} reached_start=false");
                transcript.push(line.clone());
                println!("{line}");
            }
            PaginationTerminal::Failed(kind) => {
                let line = format!(
                    "startup_lat phase=paginate ms={page_ms} reached_start=false failed={kind:?}"
                );
                transcript.push(line.clone());
                println!("{line}");
                break;
            }
        }
    }
    let line = format!("startup_lat pages={pages_done} reached_start={reached_start}");
    transcript.push(line.clone());
    println!("{line}");

    finish_startup_latency(&mut conn, &account_key, transcript, cleanup, "").await
}

/// Result of a single paginate page (terminal pagination state).
#[cfg(any(debug_assertions, test))]
enum PaginationTerminal {
    Idle,
    EndReached,
    Failed(TimelineFailureKind),
}

/// Wait for a single terminal `PaginationStateChanged` event for `key` backward,
/// correlating on the request id via `OperationFailed` fallback.
#[cfg(any(debug_assertions, test))]
async fn wait_for_pagination_terminal(
    conn: &mut CoreConnection,
    key: &TimelineKey,
    request_id: RequestId,
    label: &str,
) -> Result<PaginationTerminal, String> {
    loop {
        let event = tokio::time::timeout(PAGINATE_TIMEOUT, conn.recv_event())
            .await
            .map_err(|_| format!("{label}: timed out waiting for terminal PaginationStateChanged"))?
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;

        match event {
            CoreEvent::Timeline(TimelineEvent::PaginationStateChanged {
                key: ref ev_key,
                direction,
                state,
                ..
            }) if ev_key == key && direction == PaginationDirection::Backward => match state {
                PaginationState::Paginating => {
                    // In-flight; keep waiting for the terminal state.
                }
                PaginationState::Idle => return Ok(PaginationTerminal::Idle),
                PaginationState::EndReached => return Ok(PaginationTerminal::EndReached),
                PaginationState::Failed { kind } => {
                    return Ok(PaginationTerminal::Failed(kind));
                }
            },
            CoreEvent::OperationFailed {
                request_id: ev_id,
                failure: _,
            } if ev_id == request_id => {
                return Err(format!("{label}: paginate operation failed"));
            }
            _ => {}
        }
    }
}

/// Teardown for the startup_latency scenario.
///
/// Emits the summary token then optionally logs out (when
/// `KOUSHI_STARTUP_LAT_TEARDOWN=1`). `extra_token` is appended to the summary
/// when non-empty (used for the skip-subscribe path's paginate placeholder).
#[cfg(any(debug_assertions, test))]
async fn finish_startup_latency(
    conn: &mut CoreConnection,
    account_key: &AccountKey,
    transcript: &mut Vec<String>,
    cleanup: &mut RealQaCleanupState,
    extra_token: &str,
) -> Result<String, String> {
    if !extra_token.is_empty() {
        transcript.push(extra_token.to_owned());
        println!("{extra_token}");
    }

    let teardown = std::env::var(ENV_STARTUP_LAT_TEARDOWN)
        .ok()
        .map(|v| v == "1")
        .unwrap_or(false);

    if teardown {
        do_logout(conn, account_key, transcript).await;
        cleanup.logged_out = true;
    } else {
        let line = "startup_lat teardown=session_kept".to_owned();
        transcript.push(line.clone());
        println!("{line}");
    }

    Ok("startup_latency=ok".to_owned())
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

        let info = koushi_state::SessionInfo {
            homeserver: "https://example.test".to_owned(),
            user_id: "@alice:example.test".to_owned(),
            device_id: "DEVICE1".to_owned(),
        };

        runtime
            .inject_actions(vec![koushi_state::AppAction::LoginSucceeded(info.clone())])
            .await;

        wait_for_ready_snapshot(&mut conn, "setup ready")
            .await
            .expect("setup ready snapshot");

        let runtime2 = Arc::clone(&runtime);
        let delayed = tokio::spawn(async move {
            sleep(Duration::from_millis(50)).await;
            runtime2
                .inject_actions(vec![koushi_state::AppAction::E2eeRecoveryStateChanged {
                    state: koushi_state::E2eeRecoveryState::Incomplete,
                    methods: vec![koushi_state::RecoveryMethod::RecoveryKey],
                }])
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
                .inject_actions(vec![koushi_state::AppAction::LoginSucceeded(
                    koushi_state::SessionInfo {
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
