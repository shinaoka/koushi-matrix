use super::cleanup::{RealQaCleanupState, do_logout};
use super::config::{
    ENV_STARTUP_LAT_TEARDOWN, EVENT_TIMEOUT, PAGINATE_TIMEOUT, ROOM_LIST_TIMEOUT,
    STARTUP_LAT_PAGES, SYNC_TIMEOUT,
};
use super::credentials::RealCredentials;
use super::waiters::{
    wait_for_initial_items, wait_for_logged_in, wait_for_non_empty_room_list,
    wait_for_ready_handling_recovery, wait_for_sync_running, wait_for_sync_started,
};
use super::{
    AccountCommand, AccountEvent, AccountKey, CoreCommand, CoreConnection, CoreEvent, CoreFailure,
    CoreRuntime, LoginRequest, PaginationDirection, PaginationState, RequestId, SyncCommand,
    TimelineCommand, TimelineEvent, TimelineFailureKind, TimelineKey,
};

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
pub(super) async fn run_startup_latency_scenario(
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
                    platform: koushi_state::DisplayPlatform::Linux,
                }))
                .await
                .map_err(|e| format!("startup_latency login command submit failed: {e}"))?;

                let key = wait_for_logged_in(
                    &mut conn,
                    login_id,
                    &creds.recovery_key,
                    "startup_latency login",
                )
                .await?;
                break key.account_key;
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
        initial_backfill: koushi_protocol::command::InitialBackfillPolicy::Disabled,
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
pub(super) enum PaginationTerminal {
    Idle,
    EndReached,
    Failed(TimelineFailureKind),
}

/// Wait for a single terminal `PaginationStateChanged` event for `key` backward,
/// correlating on the request id via `OperationFailed` fallback.
#[cfg(any(debug_assertions, test))]
pub(super) async fn wait_for_pagination_terminal(
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
pub(super) async fn finish_startup_latency(
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
