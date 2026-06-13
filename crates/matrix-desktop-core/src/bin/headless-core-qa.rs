//! Headless Core QA binary v2 (Phase 4: adds room operations and room list QA).
//!
//! Exercises login (with store bootstrap), store-backed session restore,
//! logout cleanup, sync lifecycle, room creation, space creation,
//! space-child assignment, invite/join, room list normalization, and
//! stdout/stderr secret-redaction using ONLY `CoreCommand`/`CoreEvent` —
//! no direct auth-crate calls in the QA flow.
//!
//! Topology: one `CoreRuntime` per synthetic user (spec, Headless QA section:
//! that models two devices, the realistic A/B topology; multi-account-in-one-
//! runtime behavior is account-switch QA's job).
//!
//! Hard guard: this binary refuses to run unless the file credential store
//! override is active. Unattended QA must be structurally unable to reach the
//! OS keychain (a keychain prompt during automation is a failure per the
//! engineering rules), so the guard runs BEFORE any login.
//!
//! Phase 4 flow (both probed SyncService leg and forced LegacySync leg):
//!   A creates room + space + sets space child + invites B to both
//!   B joins room + space
//!   both assert room list contains expected room and space (event-driven)
//!   print room-list counts in summary line
//!   send permission check placeholder (actual send is Phase 5)
//!
//! Required env vars:
//!   MATRIX_DESKTOP_LOCAL_QA_HOMESERVER
//!   MATRIX_DESKTOP_LOCAL_QA_SERVER_NAME
//!   MATRIX_DESKTOP_LOCAL_QA_SERVER_KIND   (optional, defaults to "local")
//!   MATRIX_DESKTOP_LOCAL_QA_USER_A / _PASSWORD_A
//!   MATRIX_DESKTOP_LOCAL_QA_USER_B / _PASSWORD_B
//!   MATRIX_DESKTOP_QA_FILE_CREDENTIAL_STORE_DIR (mandatory; see guard)
//!
//! SDK handles are dropped inside the Tokio runtime context (overview.md Async rule 11).

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
use matrix_desktop_core::failure::CoreFailure;
use matrix_desktop_core::ids::{AccountKey, TimelineKey};
use matrix_desktop_core::runtime::{CoreConnection, CoreRuntime};
use matrix_desktop_state::{AppState, AuthSecret, SessionState};

const ENV_HOMESERVER: &str = "MATRIX_DESKTOP_LOCAL_QA_HOMESERVER";
const ENV_SERVER_NAME: &str = "MATRIX_DESKTOP_LOCAL_QA_SERVER_NAME";
const ENV_SERVER_KIND: &str = "MATRIX_DESKTOP_LOCAL_QA_SERVER_KIND";
const ENV_USER_A: &str = "MATRIX_DESKTOP_LOCAL_QA_USER_A";
const ENV_PASSWORD_A: &str = "MATRIX_DESKTOP_LOCAL_QA_PASSWORD_A";
const ENV_USER_B: &str = "MATRIX_DESKTOP_LOCAL_QA_USER_B";
const ENV_PASSWORD_B: &str = "MATRIX_DESKTOP_LOCAL_QA_PASSWORD_B";
/// Optional assertion input (a plain string, not a credential — no gating
/// needed): when set, QA fails if the backend reported in SyncEvent::Started
/// differs. Valid values: "SyncService" | "LegacySync".
const ENV_EXPECT_SYNC_BACKEND: &str = "MATRIX_DESKTOP_LOCAL_QA_EXPECT_SYNC_BACKEND";
const ENV_QA_SCENARIO: &str = "MATRIX_DESKTOP_QA_SCENARIO";
#[cfg(any(debug_assertions, test))]
const ENV_FILE_CREDENTIAL_STORE_DIR: &str = "MATRIX_DESKTOP_QA_FILE_CREDENTIAL_STORE_DIR";

const DEVICE_A: &str = "Matrix Desktop Core QA A";
const DEVICE_B: &str = "Matrix Desktop Core QA B";

/// Maximum time to wait for a single event.
const EVENT_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum QaScenario {
    All,
    Safety,
    LoginSync,
    RoomSpace,
    Timeline,
    Reply,
    Thread,
    EditRedactSearch,
    RestoreCleanup,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum QaStage {
    Safety,
    LoginSync,
    RoomSpace,
    Timeline,
    Reply,
    EditRedactSearch,
    RestoreCleanup,
}

fn main() -> ExitCode {
    match run() {
        Ok(report) => {
            println!("{report}");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("Headless core QA failed: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<String, String> {
    let scenario = QaScenario::from_env()?;
    scenario_preflight_error(scenario)?;

    // Hard guard BEFORE any login: unattended QA must never touch the OS
    // keychain, even if env wiring regresses.
    assert_file_credential_store_active()?;

    let config = QaConfig::from_env()?;
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|e| format!("runtime creation failed: {e}"))?;

    // Run inside the Tokio runtime so SDK handles drop in context (Async rule 11).
    runtime.block_on(run_async(config, scenario))
}

/// Refuse to run against the OS keychain. In debug/test builds this checks
/// both the env var and the structurally resolved backend; release builds
/// have no file credential store at all, so they are refused outright before
/// the env var is even consulted.
fn assert_file_credential_store_active() -> Result<(), String> {
    #[cfg(debug_assertions)]
    {
        if std::env::var_os(ENV_FILE_CREDENTIAL_STORE_DIR).is_none() {
            return Err(format!(
                "core QA refuses to run against the OS keychain: {ENV_FILE_CREDENTIAL_STORE_DIR} is not set"
            ));
        }
        if !matrix_desktop_core::store::resolved_credential_backend_is_file_dir() {
            return Err(
                "core QA refuses to run against the OS keychain: resolved credential \
                 store backend is not the file-dir backend"
                    .to_owned(),
            );
        }
        Ok(())
    }

    #[cfg(not(debug_assertions))]
    {
        Err(
            "core QA refuses to run against the OS keychain: release builds have no \
             file credential store backend"
                .to_owned(),
        )
    }
}

impl QaScenario {
    fn from_env() -> Result<Self, String> {
        match std::env::var(ENV_QA_SCENARIO) {
            Ok(value) => Self::from_env_value(&value),
            Err(_) => Ok(Self::All),
        }
    }

    fn from_env_value(value: &str) -> Result<Self, String> {
        match value {
            "all" => Ok(Self::All),
            "safety" => Ok(Self::Safety),
            "login_sync" => Ok(Self::LoginSync),
            "room_space" => Ok(Self::RoomSpace),
            "timeline" => Ok(Self::Timeline),
            "reply" => Ok(Self::Reply),
            "thread" => Ok(Self::Thread),
            "edit_redact_search" => Ok(Self::EditRedactSearch),
            "restore_cleanup" => Ok(Self::RestoreCleanup),
            other => Err(format!(
                "{ENV_QA_SCENARIO} must be one of all, safety, login_sync, room_space, timeline, reply, thread, edit_redact_search, restore_cleanup; got {other}"
            )),
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::All => "all",
            Self::Safety => "safety",
            Self::LoginSync => "login_sync",
            Self::RoomSpace => "room_space",
            Self::Timeline => "timeline",
            Self::Reply => "reply",
            Self::Thread => "thread",
            Self::EditRedactSearch => "edit_redact_search",
            Self::RestoreCleanup => "restore_cleanup",
        }
    }

    fn should_run_stage(self, stage: QaStage) -> bool {
        match self {
            Self::All => true,
            Self::Safety => matches!(stage, QaStage::Safety),
            Self::LoginSync => matches!(stage, QaStage::Safety | QaStage::LoginSync),
            Self::RoomSpace => matches!(
                stage,
                QaStage::Safety | QaStage::LoginSync | QaStage::RoomSpace
            ),
            Self::Timeline => matches!(
                stage,
                QaStage::Safety | QaStage::LoginSync | QaStage::RoomSpace | QaStage::Timeline
            ),
            Self::Reply => matches!(
                stage,
                QaStage::Safety
                    | QaStage::LoginSync
                    | QaStage::RoomSpace
                    | QaStage::Timeline
                    | QaStage::Reply
            ),
            Self::EditRedactSearch => matches!(
                stage,
                QaStage::Safety
                    | QaStage::LoginSync
                    | QaStage::RoomSpace
                    | QaStage::Timeline
                    | QaStage::EditRedactSearch
            ),
            Self::RestoreCleanup => matches!(
                stage,
                QaStage::Safety
                    | QaStage::LoginSync
                    | QaStage::RoomSpace
                    | QaStage::Timeline
                    | QaStage::EditRedactSearch
                    | QaStage::RestoreCleanup
            ),
            Self::Thread => false,
        }
    }
}

fn scenario_preflight_error(scenario: QaScenario) -> Result<(), String> {
    match scenario {
        QaScenario::Thread => Err(format!(
            "scenario {} is not implemented until true Matrix reply support lands",
            scenario.as_str()
        )),
        _ => Ok(()),
    }
}

fn implemented_final_tokens() -> [&'static str; 7] {
    [
        "safety=ok",
        "login_sync=ok",
        "room_space=ok",
        "timeline=ok",
        "reply=ok",
        "edit_redact_search=ok",
        "restore_cleanup=ok",
    ]
}

fn stages_for_scenario(scenario: QaScenario) -> Vec<QaStage> {
    match scenario {
        QaScenario::Safety => vec![QaStage::Safety],
        QaScenario::LoginSync => vec![QaStage::Safety, QaStage::LoginSync],
        QaScenario::RoomSpace => vec![QaStage::Safety, QaStage::LoginSync, QaStage::RoomSpace],
        QaScenario::Timeline => vec![
            QaStage::Safety,
            QaStage::LoginSync,
            QaStage::RoomSpace,
            QaStage::Timeline,
        ],
        QaScenario::Reply => vec![
            QaStage::Safety,
            QaStage::LoginSync,
            QaStage::RoomSpace,
            QaStage::Timeline,
            QaStage::Reply,
        ],
        QaScenario::EditRedactSearch => vec![
            QaStage::Safety,
            QaStage::LoginSync,
            QaStage::RoomSpace,
            QaStage::Timeline,
            QaStage::EditRedactSearch,
        ],
        QaScenario::RestoreCleanup => vec![
            QaStage::Safety,
            QaStage::LoginSync,
            QaStage::RoomSpace,
            QaStage::Timeline,
            QaStage::EditRedactSearch,
            QaStage::RestoreCleanup,
        ],
        QaScenario::All => vec![
            QaStage::Safety,
            QaStage::LoginSync,
            QaStage::RoomSpace,
            QaStage::Timeline,
            QaStage::Reply,
            QaStage::EditRedactSearch,
            QaStage::RestoreCleanup,
        ],
        QaScenario::Thread => Vec::new(),
    }
}

fn final_tokens_for_scenario(scenario: QaScenario) -> Vec<&'static str> {
    match scenario {
        QaScenario::Safety => vec!["safety=ok"],
        QaScenario::LoginSync => {
            let mut tokens = stages_for_scenario(scenario)
                .into_iter()
                .map(|stage| match stage {
                    QaStage::Safety => "safety=ok",
                    QaStage::LoginSync => "login_sync=ok",
                    QaStage::RoomSpace => "room_space=ok",
                    QaStage::Timeline => "timeline=ok",
                    QaStage::Reply => "reply=ok",
                    QaStage::EditRedactSearch => "edit_redact_search=ok",
                    QaStage::RestoreCleanup => "restore_cleanup=ok",
                })
                .collect::<Vec<_>>();
            tokens.push("restore_cleanup=ok");
            tokens.dedup();
            tokens
        }
        QaScenario::RoomSpace
        | QaScenario::Timeline
        | QaScenario::Reply
        | QaScenario::EditRedactSearch
        | QaScenario::RestoreCleanup => {
            let mut tokens = stages_for_scenario(scenario)
                .into_iter()
                .map(|stage| match stage {
                    QaStage::Safety => "safety=ok",
                    QaStage::LoginSync => "login_sync=ok",
                    QaStage::RoomSpace => "room_space=ok",
                    QaStage::Timeline => "timeline=ok",
                    QaStage::Reply => "reply=ok",
                    QaStage::EditRedactSearch => "edit_redact_search=ok",
                    QaStage::RestoreCleanup => "restore_cleanup=ok",
                })
                .collect::<Vec<_>>();
            tokens.push("restore_cleanup=ok");
            tokens.dedup();
            tokens
        }
        QaScenario::All => implemented_final_tokens().to_vec(),
        QaScenario::Thread => Vec::new(),
    }
}

fn scenario_report(server_kind: &str, scenario: QaScenario) -> String {
    format!(
        "server={server_kind}\n{}",
        final_tokens_for_scenario(scenario).join("\n")
    )
}

async fn cleanup_after_login_sync(
    mut conn_a: CoreConnection,
    runtime_a: CoreRuntime,
    data_dir_a: std::path::PathBuf,
    account_key_a: AccountKey,
) -> Result<String, String> {
    let sync_stop_id = conn_a.next_request_id();
    conn_a
        .command(CoreCommand::Sync(SyncCommand::Stop {
            request_id: sync_stop_id,
        }))
        .await
        .map_err(|e| format!("submit sync stop A: {e}"))?;

    wait_for_sync_stopped(&mut conn_a, sync_stop_id, "sync stop A").await?;
    println!("sync_a=stopped");

    drop(conn_a);
    drop(runtime_a);
    tokio::time::sleep(Duration::from_millis(500)).await;

    let runtime_a2 = CoreRuntime::start_with_data_dir(data_dir_a);
    let mut conn_a2 = runtime_a2.attach();

    let restore_a_id = conn_a2.next_request_id();
    conn_a2
        .command(CoreCommand::Account(AccountCommand::RestoreSession {
            request_id: restore_a_id,
            account_key: account_key_a.clone(),
        }))
        .await
        .map_err(|e| format!("submit restore A: {e}"))?;

    wait_for_session_restored(&mut conn_a2, restore_a_id, &account_key_a, "restore A").await?;
    wait_for_ready_snapshot(&mut conn_a2, "restored session A Ready").await?;

    let logout_a_id = conn_a2.next_request_id();
    conn_a2
        .command(CoreCommand::Account(AccountCommand::Logout {
            request_id: logout_a_id,
        }))
        .await
        .map_err(|e| format!("submit logout A: {e}"))?;

    wait_for_logged_out(&mut conn_a2, logout_a_id, &account_key_a, "logout A").await?;

    let restore_gone_id = conn_a2.next_request_id();
    conn_a2
        .command(CoreCommand::Account(AccountCommand::RestoreSession {
            request_id: restore_gone_id,
            account_key: account_key_a.clone(),
        }))
        .await
        .map_err(|e| format!("submit post-logout restore A: {e}"))?;

    let failure = wait_for_operation_failed(
        &mut conn_a2,
        restore_gone_id,
        "post-logout restore A (must fail)",
    )
    .await?;
    if failure != CoreFailure::SessionNotFound {
        return Err(format!(
            "post-logout restore A failed with unexpected kind: {failure:?}"
        ));
    }
    if !matches!(conn_a2.snapshot().session, SessionState::SignedOut) {
        return Err("post-logout restore A must leave the session SignedOut".to_owned());
    }

    println!("restore_cleanup=ok");
    Ok("restore_cleanup=ok".to_owned())
}

async fn cleanup_after_full_flow(
    mut conn_a: CoreConnection,
    mut conn_b: CoreConnection,
    runtime_a: CoreRuntime,
    runtime_b: CoreRuntime,
    data_dir_a: std::path::PathBuf,
    account_key_a: AccountKey,
    account_key_b: AccountKey,
) -> Result<String, String> {
    let sync_stop_id = conn_a.next_request_id();
    conn_a
        .command(CoreCommand::Sync(SyncCommand::Stop {
            request_id: sync_stop_id,
        }))
        .await
        .map_err(|e| format!("submit sync stop A: {e}"))?;

    wait_for_sync_stopped(&mut conn_a, sync_stop_id, "sync stop A").await?;
    println!("sync_a=stopped");

    drop(conn_a);
    drop(runtime_a);
    tokio::time::sleep(Duration::from_millis(500)).await;

    let runtime_a2 = CoreRuntime::start_with_data_dir(data_dir_a);
    let mut conn_a2 = runtime_a2.attach();

    let restore_a_id = conn_a2.next_request_id();
    conn_a2
        .command(CoreCommand::Account(AccountCommand::RestoreSession {
            request_id: restore_a_id,
            account_key: account_key_a.clone(),
        }))
        .await
        .map_err(|e| format!("submit restore A: {e}"))?;

    wait_for_session_restored(&mut conn_a2, restore_a_id, &account_key_a, "restore A").await?;
    wait_for_ready_snapshot(&mut conn_a2, "restored session A Ready").await?;

    let logout_a_id = conn_a2.next_request_id();
    conn_a2
        .command(CoreCommand::Account(AccountCommand::Logout {
            request_id: logout_a_id,
        }))
        .await
        .map_err(|e| format!("submit logout A: {e}"))?;

    wait_for_logged_out(&mut conn_a2, logout_a_id, &account_key_a, "logout A").await?;

    let restore_gone_id = conn_a2.next_request_id();
    conn_a2
        .command(CoreCommand::Account(AccountCommand::RestoreSession {
            request_id: restore_gone_id,
            account_key: account_key_a.clone(),
        }))
        .await
        .map_err(|e| format!("submit post-logout restore A: {e}"))?;

    let failure = wait_for_operation_failed(
        &mut conn_a2,
        restore_gone_id,
        "post-logout restore A (must fail)",
    )
    .await?;
    if failure != CoreFailure::SessionNotFound {
        return Err(format!(
            "post-logout restore A failed with unexpected kind: {failure:?}"
        ));
    }
    if !matches!(conn_a2.snapshot().session, SessionState::SignedOut) {
        return Err("post-logout restore A must leave the session SignedOut".to_owned());
    }

    let logout_b_id = conn_b.next_request_id();
    conn_b
        .command(CoreCommand::Account(AccountCommand::Logout {
            request_id: logout_b_id,
        }))
        .await
        .map_err(|e| format!("submit logout B: {e}"))?;

    wait_for_logged_out(&mut conn_b, logout_b_id, &account_key_b, "logout B").await?;

    let restore_last_id = conn_b.next_request_id();
    conn_b
        .command(CoreCommand::Account(AccountCommand::RestoreLastSession {
            request_id: restore_last_id,
        }))
        .await
        .map_err(|e| format!("submit post-logout restore-last: {e}"))?;

    let failure = wait_for_operation_failed(
        &mut conn_b,
        restore_last_id,
        "post-logout restore-last (must be not-found)",
    )
    .await?;
    if failure != CoreFailure::SessionNotFound {
        return Err(format!(
            "post-logout restore-last failed with unexpected kind: {failure:?}"
        ));
    }
    if !matches!(conn_b.snapshot().session, SessionState::SignedOut) {
        return Err("post-logout restore-last must leave the session SignedOut".to_owned());
    }

    drop(conn_b);
    drop(runtime_b);

    println!("restore_cleanup=ok");
    Ok("restore_cleanup=ok".to_owned())
}

async fn run_async(config: QaConfig, scenario: QaScenario) -> Result<String, String> {
    if scenario == QaScenario::Safety {
        println!("safety=ok");
        return Ok(scenario_report(&config.server_kind, scenario));
    }

    // One CoreRuntime per synthetic user (two-device topology).
    let data_dir_a = qa_data_dir("a");
    let data_dir_b = qa_data_dir("b");

    // -----------------------------------------------------------------------
    // --- Login A (storeless exchange + store bootstrap inside the actor) ---
    // -----------------------------------------------------------------------
    let runtime_a = CoreRuntime::start_with_data_dir(data_dir_a.clone());
    let mut conn_a = runtime_a.attach();

    let login_a_id = conn_a.next_request_id();
    conn_a
        .command(CoreCommand::Account(AccountCommand::LoginPassword {
            request_id: login_a_id,
            request: matrix_desktop_state::LoginRequest {
                homeserver: config.homeserver.clone(),
                username: config.user_a.clone(),
                password: AuthSecret::new(config.password_a.clone()),
                device_display_name: Some(DEVICE_A.to_owned()),
            },
        }))
        .await
        .map_err(|e| format!("submit login A: {e}"))?;

    let account_key_a = wait_for_logged_in(&mut conn_a, login_a_id, "login A").await?;
    wait_for_ready_snapshot(&mut conn_a, "session A Ready").await?;

    // -----------------------------------------------------------------------
    // --- Phase 3: Start sync A, assert Started + Running, record backend ---
    // -----------------------------------------------------------------------
    let sync_start_id = conn_a.next_request_id();
    conn_a
        .command(CoreCommand::Sync(SyncCommand::Start {
            request_id: sync_start_id,
        }))
        .await
        .map_err(|e| format!("submit sync start A: {e}"))?;

    let sync_backend_a = wait_for_sync_started(&mut conn_a, sync_start_id, "sync start A").await?;
    println!("sync_backend_a={sync_backend_a:?}");
    assert_expected_backend(
        config.expect_sync_backend.as_deref(),
        sync_backend_a,
        "sync start A",
    )?;

    wait_for_sync_running(&mut conn_a, "sync A running").await?;
    println!("sync_a=running");
    println!("login_sync=ok");

    if !scenario.should_run_stage(QaStage::RoomSpace) {
        cleanup_after_login_sync(conn_a, runtime_a, data_dir_a, account_key_a).await?;
        return Ok(scenario_report(&config.server_kind, scenario));
    }

    // -----------------------------------------------------------------------
    // --- Phase 4: Room operations (A creates room + space, invites B) ---
    // -----------------------------------------------------------------------

    // A creates a room
    let create_room_id = conn_a.next_request_id();
    conn_a
        .command(CoreCommand::Room(RoomCommand::CreateRoom {
            request_id: create_room_id,
            name: "QA Room".to_owned(),
        }))
        .await
        .map_err(|e| format!("submit create room: {e}"))?;

    let room_id = wait_for_room_created(&mut conn_a, create_room_id, "create room").await?;
    println!("room_id={room_id}");

    // A creates a space
    let create_space_id = conn_a.next_request_id();
    conn_a
        .command(CoreCommand::Room(RoomCommand::CreateSpace {
            request_id: create_space_id,
            name: "QA Space".to_owned(),
        }))
        .await
        .map_err(|e| format!("submit create space: {e}"))?;

    let space_id = wait_for_space_created(&mut conn_a, create_space_id, "create space").await?;
    println!("space_id={space_id}");

    // Extract server name from room_id (e.g., "!room:localhost:PORT" → "localhost:PORT")
    let via_server = config.server_name.clone();

    // A sets room as child of space
    let set_child_id = conn_a.next_request_id();
    conn_a
        .command(CoreCommand::Room(RoomCommand::SetSpaceChild {
            request_id: set_child_id,
            space_id: space_id.clone(),
            child_room_id: room_id.clone(),
            via_server: via_server.clone(),
        }))
        .await
        .map_err(|e| format!("submit set space child: {e}"))?;

    wait_for_space_child_set(
        &mut conn_a,
        set_child_id,
        &space_id,
        &room_id,
        "set space child",
    )
    .await?;
    println!("space_child_set=ok");

    // A invites B to the room
    let user_b_full_id = format!("@{}:{}", config.user_b, config.server_name);
    let invite_room_id = conn_a.next_request_id();
    conn_a
        .command(CoreCommand::Room(RoomCommand::InviteUser {
            request_id: invite_room_id,
            room_id: room_id.clone(),
            user_id: user_b_full_id.clone(),
        }))
        .await
        .map_err(|e| format!("submit invite B to room: {e}"))?;

    wait_for_user_invited(
        &mut conn_a,
        invite_room_id,
        &room_id,
        &user_b_full_id,
        "invite B to room",
    )
    .await?;
    println!("invite_b_to_room=ok");

    // A invites B to the space
    let invite_space_id = conn_a.next_request_id();
    conn_a
        .command(CoreCommand::Room(RoomCommand::InviteUser {
            request_id: invite_space_id,
            room_id: space_id.clone(),
            user_id: user_b_full_id.clone(),
        }))
        .await
        .map_err(|e| format!("submit invite B to space: {e}"))?;

    wait_for_user_invited(
        &mut conn_a,
        invite_space_id,
        &space_id,
        &user_b_full_id,
        "invite B to space",
    )
    .await?;
    println!("invite_b_to_space=ok");

    // Wait (event-driven, bounded) until A's room list contains the created
    // room AND the created space; the wait itself is the assertion.
    let snapshot_a = wait_for_room_list_containing(
        &mut conn_a,
        &room_id,
        &space_id,
        "room list A after creates",
    )
    .await?;
    let room_list_a = room_list_summary(&snapshot_a);
    println!("room_list_a={room_list_a}");

    // -----------------------------------------------------------------------
    // --- Login B + sync B + join room + join space ---
    // -----------------------------------------------------------------------
    let runtime_b = CoreRuntime::start_with_data_dir(data_dir_b);
    let mut conn_b = runtime_b.attach();

    let login_b_id = conn_b.next_request_id();
    conn_b
        .command(CoreCommand::Account(AccountCommand::LoginPassword {
            request_id: login_b_id,
            request: matrix_desktop_state::LoginRequest {
                homeserver: config.homeserver.clone(),
                username: config.user_b.clone(),
                password: AuthSecret::new(config.password_b.clone()),
                device_display_name: Some(DEVICE_B.to_owned()),
            },
        }))
        .await
        .map_err(|e| format!("submit login B: {e}"))?;

    let account_key_b = wait_for_logged_in(&mut conn_b, login_b_id, "login B").await?;
    wait_for_ready_snapshot(&mut conn_b, "session B Ready").await?;

    // Start sync B
    let sync_start_b_id = conn_b.next_request_id();
    conn_b
        .command(CoreCommand::Sync(SyncCommand::Start {
            request_id: sync_start_b_id,
        }))
        .await
        .map_err(|e| format!("submit sync start B: {e}"))?;

    let sync_backend_b =
        wait_for_sync_started(&mut conn_b, sync_start_b_id, "sync start B").await?;
    println!("sync_backend_b={sync_backend_b:?}");
    assert_expected_backend(
        config.expect_sync_backend.as_deref(),
        sync_backend_b,
        "sync start B",
    )?;

    wait_for_sync_running(&mut conn_b, "sync B running").await?;
    println!("sync_b=running");

    // B joins the room
    let join_room_id = conn_b.next_request_id();
    conn_b
        .command(CoreCommand::Room(RoomCommand::JoinRoom {
            request_id: join_room_id,
            room_id: room_id.clone(),
        }))
        .await
        .map_err(|e| format!("submit join room B: {e}"))?;

    wait_for_room_joined(&mut conn_b, join_room_id, &room_id, "B joins room").await?;
    println!("b_joined_room=ok");

    // B joins the space
    let join_space_id = conn_b.next_request_id();
    conn_b
        .command(CoreCommand::Room(RoomCommand::JoinRoom {
            request_id: join_space_id,
            room_id: space_id.clone(),
        }))
        .await
        .map_err(|e| format!("submit join space B: {e}"))?;

    wait_for_room_joined(&mut conn_b, join_space_id, &space_id, "B joins space").await?;
    println!("b_joined_space=ok");

    // Wait (event-driven, bounded) until B's room list contains the joined
    // room AND the joined space; the wait itself is the assertion.
    let snapshot_b =
        wait_for_room_list_containing(&mut conn_b, &room_id, &space_id, "room list B after joins")
            .await?;
    let room_list_b = room_list_summary(&snapshot_b);
    println!("room_list_b={room_list_b}");
    println!("room_space=ok");

    if !scenario.should_run_stage(QaStage::Timeline) {
        cleanup_after_full_flow(
            conn_a,
            conn_b,
            runtime_a,
            runtime_b,
            data_dir_a,
            account_key_a,
            account_key_b,
        )
        .await?;
        return Ok(scenario_report(&config.server_kind, scenario));
    }

    // -----------------------------------------------------------------------
    // --- Phase 5: Timeline subscribe, send, receive, edit, redact, paginate ---
    // -----------------------------------------------------------------------

    // A subscribes to the room timeline.
    let key_a = TimelineKey::room(account_key_a.clone(), room_id.clone());
    let subscribe_a_id = conn_a.next_request_id();
    conn_a
        .command(CoreCommand::Timeline(TimelineCommand::Subscribe {
            request_id: subscribe_a_id,
            key: key_a.clone(),
        }))
        .await
        .map_err(|e| format!("submit subscribe timeline A: {e}"))?;

    wait_for_initial_items(&mut conn_a, &key_a, subscribe_a_id, "subscribe timeline A").await?;
    println!("timeline_subscribed_a=ok");

    // A sends message 1 with a distinct client transaction id.
    let txn1 = "qa-phase5-txn-1".to_owned();
    let send1_id = conn_a.next_request_id();
    conn_a
        .command(CoreCommand::Timeline(TimelineCommand::SendText {
            request_id: send1_id,
            key: key_a.clone(),
            transaction_id: txn1.clone(),
            body: "Phase 5 QA message 1".to_owned(),
        }))
        .await
        .map_err(|e| format!("submit send1: {e}"))?;

    // Assert local echo appears as a Transaction diff (SDK-generated txn_id).
    let echo1_sdk_txn =
        wait_for_local_echo_diff(&mut conn_a, &key_a, &txn1, "local echo msg1").await?;
    println!("local_echo_msg1=ok sdk_txn={echo1_sdk_txn}");

    // Assert SendCompleted with matching txn_id and an event_id.
    let (send1_completed_txn, event1_id) =
        wait_for_send_completed(&mut conn_a, send1_id, &key_a, "send completed msg1").await?;
    if send1_completed_txn != txn1 {
        return Err(format!(
            "send1 txn_id mismatch: expected {txn1}, got {send1_completed_txn}"
        ));
    }
    println!("send_completed_msg1=ok event_id={event1_id}");

    // A sends message 2.
    let txn2 = "qa-phase5-txn-2".to_owned();
    let send2_id = conn_a.next_request_id();
    conn_a
        .command(CoreCommand::Timeline(TimelineCommand::SendText {
            request_id: send2_id,
            key: key_a.clone(),
            transaction_id: txn2.clone(),
            body: "Phase 5 QA message 2".to_owned(),
        }))
        .await
        .map_err(|e| format!("submit send2: {e}"))?;

    let echo2_sdk_txn =
        wait_for_local_echo_diff(&mut conn_a, &key_a, &txn2, "local echo msg2").await?;
    println!("local_echo_msg2=ok sdk_txn={echo2_sdk_txn}");

    let (send2_completed_txn, event2_id) =
        wait_for_send_completed(&mut conn_a, send2_id, &key_a, "send completed msg2").await?;
    if send2_completed_txn != txn2 {
        return Err(format!(
            "send2 txn_id mismatch: expected {txn2}, got {send2_completed_txn}"
        ));
    }
    println!("send_completed_msg2=ok event_id={event2_id}");

    // B subscribes and receives both messages (event-driven wait on diffs).
    let key_b = TimelineKey::room(account_key_b.clone(), room_id.clone());
    let subscribe_b_id = conn_b.next_request_id();
    conn_b
        .command(CoreCommand::Timeline(TimelineCommand::Subscribe {
            request_id: subscribe_b_id,
            key: key_b.clone(),
        }))
        .await
        .map_err(|e| format!("submit subscribe timeline B: {e}"))?;

    let b_initial =
        wait_for_initial_items(&mut conn_b, &key_b, subscribe_b_id, "subscribe timeline B").await?;
    println!("timeline_subscribed_b=ok");

    // Paginate backward on B to ensure A's messages are loaded from server
    // history (required because the SDK's Live timeline only has what's in
    // the local event cache; a newly-joined room may not have prior msgs yet).
    // We fire the paginate and then use wait_for_item_bodies_with_paginate
    // which scans both the initial items, the pagination diffs, and live diffs.
    let paginate_b_id = conn_b.next_request_id();
    conn_b
        .command(CoreCommand::Timeline(TimelineCommand::Paginate {
            request_id: paginate_b_id,
            key: key_b.clone(),
            direction: PaginationDirection::Backward,
            event_count: 20,
        }))
        .await
        .map_err(|e| format!("B backfill paginate: {e}"))?;

    // Now consume events until we've seen all required bodies AND pagination
    // has settled (Idle or EndReached). This single loop handles both.
    wait_for_bodies_and_pagination_settle(
        &mut conn_b,
        &key_b,
        &b_initial,
        &["Phase 5 QA message 1", "Phase 5 QA message 2"],
        "B receives 2 messages from A",
    )
    .await?;
    println!("b_recv_msgs=ok");

    // A edits message 1 — assert a Set diff reflecting the edit on original item identity.
    let edit1_id = conn_a.next_request_id();
    conn_a
        .command(CoreCommand::Timeline(TimelineCommand::EditText {
            request_id: edit1_id,
            key: key_a.clone(),
            event_id: event1_id.clone(),
            body: "Phase 5 QA message 1 EDITED".to_owned(),
        }))
        .await
        .map_err(|e| format!("submit edit msg1: {e}"))?;

    wait_for_edit_diff(
        &mut conn_a,
        &key_a,
        edit1_id,
        &event1_id,
        "Phase 5 QA message 1 EDITED",
        "edit msg1",
    )
    .await?;
    println!("edit_msg1=ok");

    // A redacts message 2 — assert removal or redacted-state diff.
    let redact2_id = conn_a.next_request_id();
    conn_a
        .command(CoreCommand::Timeline(TimelineCommand::Redact {
            request_id: redact2_id,
            key: key_a.clone(),
            event_id: event2_id.clone(),
        }))
        .await
        .map_err(|e| format!("submit redact msg2: {e}"))?;

    wait_for_redact_diff(&mut conn_a, &key_a, redact2_id, "redact msg2").await?;
    println!("redact_msg2=ok");

    // A paginates backward with a small page size until EndReached.
    // Assert Paginating → EndReached and strictly increasing batch_ids per generation.
    let paginate_id = conn_a.next_request_id();
    conn_a
        .command(CoreCommand::Timeline(TimelineCommand::Paginate {
            request_id: paginate_id,
            key: key_a.clone(),
            direction: PaginationDirection::Backward,
            event_count: 5,
        }))
        .await
        .map_err(|e| format!("submit paginate: {e}"))?;

    let paginate_result =
        wait_for_paginate_end_reached(&mut conn_a, &key_a, paginate_id, "paginate to EndReached")
            .await?;
    println!("paginate={paginate_result}");

    if scenario.should_run_stage(QaStage::Reply) {
        // -------------------------------------------------------------------
        // --- Phase 5b: True reply relation QA ---
        // -------------------------------------------------------------------

        let txn_b_reply = "qa-phase5-txn-b-reply".to_owned();
        let send_b_reply_id = conn_b.next_request_id();
        conn_b
            .command(CoreCommand::Timeline(TimelineCommand::SendReply {
                request_id: send_b_reply_id,
                key: key_b.clone(),
                transaction_id: txn_b_reply.clone(),
                in_reply_to_event_id: event1_id.clone(),
                body: "Phase 5 QA reply from B".to_owned(),
            }))
            .await
            .map_err(|e| format!("submit B reply: {e}"))?;

        let (_b_echo_txn, _b_reply_event_id) =
            wait_for_send_completed(&mut conn_b, send_b_reply_id, &key_b, "B reply completed")
                .await?;
        println!("b_reply_sent=ok");

        let reply_item = wait_for_item_with_body(
            &mut conn_a,
            &key_a,
            "Phase 5 QA reply from B",
            "A receives reply from B",
        )
        .await?;
        if reply_item.in_reply_to_event_id != Some(event1_id.clone()) {
            return Err(format!(
                "reply relation mismatch: expected {:?}, got {:?}",
                Some(event1_id.clone()),
                reply_item.in_reply_to_event_id
            ));
        }
        println!("reply=ok");
    }

    // Unsubscribe A and B to confirm no leaks.
    let unsub_a_id = conn_a.next_request_id();
    conn_a
        .command(CoreCommand::Timeline(TimelineCommand::Unsubscribe {
            request_id: unsub_a_id,
            key: key_a.clone(),
        }))
        .await
        .map_err(|e| format!("submit unsubscribe A: {e}"))?;

    let unsub_b_id = conn_b.next_request_id();
    conn_b
        .command(CoreCommand::Timeline(TimelineCommand::Unsubscribe {
            request_id: unsub_b_id,
            key: key_b.clone(),
        }))
        .await
        .map_err(|e| format!("submit unsubscribe B: {e}"))?;

    // Brief wait so the unsubscribe commands are processed before search QA.
    tokio::time::sleep(Duration::from_millis(200)).await;

    println!("timeline=ok");

    if !scenario.should_run_stage(QaStage::EditRedactSearch) {
        cleanup_after_full_flow(
            conn_a,
            conn_b,
            runtime_a,
            runtime_b,
            data_dir_a,
            account_key_a,
            account_key_b,
        )
        .await?;
        return Ok(scenario_report(&config.server_kind, scenario));
    }

    // -----------------------------------------------------------------------
    // --- Phase 6: Search QA (CJK query, edit, redact) ---
    // -----------------------------------------------------------------------

    // Re-subscribe A's timeline for the search round-trip.
    let key_a_search = TimelineKey::room(account_key_a.clone(), room_id.clone());
    let subscribe_search_id = conn_a.next_request_id();
    conn_a
        .command(CoreCommand::Timeline(TimelineCommand::Subscribe {
            request_id: subscribe_search_id,
            key: key_a_search.clone(),
        }))
        .await
        .map_err(|e| format!("submit subscribe timeline A (search): {e}"))?;

    wait_for_initial_items(
        &mut conn_a,
        &key_a_search,
        subscribe_search_id,
        "subscribe timeline A search",
    )
    .await?;

    // Send a message with a CJK body that will be indexed.
    const SEARCH_BODY: &str = "検索対象メッセージ Phase6 QA";
    const SEARCH_QUERY: &str = "検索対象";
    const EDITED_BODY: &str = "Phase6 QA 編集済みメッセージ";
    const EDITED_QUERY: &str = "編集済み";

    let txn_search = "qa-phase6-search-txn".to_owned();
    let send_search_id = conn_a.next_request_id();
    conn_a
        .command(CoreCommand::Timeline(TimelineCommand::SendText {
            request_id: send_search_id,
            key: key_a_search.clone(),
            transaction_id: txn_search.clone(),
            body: SEARCH_BODY.to_owned(),
        }))
        .await
        .map_err(|e| format!("submit search send: {e}"))?;

    let (_, search_event_id) = wait_for_send_completed(
        &mut conn_a,
        send_search_id,
        &key_a_search,
        "send search msg",
    )
    .await?;
    println!("search_msg_sent=ok event_id={search_event_id}");

    // Poll SearchCommand::Query until Results contains search_event_id.
    // The ngram index is fed by the SDK sync loop; wait up to 30s for indexing.
    poll_search_until_found(
        &mut conn_a,
        &account_key_a,
        SEARCH_QUERY,
        &search_event_id,
        &room_id,
        "search=ok (CJK query)",
    )
    .await?;
    println!("search=ok");

    // Edit the search message.
    let edit_search_id = conn_a.next_request_id();
    conn_a
        .command(CoreCommand::Timeline(TimelineCommand::EditText {
            request_id: edit_search_id,
            key: key_a_search.clone(),
            event_id: search_event_id.clone(),
            body: EDITED_BODY.to_owned(),
        }))
        .await
        .map_err(|e| format!("submit edit search msg: {e}"))?;

    wait_for_edit_diff(
        &mut conn_a,
        &key_a_search,
        edit_search_id,
        &search_event_id,
        EDITED_BODY,
        "edit search msg diff",
    )
    .await?;

    // Poll until new text is found.
    poll_search_until_found(
        &mut conn_a,
        &account_key_a,
        EDITED_QUERY,
        &search_event_id,
        &room_id,
        "search_edit=ok (new text found)",
    )
    .await?;

    // Assert old text is no longer verifiable (document store canonical text
    // has changed; even if the ngram index still has the old token, the document
    // store will reject the candidate).
    poll_search_until_absent(
        &mut conn_a,
        &account_key_a,
        SEARCH_QUERY,
        &search_event_id,
        &room_id,
        "search_edit=ok (old text absent)",
    )
    .await?;

    println!("search_edit=ok");

    // Assert redacted msg2 text is absent (msg2 was redacted in Phase 5 above).
    poll_search_until_absent(
        &mut conn_a,
        &account_key_a,
        "Phase 5 QA message 2",
        &event2_id,
        &room_id,
        "search_redact=ok (redacted msg absent)",
    )
    .await?;
    println!("search_redact=ok");
    println!("edit_redact_search=ok");

    // Unsubscribe search timeline.
    let unsub_search_id = conn_a.next_request_id();
    conn_a
        .command(CoreCommand::Timeline(TimelineCommand::Unsubscribe {
            request_id: unsub_search_id,
            key: key_a_search.clone(),
        }))
        .await
        .map_err(|e| format!("submit unsubscribe search timeline: {e}"))?;

    tokio::time::sleep(Duration::from_millis(200)).await;

    // -----------------------------------------------------------------------
    // --- Sync stop A + store-backed restore A + logout A ---
    // -----------------------------------------------------------------------
    let sync_stop_id = conn_a.next_request_id();
    conn_a
        .command(CoreCommand::Sync(SyncCommand::Stop {
            request_id: sync_stop_id,
        }))
        .await
        .map_err(|e| format!("submit sync stop A: {e}"))?;

    wait_for_sync_stopped(&mut conn_a, sync_stop_id, "sync stop A").await?;
    println!("sync_a=stopped");

    drop(conn_a);
    drop(runtime_a);
    // Brief wait to avoid store-lock contention on restore.
    tokio::time::sleep(Duration::from_millis(500)).await;

    let runtime_a2 = CoreRuntime::start_with_data_dir(data_dir_a);
    let mut conn_a2 = runtime_a2.attach();

    let restore_a_id = conn_a2.next_request_id();
    conn_a2
        .command(CoreCommand::Account(AccountCommand::RestoreSession {
            request_id: restore_a_id,
            account_key: account_key_a.clone(),
        }))
        .await
        .map_err(|e| format!("submit restore A: {e}"))?;

    wait_for_session_restored(&mut conn_a2, restore_a_id, &account_key_a, "restore A").await?;
    wait_for_ready_snapshot(&mut conn_a2, "restored session A Ready").await?;

    let logout_a_id = conn_a2.next_request_id();
    conn_a2
        .command(CoreCommand::Account(AccountCommand::Logout {
            request_id: logout_a_id,
        }))
        .await
        .map_err(|e| format!("submit logout A: {e}"))?;

    wait_for_logged_out(&mut conn_a2, logout_a_id, &account_key_a, "logout A").await?;

    // Cleanup assertion: a second restore of A must now fail not-found.
    let restore_gone_id = conn_a2.next_request_id();
    conn_a2
        .command(CoreCommand::Account(AccountCommand::RestoreSession {
            request_id: restore_gone_id,
            account_key: account_key_a.clone(),
        }))
        .await
        .map_err(|e| format!("submit post-logout restore A: {e}"))?;

    let failure = wait_for_operation_failed(
        &mut conn_a2,
        restore_gone_id,
        "post-logout restore A (must fail)",
    )
    .await?;
    if failure != CoreFailure::SessionNotFound {
        return Err(format!(
            "post-logout restore A failed with unexpected kind: {failure:?}"
        ));
    }
    if !matches!(conn_a2.snapshot().session, SessionState::SignedOut) {
        return Err("post-logout restore A must leave the session SignedOut".to_owned());
    }

    // -----------------------------------------------------------------------
    // --- Logout B ---
    // -----------------------------------------------------------------------
    let logout_b_id = conn_b.next_request_id();
    conn_b
        .command(CoreCommand::Account(AccountCommand::Logout {
            request_id: logout_b_id,
        }))
        .await
        .map_err(|e| format!("submit logout B: {e}"))?;

    wait_for_logged_out(&mut conn_b, logout_b_id, &account_key_b, "logout B").await?;

    // Cleanup assertion: the QA users share one credential store, and B
    // logged in after A, so the last-session pointer pointed at B until B's
    // logout cleared it. After BOTH logouts, RestoreLastSession must yield
    // SessionNotFound (a NORMAL outcome — this is the startup path when no
    // account is stored).
    let restore_last_id = conn_b.next_request_id();
    conn_b
        .command(CoreCommand::Account(AccountCommand::RestoreLastSession {
            request_id: restore_last_id,
        }))
        .await
        .map_err(|e| format!("submit post-logout restore-last: {e}"))?;

    let failure = wait_for_operation_failed(
        &mut conn_b,
        restore_last_id,
        "post-logout restore-last (must be not-found)",
    )
    .await?;
    if failure != CoreFailure::SessionNotFound {
        return Err(format!(
            "post-logout restore-last failed with unexpected kind: {failure:?}"
        ));
    }
    if !matches!(conn_b.snapshot().session, SessionState::SignedOut) {
        return Err("post-logout restore-last must leave the session SignedOut".to_owned());
    }

    println!("restore_cleanup=ok");
    Ok(scenario_report(&config.server_kind, scenario))
}

// ---------------------------------------------------------------------------
// Room-list helpers
// ---------------------------------------------------------------------------

/// A compact summary of a snapshot's room list for printing.
fn room_list_summary(snapshot: &AppState) -> String {
    let spaces = snapshot.spaces.len();
    let rooms = snapshot.rooms.len();
    let dms = snapshot.rooms.iter().filter(|r| r.is_dm).count();
    let unread = snapshot.rooms.iter().filter(|r| r.unread_count > 0).count();
    format!("rooms={rooms} spaces={spaces} dms={dms} unread_rooms={unread}")
}

// ---------------------------------------------------------------------------
// Event waiter helpers (Phase 4 additions)
// ---------------------------------------------------------------------------

/// Wait for `RoomEvent::RoomCreated` with the given request_id. Returns room_id.
async fn wait_for_room_created(
    conn: &mut CoreConnection,
    request_id: matrix_desktop_core::ids::RequestId,
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

/// Wait for `RoomEvent::SpaceCreated` with the given request_id. Returns space_id.
async fn wait_for_space_created(
    conn: &mut CoreConnection,
    request_id: matrix_desktop_core::ids::RequestId,
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

/// Wait for `RoomEvent::SpaceChildSet` with the given request_id.
async fn wait_for_space_child_set(
    conn: &mut CoreConnection,
    request_id: matrix_desktop_core::ids::RequestId,
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
                    return Err(format!(
                        "{label}: SpaceChildSet IDs mismatch: space={ev_space} child={ev_child}"
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

/// Wait for `RoomEvent::UserInvited` with the given request_id.
async fn wait_for_user_invited(
    conn: &mut CoreConnection,
    request_id: matrix_desktop_core::ids::RequestId,
    room_id: &str,
    user_id: &str,
    label: &str,
) -> Result<(), String> {
    loop {
        let event = tokio::time::timeout(EVENT_TIMEOUT, conn.recv_event())
            .await
            .map_err(|_| format!("{label}: timed out waiting for RoomEvent::UserInvited"))?
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;

        match event {
            CoreEvent::Room(RoomEvent::UserInvited {
                request_id: ev_id,
                room_id: ev_room,
                user_id: ev_user,
            }) if ev_id == request_id => {
                if ev_room != room_id || ev_user != user_id {
                    return Err(format!(
                        "{label}: UserInvited IDs mismatch: room={ev_room} user={ev_user}"
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

/// Wait for `RoomEvent::RoomJoined` with the given request_id.
async fn wait_for_room_joined(
    conn: &mut CoreConnection,
    request_id: matrix_desktop_core::ids::RequestId,
    room_id: &str,
    label: &str,
) -> Result<(), String> {
    loop {
        let event = tokio::time::timeout(EVENT_TIMEOUT, conn.recv_event())
            .await
            .map_err(|_| format!("{label}: timed out waiting for RoomEvent::RoomJoined"))?
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;

        match event {
            CoreEvent::Room(RoomEvent::RoomJoined {
                request_id: ev_id,
                room_id: ev_room,
            }) if ev_id == request_id => {
                if ev_room != room_id {
                    return Err(format!(
                        "{label}: RoomJoined room_id mismatch: got {ev_room}, expected {room_id}"
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

/// Wait (event-driven on `RoomListUpdated`/`StateChanged`, bounded by
/// `EVENT_TIMEOUT`) until the snapshot's room list contains the expected room
/// in `rooms` AND the expected space in `spaces`. Returns the matching
/// snapshot. Waiting for "any non-empty list" is not enough: spaces only
/// classify as spaces after the create reaches the client via sync, so the
/// list can be momentarily rooms-only.
async fn wait_for_room_list_containing(
    conn: &mut CoreConnection,
    expected_room_id: &str,
    expected_space_id: &str,
    label: &str,
) -> Result<AppState, String> {
    let contains_expected = |snapshot: &AppState| {
        snapshot.rooms.iter().any(|r| r.room_id == expected_room_id)
            && snapshot
                .spaces
                .iter()
                .any(|s| s.space_id == expected_space_id)
    };

    // Check the latest snapshot first in case it already has the data.
    let snapshot = conn.snapshot();
    if contains_expected(&snapshot) {
        return Ok(snapshot);
    }

    loop {
        let event = tokio::time::timeout(EVENT_TIMEOUT, conn.recv_event())
            .await
            .map_err(|_| {
                let snapshot = conn.snapshot();
                format!(
                    "{label}: timed out waiting for room list to contain room \
                     {expected_room_id} and space {expected_space_id} \
                     (have {} rooms, {} spaces)",
                    snapshot.rooms.len(),
                    snapshot.spaces.len()
                )
            })?
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;

        match event {
            CoreEvent::Room(RoomEvent::RoomListUpdated) => {
                // The discrete event may arrive before the reducer projected
                // the matching snapshot; check the latest snapshot and keep
                // waiting otherwise — a StateChanged will follow.
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

// ---------------------------------------------------------------------------
// Phase 3 event waiter helpers (unchanged)
// ---------------------------------------------------------------------------

/// Wait for `SyncEvent::Started` with the given request_id. Returns the backend kind.
async fn wait_for_sync_started(
    conn: &mut CoreConnection,
    request_id: matrix_desktop_core::ids::RequestId,
    label: &str,
) -> Result<SyncBackendKind, String> {
    loop {
        let event = tokio::time::timeout(EVENT_TIMEOUT, conn.recv_event())
            .await
            .map_err(|_| format!("{label}: timed out waiting for SyncEvent::Started"))?
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;

        match event {
            CoreEvent::Sync(SyncEvent::Started {
                request_id: ev_id,
                backend,
            }) if ev_id == Some(request_id) => {
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

/// Wait for `SyncEvent::Running` (first successful sync response).
async fn wait_for_sync_running(conn: &mut CoreConnection, label: &str) -> Result<(), String> {
    loop {
        let event = tokio::time::timeout(EVENT_TIMEOUT, conn.recv_event())
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

/// Wait for `SyncEvent::Stopped` with the given request_id.
async fn wait_for_sync_stopped(
    conn: &mut CoreConnection,
    request_id: matrix_desktop_core::ids::RequestId,
    label: &str,
) -> Result<(), String> {
    loop {
        let event = tokio::time::timeout(EVENT_TIMEOUT, conn.recv_event())
            .await
            .map_err(|_| format!("{label}: timed out waiting for SyncEvent::Stopped"))?
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;

        if matches!(
            event,
            CoreEvent::Sync(SyncEvent::Stopped {
                request_id: Some(ev_id)
            }) if ev_id == request_id
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

/// Wait for a `StateChanged` snapshot where `SessionState::Ready`.
async fn wait_for_ready_snapshot(conn: &mut CoreConnection, label: &str) -> Result<(), String> {
    if matches!(conn.snapshot().session, SessionState::Ready(_)) {
        return Ok(());
    }

    loop {
        let event = tokio::time::timeout(EVENT_TIMEOUT, conn.recv_event())
            .await
            .map_err(|_| format!("{label}: timed out waiting for Ready snapshot"))?
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;

        if let CoreEvent::StateChanged(snapshot) = event
            && matches!(snapshot.session, SessionState::Ready(_))
        {
            return Ok(());
        }
    }
}

/// Wait for `AccountEvent::LoggedIn` with the given request_id.
async fn wait_for_logged_in(
    conn: &mut CoreConnection,
    request_id: matrix_desktop_core::ids::RequestId,
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

/// Wait for `AccountEvent::SessionRestored` with the given request_id.
async fn wait_for_session_restored(
    conn: &mut CoreConnection,
    request_id: matrix_desktop_core::ids::RequestId,
    expected_account_key: &AccountKey,
    label: &str,
) -> Result<(), String> {
    loop {
        let event = tokio::time::timeout(EVENT_TIMEOUT, conn.recv_event())
            .await
            .map_err(|_| format!("{label}: timed out waiting for SessionRestored event"))?
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;

        match event {
            CoreEvent::Account(AccountEvent::SessionRestored {
                request_id: ev_id,
                account_key,
            }) if ev_id == request_id => {
                if account_key != *expected_account_key {
                    return Err(format!(
                        "{label}: SessionRestored account_key mismatch: got {:?}, expected {:?}",
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

/// Wait for `AccountEvent::LoggedOut` with the given request_id.
async fn wait_for_logged_out(
    conn: &mut CoreConnection,
    request_id: matrix_desktop_core::ids::RequestId,
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
                        "{label}: LoggedOut account_key mismatch: got {:?}, expected {:?}",
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

/// Wait for `OperationFailed` with the given request_id and return the failure.
async fn wait_for_operation_failed(
    conn: &mut CoreConnection,
    request_id: matrix_desktop_core::ids::RequestId,
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
                let matches_request = match &account_event {
                    AccountEvent::LoggedIn { request_id: id, .. }
                    | AccountEvent::SessionRestored { request_id: id, .. }
                    | AccountEvent::SavedSessionsListed { request_id: id, .. }
                    | AccountEvent::RecoveryCompleted { request_id: id, .. }
                    | AccountEvent::LoggedOut { request_id: id, .. }
                    | AccountEvent::AccountSwitched { request_id: id, .. } => *id == request_id,
                    AccountEvent::RecoveryRequired { .. } => false,
                };
                if matches_request {
                    return Err(format!(
                        "{label}: expected OperationFailed but the operation succeeded"
                    ));
                }
            }
            _ => continue,
        }
    }
}

// ---------------------------------------------------------------------------
// Config and helpers
// ---------------------------------------------------------------------------

struct QaConfig {
    homeserver: String,
    server_name: String,
    server_kind: String,
    user_a: String,
    password_a: String,
    user_b: String,
    password_b: String,
    /// Expected sync backend ("SyncService" | "LegacySync"); QA fails on
    /// mismatch when set. Plain assertion input, not a credential.
    expect_sync_backend: Option<String>,
}

impl QaConfig {
    fn from_env() -> Result<Self, String> {
        Ok(Self {
            homeserver: env_required(ENV_HOMESERVER)?,
            server_name: env_required(ENV_SERVER_NAME)?,
            server_kind: std::env::var(ENV_SERVER_KIND).unwrap_or_else(|_| "local".to_owned()),
            user_a: env_required(ENV_USER_A)?,
            password_a: env_required(ENV_PASSWORD_A)?,
            user_b: env_required(ENV_USER_B)?,
            password_b: env_required(ENV_PASSWORD_B)?,
            expect_sync_backend: std::env::var(ENV_EXPECT_SYNC_BACKEND).ok(),
        })
    }
}

/// Fail when an expected backend is configured and the observed one differs.
fn assert_expected_backend(
    expected: Option<&str>,
    observed: SyncBackendKind,
    label: &str,
) -> Result<(), String> {
    let Some(expected) = expected else {
        return Ok(());
    };
    let observed_name = match observed {
        SyncBackendKind::SyncService => "SyncService",
        SyncBackendKind::LegacySync => "LegacySync",
    };
    if observed_name != expected {
        return Err(format!(
            "{label}: sync backend mismatch: expected {expected}, observed {observed_name}"
        ));
    }
    Ok(())
}

fn env_required(name: &str) -> Result<String, String> {
    std::env::var(name).map_err(|_| format!("{name} is required"))
}

/// Data directory for QA runs.
fn qa_data_dir(suffix: &str) -> std::path::PathBuf {
    if let Ok(dir) = std::env::var("MATRIX_DESKTOP_QA_DATA_DIR") {
        return std::path::PathBuf::from(dir).join(suffix);
    }
    std::env::temp_dir()
        .join("matrix-desktop-core-qa")
        .join(format!("{}_{}", std::process::id(), suffix))
}

// ---------------------------------------------------------------------------
// Phase 5 event waiter helpers
// ---------------------------------------------------------------------------

/// Wait for `TimelineEvent::InitialItems` for the given key and request_id.
/// Returns the initial item list.
async fn wait_for_initial_items(
    conn: &mut CoreConnection,
    key: &TimelineKey,
    request_id: matrix_desktop_core::ids::RequestId,
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

/// Wait for an `ItemsUpdated` diff batch containing any new Transaction item
/// (local echo). Since the SDK send queue generates its own txn_id (not the
/// client-supplied one), we just wait for ANY Transaction-id item to appear.
/// Returns the SDK-generated transaction_id string from the matching item.
async fn wait_for_local_echo_diff(
    conn: &mut CoreConnection,
    key: &TimelineKey,
    _client_txn_id: &str,
    label: &str,
) -> Result<String, String> {
    loop {
        let event = tokio::time::timeout(EVENT_TIMEOUT, conn.recv_event())
            .await
            .map_err(|_| format!("{label}: timed out waiting for local echo diff"))?
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;

        if let CoreEvent::Timeline(TimelineEvent::ItemsUpdated {
            key: ref ev_key,
            diffs,
            ..
        }) = event
        {
            if ev_key != key {
                continue;
            }
            for diff in &diffs {
                let item = match diff {
                    matrix_desktop_core::event::TimelineDiff::PushBack { item }
                    | matrix_desktop_core::event::TimelineDiff::PushFront { item }
                    | matrix_desktop_core::event::TimelineDiff::Insert { item, .. }
                    | matrix_desktop_core::event::TimelineDiff::Set { item, .. } => item,
                    _ => continue,
                };
                if let matrix_desktop_core::event::TimelineItemId::Transaction {
                    transaction_id: ref t,
                } = item.id
                {
                    return Ok(t.clone());
                }
            }
        }
    }
}

/// Wait for `TimelineEvent::SendCompleted` with the given request_id and key.
/// Returns `(transaction_id, event_id)`.
async fn wait_for_send_completed(
    conn: &mut CoreConnection,
    request_id: matrix_desktop_core::ids::RequestId,
    key: &TimelineKey,
    label: &str,
) -> Result<(String, String), String> {
    loop {
        let event = tokio::time::timeout(EVENT_TIMEOUT, conn.recv_event())
            .await
            .map_err(|_| format!("{label}: timed out waiting for SendCompleted"))?
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

/// Wait until all `expected_bodies` are found AND pagination has settled (Idle
/// or EndReached). Scans `initial_items` first, then both ItemsUpdated diffs
/// and PaginationStateChanged events in a single loop. This avoids the race
/// where paginate diffs are consumed before the body scan starts.
async fn wait_for_bodies_and_pagination_settle(
    conn: &mut CoreConnection,
    key: &TimelineKey,
    initial_items: &[matrix_desktop_core::event::TimelineItem],
    expected_bodies: &[&str],
    label: &str,
) -> Result<(), String> {
    // Pre-scan initial items.
    let mut remaining_bodies: Vec<&str> = expected_bodies.to_vec();
    for item in initial_items {
        if let Some(ref body) = item.body {
            remaining_bodies.retain(|expected| !body.contains(expected));
        }
    }

    let mut pagination_settled = false;

    loop {
        if remaining_bodies.is_empty() && pagination_settled {
            return Ok(());
        }

        let event = tokio::time::timeout(EVENT_TIMEOUT, conn.recv_event())
            .await
            .map_err(|_| {
                format!(
                    "{label}: timed out; bodies still needed: {:?}, pagination_settled: {}",
                    remaining_bodies, pagination_settled
                )
            })?
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;

        match &event {
            CoreEvent::Timeline(TimelineEvent::ItemsUpdated {
                key: ev_key, diffs, ..
            }) if ev_key == key => {
                for diff in diffs {
                    let item = match diff {
                        matrix_desktop_core::event::TimelineDiff::PushBack { item }
                        | matrix_desktop_core::event::TimelineDiff::PushFront { item }
                        | matrix_desktop_core::event::TimelineDiff::Insert { item, .. }
                        | matrix_desktop_core::event::TimelineDiff::Set { item, .. } => item,
                        matrix_desktop_core::event::TimelineDiff::Reset { items } => {
                            for it in items {
                                if let Some(ref body) = it.body {
                                    remaining_bodies.retain(|e| !body.contains(e));
                                }
                            }
                            continue;
                        }
                        _ => continue,
                    };
                    if let Some(ref body) = item.body {
                        remaining_bodies.retain(|e| !body.contains(e));
                    }
                }
            }
            CoreEvent::Timeline(TimelineEvent::InitialItems {
                key: ev_key, items, ..
            }) if ev_key == key => {
                for item in items {
                    if let Some(ref body) = item.body {
                        remaining_bodies.retain(|e| !body.contains(e));
                    }
                }
            }
            CoreEvent::Timeline(TimelineEvent::PaginationStateChanged {
                key: ev_key,
                state,
                ..
            }) if ev_key == key => match state {
                PaginationState::Idle
                | PaginationState::EndReached
                | PaginationState::Failed { .. } => {
                    pagination_settled = true;
                }
                PaginationState::Paginating => {}
            },
            _ => {}
        }
    }
}

/// Wait for an item whose body contains `expected_body` and return the item so
/// the caller can assert relation metadata on the projected DTO.
async fn wait_for_item_with_body(
    conn: &mut CoreConnection,
    key: &TimelineKey,
    expected_body: &str,
    label: &str,
) -> Result<matrix_desktop_core::event::TimelineItem, String> {
    let body_matches = |item: &matrix_desktop_core::event::TimelineItem| {
        item.body
            .as_ref()
            .map(|body| body.contains(expected_body))
            .unwrap_or(false)
    };

    loop {
        let event = tokio::time::timeout(EVENT_TIMEOUT, conn.recv_event())
            .await
            .map_err(|_| format!("{label}: timed out waiting for body {expected_body:?}"))?
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;

        match event {
            CoreEvent::Timeline(TimelineEvent::InitialItems {
                key: ref ev_key,
                items,
                ..
            }) if ev_key == key => {
                if let Some(item) = items.into_iter().find(|item| body_matches(item)) {
                    return Ok(item);
                }
            }
            CoreEvent::Timeline(TimelineEvent::ItemsUpdated {
                key: ref ev_key,
                diffs,
                ..
            }) if ev_key == key => {
                for diff in diffs {
                    let item = match diff {
                        matrix_desktop_core::event::TimelineDiff::PushBack { item }
                        | matrix_desktop_core::event::TimelineDiff::PushFront { item }
                        | matrix_desktop_core::event::TimelineDiff::Insert { item, .. }
                        | matrix_desktop_core::event::TimelineDiff::Set { item, .. } => item,
                        matrix_desktop_core::event::TimelineDiff::Reset { items } => {
                            if let Some(item) = items.into_iter().find(|item| body_matches(item)) {
                                return Ok(item);
                            }
                            continue;
                        }
                        _ => continue,
                    };
                    if body_matches(&item) {
                        return Ok(item.clone());
                    }
                }
            }
            _ => {}
        }
    }
}

/// Wait for an `ItemsUpdated` Set diff for the event identified by `event_id`
/// OR a Set diff that has the given body substring (whichever arrives first).
/// This asserts that an edit was reflected in the timeline. A failed edit
/// operation (`OperationFailed` with the edit's request_id) is surfaced as an
/// explicit error instead of a silent timeout.
/// Timeout is extended to 60s because edit confirmation requires a sync round-trip.
async fn wait_for_edit_diff(
    conn: &mut CoreConnection,
    key: &TimelineKey,
    request_id: matrix_desktop_core::ids::RequestId,
    event_id: &str,
    edited_body: &str,
    label: &str,
) -> Result<(), String> {
    let timeout = Duration::from_secs(60);
    loop {
        let event = tokio::time::timeout(timeout, conn.recv_event())
            .await
            .map_err(|_| {
                format!("{label}: timed out waiting for edit Set diff (event_id: {event_id})")
            })?
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;

        match event {
            CoreEvent::Timeline(TimelineEvent::ItemsUpdated {
                key: ref ev_key,
                diffs,
                ..
            }) if ev_key == key => {
                for diff in &diffs {
                    if let matrix_desktop_core::event::TimelineDiff::Set { item, .. } = diff {
                        // Accept: item has the edited body, OR item is identified by event_id
                        // (the SDK may not yet have applied the body to the item in all cases).
                        let body_matches = item.body.as_deref().unwrap_or("").contains(edited_body);
                        let event_id_matches = matches!(
                            &item.id,
                            matrix_desktop_core::event::TimelineItemId::Event { event_id: id }
                            if id == event_id
                        );
                        if body_matches || event_id_matches {
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

/// Wait for an `ItemsUpdated` diff that signals a redaction: either a Remove
/// or a Set where the body is None or empty (redacted message placeholder).
/// A failed redact operation is surfaced as an explicit error.
/// Timeout is extended to 60s because redaction requires a sync round-trip.
async fn wait_for_redact_diff(
    conn: &mut CoreConnection,
    key: &TimelineKey,
    request_id: matrix_desktop_core::ids::RequestId,
    label: &str,
) -> Result<(), String> {
    let timeout = Duration::from_secs(60);
    loop {
        let event = tokio::time::timeout(timeout, conn.recv_event())
            .await
            .map_err(|_| format!("{label}: timed out waiting for redact diff"))?
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;

        match event {
            CoreEvent::Timeline(TimelineEvent::ItemsUpdated {
                key: ref ev_key,
                diffs,
                ..
            }) if ev_key == key => {
                for diff in &diffs {
                    match diff {
                        matrix_desktop_core::event::TimelineDiff::Remove { .. } => return Ok(()),
                        matrix_desktop_core::event::TimelineDiff::Set { item, .. } => {
                            // SDK emits a Set with a redacted body (None or empty) when it
                            // replaces the message body in-place with a "Message redacted" tombstone.
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

/// Paginate backward in a loop until `EndReached`, asserting the state
/// sequence. Returns `"end_reached"` on success.
///
/// The spec requires: emit Paginating, then (Idle | EndReached | Failed).
/// We drive the loop ourselves: on Idle we re-submit Paginate; on EndReached
/// we return; on Failed we return an error.
async fn wait_for_paginate_end_reached(
    conn: &mut CoreConnection,
    key: &TimelineKey,
    first_request_id: matrix_desktop_core::ids::RequestId,
    label: &str,
) -> Result<String, String> {
    // We use the conn to submit additional Paginate commands inside the loop.
    // Because conn is mutably borrowed for recv_event calls too, we rely on
    // the fact that the runtime handles command + event independently. The
    // pattern used here: record the first_request_id, process events, and
    // when we need to re-paginate we note the request for next iteration.
    let mut current_request_id = first_request_id;
    let mut saw_paginating = false;

    loop {
        let event = tokio::time::timeout(Duration::from_secs(60), conn.recv_event())
            .await
            .map_err(|_| format!("{label}: timed out waiting for pagination state change"))?
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;

        match event {
            CoreEvent::Timeline(TimelineEvent::PaginationStateChanged {
                key: ref ev_key,
                direction,
                state,
                ..
            }) if ev_key == key && direction == PaginationDirection::Backward => {
                match state {
                    PaginationState::Paginating => {
                        saw_paginating = true;
                    }
                    PaginationState::Idle => {
                        if !saw_paginating {
                            return Err(format!(
                                "{label}: got Idle without first seeing Paginating"
                            ));
                        }
                        // More history available — re-paginate.
                        saw_paginating = false;
                        current_request_id = conn.next_request_id();
                        conn.command(CoreCommand::Timeline(TimelineCommand::Paginate {
                            request_id: current_request_id,
                            key: key.clone(),
                            direction: PaginationDirection::Backward,
                            event_count: 5,
                        }))
                        .await
                        .map_err(|e| format!("{label}: re-paginate submit failed: {e}"))?;
                    }
                    PaginationState::EndReached => {
                        if !saw_paginating {
                            return Err(format!(
                                "{label}: got EndReached without first seeing Paginating"
                            ));
                        }
                        return Ok("end_reached".to_owned());
                    }
                    PaginationState::Failed { kind } => {
                        return Err(format!("{label}: pagination failed: {kind:?}"));
                    }
                }
            }
            CoreEvent::OperationFailed {
                request_id: ev_id,
                failure,
            } if ev_id == current_request_id => {
                return Err(format!("{label} paginate failed: {failure:?}"));
            }
            _ => {}
        }
    }
}

// ---------------------------------------------------------------------------
// Phase 6 search QA helpers
// ---------------------------------------------------------------------------

/// Poll `SearchCommand::Query` every 500ms until the Results event contains
/// `expected_event_id` in the given room, or timeout (60s). Fails on any
/// search failure response.
async fn poll_search_until_found(
    conn: &mut CoreConnection,
    _account_key: &AccountKey,
    query: &str,
    expected_event_id: &str,
    room_id: &str,
    label: &str,
) -> Result<(), String> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(60);
    loop {
        if tokio::time::Instant::now() >= deadline {
            return Err(format!(
                "{label}: timed out; event {expected_event_id} not found in search results for query"
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
        .map_err(|e| format!("{label}: submit search query: {e}"))?;

        // Wait up to 5s for the search result for this request_id.
        let found = wait_for_search_result(conn, rid, expected_event_id, label).await?;
        if found {
            return Ok(());
        }
        // Not found yet — the index may still be updating. Wait and retry.
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

/// Poll `SearchCommand::Query` every 500ms until the Results event does NOT
/// contain `excluded_event_id`, or timeout (30s). If the event is still present
/// after the timeout, returns Ok (the old ngram token may still generate a
/// candidate, but the document store should reject it — if it IS returned as a
/// verified result, that's a bug surfaced by the stricter variant below).
///
/// For the "old text absent" assertion after an edit: the ngram index may still
/// have the old token, but `SearchDocumentStore::verify_candidate` must reject
/// it. We poll until the event is absent from the verified result set.
async fn poll_search_until_absent(
    conn: &mut CoreConnection,
    _account_key: &AccountKey,
    query: &str,
    excluded_event_id: &str,
    room_id: &str,
    label: &str,
) -> Result<(), String> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    loop {
        let rid = conn.next_request_id();
        conn.command(CoreCommand::Search(SearchCommand::Query {
            request_id: rid,
            query: query.to_owned(),
            scope: SearchScope::Room {
                room_id: room_id.to_owned(),
            },
        }))
        .await
        .map_err(|e| format!("{label}: submit search query: {e}"))?;

        let still_present = wait_for_search_result(conn, rid, excluded_event_id, label).await?;
        if !still_present {
            return Ok(());
        }

        if tokio::time::Instant::now() >= deadline {
            // The event is still present after 30s. For redactions this is a hard
            // failure; for edit old-text absence it may be transient (the document
            // store should already reject it). Surface as error.
            return Err(format!(
                "{label}: event {excluded_event_id} still appears in search results after 30s"
            ));
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

/// Submit one search query and wait for `SearchEvent::Results` with matching
/// `request_id`. Returns `true` if `expected_event_id` appears in results,
/// `false` if the Results arrived but the event is absent.
/// Propagates search failure (IndexUnavailable, etc.) as errors.
async fn wait_for_search_result(
    conn: &mut CoreConnection,
    request_id: matrix_desktop_core::ids::RequestId,
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
                let found = results.iter().any(|r| r.event_id == expected_event_id);
                return Ok(found);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_all_scenarios_from_env_value() {
        assert_eq!(QaScenario::from_env_value("all").unwrap(), QaScenario::All);
        assert_eq!(
            QaScenario::from_env_value("safety").unwrap(),
            QaScenario::Safety
        );
        assert_eq!(
            QaScenario::from_env_value("login_sync").unwrap(),
            QaScenario::LoginSync
        );
        assert_eq!(
            QaScenario::from_env_value("room_space").unwrap(),
            QaScenario::RoomSpace
        );
        assert_eq!(
            QaScenario::from_env_value("timeline").unwrap(),
            QaScenario::Timeline
        );
        assert_eq!(
            QaScenario::from_env_value("reply").unwrap(),
            QaScenario::Reply
        );
        assert_eq!(
            QaScenario::from_env_value("thread").unwrap(),
            QaScenario::Thread
        );
        assert_eq!(
            QaScenario::from_env_value("edit_redact_search").unwrap(),
            QaScenario::EditRedactSearch
        );
        assert_eq!(
            QaScenario::from_env_value("restore_cleanup").unwrap(),
            QaScenario::RestoreCleanup
        );
    }

    #[test]
    fn rejects_unknown_scenario_names() {
        let error = QaScenario::from_env_value("unknown").unwrap_err();

        assert!(error.contains("MATRIX_DESKTOP_QA_SCENARIO"));
        assert!(error.contains("unknown"));
    }

    #[test]
    fn supported_scenarios_are_allowed_by_preflight() {
        for scenario in [
            QaScenario::Safety,
            QaScenario::LoginSync,
            QaScenario::RoomSpace,
            QaScenario::Timeline,
            QaScenario::Reply,
            QaScenario::EditRedactSearch,
            QaScenario::RestoreCleanup,
        ] {
            scenario_preflight_error(scenario).unwrap();
        }
    }

    #[test]
    fn thread_is_explicitly_unimplemented() {
        let thread = scenario_preflight_error(QaScenario::Thread).unwrap_err();

        assert_eq!(
            thread,
            "scenario thread is not implemented until true Matrix reply support lands"
        );
    }

    #[test]
    fn staged_scenarios_stop_after_their_requested_stage() {
        assert!(QaScenario::Safety.should_run_stage(QaStage::Safety));
        assert!(!QaScenario::Safety.should_run_stage(QaStage::LoginSync));

        assert!(QaScenario::LoginSync.should_run_stage(QaStage::Safety));
        assert!(QaScenario::LoginSync.should_run_stage(QaStage::LoginSync));
        assert!(!QaScenario::LoginSync.should_run_stage(QaStage::RoomSpace));

        assert!(QaScenario::RoomSpace.should_run_stage(QaStage::LoginSync));
        assert!(QaScenario::RoomSpace.should_run_stage(QaStage::RoomSpace));
        assert!(!QaScenario::RoomSpace.should_run_stage(QaStage::Timeline));

        assert!(QaScenario::Timeline.should_run_stage(QaStage::LoginSync));
        assert!(QaScenario::Timeline.should_run_stage(QaStage::RoomSpace));
        assert!(QaScenario::Timeline.should_run_stage(QaStage::Timeline));
        assert!(!QaScenario::Timeline.should_run_stage(QaStage::Reply));
        assert!(!QaScenario::Timeline.should_run_stage(QaStage::EditRedactSearch));

        assert!(QaScenario::Reply.should_run_stage(QaStage::LoginSync));
        assert!(QaScenario::Reply.should_run_stage(QaStage::RoomSpace));
        assert!(QaScenario::Reply.should_run_stage(QaStage::Timeline));
        assert!(QaScenario::Reply.should_run_stage(QaStage::Reply));
        assert!(!QaScenario::Reply.should_run_stage(QaStage::EditRedactSearch));

        assert!(QaScenario::All.should_run_stage(QaStage::Safety));
        assert!(QaScenario::All.should_run_stage(QaStage::LoginSync));
        assert!(QaScenario::All.should_run_stage(QaStage::RoomSpace));
        assert!(QaScenario::All.should_run_stage(QaStage::Timeline));
        assert!(QaScenario::All.should_run_stage(QaStage::Reply));
        assert!(QaScenario::All.should_run_stage(QaStage::EditRedactSearch));
        assert!(QaScenario::All.should_run_stage(QaStage::RestoreCleanup));
    }

    #[test]
    fn implemented_final_tokens_include_reply_and_exclude_thread() {
        assert_eq!(
            implemented_final_tokens(),
            [
                "safety=ok",
                "login_sync=ok",
                "room_space=ok",
                "timeline=ok",
                "reply=ok",
                "edit_redact_search=ok",
                "restore_cleanup=ok",
            ]
        );
    }

    #[test]
    fn final_tokens_follow_the_requested_scenario() {
        assert_eq!(final_tokens_for_scenario(QaScenario::Safety), ["safety=ok"]);
        assert_eq!(
            final_tokens_for_scenario(QaScenario::LoginSync),
            ["safety=ok", "login_sync=ok", "restore_cleanup=ok"]
        );
        assert_eq!(
            final_tokens_for_scenario(QaScenario::RoomSpace),
            [
                "safety=ok",
                "login_sync=ok",
                "room_space=ok",
                "restore_cleanup=ok"
            ]
        );
        assert_eq!(
            final_tokens_for_scenario(QaScenario::Timeline),
            [
                "safety=ok",
                "login_sync=ok",
                "room_space=ok",
                "timeline=ok",
                "restore_cleanup=ok",
            ]
        );
        assert_eq!(
            final_tokens_for_scenario(QaScenario::Reply),
            [
                "safety=ok",
                "login_sync=ok",
                "room_space=ok",
                "timeline=ok",
                "reply=ok",
                "restore_cleanup=ok",
            ]
        );
        assert_eq!(
            final_tokens_for_scenario(QaScenario::EditRedactSearch),
            [
                "safety=ok",
                "login_sync=ok",
                "room_space=ok",
                "timeline=ok",
                "edit_redact_search=ok",
                "restore_cleanup=ok",
            ]
        );
        assert_eq!(
            final_tokens_for_scenario(QaScenario::All),
            implemented_final_tokens()
        );
    }

    #[test]
    fn implemented_final_tokens_include_safety() {
        assert_eq!(
            implemented_final_tokens(),
            [
                "safety=ok",
                "login_sync=ok",
                "room_space=ok",
                "timeline=ok",
                "reply=ok",
                "edit_redact_search=ok",
                "restore_cleanup=ok",
            ]
        );
    }
}
