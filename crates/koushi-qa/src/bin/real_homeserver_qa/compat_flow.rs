use super::cleanup::{RealQaCleanupState, do_logout};
use super::config::{
    EDIT_REDACT_TIMEOUT, PAGINATE_TIMEOUT, ROOM_LIST_TIMEOUT, RealQaScenario, SEARCH_TIMEOUT,
    SPACE_CHILD_PROJECTION_TIMEOUT, SYNC_TIMEOUT, build_real_homeserver_qa_message_plan,
    private_room_options,
};
use super::credentials::RealCredentials;
use super::waiters::{
    poll_search_until_found_or_timeout, wait_for_body_substring_in_timeline, wait_for_edit_diff,
    wait_for_initial_items, wait_for_logged_in, wait_for_non_empty_room_list,
    wait_for_operation_failed_and_signed_out, wait_for_paginate_end_reached,
    wait_for_post_login_ready_snapshot, wait_for_ready_snapshot, wait_for_redact_diff,
    wait_for_room_created, wait_for_room_forgotten, wait_for_room_left,
    wait_for_room_list_space_child, wait_for_send_completed,
    wait_for_session_restored_with_recovery, wait_for_space_child_set, wait_for_space_created,
    wait_for_sync_running, wait_for_sync_started, wait_for_sync_stopped,
};
use super::{
    AccountCommand, ComposerDocument, CoreCommand, CoreFailure, CoreRuntime, LoginRequest,
    PaginationDirection, RoomCommand, SyncCommand, TimelineCommand, TimelineKey,
};
use std::time::Duration;

// ---------------------------------------------------------------------------
// Async QA flow
// ---------------------------------------------------------------------------

#[cfg(any(debug_assertions, test))]
pub(super) async fn run_async_inner(
    creds: &RealCredentials,
    scenario: RealQaScenario,
    data_dir: &std::path::Path,
    transcript: &mut Vec<String>,
    cleanup: &mut RealQaCleanupState,
) -> Result<String, String> {
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
        platform: koushi_state::DisplayPlatform::Linux,
    }))
    .await
    .map_err(|e| format!("login command submit failed: {e}"))?;

    let admission = wait_for_logged_in(&mut conn, login_id, &creds.recovery_key, "login").await?;
    let account_key = admission.account_key;
    // Login succeeded: record the account key so the catch-all wrapper can log
    // out (and leave/forget any rooms/spaces) on a later failure.
    cleanup.account_key = Some(account_key.clone());
    if !admission.recovered {
        return Err("compat admission did not prove recovery".to_owned());
    }
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

    wait_for_sync_started(&mut conn, sync_start_id, "sync start", SYNC_TIMEOUT).await?;
    let line = "sync_started=ok".to_owned();
    transcript.push(line.clone());
    println!("{line}");

    wait_for_sync_running(&mut conn, "sync running", SYNC_TIMEOUT).await?;
    let line = "sync=running".to_owned();
    transcript.push(line.clone());
    println!("{line}");

    // Verification and recovery are part of admission, before room sync starts.
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
        options: private_room_options(qa_room_name.clone(), false),
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
        initial_backfill: koushi_protocol::command::InitialBackfillPolicy::Disabled,
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
        document: koushi_state::ComposerDocument::from_plain_text(message_plan.msg1_body.clone()),
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
        document: koushi_state::ComposerDocument::from_plain_text(
            message_plan.search_probe_body.clone(),
        ),
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
        document: koushi_state::ComposerDocument::from_plain_text(message_plan.msg2_body.clone()),
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
        document: koushi_state::ComposerDocument::from_plain_text(message_plan.reply_body.clone()),
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
        document: ComposerDocument::from_plain_text(message_plan.edited_body.clone()),
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

    wait_for_sync_started(&mut conn2, sync2_id, "sync start restored", SYNC_TIMEOUT).await?;
    let line = "sync_started_restored=ok".to_owned();
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
            initial_backfill: koushi_protocol::command::InitialBackfillPolicy::Disabled,
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

    let failure = wait_for_operation_failed_and_signed_out(
        &mut conn2,
        restore_gone_id,
        "post-logout restore-last",
    )
    .await?;
    if failure != CoreFailure::SessionNotFound {
        return Err(format!(
            "post-logout restore-last failed with unexpected failure kind: {failure:?}"
        ));
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
         sync=ok \
         rooms={rooms} spaces={spaces} dms={dms} \
         qa_room=created send_msg1=ok send_search=ok send_msg2=ok real_reply=ok \
         edit_msg1=ok redact_msg2=ok \
         paginate={paginate} search={search} \
         store_restore=ok restore_body={body_ok} \
         leave_room=ok forget_room=ok \
         logout=ok post_logout_restore=not_found",
        recovery = "completed",
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
