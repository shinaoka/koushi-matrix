use super::cleanup::{
    cleanup_after_full_flow, cleanup_after_login_sync, cleanup_e2ee_callers_after_stage_failure,
    cleanup_normal_secondary_participant_for_qa,
};
use super::diagnostics::room_list_summary;
use super::event_wait::{
    find_timeline_item_with_body, wait_for_bodies_and_pagination_settle, wait_for_initial_items,
    wait_for_item_with_body, wait_for_logged_in, wait_for_logged_out,
    wait_for_operation_failed_and_signed_out, wait_for_ready_snapshot, wait_for_room_created,
    wait_for_room_joined, wait_for_send_completed, wait_for_send_flow_completion,
    wait_for_session_restored, wait_for_space_child_set, wait_for_space_created,
    wait_for_sync_started_and_running, wait_for_sync_stopped, wait_for_user_invited,
};
use super::fixtures::private_room_options;
use super::participants::{
    QaOwnedLoggedInRuntime, QaOwnedRuntimeParticipant, QaParticipantLoginGate,
    QaParticipantLoginOutcome, complete_new_identity_gate_for_qa, login_synced_participant_for_qa,
    qa_data_dir, retain_or_cleanup_e2ee_callers_after_stage,
};
use super::registry::{
    DEVICE_A, DEVICE_B, EVENT_TIMEOUT, QaConfig, QaScenario, QaStage, THREAD_REPLY_BODY,
    TimelineStressConfig, scenario_report, should_run_focused_send_queue_route,
    should_run_normal_secondary_participant,
};
use super::scenario_identity::{
    run_credential_health_stage, run_e2ee_login_store_scenario, run_e2ee_trust_stage,
    run_encryption_debug_stage, run_gate_negative_stage, run_gate_no_proof_stage,
    run_gate_restore_stage, run_native_attention_stage, run_provisional_device_cleanup_qa,
    run_session_status_stage,
};
use super::scenario_read_state::run_read_state_convergence_scenario;
use super::scenario_rooms::{
    run_directory_stage, run_invites_dm_stage, run_room_management_stage,
    run_room_people_projection_stage, wait_for_pin_event_completed, wait_for_pinned_state,
    wait_for_room_list_containing, wait_for_unpin_event_completed,
};
use super::scenario_search::{
    poll_search_until_absent, poll_search_until_found, run_hide_redacted_stage,
    run_search_crawler_stage, wait_for_paginate_end_reached,
};
use super::scenario_timeline::{
    assert_thread_reply_relation, run_activity_stage, run_cache_restore_scenario,
    run_composer_stage, run_focused_send_queue_scenario, run_link_preview_stage,
    run_live_signals_stage, run_media_stage, run_scheduled_send_stage, run_send_queue_stage,
    run_timeline_reconnect_scenario, run_timeline_stress_replay_stage, run_timeline_stress_stage,
    thread_initial_items_need_paginate_backfill, wait_for_edit_diff, wait_for_redact_diff,
    wait_for_room_timeline_thread_summary, wait_for_thread_panel_and_room_summary,
    wait_for_thread_reply_item, wait_for_timeline_navigation,
};
use super::{
    AccountCommand, AppCommand, AppState, AuthSecret, ComposerDocument, CoreCommand,
    CoreConnection, CoreFailure, CoreRuntime, PaginationDirection, ReplyQuoteState, RoomCommand,
    SyncCommand, TimelineCommand, TimelineKey, TimelineKind, TimelineUnreadPosition,
    TimelineViewportObservation,
};

async fn wait_for_redact_edit_snapshot(
    conn: &mut CoreConnection,
    label: &str,
    predicate: impl Fn(&AppState) -> bool,
) -> Result<(), String> {
    if predicate(&conn.snapshot()) {
        return Ok(());
    }
    tokio::time::timeout(EVENT_TIMEOUT, async {
        loop {
            conn.recv_event()
                .await
                .map_err(|error| format!("{label}: event stream failed: {error:?}"))?;
            if predicate(&conn.snapshot()) {
                return Ok(());
            }
        }
    })
    .await
    .map_err(|_| format!("{label}: timed out waiting for authoritative snapshot"))?
}

pub(super) async fn run_async(config: QaConfig, scenario: QaScenario) -> Result<String, String> {
    if scenario == QaScenario::Safety {
        println!("safety=ok");
        return Ok(scenario_report(&config.server_kind, scenario));
    }

    if scenario == QaScenario::CacheRestore {
        println!("safety=ok");
        run_cache_restore_scenario(&config).await?;
        return Ok(scenario_report(&config.server_kind, scenario));
    }

    if scenario == QaScenario::TimelineReconnect {
        println!("safety=ok");
        run_timeline_reconnect_scenario(&config).await?;
        return Ok(scenario_report(&config.server_kind, scenario));
    }
    if scenario == QaScenario::ReadStateConvergence {
        println!("safety=ok");
        run_read_state_convergence_scenario(&config).await?;
        return Ok(scenario_report(&config.server_kind, scenario));
    }
    if scenario == QaScenario::GateNoProof {
        println!("safety=ok");
        run_gate_no_proof_stage(&config).await?;
        return Ok(scenario_report(&config.server_kind, scenario));
    }
    if should_run_focused_send_queue_route(scenario) {
        println!("safety=ok");
        run_focused_send_queue_scenario(&config).await?;
        return Ok(scenario_report(&config.server_kind, scenario));
    }
    if scenario == QaScenario::E2eeLoginStore {
        println!("safety=ok");
        run_e2ee_login_store_scenario(&config).await?;
        return Ok(scenario_report(&config.server_kind, scenario));
    }

    // One CoreRuntime per synthetic user (two-device topology).
    let data_dir_a = qa_data_dir("a");
    let data_dir_b = qa_data_dir("b");

    // -----------------------------------------------------------------------
    // --- Login A (persistent store selected before authentication) ---
    // -----------------------------------------------------------------------
    let mut runtime_a = CoreRuntime::start_with_data_dir(data_dir_a.clone());
    let mut conn_a = runtime_a.attach();

    let login_a_id = conn_a.next_request_id();
    conn_a
        .command(CoreCommand::Account(AccountCommand::LoginPassword {
            request_id: login_a_id,
            request: koushi_state::LoginRequest {
                homeserver: config.homeserver.clone(),
                username: config.user_a.clone(),
                password: AuthSecret::new(config.password_a.clone()),
                device_display_name: Some(DEVICE_A.to_owned()),
            },
            platform: koushi_state::DisplayPlatform::Linux,
        }))
        .await
        .map_err(|e| format!("submit login A: {e}"))?;

    // The runner registers a fresh synthetic user per leg, so primary A always
    // arrives as a new identity and parks in the verification gate. `LoggedIn`
    // is held in the actor's pending-ready events until the session is promoted,
    // so the gate has to be completed here or the wait below can only time out.
    //
    // This used to be an allowlist of scenarios, which silently broke every
    // scenario missing from it — `media`, `login_sync`, `timeline`, `reply`, and
    // the rest all timed out at login. Scenarios that must not bootstrap have
    // their own login route and return from `run_async` above this point, so
    // completing the gate unconditionally here cannot reach them. The helper
    // returns `Ok(None)` when the session is already `Ready`.
    let bootstrap_recovery_secret_a =
        complete_new_identity_gate_for_qa(&mut conn_a, &config.password_a, "gate-bootstrap-a")
            .await?;
    println!("gate_new_identity_bootstrap=ok");

    let mut account_key_a = wait_for_logged_in(&mut conn_a, login_a_id, "login A").await?;
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

    wait_for_sync_started_and_running(&mut conn_a, sync_start_id, "sync start A").await?;

    println!("sync_a=running");
    println!("login_sync=ok");

    if scenario == QaScenario::TimelineStress {
        let stress = TimelineStressConfig::from_env()?;
        if stress.replay_existing {
            let runtime_b = CoreRuntime::start_with_data_dir(data_dir_b.clone());
            let mut conn_b = runtime_b.attach();

            let login_b_id = conn_b.next_request_id();
            conn_b
                .command(CoreCommand::Account(AccountCommand::LoginPassword {
                    request_id: login_b_id,
                    request: koushi_state::LoginRequest {
                        homeserver: config.homeserver.clone(),
                        username: config.user_b.clone(),
                        password: AuthSecret::new(config.password_b.clone()),
                        device_display_name: Some(DEVICE_B.to_owned()),
                    },
                    platform: koushi_state::DisplayPlatform::Linux,
                }))
                .await
                .map_err(|e| format!("timeline_stress replay: submit login B failed: {e}"))?;

            let account_key_b =
                wait_for_logged_in(&mut conn_b, login_b_id, "timeline_stress replay login B")
                    .await?;
            wait_for_ready_snapshot(&mut conn_b, "timeline_stress replay session B Ready").await?;

            let sync_start_b_id = conn_b.next_request_id();
            conn_b
                .command(CoreCommand::Sync(SyncCommand::Start {
                    request_id: sync_start_b_id,
                }))
                .await
                .map_err(|e| format!("timeline_stress replay: submit sync start B failed: {e}"))?;

            wait_for_sync_started_and_running(
                &mut conn_b,
                sync_start_b_id,
                "timeline_stress replay sync start B",
            )
            .await?;
            println!("sync_b=running");

            run_timeline_stress_replay_stage(
                &mut conn_a,
                &mut conn_b,
                &account_key_a,
                &account_key_b,
                stress,
            )
            .await?;
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
    }

    if scenario.should_run_stage(QaStage::SessionStatus) {
        run_session_status_stage(&mut conn_a).await?;
    }

    if scenario.should_run_stage(QaStage::CredentialHealth) {
        run_credential_health_stage(&mut conn_a).await?;
    }

    if scenario.should_run_stage(QaStage::NativeAttention) {
        run_native_attention_stage(&mut conn_a).await?;
    }

    if scenario.should_run_stage(QaStage::EncryptionDebug) {
        run_encryption_debug_stage(&config, &mut conn_a, &account_key_a).await?;
    }

    if scenario == QaScenario::DeviceCleanup {
        run_provisional_device_cleanup_qa(&config).await?;
        drop(conn_a);
        runtime_a.shutdown().await;
        return Ok(scenario_report(&config.server_kind, scenario));
    }

    if scenario == QaScenario::E2eeTrust {
        run_e2ee_trust_stage(&config, &mut conn_a, &account_key_a, None).await?;
        drop(conn_a);
        runtime_a.shutdown().await;
        return Ok(scenario_report(&config.server_kind, scenario));
    }

    if scenario == QaScenario::GateRestore {
        run_gate_restore_stage(conn_a, runtime_a, data_dir_a, account_key_a).await?;
        return Ok(scenario_report(&config.server_kind, scenario));
    }

    if scenario == QaScenario::GateNegative {
        run_gate_negative_stage(
            &config,
            &mut conn_a,
            bootstrap_recovery_secret_a
                .as_ref()
                .ok_or_else(|| "gate negative bootstrap recovery secret unavailable".to_owned())?,
        )
        .await?;
        drop(conn_a);
        runtime_a.shutdown().await;
        return Ok(scenario_report(&config.server_kind, scenario));
    }

    let mut normal_secondary = if should_run_normal_secondary_participant(scenario) {
        let participant = login_synced_participant_for_qa(
            &config.homeserver,
            data_dir_b.clone(),
            &config.user_b,
            &config.password_b,
            DEVICE_B,
            "normal secondary login B",
            "normal secondary bootstrap gate B",
            QaParticipantLoginGate::BootstrapNewIdentity,
        )
        .await?;
        println!("sync_b=running");
        Some(participant)
    } else {
        None
    };

    if scenario.should_run_stage(QaStage::InvitesDm) {
        let participant_b = normal_secondary
            .as_mut()
            .ok_or_else(|| "InvitesDm requires the normal secondary participant".to_owned())?;
        run_invites_dm_stage(&config, &mut conn_a, &mut participant_b.conn).await?;
    }

    if scenario == QaScenario::InvitesDm {
        cleanup_normal_secondary_participant_for_qa(
            &mut normal_secondary,
            "InvitesDm normal secondary cleanup",
        )
        .await?;
        cleanup_after_login_sync(conn_a, runtime_a, data_dir_a, account_key_a).await?;
        return Ok(scenario_report(&config.server_kind, scenario));
    }

    if scenario.should_run_stage(QaStage::Directory) {
        let conn_b = &mut normal_secondary
            .as_mut()
            .ok_or_else(|| "Directory requires the normal secondary participant".to_owned())?
            .conn;
        run_directory_stage(&config, &mut conn_a, conn_b).await?;
    }

    if !scenario.should_run_stage(QaStage::RoomSpace) {
        cleanup_normal_secondary_participant_for_qa(
            &mut normal_secondary,
            "pre-RoomSpace normal secondary cleanup",
        )
        .await?;
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
            options: private_room_options("QA Room", false),
        }))
        .await
        .map_err(|e| format!("submit create room: {e}"))?;

    let room_id = wait_for_room_created(&mut conn_a, create_room_id, "create room").await?;
    println!("room_created=ok");

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
    println!("space_created=ok");

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
    // --- Reuse centrally logged-in B + join room + join space ---
    // -----------------------------------------------------------------------
    let normal_secondary = normal_secondary.take();
    let normal_secondary = normal_secondary
        .ok_or_else(|| "RoomSpace requires the normal secondary participant".to_owned())?;
    let QaParticipantLoginOutcome {
        runtime: mut runtime_b,
        conn: mut conn_b,
        account_key: mut account_key_b,
        bootstrap_recovery_secret: _,
    } = normal_secondary;

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

    if scenario.should_run_stage(QaStage::RoomPeopleProjection) {
        run_room_people_projection_stage(
            &config,
            &mut conn_a,
            &mut conn_b,
            &account_key_a,
            &account_key_b,
            &room_id,
        )
        .await?;
    }

    if scenario.should_run_stage(QaStage::RoomManagement) {
        run_room_management_stage(
            &config,
            &mut conn_a,
            &mut conn_b,
            &account_key_a,
            &account_key_b,
        )
        .await?;
    }

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
            document: koushi_state::ComposerDocument::from_plain_text(
                "Phase 5 QA message 1".to_owned(),
            ),
        }))
        .await
        .map_err(|e| format!("submit send1: {e}"))?;

    let send1_outcome = wait_for_send_flow_completion(
        &mut conn_a,
        send1_id,
        &key_a,
        &txn1,
        "Phase 5 QA message 1",
        "send flow msg1",
    )
    .await?;
    let _echo1_sdk_txn = send1_outcome.sdk_transaction_id;
    let event1_id = send1_outcome.event_id;
    println!("local_echo_msg1=ok");
    println!("send_completed_msg1=ok");

    // A sends message 2.
    let txn2 = "qa-phase5-txn-2".to_owned();
    let send2_id = conn_a.next_request_id();
    conn_a
        .command(CoreCommand::Timeline(TimelineCommand::SendText {
            request_id: send2_id,
            key: key_a.clone(),
            transaction_id: txn2.clone(),
            document: koushi_state::ComposerDocument::from_plain_text(
                "Phase 5 QA message 2".to_owned(),
            ),
        }))
        .await
        .map_err(|e| format!("submit send2: {e}"))?;

    let send2_outcome = wait_for_send_flow_completion(
        &mut conn_a,
        send2_id,
        &key_a,
        &txn2,
        "Phase 5 QA message 2",
        "send flow msg2",
    )
    .await?;
    let _echo2_sdk_txn = send2_outcome.sdk_transaction_id;
    let event2_id = send2_outcome.event_id;
    println!("local_echo_msg2=ok");
    println!("send_completed_msg2=ok");

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

    let nav_marker_id = conn_b.next_request_id();
    conn_b
        .command(CoreCommand::Timeline(TimelineCommand::SetFullyRead {
            request_id: nav_marker_id,
            key: key_b.clone(),
            event_id: event1_id.clone(),
        }))
        .await
        .map_err(|e| format!("submit navigation fully-read marker: {e}"))?;
    let nav_viewport_id = conn_b.next_request_id();
    conn_b
        .command(CoreCommand::Timeline(TimelineCommand::ObserveViewport {
            request_id: nav_viewport_id,
            key: key_b.clone(),
            observation: TimelineViewportObservation {
                first_visible_event_id: Some(event1_id.clone()),
                last_visible_event_id: Some(event1_id.clone()),
                visible_gap_ids: Vec::new(),
                at_bottom: false,
            },
        }))
        .await
        .map_err(|e| format!("submit navigation viewport observation: {e}"))?;
    wait_for_timeline_navigation(
        &mut conn_b,
        &key_b,
        TimelineUnreadPosition::BelowViewport,
        1,
        1,
        "timeline navigation",
    )
    .await?;
    println!("timeline_nav=ok");

    // A edits message 1 — assert a Set diff reflecting the edit on original item identity.
    let edit1_id = conn_a.next_request_id();
    conn_a
        .command(CoreCommand::Timeline(TimelineCommand::EditText {
            request_id: edit1_id,
            key: key_a.clone(),
            event_id: event1_id.clone(),
            document: ComposerDocument::from_plain_text("Phase 5 QA message 1 EDITED"),
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

    run_hide_redacted_stage(&mut conn_a, &key_a).await?;

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

    if scenario.should_run_stage(QaStage::LiveSignals) {
        run_live_signals_stage(
            &mut conn_a,
            &mut conn_b,
            &key_a,
            &key_b,
            &event1_id,
            &account_key_b.0,
        )
        .await?;
    }

    if scenario.should_run_stage(QaStage::Activity) {
        run_activity_stage(&mut conn_a, &mut conn_b, &key_a, &key_b, &room_id).await?;
    }

    if scenario.should_run_stage(QaStage::Composer) {
        run_composer_stage(&mut conn_a, &key_a, &account_key_b.0).await?;
    }

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
                document: koushi_state::ComposerDocument::from_plain_text(
                    "Phase 5 QA reply from B".to_owned(),
                ),
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
            return Err("reply relation mismatch".to_owned());
        }
        println!("reply=ok");

        let Some(reply_quote) = reply_item.reply_quote.as_ref() else {
            return Err("reply_quote failed: missing quote".to_owned());
        };
        if reply_quote.event_id != event1_id
            || reply_quote.state != ReplyQuoteState::Ready
            || reply_quote.body_preview.is_none()
        {
            return Err("reply_quote failed: quote was not ready".to_owned());
        }
        println!("reply_quote=ok");

        let pin_id = conn_a.next_request_id();
        conn_a
            .command(CoreCommand::Room(RoomCommand::PinEvent {
                request_id: pin_id,
                room_id: room_id.clone(),
                event_id: event1_id.clone(),
            }))
            .await
            .map_err(|e| format!("submit pin event: {e}"))?;
        wait_for_pin_event_completed(&mut conn_a, pin_id, "pin event completed").await?;
        println!("pin_event=ok");

        wait_for_pinned_state(
            &mut conn_a,
            &room_id,
            &event1_id,
            true,
            "pinned state after pin",
        )
        .await?;
        println!("pinned_state=ok");

        let unpin_id = conn_a.next_request_id();
        conn_a
            .command(CoreCommand::Room(RoomCommand::UnpinEvent {
                request_id: unpin_id,
                room_id: room_id.clone(),
                event_id: event1_id.clone(),
            }))
            .await
            .map_err(|e| format!("submit unpin event: {e}"))?;
        wait_for_unpin_event_completed(&mut conn_a, unpin_id, "unpin event completed").await?;
        wait_for_pinned_state(
            &mut conn_a,
            &room_id,
            &event1_id,
            false,
            "pinned state after unpin",
        )
        .await?;
        println!("unpin_event=ok");
    }

    if scenario.should_run_stage(QaStage::Media) {
        run_media_stage(&mut conn_a, &mut conn_b, &key_a, &key_b).await?;
    }

    if scenario.should_run_stage(QaStage::LinkPreview) {
        run_link_preview_stage(&mut conn_a, &mut conn_b, &key_a, &key_b).await?;
    }

    let mut thread_summary_restore_expectation: Option<(String, String, u32)> = None;
    if scenario.should_run_stage(QaStage::Thread) {
        // -------------------------------------------------------------------
        // --- Phase 5c: Thread timeline QA ---
        // -------------------------------------------------------------------

        let thread_key_b = TimelineKey {
            account_key: account_key_b.clone(),
            kind: TimelineKind::Thread {
                room_id: room_id.clone(),
                root_event_id: event1_id.clone(),
            },
        };
        let subscribe_thread_b_id = conn_b.next_request_id();
        conn_b
            .command(CoreCommand::Timeline(TimelineCommand::Subscribe {
                request_id: subscribe_thread_b_id,
                key: thread_key_b.clone(),
            }))
            .await
            .map_err(|e| format!("submit subscribe thread B: {e}"))?;

        wait_for_initial_items(
            &mut conn_b,
            &thread_key_b,
            subscribe_thread_b_id,
            "subscribe thread B",
        )
        .await?;

        let txn_b_thread_reply = "qa-phase11-txn-b-thread-reply".to_owned();
        let send_b_thread_reply_id = conn_b.next_request_id();
        conn_b
            .command(CoreCommand::Timeline(TimelineCommand::SendReply {
                request_id: send_b_thread_reply_id,
                key: thread_key_b.clone(),
                transaction_id: txn_b_thread_reply.clone(),
                in_reply_to_event_id: event1_id.clone(),
                document: koushi_state::ComposerDocument::from_plain_text(
                    THREAD_REPLY_BODY.to_owned(),
                ),
            }))
            .await
            .map_err(|e| format!("submit B thread reply: {e}"))?;

        let (_thread_b_echo_txn, thread_b_reply_event_id) = wait_for_send_completed(
            &mut conn_b,
            send_b_thread_reply_id,
            &thread_key_b,
            "B thread reply completed",
        )
        .await?;

        let refresh_room_a_id = conn_a.next_request_id();
        conn_a
            .command(CoreCommand::Timeline(TimelineCommand::Subscribe {
                request_id: refresh_room_a_id,
                key: key_a.clone(),
            }))
            .await
            .map_err(|e| format!("submit refresh room timeline A: {e}"))?;

        let refreshed_room_items = wait_for_initial_items(
            &mut conn_a,
            &key_a,
            refresh_room_a_id,
            "refresh room timeline A after thread send",
        )
        .await?;
        wait_for_room_timeline_thread_summary(
            &mut conn_a,
            &key_a,
            &refreshed_room_items,
            THREAD_REPLY_BODY,
            &thread_b_reply_event_id,
            1,
            &event1_id,
            "wait for A room live thread summary",
        )
        .await?;
        println!("thread_canonical=ok");
        println!("thread_summary=ok");

        let thread_key_a = TimelineKey {
            account_key: account_key_a.clone(),
            kind: TimelineKind::Thread {
                room_id: room_id.clone(),
                root_event_id: event1_id.clone(),
            },
        };
        let subscribe_thread_a_id = conn_a.next_request_id();
        conn_a
            .command(CoreCommand::Timeline(TimelineCommand::Subscribe {
                request_id: subscribe_thread_a_id,
                key: thread_key_a.clone(),
            }))
            .await
            .map_err(|e| format!("submit subscribe thread A: {e}"))?;

        let thread_initial_items = wait_for_initial_items(
            &mut conn_a,
            &thread_key_a,
            subscribe_thread_a_id,
            "subscribe thread A after thread send",
        )
        .await?;

        let thread_item = if thread_initial_items_need_paginate_backfill(
            &thread_initial_items,
            THREAD_REPLY_BODY,
        ) {
            wait_for_thread_reply_item(
                &mut conn_a,
                &thread_key_a,
                &thread_initial_items,
                THREAD_REPLY_BODY,
                "A receives thread reply from B",
            )
            .await?
        } else {
            find_timeline_item_with_body(&thread_initial_items, THREAD_REPLY_BODY)
                .expect("thread reply present after initial scan")
        };
        assert_thread_reply_relation(&thread_item, &event1_id)?;
        println!("thread_recv=ok");

        if scenario.should_run_stage(QaStage::RedactEditConvergence) {
            const LIVE_THREAD_BODY: &str = "Phase 11 QA live thread reply B";
            const EDITED_LIVE_THREAD_BODY: &str = "Phase 11 QA live thread reply B edited";
            let live_send_id = conn_b.next_request_id();
            conn_b
                .command(CoreCommand::Timeline(TimelineCommand::SendReply {
                    request_id: live_send_id,
                    key: thread_key_b.clone(),
                    transaction_id: "qa-thread-summary-live-b".to_owned(),
                    in_reply_to_event_id: event1_id.clone(),
                    document: ComposerDocument::from_plain_text(LIVE_THREAD_BODY),
                }))
                .await
                .map_err(|e| format!("submit live thread-summary reply: {e}"))?;
            let (_, live_reply_event_id) = wait_for_send_completed(
                &mut conn_b,
                live_send_id,
                &thread_key_b,
                "live thread-summary reply completed",
            )
            .await?;
            wait_for_thread_panel_and_room_summary(
                &mut conn_a,
                &key_a,
                &refreshed_room_items,
                &thread_key_a,
                &thread_initial_items,
                LIVE_THREAD_BODY,
                &live_reply_event_id,
                2,
                &event1_id,
                "live thread summary",
            )
            .await?;

            let edit_live_id = conn_b.next_request_id();
            conn_b
                .command(CoreCommand::Timeline(TimelineCommand::EditText {
                    request_id: edit_live_id,
                    key: thread_key_b.clone(),
                    event_id: live_reply_event_id.clone(),
                    document: ComposerDocument::from_plain_text(EDITED_LIVE_THREAD_BODY),
                }))
                .await
                .map_err(|e| format!("submit live thread-summary edit: {e}"))?;
            wait_for_edit_diff(
                &mut conn_b,
                &thread_key_b,
                edit_live_id,
                &live_reply_event_id,
                EDITED_LIVE_THREAD_BODY,
                "live thread-summary edit",
            )
            .await?;
            wait_for_thread_panel_and_room_summary(
                &mut conn_a,
                &key_a,
                &refreshed_room_items,
                &thread_key_a,
                &thread_initial_items,
                EDITED_LIVE_THREAD_BODY,
                &live_reply_event_id,
                2,
                &event1_id,
                "edited live thread summary",
            )
            .await?;

            let redact_live_id = conn_b.next_request_id();
            conn_b
                .command(CoreCommand::Timeline(TimelineCommand::Redact {
                    request_id: redact_live_id,
                    key: thread_key_b.clone(),
                    event_id: live_reply_event_id,
                }))
                .await
                .map_err(|e| format!("submit live thread-summary redaction: {e}"))?;
            wait_for_redact_diff(
                &mut conn_b,
                &thread_key_b,
                redact_live_id,
                "live thread-summary redaction",
            )
            .await?;
            wait_for_thread_panel_and_room_summary(
                &mut conn_a,
                &key_a,
                &refreshed_room_items,
                &thread_key_a,
                &thread_initial_items,
                THREAD_REPLY_BODY,
                &thread_b_reply_event_id,
                1,
                &event1_id,
                "redacted live thread summary",
            )
            .await?;
            thread_summary_restore_expectation = Some((
                thread_b_reply_event_id.clone(),
                THREAD_REPLY_BODY.to_owned(),
                1,
            ));
        }

        let thread_paginate_id = conn_a.next_request_id();
        conn_a
            .command(CoreCommand::Timeline(TimelineCommand::Paginate {
                request_id: thread_paginate_id,
                key: thread_key_a.clone(),
                direction: PaginationDirection::Backward,
                event_count: 5,
            }))
            .await
            .map_err(|e| format!("submit thread paginate: {e}"))?;

        let thread_paginate_result = wait_for_paginate_end_reached(
            &mut conn_a,
            &thread_key_a,
            thread_paginate_id,
            "thread paginate to EndReached",
        )
        .await?;
        println!("thread_paginate={thread_paginate_result}");

        let unsub_thread_a_id = conn_a.next_request_id();
        conn_a
            .command(CoreCommand::Timeline(TimelineCommand::Unsubscribe {
                request_id: unsub_thread_a_id,
                key: thread_key_a.clone(),
            }))
            .await
            .map_err(|e| format!("submit unsubscribe thread A: {e}"))?;

        let unsub_thread_b_id = conn_b.next_request_id();
        conn_b
            .command(CoreCommand::Timeline(TimelineCommand::Unsubscribe {
                request_id: unsub_thread_b_id,
                key: thread_key_b.clone(),
            }))
            .await
            .map_err(|e| format!("submit unsubscribe thread B: {e}"))?;
    }

    if scenario.should_run_stage(QaStage::ScheduledSend) {
        run_scheduled_send_stage(&mut conn_a, &key_a, &room_id).await?;
    }

    if scenario.should_run_stage(QaStage::TimelineStress) {
        run_timeline_stress_stage(
            &config,
            &mut conn_a,
            &mut conn_b,
            &account_key_a,
            &account_key_b,
        )
        .await?;
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

    // Unsubscribe has no completion event (it just drops the timeline actor,
    // per the timeline spec). No blind sleep is needed: the next step that
    // depends on this connection — a re-subscribe awaiting InitialItems, or a
    // sync stop awaiting SyncStopped — is dispatched after these unsubscribes
    // on the same FIFO-ordered connection, so the actor is dropped first and
    // the following request-id-scoped wait provides the real synchronization.
    println!("timeline=ok");

    if scenario.should_run_stage(QaStage::SendQueue) {
        let recovery_secret = bootstrap_recovery_secret_a
            .as_ref()
            .ok_or_else(|| "send_queue: primary recovery secret unavailable".to_owned())?;
        run_send_queue_stage(&config, recovery_secret).await?;
    }

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
            document: koushi_state::ComposerDocument::from_plain_text(SEARCH_BODY.to_owned()),
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
    println!("search_msg_sent=ok");

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
            document: ComposerDocument::from_plain_text(EDITED_BODY),
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
    if scenario.should_run_stage(QaStage::RedactEditConvergence) {
        wait_for_redact_edit_snapshot(&mut conn_a, "edited room latest", |snapshot| {
            snapshot.rooms.iter().any(|room| {
                room.room_id == room_id
                    && room.latest_event.as_ref().is_some_and(|latest| {
                        latest.event_id == search_event_id
                            && !latest.is_redacted
                            && latest.preview.as_deref() == Some(EDITED_BODY)
                    })
            })
        })
        .await?;
    }

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

    if scenario.should_run_stage(QaStage::RedactEditConvergence) {
        let redact_latest_id = conn_a.next_request_id();
        conn_a
            .command(CoreCommand::Timeline(TimelineCommand::Redact {
                request_id: redact_latest_id,
                key: key_a_search.clone(),
                event_id: search_event_id.clone(),
            }))
            .await
            .map_err(|e| format!("submit redact latest convergence msg: {e}"))?;
        wait_for_redact_diff(
            &mut conn_a,
            &key_a_search,
            redact_latest_id,
            "redact latest convergence msg",
        )
        .await?;

        let open_activity_id = conn_a.next_request_id();
        conn_a
            .command(CoreCommand::App(AppCommand::OpenActivity {
                request_id: open_activity_id,
            }))
            .await
            .map_err(|e| format!("submit convergence Activity open: {e}"))?;
        wait_for_redact_edit_snapshot(&mut conn_a, "redact/edit convergence", |snapshot| {
            let room_latest_converged = snapshot.rooms.iter().any(|room| {
                room.room_id == room_id
                    && room.latest_event.as_ref().is_some_and(|latest| {
                        latest.event_id != search_event_id && !latest.is_redacted
                    })
            });
            let activity_converged = matches!(
                &snapshot.activity,
                koushi_state::ActivityState::Open { recent, unread, .. }
                    if recent.rows.iter().chain(&unread.rows).all(|row| {
                        row.event_id.as_deref() != Some(search_event_id.as_str())
                    })
            );
            room_latest_converged && activity_converged
        })
        .await?;
        println!("redact_edit_convergence=ok");
    }

    if scenario.should_run_stage(QaStage::SearchCrawler) {
        run_search_crawler_stage(&mut conn_a, &account_key_a, &room_id).await?;
    }

    // Unsubscribe search timeline.
    let unsub_search_id = conn_a.next_request_id();
    conn_a
        .command(CoreCommand::Timeline(TimelineCommand::Unsubscribe {
            request_id: unsub_search_id,
            key: key_a_search.clone(),
        }))
        .await
        .map_err(|e| format!("submit unsubscribe search timeline: {e}"))?;

    // Unsubscribe has no completion event (it just drops the timeline actor).
    // The sync stop below is dispatched after it on the same FIFO-ordered
    // connection, so the actor is dropped before sync stop runs and
    // `wait_for_sync_stopped` (request-id-scoped) is the concrete wait.

    if scenario == QaScenario::All {
        let e2ee_stage_result = run_e2ee_trust_stage(
            &config,
            &mut conn_a,
            &account_key_a,
            Some((&mut conn_b, &account_key_b)),
        )
        .await;
        let (caller_a, caller_b) = retain_or_cleanup_e2ee_callers_after_stage(
            e2ee_stage_result,
            (
                QaOwnedRuntimeParticipant::from_logged_in(QaOwnedLoggedInRuntime {
                    runtime: runtime_a,
                    conn: conn_a,
                    account_key: account_key_a,
                }),
                QaOwnedRuntimeParticipant::from_logged_in(QaOwnedLoggedInRuntime {
                    runtime: runtime_b,
                    conn: conn_b,
                    account_key: account_key_b,
                }),
            ),
            cleanup_e2ee_callers_after_stage_failure,
        )
        .await?;
        let caller_a = caller_a.into_logged_in_runtime();
        let caller_b = caller_b.into_logged_in_runtime();
        runtime_a = caller_a.runtime;
        conn_a = caller_a.conn;
        account_key_a = caller_a.account_key;
        runtime_b = caller_b.runtime;
        conn_b = caller_b.conn;
        account_key_b = caller_b.account_key;
    }

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
    runtime_a.shutdown().await;

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

    if let Some((latest_event_id, latest_body, reply_count)) =
        thread_summary_restore_expectation.as_ref()
    {
        let restored_room_key = TimelineKey::room(account_key_a.clone(), room_id.clone());
        let restored_subscribe_id = conn_a2.next_request_id();
        conn_a2
            .command(CoreCommand::Timeline(TimelineCommand::Subscribe {
                request_id: restored_subscribe_id,
                key: restored_room_key.clone(),
            }))
            .await
            .map_err(|e| format!("submit restored thread-summary room: {e}"))?;
        let restored_items = wait_for_initial_items(
            &mut conn_a2,
            &restored_room_key,
            restored_subscribe_id,
            "restored thread-summary room",
        )
        .await?;
        wait_for_room_timeline_thread_summary(
            &mut conn_a2,
            &restored_room_key,
            &restored_items,
            latest_body,
            latest_event_id,
            *reply_count,
            &event1_id,
            "restored thread summary",
        )
        .await?;
        println!("thread_summary_convergence=ok");
    }

    let logout_a_id = conn_a2.next_request_id();
    conn_a2
        .command(CoreCommand::Account(AccountCommand::Logout {
            request_id: logout_a_id,
        }))
        .await
        .map_err(|e| format!("submit logout A: {e}"))?;

    wait_for_logged_out(&mut conn_a2, logout_a_id, &account_key_a, "logout A").await?;

    // Cleanup assertion: normal logout preserves local account persistence for
    // explicit restore while still clearing the last-session startup pointer.
    let restore_preserved_id = conn_a2.next_request_id();
    conn_a2
        .command(CoreCommand::Account(AccountCommand::RestoreSession {
            request_id: restore_preserved_id,
            account_key: account_key_a.clone(),
        }))
        .await
        .map_err(|e| format!("submit post-logout restore A: {e}"))?;

    wait_for_session_restored(
        &mut conn_a2,
        restore_preserved_id,
        &account_key_a,
        "post-logout explicit restore A",
    )
    .await?;
    wait_for_ready_snapshot(&mut conn_a2, "post-logout explicit restore A Ready").await?;

    let restored_logout_id = conn_a2.next_request_id();
    conn_a2
        .command(CoreCommand::Account(AccountCommand::Logout {
            request_id: restored_logout_id,
        }))
        .await
        .map_err(|e| format!("submit restored logout A: {e}"))?;
    wait_for_logged_out(
        &mut conn_a2,
        restored_logout_id,
        &account_key_a,
        "restored logout A",
    )
    .await?;
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

    let failure = wait_for_operation_failed_and_signed_out(
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
    drop(conn_b);
    runtime_b.shutdown().await;

    println!("restore_cleanup=ok");
    Ok(scenario_report(&config.server_kind, scenario))
}
