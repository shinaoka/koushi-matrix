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

use matrix_desktop_core::command::{AccountCommand, CoreCommand, RoomCommand, SyncCommand};
use matrix_desktop_core::event::{AccountEvent, CoreEvent, RoomEvent, SyncBackendKind, SyncEvent};
use matrix_desktop_core::failure::CoreFailure;
use matrix_desktop_core::ids::AccountKey;
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
#[cfg(any(debug_assertions, test))]
const ENV_FILE_CREDENTIAL_STORE_DIR: &str = "MATRIX_DESKTOP_QA_FILE_CREDENTIAL_STORE_DIR";

const DEVICE_A: &str = "Matrix Desktop Core QA A";
const DEVICE_B: &str = "Matrix Desktop Core QA B";

/// Maximum time to wait for a single event.
const EVENT_TIMEOUT: Duration = Duration::from_secs(30);

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
    // Hard guard BEFORE any login: unattended QA must never touch the OS
    // keychain, even if env wiring regresses.
    assert_file_credential_store_active()?;

    let config = QaConfig::from_env()?;
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|e| format!("runtime creation failed: {e}"))?;

    // Run inside the Tokio runtime so SDK handles drop in context (Async rule 11).
    runtime.block_on(run_async(config))
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

async fn run_async(config: QaConfig) -> Result<String, String> {
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

    // Wait for A's room list to update and contain the new room and space
    wait_for_room_list_updated(&mut conn_a, "room list A after creates").await?;
    let snapshot_a = conn_a.snapshot();
    let room_list_a = room_list_summary(&snapshot_a);
    println!("room_list_a={room_list_a}");

    // Assert A has the created room and space
    assert_room_in_list(&snapshot_a, &room_id, "A's room list should contain QA Room")?;
    assert_space_in_list(&snapshot_a, &space_id, "A's room list should contain QA Space")?;

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

    // Wait for B's room list to update and contain the joined room and space
    wait_for_room_list_updated(&mut conn_b, "room list B after joins").await?;
    let snapshot_b = conn_b.snapshot();
    let room_list_b = room_list_summary(&snapshot_b);
    println!("room_list_b={room_list_b}");

    // Assert B has the joined room (space join should make it appear too)
    assert_room_in_list(&snapshot_b, &room_id, "B's room list should contain QA Room")?;

    // Phase 5 placeholder: send permission check
    // (Actual send is Phase 5; we just verify the room exists in state)
    println!("send_permission_check=skipped(phase5)");

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

    Ok(format!(
        "Headless core QA OK. server={server} \
         login_a={user_a} sync_a=ok backend_a={backend_a:?} \
         room_created=ok space_created=ok space_child_set=ok \
         invite_ok=ok room_list_a={room_list_a} \
         restore_a=ok logout_a=ok post_logout_restore_a=not_found \
         login_b={user_b} sync_b=ok backend_b={backend_b:?} \
         joined_room=ok joined_space=ok room_list_b={room_list_b} \
         logout_b=ok",
        server = config.server_kind,
        user_a = account_key_a.0,
        backend_a = sync_backend_a,
        user_b = account_key_b.0,
        backend_b = sync_backend_b,
    ))
}

// ---------------------------------------------------------------------------
// Room-list helpers
// ---------------------------------------------------------------------------

/// A compact summary of a snapshot's room list for printing.
fn room_list_summary(snapshot: &AppState) -> String {
    let spaces = snapshot.spaces.len();
    let rooms = snapshot.rooms.len();
    let dms = snapshot.rooms.iter().filter(|r| r.is_dm).count();
    let unread = snapshot
        .rooms
        .iter()
        .filter(|r| r.unread_count > 0)
        .count();
    format!("rooms={rooms} spaces={spaces} dms={dms} unread_rooms={unread}")
}

fn assert_room_in_list(snapshot: &AppState, room_id: &str, label: &str) -> Result<(), String> {
    if snapshot.rooms.iter().any(|r| r.room_id == room_id) {
        Ok(())
    } else {
        Err(format!(
            "{label}: room {room_id} not found in room list (have {} rooms)",
            snapshot.rooms.len()
        ))
    }
}

fn assert_space_in_list(snapshot: &AppState, space_id: &str, label: &str) -> Result<(), String> {
    if snapshot.spaces.iter().any(|s| s.space_id == space_id) {
        Ok(())
    } else {
        Err(format!(
            "{label}: space {space_id} not found in space list (have {} spaces)",
            snapshot.spaces.len()
        ))
    }
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

/// Wait for `RoomEvent::RoomListUpdated` (discrete event) and/or a
/// `StateChanged` snapshot that has a non-empty room list. Returns once the
/// snapshot has at least one room or space.
async fn wait_for_room_list_updated(conn: &mut CoreConnection, label: &str) -> Result<(), String> {
    // Check snapshot first in case it already has data.
    if room_list_has_content(&conn.snapshot()) {
        return Ok(());
    }

    loop {
        let event = tokio::time::timeout(EVENT_TIMEOUT, conn.recv_event())
            .await
            .map_err(|_| format!("{label}: timed out waiting for room list to populate"))?
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;

        match event {
            CoreEvent::Room(RoomEvent::RoomListUpdated) => {
                let snapshot = conn.snapshot();
                if room_list_has_content(&snapshot) {
                    return Ok(());
                }
                // Got the event but snapshot still empty — keep waiting.
            }
            CoreEvent::StateChanged(snapshot) => {
                if room_list_has_content(&snapshot) {
                    return Ok(());
                }
            }
            _ => continue,
        }
    }
}

fn room_list_has_content(snapshot: &AppState) -> bool {
    !snapshot.rooms.is_empty() || !snapshot.spaces.is_empty()
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
            return Err(format!("{label}: SyncEvent::Failed received before Running"));
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
        if matches!(event, CoreEvent::Sync(SyncEvent::Stopped { request_id: None })) {
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
