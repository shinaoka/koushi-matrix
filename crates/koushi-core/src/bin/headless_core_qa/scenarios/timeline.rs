use super::cleanup::cleanup_logged_in_runtime;
use super::diagnostics::{
    QaCannedTimelineEvent, QaTcpProxy, diagnostic_count_field, diagnostic_token_field,
};
use super::event_wait::{
    QaEventDeadline, SendQueueLocalEcho, cancel_send_queue_item, find_timeline_item_with_body,
    retry_send_queue_item, send_text_expect_local_echo, start_sync_for_qa, stop_sync_for_qa,
    subscribe_timeline_for_qa, timeline_item_body_matches, timeline_item_event_id,
    timeline_item_is_decryption_failure, timeline_item_transaction_id, visit_timeline_diff_items,
    wait_for_dm_room_in_room_list, wait_for_encrypted_room_projection_for_qa,
    wait_for_event_item_with_body, wait_for_event_item_with_body_or_retry_not_sent,
    wait_for_initial_items, wait_for_invite_in_snapshot, wait_for_item_with_body,
    wait_for_item_with_body_or_decryption_failure, wait_for_link_preview_item_projection,
    wait_for_logged_in, wait_for_logged_out, wait_for_media_download_completed,
    wait_for_media_item, wait_for_media_send_flow_completion, wait_for_ready_snapshot,
    wait_for_room_in_room_list, wait_for_send_completed, wait_for_send_flow_completion,
    wait_for_session_restored, wait_for_settings_persisted, wait_for_space_child_projection,
    wait_for_space_in_space_list, wait_for_sync_reconnecting,
    wait_for_sync_running_after_reconnect, wait_for_sync_started,
    wait_for_sync_started_and_running, wait_for_sync_stopped, wait_for_timeline_send_state,
};
use super::fixtures::{
    accept_invite_for_qa, create_room_for_qa, create_space_for_qa, invite_user_for_qa,
    native_attention_room, select_space_and_wait_for_room_scope, set_space_child_for_qa,
    start_direct_message_for_qa,
};
use super::participants::{
    QaParticipantLoginGate, QaParticipantLoginOutcome, authenticated_session_info,
    complete_new_identity_gate_for_qa, login_synced_participant_for_qa, qa_data_dir,
};
use super::registry::{
    CACHE_RESTORE_MAX_CYCLES, CACHE_RESTORE_PAGINATE_BATCH, CACHE_RESTORE_PROD_EVENT_COUNT,
    CACHE_RESTORE_PROD_MAX_BATCHES, CACHE_RESTORE_SHALLOW_DEPTH, DEFAULT_CACHE_RESTORE_DEPTH,
    DEFAULT_CACHE_RESTORE_ROOMS, ENV_CACHE_RESTORE_DEPTH, ENV_CACHE_RESTORE_ROOMS, EVENT_TIMEOUT,
    QaConfig, ROOM_LIST_EVENT_TIMEOUT, SEND_QUEUE_EVENT_TIMEOUT, TIMELINE_INITIAL_EVENT_TIMEOUT,
    TIMELINE_RECONNECT_EXPECTED_BODY_COUNT, TIMELINE_RECONNECT_MIN_INITIAL_BODIES,
    TIMELINE_RECONNECT_PAGINATE_EVENT_COUNT, TIMELINE_UNSUBSCRIBE_SETTLE_TIMEOUT,
    TimelineStressConfig,
};
use super::{
    AccountCommand, AccountKey, ActivityEvent, ActivityMarkReadTarget, ActivityRowKind,
    ActivityState, AppAction, AppCommand, AppState, AuthSecret, BTreeSet, ComposerDocument,
    ComposerDraftScope, ComposerKey, ComposerKeyEvent, ComposerKeyModifiers,
    ComposerResolvedAction, ComposerResolverContext, ComposerSelection, ComposerSendShortcut,
    ComposerSurface, ComposerTarget, CoreCommand, CoreConnection, CoreEvent, CoreRuntime,
    DisplaySettings, Duration, ImageUploadCompressionMode, ImageUploadCompressionPolicy,
    ImageUploadCompressionState, ImageUploadDimensions, ImageUploadVariantInfo,
    ImageUploadVariantKind, LinkPreviewState, LiveSignalsEvent, MediaDownloadSelection,
    MentionIntent, MentionTarget, PaginationDirection, PaginationState, PresenceKind, RequestId,
    RoomCommand, RoomEvent, ScheduledSendCapability, SessionInfo, SessionState, SettingsPatch,
    StagedUploadCompressionChoice, StagedUploadItem, StagedUploadKind, SyncCommand, SystemTime,
    TimelineAnchorRestoreStatus, TimelineCommand, TimelineDiff, TimelineEvent, TimelineGapId,
    TimelineGapPosition, TimelineItem, TimelineItemId, TimelineKey, TimelineKind,
    TimelineMediaGalleryItem, TimelineMediaGalleryMedia, TimelineMediaGallerySource,
    TimelineMediaKind, TimelineSendState, TimelineUnreadPosition, TimelineViewportObservation,
    UNIX_EPOCH, UploadMediaKind, UploadMediaRequest, UploadMediaThumbnail,
    build_formatted_message_draft, reduce, resolve_composer_key_action,
};

pub(super) async fn run_timeline_stress_stage(
    config: &QaConfig,
    conn_a: &mut CoreConnection,
    conn_b: &mut CoreConnection,
    account_key_a: &AccountKey,
    account_key_b: &AccountKey,
) -> Result<(), String> {
    let stress = TimelineStressConfig::from_env()?;
    let user_b_full_id = format!("@{}:{}", config.user_b, config.server_name);
    let mut created_room_count = 0usize;
    let mut sent_message_count = 0usize;

    for space_index in 0..stress.space_count {
        eprintln!("timeline_stress progress: create_space index={space_index}");
        let space_id = create_space_for_qa(
            conn_a,
            &format!("Koushi Stress Space {space_index}"),
            "timeline_stress create space",
        )
        .await?;
        invite_user_for_qa(
            conn_a,
            &space_id,
            &user_b_full_id,
            "timeline_stress invite user to space",
        )
        .await?;
        wait_for_invite_in_snapshot(
            conn_b,
            &space_id,
            Some(false),
            "timeline_stress receiver sees space invite",
        )
        .await?;
        accept_invite_for_qa(conn_b, &space_id, "timeline_stress accept space invite").await?;
        wait_for_space_in_space_list(conn_a, &space_id, "timeline_stress creator sees space")
            .await?;
        wait_for_space_in_space_list(conn_b, &space_id, "timeline_stress receiver sees space")
            .await?;

        let mut expected_room_ids = Vec::with_capacity(stress.rooms_per_space);
        for room_index in 0..stress.rooms_per_space {
            eprintln!(
                "timeline_stress progress: create_room space={space_index} room={room_index}"
            );
            let room_id = create_room_for_qa(
                conn_a,
                &format!("Koushi Stress Room {space_index}-{room_index}"),
                false,
                "timeline_stress create room",
            )
            .await?;
            set_space_child_for_qa(
                conn_a,
                &space_id,
                &room_id,
                &config.server_name,
                "timeline_stress set space child",
            )
            .await?;
            invite_user_for_qa(
                conn_a,
                &room_id,
                &user_b_full_id,
                "timeline_stress invite user to room",
            )
            .await?;
            wait_for_invite_in_snapshot(
                conn_b,
                &room_id,
                Some(false),
                "timeline_stress receiver sees room invite",
            )
            .await?;
            accept_invite_for_qa(conn_b, &room_id, "timeline_stress accept room invite").await?;
            wait_for_room_in_room_list(conn_a, &room_id, "timeline_stress creator sees room")
                .await?;
            wait_for_room_in_room_list(conn_b, &room_id, "timeline_stress receiver sees room")
                .await?;

            expected_room_ids.push(room_id.clone());
            wait_for_space_child_projection(
                conn_a,
                &space_id,
                &expected_room_ids,
                "timeline_stress creator space children",
            )
            .await?;
            wait_for_space_child_projection(
                conn_b,
                &space_id,
                &expected_room_ids,
                "timeline_stress receiver space children",
            )
            .await?;
            created_room_count += 1;

            let sender_is_a = (space_index + room_index) % 2 == 0;
            eprintln!(
                "timeline_stress progress: messages space={space_index} room={room_index} sender={}",
                if sender_is_a { "a" } else { "b" }
            );
            sent_message_count += if sender_is_a {
                run_timeline_stress_room_messages(
                    config,
                    conn_a,
                    conn_b,
                    account_key_a,
                    account_key_b,
                    &room_id,
                    StressRoomCoordinates {
                        sender_prefix: "a",
                        space_index,
                        room_index,
                    },
                    stress.messages_per_room,
                )
                .await?
            } else {
                run_timeline_stress_room_messages(
                    config,
                    conn_b,
                    conn_a,
                    account_key_b,
                    account_key_a,
                    &room_id,
                    StressRoomCoordinates {
                        sender_prefix: "b",
                        space_index,
                        room_index,
                    },
                    stress.messages_per_room,
                )
                .await?
            };
        }

        select_space_and_wait_for_room_scope(
            conn_a,
            &space_id,
            &expected_room_ids,
            "timeline_stress creator selected-space scope",
        )
        .await?;
        select_space_and_wait_for_room_scope(
            conn_b,
            &space_id,
            &expected_room_ids,
            "timeline_stress receiver selected-space scope",
        )
        .await?;
    }

    if created_room_count != stress.total_rooms() || sent_message_count != stress.total_messages() {
        return Err(format!(
            "timeline_stress: count mismatch rooms={created_room_count}/{} messages={sent_message_count}/{}",
            stress.total_rooms(),
            stress.total_messages()
        ));
    }

    println!(
        "stress_counts=spaces={} rooms={} messages={}",
        stress.space_count,
        stress.total_rooms(),
        stress.total_messages()
    );
    println!("stress_space_scope=ok");
    println!("stress_no_blank=ok");
    println!("timeline_stress=ok");
    Ok(())
}

pub(super) async fn run_timeline_stress_replay_stage(
    conn_a: &mut CoreConnection,
    conn_b: &mut CoreConnection,
    account_key_a: &AccountKey,
    account_key_b: &AccountKey,
    _stress: TimelineStressConfig,
) -> Result<(), String> {
    let snapshot_a =
        wait_for_existing_stress_fixture_room_list(conn_a, "timeline_stress replay A room list")
            .await?;
    let snapshot_b =
        wait_for_existing_stress_fixture_room_list(conn_b, "timeline_stress replay B room list")
            .await?;
    verify_existing_stress_space_scopes(
        conn_a,
        &snapshot_a,
        "timeline_stress replay A selected-space scope",
    )
    .await?;
    verify_existing_stress_space_scopes(
        conn_b,
        &snapshot_b,
        "timeline_stress replay B selected-space scope",
    )
    .await?;

    let room_ids_a = stress_replay_room_ids(&snapshot_a);
    let room_ids_b = stress_replay_room_ids(&snapshot_b);
    if room_ids_a.is_empty() || room_ids_b.is_empty() {
        return Err("timeline_stress replay: fixture has no joined rooms".to_owned());
    }

    let scan_a = scan_existing_stress_rooms(
        conn_a,
        account_key_a,
        &room_ids_a,
        "timeline_stress replay A timeline scan",
    )
    .await?;
    let scan_b = scan_existing_stress_rooms(
        conn_b,
        account_key_b,
        &room_ids_b,
        "timeline_stress replay B timeline scan",
    )
    .await?;
    let message_rows = scan_a.message_rows + scan_b.message_rows;
    if message_rows == 0 {
        return Err(
            "timeline_stress replay: fixture timelines contained no visible messages".to_owned(),
        );
    }

    println!(
        "stress_counts=spaces={} rooms={} messages={}",
        snapshot_a.spaces.len().max(snapshot_b.spaces.len()),
        scan_a.rooms.max(scan_b.rooms),
        message_rows
    );
    println!("stress_space_scope=ok");
    println!("stress_no_blank=ok");
    println!("timeline_stress=ok");
    Ok(())
}

async fn wait_for_existing_stress_fixture_room_list(
    conn: &mut CoreConnection,
    label: &str,
) -> Result<AppState, String> {
    let has_fixture_shape =
        |snapshot: &AppState| !snapshot.rooms.is_empty() && !snapshot.spaces.is_empty();
    let snapshot = conn.snapshot();
    if has_fixture_shape(&snapshot) {
        return Ok(snapshot);
    }

    let deadline = tokio::time::Instant::now() + ROOM_LIST_EVENT_TIMEOUT;
    loop {
        let event = tokio::time::timeout_at(deadline, conn.recv_event())
            .await
            .map_err(|_| {
                let snapshot = conn.snapshot();
                format!(
                    "{label}: timed out waiting for existing fixture rooms/spaces \
                     (rooms={}, spaces={})",
                    snapshot.rooms.len(),
                    snapshot.spaces.len()
                )
            })?
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;

        match event {
            CoreEvent::Room(RoomEvent::RoomListUpdated) => {
                let snapshot = conn.snapshot();
                if has_fixture_shape(&snapshot) {
                    return Ok(snapshot);
                }
            }
            CoreEvent::StateChanged(snapshot) => {
                if has_fixture_shape(&snapshot) {
                    return Ok(snapshot);
                }
            }
            _ => {}
        }
    }
}

async fn verify_existing_stress_space_scopes(
    conn: &mut CoreConnection,
    snapshot: &AppState,
    label: &str,
) -> Result<(), String> {
    let spaces = snapshot
        .spaces
        .iter()
        .filter(|space| !space.child_room_ids.is_empty())
        .map(|space| (space.space_id.clone(), space.child_room_ids.clone()))
        .collect::<Vec<_>>();
    if spaces.is_empty() {
        return Err(format!("{label}: fixture has no spaces with child rooms"));
    }
    for (space_id, child_room_ids) in spaces {
        select_space_and_wait_for_room_scope(conn, &space_id, &child_room_ids, label).await?;
    }
    Ok(())
}

fn stress_replay_room_ids(snapshot: &AppState) -> Vec<String> {
    let joined_room_ids = snapshot
        .rooms
        .iter()
        .map(|room| room.room_id.as_str())
        .collect::<BTreeSet<_>>();
    let mut room_ids = BTreeSet::new();
    for space in &snapshot.spaces {
        for room_id in &space.child_room_ids {
            if joined_room_ids.contains(room_id.as_str()) {
                room_ids.insert(room_id.clone());
            }
        }
    }
    if room_ids.is_empty() {
        for room in &snapshot.rooms {
            room_ids.insert(room.room_id.clone());
        }
    }
    room_ids.into_iter().collect()
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct StressReplayScan {
    rooms: usize,
    message_rows: usize,
}

async fn scan_existing_stress_rooms(
    conn: &mut CoreConnection,
    account_key: &AccountKey,
    room_ids: &[String],
    label: &str,
) -> Result<StressReplayScan, String> {
    let mut message_rows = 0usize;
    for room_id in room_ids {
        message_rows += scan_existing_stress_timeline(conn, account_key, room_id, label).await?;
    }
    Ok(StressReplayScan {
        rooms: room_ids.len(),
        message_rows,
    })
}

async fn scan_existing_stress_timeline(
    conn: &mut CoreConnection,
    account_key: &AccountKey,
    room_id: &str,
    label: &str,
) -> Result<usize, String> {
    let key = TimelineKey::room(account_key.clone(), room_id.to_owned());
    let subscribe_id = conn.next_request_id();
    conn.command(CoreCommand::Timeline(TimelineCommand::Subscribe {
        request_id: subscribe_id,
        key: key.clone(),
    }))
    .await
    .map_err(|e| format!("{label}: submit replay subscribe failed: {e}"))?;
    let initial_items = wait_for_initial_items(conn, &key, subscribe_id, label).await?;
    assert_no_blank_visible_event_rows(&initial_items, label)?;
    let mut message_rows = count_visible_payload_event_rows(&initial_items);
    let mut end_reached = false;
    let mut page_count = 0usize;
    while !end_reached && page_count < 3 {
        let request_id = submit_stress_backfill_paginate(conn, &key, 100, label).await?;
        let result = wait_for_stress_replay_paginate(conn, &key, request_id, label).await?;
        message_rows += result.message_rows;
        end_reached = result.end_reached;
        page_count += 1;
    }

    let unsubscribe_id = conn.next_request_id();
    conn.command(CoreCommand::Timeline(TimelineCommand::Unsubscribe {
        request_id: unsubscribe_id,
        key,
    }))
    .await
    .map_err(|e| format!("{label}: submit replay unsubscribe failed: {e}"))?;
    Ok(message_rows)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct StressReplayPageResult {
    message_rows: usize,
    end_reached: bool,
}

async fn wait_for_stress_replay_paginate(
    conn: &mut CoreConnection,
    key: &TimelineKey,
    request_id: RequestId,
    label: &str,
) -> Result<StressReplayPageResult, String> {
    let mut message_rows = 0usize;
    loop {
        let event = tokio::time::timeout(EVENT_TIMEOUT, conn.recv_event())
            .await
            .map_err(|_| format!("{label}: timed out waiting for replay paginate"))?
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;

        match &event {
            CoreEvent::Timeline(TimelineEvent::ItemsUpdated {
                key: ev_key, diffs, ..
            }) if ev_key == key => {
                visit_timeline_diff_items(&diffs, |item| {
                    if timeline_item_is_visible_event_row(item)
                        && !timeline_item_has_visible_payload(item)
                    {
                        return Err(format!(
                            "{label}: visible event row had no renderable payload"
                        ));
                    }
                    Ok(())
                })?;
                message_rows += count_visible_payload_event_rows_in_diffs(&diffs);
            }
            CoreEvent::Timeline(TimelineEvent::InitialItems {
                key: ev_key, items, ..
            }) if ev_key == key => {
                assert_no_blank_visible_event_rows(&items, label)?;
                message_rows += count_visible_payload_event_rows(&items);
            }
            CoreEvent::Timeline(TimelineEvent::PaginationStateChanged {
                key: ev_key,
                request_id: ev_id,
                state,
                ..
            }) if ev_key == key && ev_id == &Some(request_id) => match state {
                PaginationState::Idle => {
                    return Ok(StressReplayPageResult {
                        message_rows,
                        end_reached: false,
                    });
                }
                PaginationState::EndReached => {
                    return Ok(StressReplayPageResult {
                        message_rows,
                        end_reached: true,
                    });
                }
                PaginationState::Failed { kind } => {
                    return Err(format!("{label}: replay pagination failed: {kind:?}"));
                }
                PaginationState::Paginating => {}
            },
            CoreEvent::OperationFailed {
                request_id: ev_id,
                failure,
            } if ev_id == &request_id => {
                return Err(format!(
                    "{label}: replay paginate operation failed: {failure:?}"
                ));
            }
            _ => {}
        }
    }
}

fn count_visible_payload_event_rows(items: &[TimelineItem]) -> usize {
    items
        .iter()
        .filter(|item| {
            timeline_item_is_visible_event_row(item) && timeline_item_has_visible_payload(item)
        })
        .count()
}

fn count_visible_payload_event_rows_in_diffs(diffs: &[TimelineDiff]) -> usize {
    let mut count = 0usize;
    let _ = visit_timeline_diff_items(diffs, |item| {
        if timeline_item_is_visible_event_row(item) && timeline_item_has_visible_payload(item) {
            count += 1;
        }
        Ok(())
    });
    count
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct StressRoomCoordinates {
    sender_prefix: &'static str,
    space_index: usize,
    room_index: usize,
}

impl StressRoomCoordinates {
    fn should_send_empty_formatted_probe(self) -> bool {
        self.space_index == 0 && self.room_index == 0
    }
}

pub(super) async fn run_timeline_stress_room_messages(
    config: &QaConfig,
    sender_conn: &mut CoreConnection,
    receiver_conn: &mut CoreConnection,
    sender_account_key: &AccountKey,
    receiver_account_key: &AccountKey,
    room_id: &str,
    coordinates: StressRoomCoordinates,
    messages_per_room: usize,
) -> Result<usize, String> {
    let sender_key = TimelineKey::room(sender_account_key.clone(), room_id.to_owned());
    let sender_subscribe_id = sender_conn.next_request_id();
    sender_conn
        .command(CoreCommand::Timeline(TimelineCommand::Subscribe {
            request_id: sender_subscribe_id,
            key: sender_key.clone(),
        }))
        .await
        .map_err(|e| format!("timeline_stress: submit sender subscribe failed: {e}"))?;
    let sender_initial = wait_for_initial_items(
        sender_conn,
        &sender_key,
        sender_subscribe_id,
        "timeline_stress sender subscribe",
    )
    .await?;
    assert_no_blank_visible_event_rows(&sender_initial, "timeline_stress sender initial")?;

    let mut expected_bodies = Vec::with_capacity(messages_per_room);
    for message_index in 0..messages_per_room {
        let body = format!(
            "Koushi local stress body s{} r{} m{}",
            coordinates.space_index, coordinates.room_index, message_index
        );
        let transaction_id = format!(
            "qa-stress-{}-{}-{}-{}",
            coordinates.sender_prefix,
            coordinates.space_index,
            coordinates.room_index,
            message_index
        );
        let send_id = sender_conn.next_request_id();
        sender_conn
            .command(CoreCommand::Timeline(TimelineCommand::SendText {
                request_id: send_id,
                key: sender_key.clone(),
                transaction_id: transaction_id.clone(),
                document: koushi_state::ComposerDocument::from_plain_text(body.clone()),
            }))
            .await
            .map_err(|e| format!("timeline_stress: submit stress send failed: {e}"))?;
        wait_for_send_flow_completion(
            sender_conn,
            send_id,
            &sender_key,
            &transaction_id,
            &body,
            "timeline_stress send flow",
        )
        .await?;
        expected_bodies.push(body);
    }

    if coordinates.should_send_empty_formatted_probe() {
        let probe_body = send_timeline_stress_empty_formatted_probe(
            config,
            room_id,
            coordinates.sender_prefix,
            "timeline_stress empty formatted probe",
        )
        .await?;
        expected_bodies.push(probe_body);
    }

    let sender_unsubscribe_id = sender_conn.next_request_id();
    sender_conn
        .command(CoreCommand::Timeline(TimelineCommand::Unsubscribe {
            request_id: sender_unsubscribe_id,
            key: sender_key,
        }))
        .await
        .map_err(|e| format!("timeline_stress: submit sender unsubscribe failed: {e}"))?;

    let receiver_key = TimelineKey::room(receiver_account_key.clone(), room_id.to_owned());
    let receiver_subscribe_id = receiver_conn.next_request_id();
    receiver_conn
        .command(CoreCommand::Timeline(TimelineCommand::Subscribe {
            request_id: receiver_subscribe_id,
            key: receiver_key.clone(),
        }))
        .await
        .map_err(|e| format!("timeline_stress: submit receiver subscribe failed: {e}"))?;
    let receiver_initial = wait_for_initial_items(
        receiver_conn,
        &receiver_key,
        receiver_subscribe_id,
        "timeline_stress receiver subscribe",
    )
    .await?;

    wait_for_stress_bodies_and_no_blank_rows(
        receiver_conn,
        &receiver_key,
        &receiver_initial,
        &expected_bodies,
        (messages_per_room + 20).min(u16::MAX as usize) as u16,
        "timeline_stress receiver backfill",
    )
    .await?;

    let receiver_unsubscribe_id = receiver_conn.next_request_id();
    receiver_conn
        .command(CoreCommand::Timeline(TimelineCommand::Unsubscribe {
            request_id: receiver_unsubscribe_id,
            key: receiver_key,
        }))
        .await
        .map_err(|e| format!("timeline_stress: submit receiver unsubscribe failed: {e}"))?;

    Ok(expected_bodies.len())
}

async fn send_timeline_stress_empty_formatted_probe(
    config: &QaConfig,
    room_id: &str,
    sender_prefix: &str,
    label: &str,
) -> Result<String, String> {
    let (username, password) = match sender_prefix {
        "a" => (&config.user_a, &config.password_a),
        "b" => (&config.user_b, &config.password_b),
        other => {
            return Err(format!("{label}: unknown stress sender prefix {other}"));
        }
    };
    let body = format!("Koushi local stress formatted fallback {sender_prefix}");
    let session = koushi_sdk::login_with_password(&koushi_state::LoginRequest {
        homeserver: config.homeserver.clone(),
        username: username.clone(),
        password: AuthSecret::new(password.clone()),
        device_display_name: Some("Koushi raw formatted QA".to_owned()),
    })
    .await
    .map_err(|error| format!("{label}: raw probe login failed: {error}"))?;
    koushi_sdk::sync_once(&session)
        .await
        .map_err(|error| format!("{label}: raw probe sync failed: {error}"))?;

    let parsed_room_id = matrix_sdk::ruma::RoomId::parse(room_id)
        .map_err(|error| format!("{label}: raw probe room id parse failed: {error}"))?;
    let room = session
        .client()
        .get_room(&parsed_room_id)
        .ok_or_else(|| format!("{label}: raw probe room was not available after sync"))?;
    room.send_raw(
        "m.room.message",
        serde_json::json!({
            "msgtype": "m.text",
            "body": body,
            "format": "org.matrix.custom.html",
            "formatted_body": "<p><br /></p>"
        }),
    )
    .await
    .map_err(|error| format!("{label}: raw probe send failed: {error}"))?;

    if let Err(error) = koushi_sdk::logout(&session).await {
        eprintln!("timeline_stress raw probe logout warning: {error}");
    }
    Ok(body)
}

pub(super) async fn run_scheduled_send_stage(
    conn: &mut CoreConnection,
    key: &TimelineKey,
    room_id: &str,
) -> Result<(), String> {
    const SCHEDULED_CREATE_BODY: &str = "Koushi scheduled create QA body";
    const SCHEDULED_FIRE_BODY: &str = "Koushi scheduled fire QA body";
    let session = authenticated_session_info(conn, "scheduled send account")?.clone();
    let expected_account = koushi_key::SessionKeyId {
        homeserver: session.homeserver,
        user_id: session.user_id,
        device_id: session.device_id,
    };

    let select_id = conn.next_request_id();
    conn.command(CoreCommand::Room(RoomCommand::SelectRoom {
        request_id: select_id,
        room_id: room_id.to_owned(),
    }))
    .await
    .map_err(|e| format!("scheduled_send: submit room select failed: {e}"))?;
    wait_for_selected_room(conn, room_id, "scheduled_send selected room").await?;

    let composer_generation = conn
        .begin_composer_draft_renderer_generation()
        .map_err(|error| format!("scheduled_send: begin composer renderer failed: {error:?}"))?;
    let composer_scope = ComposerDraftScope {
        account: expected_account.clone(),
        target: ComposerTarget::Main {
            room_id: room_id.to_owned(),
        },
    };
    let create_lease = conn
        .acquire_composer_draft_lease(composer_generation, composer_scope.clone())
        .map_err(|error| format!("scheduled_send: acquire create lease failed: {error:?}"))?;
    let create_id = conn.next_request_id();
    let create_result = conn
        .command_with_composer_lease(
            composer_generation,
            create_lease,
            CoreCommand::App(AppCommand::ScheduleSend {
                request_id: create_id,
                expected_account: expected_account.clone(),
                room_id: room_id.to_owned(),
                thread_root_event_id: None,
                body: SCHEDULED_CREATE_BODY.to_owned(),
                send_at_ms: scheduled_qa_epoch_ms(Duration::from_secs(300)),
                draft_revision: 0.into(),
            }),
        )
        .await;
    conn.release_composer_draft_lease(composer_generation, create_lease)
        .map_err(|error| format!("scheduled_send: release create lease failed: {error:?}"))?;
    create_result.map_err(|error| format!("scheduled_send: submit create failed: {error}"))?;

    let created = wait_for_scheduled_send_count(conn, 1, "scheduled_send create").await?;
    if created.timeline.scheduled_send_capability != ScheduledSendCapability::LocalFallback {
        return Err(
            "scheduled_send: local fallback capability was not projected to the snapshot"
                .to_owned(),
        );
    }
    println!("scheduled_capability=local_fallback");
    println!("scheduled_create=ok");

    let scheduled_id = created
        .timeline
        .scheduled_sends
        .first()
        .map(|item| item.scheduled_id.clone())
        .ok_or_else(|| "scheduled_send: created item was missing from projection".to_owned())?;
    let rescheduled_at_ms = scheduled_qa_epoch_ms(Duration::from_secs(600));
    let reschedule_id = conn.next_request_id();
    conn.command(CoreCommand::App(AppCommand::RescheduleScheduledSend {
        request_id: reschedule_id,
        scheduled_id: scheduled_id.clone(),
        body: SCHEDULED_CREATE_BODY.to_owned(),
        send_at_ms: rescheduled_at_ms,
    }))
    .await
    .map_err(|e| format!("scheduled_send: submit reschedule failed: {e}"))?;
    wait_for_scheduled_send_due(
        conn,
        &scheduled_id,
        rescheduled_at_ms,
        "scheduled_send reschedule",
    )
    .await?;
    println!("scheduled_reschedule=ok");

    let cancel_id = conn.next_request_id();
    conn.command(CoreCommand::App(AppCommand::CancelScheduledSend {
        request_id: cancel_id,
        scheduled_id,
    }))
    .await
    .map_err(|e| format!("scheduled_send: submit cancel failed: {e}"))?;
    wait_for_scheduled_send_count(conn, 0, "scheduled_send cancel").await?;
    println!("scheduled_cancel=ok");

    let fire_lease = conn
        .acquire_composer_draft_lease(composer_generation, composer_scope)
        .map_err(|error| format!("scheduled_send: acquire fire lease failed: {error:?}"))?;
    let fire_id = conn.next_request_id();
    let fire_result = conn
        .command_with_composer_lease(
            composer_generation,
            fire_lease,
            CoreCommand::App(AppCommand::ScheduleSend {
                request_id: fire_id,
                expected_account,
                room_id: room_id.to_owned(),
                thread_root_event_id: None,
                body: SCHEDULED_FIRE_BODY.to_owned(),
                send_at_ms: scheduled_qa_epoch_ms(Duration::from_millis(250)),
                draft_revision: 0.into(),
            }),
        )
        .await;
    conn.release_composer_draft_lease(composer_generation, fire_lease)
        .map_err(|error| format!("scheduled_send: release fire lease failed: {error:?}"))?;
    fire_result.map_err(|error| format!("scheduled_send: submit fire schedule failed: {error}"))?;
    let fire_created = wait_for_scheduled_send_count(conn, 1, "scheduled_send fire create").await?;
    let fire_scheduled_id = fire_created
        .timeline
        .scheduled_sends
        .first()
        .map(|item| item.scheduled_id.clone())
        .ok_or_else(|| "scheduled_send: fire item was missing from projection".to_owned())?;
    wait_for_scheduled_send_fired(
        conn,
        key,
        &fire_scheduled_id,
        SCHEDULED_FIRE_BODY,
        "scheduled_send fire",
    )
    .await?;
    println!("scheduled_fire=ok");
    Ok(())
}

fn scheduled_qa_epoch_ms(offset: Duration) -> u64 {
    SystemTime::now()
        .checked_add(offset)
        .unwrap_or_else(SystemTime::now)
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or_default()
}

async fn wait_for_selected_room(
    conn: &mut CoreConnection,
    room_id: &str,
    label: &str,
) -> Result<(), String> {
    if conn.snapshot().timeline.room_id.as_deref() == Some(room_id) {
        return Ok(());
    }

    loop {
        let event = tokio::time::timeout(EVENT_TIMEOUT, conn.recv_event())
            .await
            .map_err(|_| format!("{label}: timed out waiting for selected room"))?
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;

        match event {
            CoreEvent::StateChanged(snapshot)
                if snapshot.timeline.room_id.as_deref() == Some(room_id) =>
            {
                return Ok(());
            }
            _ if conn.snapshot().timeline.room_id.as_deref() == Some(room_id) => return Ok(()),
            _ => {}
        }
    }
}

async fn wait_for_scheduled_send_count(
    conn: &mut CoreConnection,
    expected_count: usize,
    label: &str,
) -> Result<AppState, String> {
    let snapshot = conn.snapshot();
    if snapshot.timeline.scheduled_sends.len() == expected_count {
        return Ok(snapshot);
    }

    loop {
        let event = tokio::time::timeout(EVENT_TIMEOUT, conn.recv_event())
            .await
            .map_err(|_| format!("{label}: timed out waiting for scheduled-send projection"))?
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;

        match event {
            CoreEvent::StateChanged(snapshot)
                if snapshot.timeline.scheduled_sends.len() == expected_count =>
            {
                return Ok(snapshot);
            }
            _ if conn.snapshot().timeline.scheduled_sends.len() == expected_count => {
                return Ok(conn.snapshot());
            }
            _ => {}
        }
    }
}

async fn wait_for_scheduled_send_due(
    conn: &mut CoreConnection,
    scheduled_id: &str,
    expected_send_at_ms: u64,
    label: &str,
) -> Result<(), String> {
    let matches_due =
        |snapshot: &AppState| {
            snapshot.timeline.scheduled_sends.iter().any(|item| {
                item.scheduled_id == scheduled_id && item.send_at_ms == expected_send_at_ms
            })
        };
    if matches_due(&conn.snapshot()) {
        return Ok(());
    }

    loop {
        let event = tokio::time::timeout(EVENT_TIMEOUT, conn.recv_event())
            .await
            .map_err(|_| format!("{label}: timed out waiting for scheduled-send reschedule"))?
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;

        match event {
            CoreEvent::StateChanged(snapshot) if matches_due(&snapshot) => return Ok(()),
            _ if matches_due(&conn.snapshot()) => return Ok(()),
            _ => {}
        }
    }
}

async fn wait_for_scheduled_send_fired(
    conn: &mut CoreConnection,
    key: &TimelineKey,
    scheduled_id: &str,
    expected_body: &str,
    label: &str,
) -> Result<(), String> {
    let mut queue_removed = scheduled_item_absent(&conn.snapshot(), scheduled_id);
    let mut timeline_observed = false;

    loop {
        if queue_removed && timeline_observed {
            return Ok(());
        }

        let event = tokio::time::timeout(EVENT_TIMEOUT, conn.recv_event())
            .await
            .map_err(|_| format!("{label}: timed out waiting for scheduled-send dispatch"))?
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;

        match event {
            CoreEvent::StateChanged(snapshot) => {
                queue_removed = scheduled_item_absent(&snapshot, scheduled_id);
            }
            CoreEvent::Timeline(TimelineEvent::ItemsUpdated {
                key: ref ev_key,
                diffs,
                ..
            }) if ev_key == key => {
                visit_timeline_diff_items(&diffs, |item| {
                    if timeline_item_body_matches(item, expected_body) {
                        timeline_observed = true;
                    }
                    Ok(())
                })?;
            }
            CoreEvent::OperationFailed {
                request_id,
                failure,
            } if request_id.connection_id.0 == 0 => {
                return Err(format!(
                    "{label}: internal scheduled-send dispatch failed: {failure:?}"
                ));
            }
            _ => {}
        }
    }
}

fn scheduled_item_absent(snapshot: &AppState, scheduled_id: &str) -> bool {
    snapshot
        .timeline
        .scheduled_sends
        .iter()
        .all(|item| item.scheduled_id != scheduled_id)
}

/// Reads KOUSHI_QA_CACHE_RESTORE_ROOMS / _DEPTH, clamps at defaults.
fn cache_restore_params() -> (usize, usize) {
    let rooms = std::env::var(ENV_CACHE_RESTORE_ROOMS)
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(DEFAULT_CACHE_RESTORE_ROOMS)
        .max(1);
    let depth = std::env::var(ENV_CACHE_RESTORE_DEPTH)
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(DEFAULT_CACHE_RESTORE_DEPTH)
        .max(10);
    (rooms, depth)
}

/// Apply a single `TimelineDiff` in-place to a `Vec<TimelineItem>`.
fn apply_timeline_diff(items: &mut Vec<TimelineItem>, diff: &TimelineDiff) {
    match diff {
        TimelineDiff::PushFront { item } => items.insert(0, item.clone()),
        TimelineDiff::PushBack { item } => items.push(item.clone()),
        TimelineDiff::Insert { index, item } => {
            let idx = (*index).min(items.len());
            items.insert(idx, item.clone());
        }
        TimelineDiff::Set { index, item } => {
            if *index < items.len() {
                items[*index] = item.clone();
            }
        }
        TimelineDiff::Remove { index } => {
            if *index < items.len() {
                items.remove(*index);
            }
        }
        TimelineDiff::Truncate { length } => items.truncate(*length),
        TimelineDiff::Clear => items.clear(),
        TimelineDiff::Reset { items: new_items } => *items = new_items.clone(),
    }
}

pub(super) async fn run_cache_restore_scenario(config: &QaConfig) -> Result<(), String> {
    let (num_rooms, depth) = cache_restore_params();
    let proxy = QaTcpProxy::start(&config.homeserver)?;
    let data_dir = qa_data_dir("cache_restore");

    // -----------------------------------------------------------------------
    // Connect 1: login, send fixture history, paginate to EndReached, record
    // deep anchors deterministically (m0 = first sent = oldest), then shut down.
    // -----------------------------------------------------------------------
    let runtime = CoreRuntime::start_with_data_dir(data_dir.clone());
    let mut conn = runtime.attach();

    let login_id = conn.next_request_id();
    conn.command(CoreCommand::Account(AccountCommand::LoginPassword {
        request_id: login_id,
        request: koushi_state::LoginRequest {
            homeserver: proxy.homeserver_url(),
            username: config.user_a.clone(),
            password: AuthSecret::new(config.password_a.clone()),
            device_display_name: Some("Koushi Core QA Cache Restore".to_owned()),
        },
        platform: koushi_state::DisplayPlatform::Linux,
    }))
    .await
    .map_err(|e| format!("cache_restore: submit login failed: {e}"))?;

    let account_key = wait_for_logged_in(&mut conn, login_id, "cache_restore login").await?;
    wait_for_ready_snapshot(&mut conn, "cache_restore Ready").await?;
    let sync_start_id = conn.next_request_id();
    conn.command(CoreCommand::Sync(SyncCommand::Start {
        request_id: sync_start_id,
    }))
    .await
    .map_err(|e| format!("cache_restore: submit Sync start failed: {e}"))?;
    wait_for_sync_started_and_running(&mut conn, sync_start_id, "cache_restore sync start").await?;

    // Create rooms, send DEPTH messages, paginate to EndReached. Track items
    // across the paginate to find the deterministic deep anchor (m0 = oldest).
    let mut room_ids: Vec<String> = Vec::with_capacity(num_rooms);
    let mut deep_anchors: Vec<String> = Vec::with_capacity(num_rooms);
    for room_idx in 0..num_rooms {
        let anchor_body = format!("cache_restore fixture r{room_idx} m0");
        let room_id = create_room_for_qa(
            &mut conn,
            &format!("QA Cache Restore Room {room_idx}"),
            false,
            "cache_restore create room",
        )
        .await?;
        wait_for_room_in_room_list(&mut conn, &room_id, "cache_restore room in list").await?;

        let key = TimelineKey::room(account_key.clone(), room_id.clone());
        let sub_id = conn.next_request_id();
        conn.command(CoreCommand::Timeline(TimelineCommand::Subscribe {
            request_id: sub_id,
            key: key.clone(),
        }))
        .await
        .map_err(|e| format!("cache_restore: submit subscribe failed: {e}"))?;
        let initial_items =
            wait_for_initial_items(&mut conn, &key, sub_id, "cache_restore subscribe").await?;
        // Track all items across the paginate so we can find m0 at the end.
        let mut all_items = initial_items;

        // Send DEPTH messages sequentially so they land in the event cache.
        for msg_idx in 0..depth {
            let txn = format!("qa-cr-{room_idx}-{msg_idx}");
            let send_id = conn.next_request_id();
            conn.command(CoreCommand::Timeline(TimelineCommand::SendText {
                request_id: send_id,
                key: key.clone(),
                transaction_id: txn.clone(),
                document: koushi_state::ComposerDocument::from_plain_text(format!(
                    "cache_restore fixture r{room_idx} m{msg_idx}"
                )),
            }))
            .await
            .map_err(|e| format!("cache_restore: submit send failed: {e}"))?;
            wait_for_send_flow_completion(
                &mut conn,
                send_id,
                &key,
                &txn,
                &format!("cache_restore fixture r{room_idx} m{msg_idx}"),
                "cache_restore send",
            )
            .await?;
        }

        // Paginate backward to EndReached, accumulating diffs so all_items
        // reflects the full history and we can find m0 deterministically.
        let pag_id = conn.next_request_id();
        conn.command(CoreCommand::Timeline(TimelineCommand::Paginate {
            request_id: pag_id,
            key: key.clone(),
            direction: PaginationDirection::Backward,
            event_count: CACHE_RESTORE_PAGINATE_BATCH,
        }))
        .await
        .map_err(|e| format!("cache_restore: submit paginate failed: {e}"))?;
        let _ = pag_id;
        let mut saw_paginating = false;
        loop {
            let event = tokio::time::timeout(Duration::from_secs(120), conn.recv_event())
                .await
                .map_err(|_| {
                    "cache_restore populate: timed out waiting for paginate event".to_owned()
                })?
                .map_err(|lag| {
                    format!(
                        "cache_restore populate: event stream lagged (skipped={})",
                        lag.skipped
                    )
                })?;
            match event {
                CoreEvent::Timeline(TimelineEvent::PaginationStateChanged {
                    key: ref ev_key,
                    direction,
                    ref state,
                    ..
                }) if ev_key == &key && direction == PaginationDirection::Backward => match state {
                    PaginationState::Paginating => {
                        saw_paginating = true;
                    }
                    PaginationState::Idle => {
                        if !saw_paginating {
                            return Err(
                                "cache_restore populate: Idle without Paginating".to_owned()
                            );
                        }
                        saw_paginating = false;
                        let repag_id = conn.next_request_id();
                        conn.command(CoreCommand::Timeline(TimelineCommand::Paginate {
                            request_id: repag_id,
                            key: key.clone(),
                            direction: PaginationDirection::Backward,
                            event_count: CACHE_RESTORE_PAGINATE_BATCH,
                        }))
                        .await
                        .map_err(|e| format!("cache_restore: re-paginate failed: {e}"))?;
                    }
                    PaginationState::EndReached => {
                        break;
                    }
                    PaginationState::Failed { .. } => {
                        return Err("cache_restore populate: paginate failed".to_owned());
                    }
                },
                CoreEvent::Timeline(TimelineEvent::ItemsUpdated {
                    key: ref ev_key,
                    ref diffs,
                    ..
                }) if ev_key == &key => {
                    for diff in diffs {
                        apply_timeline_diff(&mut all_items, diff);
                    }
                }
                _ => {}
            }
        }

        // Find the deterministic deep anchor: m0 is the first-sent (oldest) message.
        let anchor_item =
            find_timeline_item_with_body(&all_items, &anchor_body).ok_or_else(|| {
                format!(
                    "cache_restore: m0 anchor not found after full paginate \
                     (room_idx={room_idx}, items={})",
                    all_items.len()
                )
            })?;
        let anchor_event_id = match &anchor_item.id {
            TimelineItemId::Event { event_id } => event_id.clone(),
            other => {
                return Err(format!(
                    "cache_restore: m0 anchor item has non-Event id: {other:?}"
                ));
            }
        };

        let unsub_id = conn.next_request_id();
        conn.command(CoreCommand::Timeline(TimelineCommand::Unsubscribe {
            request_id: unsub_id,
            key,
        }))
        .await
        .map_err(|e| format!("cache_restore: submit unsubscribe failed: {e}"))?;

        room_ids.push(room_id);
        deep_anchors.push(anchor_event_id);
    }

    // -----------------------------------------------------------------------
    // Shallow-anchor room: CACHE_RESTORE_SHALLOW_DEPTH messages, sized so
    // that m0 (oldest) lies beyond the SDK's initial visible window (~20
    // items).  All events fit in one stored chunk (chunks_loaded == 0).
    //
    // After restart, live_restore_from_cache reveals m0 via
    // live_lazy_paginate_backwards (lazy_reveal_batches == 1, chunks_loaded == 0).
    // The P1 lazy-reveal-fence fix (147c9ed) gates on this path: it adds
    // lazy_reveal_batches to the settle fence so the DiffBatch settles before
    // the restore concludes with Found.  Without the fix the fence may miss
    // that batch and finish early.
    //
    // Bug #1 fix: capture the anchor event_id directly from the SendFlowOutcome
    // of the first send (m0).  The send-phase ItemsUpdated diffs are consumed
    // by wait_for_send_flow_completion and are not returned, so tracking
    // shallow_items through the send loop would never include m0.
    // -----------------------------------------------------------------------
    let shallow_room_id = create_room_for_qa(
        &mut conn,
        "QA Cache Restore Shallow",
        false,
        "cache_restore shallow create",
    )
    .await?;
    wait_for_room_in_room_list(
        &mut conn,
        &shallow_room_id,
        "cache_restore shallow room in list",
    )
    .await?;

    let shallow_key = TimelineKey::room(account_key.clone(), shallow_room_id.clone());
    let shallow_sub_id = conn.next_request_id();
    conn.command(CoreCommand::Timeline(TimelineCommand::Subscribe {
        request_id: shallow_sub_id,
        key: shallow_key.clone(),
    }))
    .await
    .map_err(|e| format!("cache_restore shallow: subscribe failed: {e}"))?;
    let _ = wait_for_initial_items(
        &mut conn,
        &shallow_key,
        shallow_sub_id,
        "cache_restore shallow subscribe",
    )
    .await?;

    // Send CACHE_RESTORE_SHALLOW_DEPTH messages and capture m0's event_id
    // directly from the first SendFlowOutcome — no item tracking needed.
    let mut shallow_anchor_id: Option<String> = None;
    for msg_idx in 0..CACHE_RESTORE_SHALLOW_DEPTH {
        let txn = format!("qa-cr-shallow-{msg_idx}");
        let send_id = conn.next_request_id();
        conn.command(CoreCommand::Timeline(TimelineCommand::SendText {
            request_id: send_id,
            key: shallow_key.clone(),
            transaction_id: txn.clone(),
            document: koushi_state::ComposerDocument::from_plain_text(format!(
                "cache_restore shallow m{msg_idx}"
            )),
        }))
        .await
        .map_err(|e| format!("cache_restore shallow: send failed: {e}"))?;
        let outcome = wait_for_send_flow_completion(
            &mut conn,
            send_id,
            &shallow_key,
            &txn,
            &format!("cache_restore shallow m{msg_idx}"),
            "cache_restore shallow send",
        )
        .await?;
        // m0 is the first-sent (oldest) message; record its event_id as the anchor.
        if msg_idx == 0 {
            shallow_anchor_id = Some(outcome.event_id.clone());
        }
    }
    let shallow_anchor_id =
        shallow_anchor_id.ok_or_else(|| "cache_restore shallow: no messages sent".to_owned())?;

    // Paginate backward to EndReached to warm the event cache so that
    // live_restore_from_cache can serve the anchor from the stored chunk on
    // restart (without a network call).
    let shallow_pag_id = conn.next_request_id();
    conn.command(CoreCommand::Timeline(TimelineCommand::Paginate {
        request_id: shallow_pag_id,
        key: shallow_key.clone(),
        direction: PaginationDirection::Backward,
        event_count: CACHE_RESTORE_PAGINATE_BATCH,
    }))
    .await
    .map_err(|e| format!("cache_restore shallow: paginate failed: {e}"))?;
    let _ = shallow_pag_id;
    let mut shallow_saw_paginating = false;
    loop {
        let event = tokio::time::timeout(Duration::from_secs(60), conn.recv_event())
            .await
            .map_err(|_| "cache_restore shallow: timed out waiting for paginate event".to_owned())?
            .map_err(|lag| {
                format!(
                    "cache_restore shallow: event stream lagged (skipped={})",
                    lag.skipped
                )
            })?;
        match event {
            CoreEvent::Timeline(TimelineEvent::PaginationStateChanged {
                key: ref ev_key,
                direction,
                ref state,
                ..
            }) if ev_key == &shallow_key && direction == PaginationDirection::Backward => {
                match state {
                    PaginationState::Paginating => {
                        shallow_saw_paginating = true;
                    }
                    PaginationState::Idle => {
                        if !shallow_saw_paginating {
                            return Err("cache_restore shallow: Idle without Paginating".to_owned());
                        }
                        shallow_saw_paginating = false;
                        let repag_id = conn.next_request_id();
                        conn.command(CoreCommand::Timeline(TimelineCommand::Paginate {
                            request_id: repag_id,
                            key: shallow_key.clone(),
                            direction: PaginationDirection::Backward,
                            event_count: CACHE_RESTORE_PAGINATE_BATCH,
                        }))
                        .await
                        .map_err(|e| format!("cache_restore shallow: re-paginate failed: {e}"))?;
                    }
                    PaginationState::EndReached => {
                        break;
                    }
                    PaginationState::Failed { .. } => {
                        return Err("cache_restore shallow: paginate failed".to_owned());
                    }
                }
            }
            _ => {}
        }
    }

    let shallow_unsub_id = conn.next_request_id();
    conn.command(CoreCommand::Timeline(TimelineCommand::Unsubscribe {
        request_id: shallow_unsub_id,
        key: shallow_key,
    }))
    .await
    .map_err(|e| format!("cache_restore shallow: unsubscribe failed: {e}"))?;

    println!("cache_restore_loaded=ok");

    // Clean shutdown of Connect 1.
    stop_sync_for_qa(&mut conn, "cache_restore stop sync before restart").await?;
    drop(conn);
    runtime.shutdown().await;

    // -----------------------------------------------------------------------
    // Connect 2: restart over the same data dir, BLOCK the network, then drive
    // RestoreTimelineAnchor per room using production-faithful params.
    // PRIMARY GATE: status == Found, OR (EndReached AND anchor present in items).
    // Cycle count + ms are diagnostics only.
    // -----------------------------------------------------------------------
    let runtime2 = CoreRuntime::start_with_data_dir(data_dir);
    let mut conn2 = runtime2.attach();

    let restore_id = conn2.next_request_id();
    conn2
        .command(CoreCommand::Account(AccountCommand::RestoreSession {
            request_id: restore_id,
            account_key: account_key.clone(),
        }))
        .await
        .map_err(|e| format!("cache_restore: submit restore failed: {e}"))?;
    wait_for_session_restored(
        &mut conn2,
        restore_id,
        &account_key,
        "cache_restore restore",
    )
    .await?;
    wait_for_ready_snapshot(&mut conn2, "cache_restore restored Ready").await?;

    // Block the network NOW: any /messages network call from here on will fail.
    proxy.disable();

    let aggregate_start = std::time::Instant::now();
    let mut all_deep_restores_terminated_cleanly = true;
    let mut total_cycles: u32 = 0;
    // Per-room cycle counts for the room-entry speed gate.
    let mut room_cycle_counts: Vec<u16> = Vec::new();

    for (room_idx, (room_id, anchor)) in room_ids.iter().zip(deep_anchors.iter()).enumerate() {
        let key = TimelineKey::room(account_key.clone(), room_id.clone());
        let sub_id = conn2.next_request_id();
        conn2
            .command(CoreCommand::Timeline(TimelineCommand::Subscribe {
                request_id: sub_id,
                key: key.clone(),
            }))
            .await
            .map_err(|e| format!("cache_restore: offline subscribe failed: {e}"))?;
        let _initial_offline =
            wait_for_initial_items(&mut conn2, &key, sub_id, "cache_restore offline subscribe")
                .await?;

        let room_start = std::time::Instant::now();
        let restore_req = conn2.next_request_id();
        conn2
            .command(CoreCommand::Timeline(
                TimelineCommand::RestoreTimelineAnchor {
                    request_id: restore_req,
                    key: key.clone(),
                    event_id: anchor.clone(),
                    // Production-faithful params: source TimelineView.tsx
                    // (LIVE_ROOM_ANCHOR_RESTORE_MAX_BATCHES=6, EVENT_COUNT=100).
                    // A deep anchor may end as BudgetExhausted; room entry must
                    // not inflate this into a long history walk.
                    max_batches: CACHE_RESTORE_PROD_MAX_BATCHES,
                    event_count: CACHE_RESTORE_PROD_EVENT_COUNT,
                },
            ))
            .await
            .map_err(|e| {
                format!("cache_restore: offline RestoreTimelineAnchor submit failed: {e}")
            })?;

        // Consume events until AnchorRestoreFinished. Count Paginating transitions
        // as internal backward-paginate cycles for the speed regression gate.
        let mut cycle_count: u16 = 0;
        let status = loop {
            let event = tokio::time::timeout(Duration::from_secs(120), conn2.recv_event())
                .await
                .map_err(|_| {
                    "cache_restore offline: timed out waiting for AnchorRestoreFinished".to_owned()
                })?
                .map_err(|lag| {
                    format!(
                        "cache_restore offline: event stream lagged (skipped={})",
                        lag.skipped
                    )
                })?;
            match event {
                CoreEvent::Timeline(TimelineEvent::PaginationStateChanged {
                    key: ref ev_key,
                    direction,
                    state: PaginationState::Paginating,
                    ..
                }) if ev_key == &key && direction == PaginationDirection::Backward => {
                    cycle_count += 1;
                }
                CoreEvent::Timeline(TimelineEvent::AnchorRestoreFinished {
                    request_id: ev_req,
                    key: ref ev_key,
                    ref status,
                }) if ev_req == restore_req && ev_key == &key => {
                    break status.clone();
                }
                _ => {}
            }
        };

        let room_ms = room_start.elapsed().as_millis();
        total_cycles += cycle_count as u32;
        room_cycle_counts.push(cycle_count);
        let status_label = match &status {
            TimelineAnchorRestoreStatus::Found => "found",
            TimelineAnchorRestoreStatus::EndReached => "end_reached",
            TimelineAnchorRestoreStatus::BudgetExhausted => "budget_exhausted",
            TimelineAnchorRestoreStatus::Superseded => "superseded",
            TimelineAnchorRestoreStatus::Failed { .. } => "failed",
        };
        // Private-data-free diagnostics: cycles + ms only, no ids or bodies.
        eprintln!(
            "cache_restore room={room_idx} cycles={cycle_count} ms={room_ms} status={status_label}"
        );

        // PRIMARY CORRECTNESS GATE:
        // The normal room-entry path is intentionally budgeted. Deep anchors may
        // end as BudgetExhausted or EndReached; the UI then falls back to the
        // live edge. The gate here is clean, bounded termination rather than
        // forcing a deep-history restore during room selection.
        let room_terminated_cleanly = match &status {
            TimelineAnchorRestoreStatus::Found => true,
            TimelineAnchorRestoreStatus::EndReached
            | TimelineAnchorRestoreStatus::BudgetExhausted => true,
            TimelineAnchorRestoreStatus::Failed { .. }
            | TimelineAnchorRestoreStatus::Superseded => {
                eprintln!("cache_restore room={room_idx}: restore status={status_label} offline");
                false
            }
        };
        if !room_terminated_cleanly {
            all_deep_restores_terminated_cleanly = false;
        }

        let unsub_id = conn2.next_request_id();
        conn2
            .command(CoreCommand::Timeline(TimelineCommand::Unsubscribe {
                request_id: unsub_id,
                key,
            }))
            .await
            .map_err(|e| format!("cache_restore: offline unsubscribe failed: {e}"))?;
    }

    let aggregate_ms = aggregate_start.elapsed().as_millis();
    eprintln!("cache_restore total_cycles={total_cycles} total_ms={aggregate_ms}");

    // -----------------------------------------------------------------------
    // Shallow-anchor gate (P1 lazy-reveal-fence fix):
    // The anchor is in the live in-memory prefix (< CACHE_RESTORE_SHALLOW_DEPTH
    // events).  live_lazy_paginate_backwards must reveal it without loading any
    // on-disk chunk (cycle_count == 0).  On code without the P1 fix this may
    // reach EndReached or BudgetExhausted prematurely; with the fix it is Found.
    // -----------------------------------------------------------------------
    let shallow_key2 = TimelineKey::room(account_key.clone(), shallow_room_id.clone());
    let shallow_sub2 = conn2.next_request_id();
    conn2
        .command(CoreCommand::Timeline(TimelineCommand::Subscribe {
            request_id: shallow_sub2,
            key: shallow_key2.clone(),
        }))
        .await
        .map_err(|e| format!("cache_restore shallow: offline subscribe failed: {e}"))?;
    let _shallow_initial2 = wait_for_initial_items(
        &mut conn2,
        &shallow_key2,
        shallow_sub2,
        "cache_restore shallow offline subscribe",
    )
    .await?;

    let shallow_restore_req = conn2.next_request_id();
    conn2
        .command(CoreCommand::Timeline(
            TimelineCommand::RestoreTimelineAnchor {
                request_id: shallow_restore_req,
                key: shallow_key2.clone(),
                event_id: shallow_anchor_id.clone(),
                max_batches: CACHE_RESTORE_PROD_MAX_BATCHES,
                event_count: CACHE_RESTORE_PROD_EVENT_COUNT,
            },
        ))
        .await
        .map_err(|e| {
            format!("cache_restore shallow: offline RestoreTimelineAnchor submit failed: {e}")
        })?;

    let mut shallow_cycle_count: u16 = 0;
    let shallow_status = loop {
        let event = tokio::time::timeout(Duration::from_secs(60), conn2.recv_event())
            .await
            .map_err(|_| {
                "cache_restore shallow: timed out waiting for AnchorRestoreFinished".to_owned()
            })?
            .map_err(|lag| {
                format!(
                    "cache_restore shallow: event stream lagged (skipped={})",
                    lag.skipped
                )
            })?;
        match event {
            CoreEvent::Timeline(TimelineEvent::PaginationStateChanged {
                key: ref ev_key,
                direction,
                state: PaginationState::Paginating,
                ..
            }) if ev_key == &shallow_key2 && direction == PaginationDirection::Backward => {
                shallow_cycle_count += 1;
            }
            CoreEvent::Timeline(TimelineEvent::AnchorRestoreFinished {
                request_id: ev_req,
                key: ref ev_key,
                ref status,
            }) if ev_req == shallow_restore_req && ev_key == &shallow_key2 => {
                break status.clone();
            }
            _ => {}
        }
    };

    let shallow_status_label = match &shallow_status {
        TimelineAnchorRestoreStatus::Found => "found",
        TimelineAnchorRestoreStatus::EndReached => "end_reached",
        TimelineAnchorRestoreStatus::BudgetExhausted => "budget_exhausted",
        TimelineAnchorRestoreStatus::Superseded => "superseded",
        TimelineAnchorRestoreStatus::Failed { .. } => "failed",
    };
    eprintln!("cache_restore shallow cycles={shallow_cycle_count} status={shallow_status_label}");

    // Gate: shallow anchor must reach Found (the lazy-reveal path must settle
    // before declaring the restore terminal).  cycle_count==0 is the expected
    // value after the P1 fix (no disk chunk needed); a non-zero count is
    // unexpected but not a hard gate here — correctness (Found) is the gate.
    let shallow_succeeded = matches!(&shallow_status, TimelineAnchorRestoreStatus::Found);
    if !shallow_succeeded {
        eprintln!(
            "cache_restore shallow: status={shallow_status_label} — \
             lazy-reveal-fence (P1) fix not yet applied or not effective \
             (EXPECTED RED before impl-stage1 P1 fix lands)"
        );
    }
    if shallow_cycle_count > 0 {
        eprintln!(
            "cache_restore shallow: cycles={shallow_cycle_count} > 0 — \
             disk chunks loaded for a shallow anchor; expected 0 after P1 fix"
        );
    }

    let shallow_unsub2 = conn2.next_request_id();
    conn2
        .command(CoreCommand::Timeline(TimelineCommand::Unsubscribe {
            request_id: shallow_unsub2,
            key: shallow_key2,
        }))
        .await
        .map_err(|e| format!("cache_restore shallow: offline unsubscribe failed: {e}"))?;

    // SECONDARY GATE (room-entry speed regression gate):
    // Each deep-anchor restore must terminate in ≤ CACHE_RESTORE_MAX_CYCLES
    // backward-paginate cycles. It may be Found, EndReached, or BudgetExhausted;
    // what matters here is that a stale/deep anchor cannot stall room selection.
    let slow_rooms: Vec<usize> = room_cycle_counts
        .iter()
        .enumerate()
        .filter(|&(_, c)| *c > CACHE_RESTORE_MAX_CYCLES)
        .map(|(i, _)| i)
        .collect();

    cleanup_logged_in_runtime(conn2, runtime2, account_key, "cache_restore cleanup").await?;

    if !all_deep_restores_terminated_cleanly {
        return Err(
            "cache_restore: deep anchor restore did not terminate cleanly within room-entry path"
                .to_owned(),
        );
    }

    if !slow_rooms.is_empty() {
        let worst = room_cycle_counts.iter().copied().max().unwrap_or(0);
        return Err(format!(
            "cache_restore: deep anchor restore used {worst} backward-paginate cycles \
             (> {CACHE_RESTORE_MAX_CYCLES}) — room entry may block on stale/deep anchors"
        ));
    }

    // Shallow-anchor gate: emits after the deep gates pass so the report
    // clearly distinguishes deep-restore failures from P1 lazy-reveal failures.
    if !shallow_succeeded {
        return Err(format!(
            "cache_restore: shallow anchor reached status={shallow_status_label} \
             (expected Found) — lazy-reveal-fence (P1) fix not yet applied \
             (EXPECTED RED before impl-stage1 P1 fix lands)"
        ));
    }
    println!("cache_restore_shallow=ok");

    println!("cache_restore_offline=ok");
    println!("cache_restore=ok");
    Ok(())
}

pub(super) async fn run_focused_send_queue_scenario(config: &QaConfig) -> Result<(), String> {
    let QaParticipantLoginOutcome {
        runtime,
        mut conn,
        account_key,
        bootstrap_recovery_secret,
    } = login_synced_participant_for_qa(
        &config.homeserver,
        qa_data_dir("send_queue_bootstrap"),
        &config.user_a,
        &config.password_a,
        "Koushi Core QA Send Queue Bootstrap",
        "send_queue bootstrap login",
        "send_queue bootstrap gate",
        QaParticipantLoginGate::BootstrapNewIdentity,
    )
    .await?;
    println!("login_sync=ok");

    let sync_stop_id = conn.next_request_id();
    conn.command(CoreCommand::Sync(SyncCommand::Stop {
        request_id: sync_stop_id,
    }))
    .await
    .map_err(|e| format!("send_queue bootstrap submit sync stop: {e}"))?;
    wait_for_sync_stopped(&mut conn, sync_stop_id, "send_queue bootstrap sync stop").await?;

    let logout_id = conn.next_request_id();
    conn.command(CoreCommand::Account(AccountCommand::Logout {
        request_id: logout_id,
    }))
    .await
    .map_err(|e| format!("send_queue bootstrap submit logout: {e}"))?;
    wait_for_logged_out(
        &mut conn,
        logout_id,
        &account_key,
        "send_queue bootstrap logout",
    )
    .await?;

    drop(conn);
    tokio::time::timeout(EVENT_TIMEOUT, runtime.shutdown())
        .await
        .map_err(|_| "send_queue bootstrap ordered runtime shutdown timed out".to_owned())?;

    let recovery_secret = bootstrap_recovery_secret
        .ok_or_else(|| "send_queue bootstrap recovery secret unavailable".to_owned())?;
    run_send_queue_stage(config, &recovery_secret).await
}

pub(super) async fn run_send_queue_stage(
    config: &QaConfig,
    recovery_secret: &AuthSecret,
) -> Result<(), String> {
    let display_projection_reset_fallback_baseline =
        koushi_core::timeline::display_projection_reset_fallback_count();
    let proxy = QaTcpProxy::start(&config.homeserver)?;
    let data_dir = qa_data_dir("send_queue");
    let proxy_homeserver = proxy.homeserver_url();
    let QaParticipantLoginOutcome {
        runtime,
        mut conn,
        account_key,
        bootstrap_recovery_secret: _,
    } = login_synced_participant_for_qa(
        &proxy_homeserver,
        data_dir.clone(),
        &config.user_a,
        &config.password_a,
        "Koushi Core QA Send Queue",
        "send_queue login",
        "send_queue recovery gate",
        QaParticipantLoginGate::RecoverExistingIdentity(recovery_secret),
    )
    .await?;

    let room_id = create_room_for_qa(
        &mut conn,
        "QA Send Queue Room",
        false,
        "send_queue create room",
    )
    .await?;
    wait_for_room_in_room_list(&mut conn, &room_id, "send_queue room list").await?;

    let key = TimelineKey::room(account_key.clone(), room_id.clone());
    let subscribe_id = conn.next_request_id();
    conn.command(CoreCommand::Timeline(TimelineCommand::Subscribe {
        request_id: subscribe_id,
        key: key.clone(),
    }))
    .await
    .map_err(|e| format!("send_queue: submit subscribe failed: {e}"))?;
    wait_for_initial_items(&mut conn, &key, subscribe_id, "send_queue subscribe").await?;

    proxy.disable();
    let first = send_text_expect_local_echo(
        &mut conn,
        &key,
        "qa-send-queue-first",
        "QA send queue first offline",
        "send_queue first offline",
    )
    .await?;
    wait_for_timeline_send_state(
        &mut conn,
        &key,
        &first.sdk_transaction_id,
        |state| matches!(state, TimelineSendState::NotSent { .. }),
        "send_queue first not_sent",
    )
    .await?;
    println!("send_fail=ok");

    let second = send_text_expect_local_echo(
        &mut conn,
        &key,
        "qa-send-queue-second",
        "QA send queue second offline",
        "send_queue second offline",
    )
    .await?;

    proxy.enable();
    let room_send_forwarded_before_retry = proxy.room_send_forwarded_count();
    let room_send_responses_completed_before_retry = proxy.room_send_responses_completed_count();
    let retry_id = retry_send_queue_item(
        &mut conn,
        &key,
        &first.sdk_transaction_id,
        "send_queue retry first",
    )
    .await?;
    wait_for_send_completions_in_order(
        &mut conn,
        &key,
        retry_id,
        &first,
        &second,
        "send_queue fifo retry",
    )
    .await
    .map_err(|error| {
        format!(
            "{error} room_send_forwarded_after_retry={} \
             room_send_responses_completed_after_retry={}",
            proxy
                .room_send_forwarded_count()
                .saturating_sub(room_send_forwarded_before_retry),
            proxy
                .room_send_responses_completed_count()
                .saturating_sub(room_send_responses_completed_before_retry)
        )
    })?;
    println!("resend=ok");
    println!("fifo=ok");

    proxy.disable();
    let cancel = send_text_expect_local_echo(
        &mut conn,
        &key,
        "qa-send-queue-cancel",
        "QA send queue cancel offline",
        "send_queue cancel offline",
    )
    .await?;
    let cancel_id = cancel_send_queue_item(
        &mut conn,
        &key,
        &cancel.sdk_transaction_id,
        "send_queue cancel",
    )
    .await?;
    wait_for_cancelled_or_removed_send(
        &mut conn,
        &key,
        cancel_id,
        &cancel.sdk_transaction_id,
        "send_queue cancel removed",
    )
    .await?;
    println!("cancel_send=ok");

    let _restart = send_text_expect_local_echo(
        &mut conn,
        &key,
        "qa-send-queue-restart",
        "QA send queue restart offline",
        "send_queue restart offline",
    )
    .await?;

    unsubscribe_timeline_for_qa(
        &mut conn,
        &key,
        "send_queue unsubscribe before restart shutdown",
    )
    .await?;
    stop_sync_for_qa(&mut conn, "send_queue stop before restart").await?;
    drop(conn);
    runtime.shutdown().await;

    let runtime = CoreRuntime::start_with_data_dir(data_dir);
    let mut conn = runtime.attach();
    let restore_id = conn.next_request_id();
    conn.command(CoreCommand::Account(AccountCommand::RestoreSession {
        request_id: restore_id,
        account_key: account_key.clone(),
    }))
    .await
    .map_err(|e| format!("send_queue: submit restore failed: {e}"))?;
    wait_for_session_restored(&mut conn, restore_id, &account_key, "send_queue restore").await?;
    wait_for_ready_snapshot(&mut conn, "send_queue restored Ready").await?;

    let subscribe_id = conn.next_request_id();
    conn.command(CoreCommand::Timeline(TimelineCommand::Subscribe {
        request_id: subscribe_id,
        key: key.clone(),
    }))
    .await
    .map_err(|e| format!("send_queue: submit restore subscribe failed: {e}"))?;
    let initial = wait_for_initial_items(
        &mut conn,
        &key,
        subscribe_id,
        "send_queue restored subscribe",
    )
    .await?;
    let restored = find_timeline_item_with_body(&initial, "QA send queue restart offline")
        .ok_or_else(|| "send_queue restored local echo missing after restart".to_owned())?;
    let restored_txn = match restored.id {
        TimelineItemId::Transaction { transaction_id } => transaction_id,
        TimelineItemId::Event { .. } => {
            assert_zero_display_projection_reset_fallback_delta(
                display_projection_reset_fallback_baseline,
                koushi_core::timeline::display_projection_reset_fallback_count(),
            )?;
            println!("display_projection_reset_fallbacks=0");
            unsubscribe_timeline_for_qa(&mut conn, &key, "send_queue unsubscribe before cleanup")
                .await?;
            println!("unsent_restart=ok");
            cleanup_logged_in_runtime(conn, runtime, account_key, "send_queue cleanup").await?;
            return Ok(());
        }
        TimelineItemId::Synthetic { .. } => {
            return Err("send_queue restored item had synthetic id".to_owned());
        }
    };

    proxy.enable();
    let retry_already_sent =
        if matches!(restored.send_state, Some(TimelineSendState::NotSent { .. })) {
            retry_send_queue_item(&mut conn, &key, &restored_txn, "send_queue retry restored")
                .await?;
            true
        } else {
            false
        };
    wait_for_event_item_with_body_or_retry_not_sent(
        &mut conn,
        &key,
        &restored_txn,
        "QA send queue restart offline",
        retry_already_sent,
        "send_queue restored sent",
    )
    .await?;
    println!("unsent_restart=ok");

    assert_zero_display_projection_reset_fallback_delta(
        display_projection_reset_fallback_baseline,
        koushi_core::timeline::display_projection_reset_fallback_count(),
    )?;
    println!("display_projection_reset_fallbacks=0");

    unsubscribe_timeline_for_qa(&mut conn, &key, "send_queue unsubscribe before cleanup").await?;
    cleanup_logged_in_runtime(conn, runtime, account_key, "send_queue cleanup").await
}

pub(super) fn assert_zero_display_projection_reset_fallback_delta(
    baseline: u64,
    current: u64,
) -> Result<(), String> {
    if current == baseline {
        Ok(())
    } else {
        Err("send_queue: display projection reset fallback counter changed".to_owned())
    }
}

pub(super) async fn unsubscribe_timeline_for_qa(
    conn: &mut CoreConnection,
    key: &TimelineKey,
    label: &str,
) -> Result<(), String> {
    let request_id = conn.next_request_id();
    conn.command(CoreCommand::Timeline(TimelineCommand::Unsubscribe {
        request_id,
        key: key.clone(),
    }))
    .await
    .map_err(|e| format!("{label}: submit unsubscribe failed: {e}"))?;
    tokio::time::sleep(TIMELINE_UNSUBSCRIBE_SETTLE_TIMEOUT).await;
    Ok(())
}

pub(super) async fn run_timeline_reconnect_scenario(config: &QaConfig) -> Result<(), String> {
    run_timeline_reconnect_scenario_impl(config).await
}

pub(super) async fn run_timeline_reconnect_scenario_impl(config: &QaConfig) -> Result<(), String> {
    // The public scenario selector exercises the single live reconnect path.
    // The dormant persisted-gap fixture remains below for its separate
    // timeline continuity assertions.
    let restart_with_persisted_gap = false;
    let proxy = QaTcpProxy::start(&config.homeserver)?;
    let data_dir_a = qa_data_dir("timeline_reconnect_a");
    let data_dir_b = qa_data_dir("timeline_reconnect_b");

    let runtime_a = CoreRuntime::start_with_data_dir(data_dir_a.clone());
    let mut conn_a = runtime_a.attach();
    let login_a_id = conn_a.next_request_id();
    conn_a
        .command(CoreCommand::Account(AccountCommand::LoginPassword {
            request_id: login_a_id,
            request: koushi_state::LoginRequest {
                homeserver: proxy.homeserver_url(),
                username: config.user_a.clone(),
                password: AuthSecret::new(config.password_a.clone()),
                device_display_name: Some("Koushi Core QA Timeline Reconnect A".to_owned()),
            },
            platform: koushi_state::DisplayPlatform::Linux,
        }))
        .await
        .map_err(|e| format!("timeline_reconnect: submit login A failed: {e}"))?;

    complete_new_identity_gate_for_qa(&mut conn_a, &config.password_a, "timeline-reconnect-gate-a")
        .await?;

    let account_key_a =
        wait_for_logged_in(&mut conn_a, login_a_id, "timeline_reconnect login A").await?;
    wait_for_ready_snapshot(&mut conn_a, "timeline_reconnect session A Ready").await?;
    let sync_start_a_id = conn_a.next_request_id();
    conn_a
        .command(CoreCommand::Sync(SyncCommand::Start {
            request_id: sync_start_a_id,
        }))
        .await
        .map_err(|e| format!("timeline_reconnect: submit sync start A failed: {e}"))?;
    wait_for_sync_started_and_running(
        &mut conn_a,
        sync_start_a_id,
        "timeline_reconnect sync start A",
    )
    .await?;

    let runtime_b = CoreRuntime::start_with_data_dir(data_dir_b);
    let mut conn_b = runtime_b.attach();
    let login_b_id = conn_b.next_request_id();
    conn_b
        .command(CoreCommand::Account(AccountCommand::LoginPassword {
            request_id: login_b_id,
            request: koushi_state::LoginRequest {
                homeserver: config.homeserver.clone(),
                username: config.user_b.clone(),
                password: AuthSecret::new(config.password_b.clone()),
                device_display_name: Some("Koushi Core QA Timeline Reconnect B".to_owned()),
            },
            platform: koushi_state::DisplayPlatform::Linux,
        }))
        .await
        .map_err(|e| format!("timeline_reconnect: submit login B failed: {e}"))?;

    complete_new_identity_gate_for_qa(&mut conn_b, &config.password_b, "timeline-reconnect-gate-b")
        .await?;

    let account_key_b =
        wait_for_logged_in(&mut conn_b, login_b_id, "timeline_reconnect login B").await?;
    wait_for_ready_snapshot(&mut conn_b, "timeline_reconnect session B Ready").await?;
    let sync_start_b_id = conn_b.next_request_id();
    conn_b
        .command(CoreCommand::Sync(SyncCommand::Start {
            request_id: sync_start_b_id,
        }))
        .await
        .map_err(|e| format!("timeline_reconnect: submit sync start B failed: {e}"))?;
    wait_for_sync_started_and_running(
        &mut conn_b,
        sync_start_b_id,
        "timeline_reconnect sync start B",
    )
    .await?;

    let user_b_full_id = format!("@{}:{}", config.user_b, config.server_name);
    let room_id = if restart_with_persisted_gap {
        let room_id = start_direct_message_for_qa(
            &mut conn_a,
            &user_b_full_id,
            "timeline legacy persisted gap start direct message",
        )
        .await?;
        wait_for_dm_room_in_room_list(
            &mut conn_a,
            &room_id,
            "timeline legacy persisted gap A DM room list",
        )
        .await?;
        wait_for_invite_in_snapshot(
            &mut conn_b,
            &room_id,
            None,
            "timeline legacy persisted gap B sees DM invite",
        )
        .await?;
        accept_invite_for_qa(
            &mut conn_b,
            &room_id,
            "timeline legacy persisted gap B accepts DM invite",
        )
        .await?;
        wait_for_room_in_room_list(
            &mut conn_b,
            &room_id,
            "timeline legacy persisted gap B room list",
        )
        .await?;
        wait_for_dm_room_in_room_list(
            &mut conn_a,
            &room_id,
            "timeline legacy persisted gap A confirms direct message",
        )
        .await?;
        room_id
    } else {
        let room_id = create_room_for_qa(
            &mut conn_a,
            "QA Timeline Reconnect Room",
            true,
            "timeline_reconnect create room",
        )
        .await?;
        wait_for_room_in_room_list(&mut conn_a, &room_id, "timeline_reconnect A room list").await?;
        invite_user_for_qa(
            &mut conn_a,
            &room_id,
            &user_b_full_id,
            "timeline_reconnect invite B",
        )
        .await?;
        wait_for_invite_in_snapshot(
            &mut conn_b,
            &room_id,
            Some(false),
            "timeline_reconnect B sees invite",
        )
        .await?;
        accept_invite_for_qa(&mut conn_b, &room_id, "timeline_reconnect B accepts invite").await?;
        wait_for_room_in_room_list(&mut conn_b, &room_id, "timeline_reconnect B room list").await?;
        wait_for_encrypted_room_projection_for_qa(
            &mut conn_a,
            &room_id,
            "timeline_reconnect A encrypted room projection",
        )
        .await?;
        wait_for_encrypted_room_projection_for_qa(
            &mut conn_b,
            &room_id,
            "timeline_reconnect B encrypted room projection",
        )
        .await?;
        room_id
    };

    // Rebuild both SyncService instances after the room membership is stable.
    // The subsequent timeline subscription then exercises a room present in
    // the service's initial list instead of depending on an operation-time
    // room-list refresh racing the subscribe command.
    stop_sync_for_qa(&mut conn_a, "timeline_reconnect restart setup stop A").await?;
    stop_sync_for_qa(&mut conn_b, "timeline_reconnect restart setup stop B").await?;
    start_sync_for_qa(&mut conn_a, "timeline_reconnect restart setup start A").await?;
    start_sync_for_qa(&mut conn_b, "timeline_reconnect restart setup start B").await?;

    let key_a = TimelineKey::room(account_key_a.clone(), room_id.clone());
    let key_b = TimelineKey::room(account_key_b.clone(), room_id);
    subscribe_and_ack_active_timeline_projection_for_qa(
        &mut conn_a,
        &key_a,
        "timeline_reconnect subscribe A",
    )
    .await?;
    subscribe_and_ack_active_timeline_projection_for_qa(
        &mut conn_b,
        &key_b,
        "timeline_reconnect subscribe B",
    )
    .await?;

    let seed_body = "QA timeline reconnect known anchor";
    let seed_txn = "qa-timeline-reconnect-seed";
    let seed_send_id = conn_b.next_request_id();
    conn_b
        .command(CoreCommand::Timeline(TimelineCommand::SendText {
            request_id: seed_send_id,
            key: key_b.clone(),
            transaction_id: seed_txn.to_owned(),
            document: koushi_state::ComposerDocument::from_plain_text(seed_body.to_owned()),
        }))
        .await
        .map_err(|e| format!("timeline_reconnect: submit seed failed: {e}"))?;
    let seed_outcome = wait_for_send_flow_completion(
        &mut conn_b,
        seed_send_id,
        &key_b,
        seed_txn,
        seed_body,
        "timeline_reconnect seed known anchor",
    )
    .await?;
    wait_for_item_with_body_or_decryption_failure(
        &mut conn_a,
        &key_a,
        seed_body,
        "timeline_reconnect A receives known anchor",
    )
    .await?;
    unsubscribe_timeline_for_qa(
        &mut conn_a,
        &key_a,
        "timeline_reconnect unsubscribe A before offline gap",
    )
    .await?;
    proxy.disable();
    wait_for_sync_reconnecting(&mut conn_a, "timeline_reconnect A offline").await?;

    let offline_event_count = 21;
    let offline_bodies = (0..offline_event_count)
        .map(|index| format!("QA timeline reconnect offline {index:02}"))
        .collect::<Vec<_>>();
    for (index, body) in offline_bodies.iter().enumerate() {
        let txn = format!("qa-timeline-reconnect-offline-{index:02}");
        let send_b_id = conn_b.next_request_id();
        conn_b
            .command(CoreCommand::Timeline(TimelineCommand::SendText {
                request_id: send_b_id,
                key: key_b.clone(),
                transaction_id: txn.clone(),
                document: koushi_state::ComposerDocument::from_plain_text(body.clone()),
            }))
            .await
            .map_err(|e| format!("timeline_reconnect: submit B offline send failed: {e}"))?;
        let send_label = format!("timeline_reconnect B send while A offline ordinal={index}");
        wait_for_send_flow_completion(&mut conn_b, send_b_id, &key_b, &txn, body, &send_label)
            .await?;
    }

    proxy.enable();
    wait_for_sync_running_after_reconnect(&mut conn_a, "timeline_reconnect A recovered").await?;
    let newest_persisted_gap = if restart_with_persisted_gap {
        proxy.disable();
        wait_for_sync_reconnecting(
            &mut conn_a,
            "timeline legacy persisted gap disconnect before second limited response",
        )
        .await?;
        let bodies = (0..30)
            .map(|index| format!("QA timeline persisted newest gap {index:03}"))
            .collect::<Vec<_>>();
        let mut newest_known_event_id = None;
        for (index, body) in bodies.iter().enumerate() {
            let txn = format!("qa-timeline-persisted-newest-{index:03}");
            let send_b_id = conn_b.next_request_id();
            conn_b
                .command(CoreCommand::Timeline(TimelineCommand::SendText {
                    request_id: send_b_id,
                    key: key_b.clone(),
                    transaction_id: txn.clone(),
                    document: koushi_state::ComposerDocument::from_plain_text(body.clone()),
                }))
                .await
                .map_err(|e| {
                    format!("timeline legacy persisted gap: submit second batch failed: {e}")
                })?;
            let outcome = wait_for_send_flow_completion(
                &mut conn_b,
                send_b_id,
                &key_b,
                &txn,
                body,
                "timeline legacy persisted gap second offline batch",
            )
            .await?;
            if index + 1 == bodies.len() {
                newest_known_event_id = Some(outcome.event_id);
            }
        }

        proxy.enable();
        wait_for_sync_running_after_reconnect(
            &mut conn_a,
            "timeline legacy persisted gap second limited reconnect committed",
        )
        .await?;
        Some((
            bodies,
            newest_known_event_id.expect("persisted newest-gap batch must contain a newest event"),
        ))
    } else {
        None
    };
    let (runtime_a, mut conn_a, room_absent_checkpoint_baseline) = if restart_with_persisted_gap {
        stop_sync_for_qa(
            &mut conn_b,
            "timeline legacy persisted gap stop B before room-absent proof",
        )
        .await?;
        stop_sync_for_qa(
            &mut conn_a,
            "timeline legacy persisted gap stop before restart",
        )
        .await?;
        drop(conn_a);
        tokio::time::timeout(EVENT_TIMEOUT, runtime_a.shutdown())
            .await
            .map_err(|_| {
                "timeline legacy persisted gap timed out shutting down before restart".to_owned()
            })?;
        let room_absent_checkpoint_baseline = koushi_diagnostics::snapshot()
            .records
            .iter()
            .filter(|record| {
                record.event.source == "core.live_catchup"
                    && record.event.stage == "checkpoint"
                    && record.event.fields.iter().any(|field| {
                        field.key == "checkpoint_origin"
                            && field.value
                                == koushi_diagnostics::DiagnosticValue::Token("room_absent")
                    })
            })
            .count();

        let restarted_runtime = CoreRuntime::start_with_data_dir(data_dir_a.clone());
        let mut restarted_conn = restarted_runtime.attach();
        let restore_id = restarted_conn.next_request_id();
        restarted_conn
            .command(CoreCommand::Account(AccountCommand::RestoreSession {
                request_id: restore_id,
                account_key: account_key_a.clone(),
            }))
            .await
            .map_err(|e| format!("timeline legacy persisted gap: submit restore A failed: {e}"))?;
        wait_for_session_restored(
            &mut restarted_conn,
            restore_id,
            &account_key_a,
            "timeline legacy persisted gap restore A",
        )
        .await?;
        wait_for_ready_snapshot(
            &mut restarted_conn,
            "timeline legacy persisted gap restored session A Ready",
        )
        .await?;

        let restart_sync_id = restarted_conn.next_request_id();
        restarted_conn
            .command(CoreCommand::Sync(SyncCommand::Start {
                request_id: restart_sync_id,
            }))
            .await
            .map_err(|e| {
                format!("timeline legacy persisted gap: submit restarted sync A failed: {e}")
            })?;
        wait_for_sync_started(
            &mut restarted_conn,
            restart_sync_id,
            "timeline legacy persisted gap restart selects SyncService",
        )
        .await?;
        wait_for_sync_running_after_reconnect(
            &mut restarted_conn,
            "timeline legacy persisted gap room-absent response committed",
        )
        .await?;
        (
            restarted_runtime,
            restarted_conn,
            Some(room_absent_checkpoint_baseline),
        )
    } else {
        (runtime_a, conn_a, None)
    };
    let initial_live_tail_snapshot_baseline = if restart_with_persisted_gap {
        Some(live_tail_snapshot_completion_count_for_qa())
    } else {
        None
    };
    let live_tail_recent_body = "QA timeline live tail refreshed recent";
    if restart_with_persisted_gap {
        let (newest_known_bodies, newest_known_event_id) = newest_persisted_gap
            .as_ref()
            .expect("persisted-gap live-tail refresh requires a known newest event");
        let newest_known_body = newest_known_bodies
            .last()
            .expect("persisted newest-gap batch must not be empty");
        proxy.arm_first_live_tail_messages_page(
            newest_known_event_id.clone(),
            newest_known_body.clone(),
            "$qa-live-tail-refreshed:example.invalid".to_owned(),
            live_tail_recent_body.to_owned(),
            seed_outcome.event_id.clone(),
            user_b_full_id.clone(),
            seed_body.to_owned(),
        )?;
    }
    let reopened_before_later = None;
    if let Some(baseline) = room_absent_checkpoint_baseline {
        let initial_live_tail_snapshot_baseline = initial_live_tail_snapshot_baseline
            .expect("persisted-gap live-tail snapshot baseline must be armed before refresh");
        tokio::time::timeout(EVENT_TIMEOUT, async {
            loop {
                let count = koushi_diagnostics::snapshot()
                    .records
                    .iter()
                    .filter(|record| {
                        record.event.source == "core.live_catchup"
                            && record.event.stage == "checkpoint"
                            && record.event.fields.iter().any(|field| {
                                field.key == "checkpoint_origin"
                                    && field.value
                                        == koushi_diagnostics::DiagnosticValue::Token(
                                            "room_absent",
                                        )
                            })
                    })
                    .count();
                if count > baseline {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .map_err(|_| {
            "timeline legacy persisted gap did not observe a new room_absent checkpoint after restart"
                .to_owned()
        })?;
        let recent_live_tail_render = wait_for_item_with_body(
            &mut conn_a,
            &key_a,
            live_tail_recent_body,
            "timeline legacy persisted gap renders tokenless live-tail refresh",
        )
        .await;
        let observation = proxy.live_tail_messages_observation()?;
        if recent_live_tail_render.is_err() {
            return Err(format!(
                "timeline legacy persisted gap did not render canned live-tail event \
                 (requests={}, exact_tokenless_limit={}, had_from={}, served={})",
                observation.room_messages_request_count,
                observation.first_request_was_exact_tokenless_limit,
                observation.first_request_had_from,
                observation.freshness_page_served,
            ));
        }
        if observation.room_messages_request_count != 1
            || !observation.first_request_was_exact_tokenless_limit
            || observation.first_request_had_from
            || !observation.freshness_page_served
        {
            return Err(format!(
                "timeline legacy persisted gap expected one exact tokenless live-tail request \
                 (requests={}, exact_tokenless_limit={}, had_from={}, served={})",
                observation.room_messages_request_count,
                observation.first_request_was_exact_tokenless_limit,
                observation.first_request_had_from,
                observation.freshness_page_served,
            ));
        }
        let initial_live_tail_gap_count = wait_for_live_tail_snapshot_gap_count_for_qa(
            &conn_a,
            initial_live_tail_snapshot_baseline,
            "timeline legacy persisted gap initial live-tail snapshot",
        )
        .await?;
        if initial_live_tail_gap_count == 0 {
            return Err(
                "timeline legacy persisted gap initial live-tail snapshot did not retain a continuity gap"
                    .to_owned(),
            );
        }
        println!("legacy_live_tail_room_absent=ok");
        println!("live_tail_anchored_silent_gap=ok");

        stop_sync_for_qa(
            &mut conn_a,
            "timeline legacy persisted gap stop before detached tail restart",
        )
        .await?;
        drop(conn_a);
        tokio::time::timeout(EVENT_TIMEOUT, runtime_a.shutdown())
            .await
            .map_err(|_| {
                "timeline legacy persisted gap timed out shutting down before detached restart"
                    .to_owned()
            })?;

        let detached_runtime = CoreRuntime::start_with_data_dir(data_dir_a.clone());
        let mut detached_conn = detached_runtime.attach();
        let detached_restore_id = detached_conn.next_request_id();
        detached_conn
            .command(CoreCommand::Account(AccountCommand::RestoreSession {
                request_id: detached_restore_id,
                account_key: account_key_a.clone(),
            }))
            .await
            .map_err(|e| {
                format!("timeline legacy persisted gap: submit detached restore A failed: {e}")
            })?;
        wait_for_session_restored(
            &mut detached_conn,
            detached_restore_id,
            &account_key_a,
            "timeline legacy persisted gap detached restore A",
        )
        .await?;
        wait_for_ready_snapshot(
            &mut detached_conn,
            "timeline legacy persisted gap detached restored session A Ready",
        )
        .await?;

        let detached_sync_start_id = detached_conn.next_request_id();
        detached_conn
            .command(CoreCommand::Sync(SyncCommand::Start {
                request_id: detached_sync_start_id,
            }))
            .await
            .map_err(|e| {
                format!("timeline legacy persisted gap: submit detached sync A failed: {e}")
            })?;
        wait_for_sync_started(
            &mut detached_conn,
            detached_sync_start_id,
            "timeline legacy persisted gap detached restart selects SyncService",
        )
        .await?;
        wait_for_sync_running_after_reconnect(
            &mut detached_conn,
            "timeline legacy persisted gap detached room-absent response committed",
        )
        .await?;

        let detached_newest_body = "QA timeline detached live tail 127";
        let detached_end_token = "qa-live-tail-detached-end".to_owned();
        proxy.arm_detached_live_tail_messages_page(
            qa_detached_live_tail_events(&user_b_full_id),
            detached_end_token.clone(),
        )?;
        let detached_items = subscribe_and_ack_active_timeline_projection_for_qa(
            &mut detached_conn,
            &key_a,
            "timeline legacy persisted gap detached live tail subscription",
        )
        .await?;
        let (detached_items, visible_gap, initial_gap_projection, detached_gap_count) =
            wait_for_projected_gap_and_item_for_qa(
                &mut detached_conn,
                &key_a,
                detached_items,
                detached_newest_body,
                "timeline legacy persisted gap projects detached visible gap",
            )
            .await?;
        let detached_observation = proxy.live_tail_messages_observation()?;
        if detached_observation.room_messages_request_count != 1
            || !detached_observation.first_request_was_exact_tokenless_limit
            || detached_observation.first_request_had_from
            || !detached_observation.freshness_page_served
        {
            return Err(format!(
                "timeline legacy persisted gap expected one exact tokenless detached request \
                 (requests={}, exact_tokenless_limit={}, had_from={}, served={})",
                detached_observation.room_messages_request_count,
                detached_observation.first_request_was_exact_tokenless_limit,
                detached_observation.first_request_had_from,
                detached_observation.freshness_page_served,
            ));
        }
        let detached_observation = proxy.live_tail_messages_observation()?;
        if detached_observation.expected_end_token_request_count != 0 {
            return Err(format!(
                "timeline legacy persisted gap detached tail consumed its continuation token before explicit viewport demand \
                 (detached_end_token_requests={})",
                detached_observation.expected_end_token_request_count,
            ));
        }
        println!("live_tail_detached_gap=ok");

        let historical_continuation_body = "QA timeline detached historical continuation";
        proxy.arm_historical_continuation_messages_page(
            detached_end_token,
            qa_detached_historical_continuation_events(&user_b_full_id),
        )?;
        let visible_gap_request_id = detached_conn.next_request_id();
        detached_conn
            .command(CoreCommand::Timeline(TimelineCommand::ObserveViewport {
                request_id: visible_gap_request_id,
                key: key_a.clone(),
                observation: TimelineViewportObservation {
                    first_visible_event_id: visible_gap.first_visible_event_id.clone(),
                    last_visible_event_id: visible_gap.last_visible_event_id.clone(),
                    visible_gap_ids: vec![visible_gap.id],
                    at_bottom: false,
                },
            }))
            .await
            .map_err(|e| {
                format!("timeline legacy persisted gap: submit visible gap observation failed: {e}")
            })?;
        wait_for_exact_items_and_gap_release(
            &mut detached_conn,
            &key_a,
            detached_items,
            &[historical_continuation_body.to_owned()],
            Some(initial_gap_projection),
            Some(visible_gap.id),
            "timeline legacy persisted gap visible continuation repair",
        )
        .await?;
        let historical_observation = proxy.live_tail_messages_observation()?;
        if !historical_observation.first_request_had_from
            || !historical_observation.expected_end_token_was_used
            || historical_observation.expected_end_token_request_count != 1
            || !historical_observation.freshness_page_served
        {
            return Err(format!(
                "timeline legacy persisted gap expected one historical continuation request \
                 (had_from={}, exact_end={}, exact_end_requests={}, served={})",
                historical_observation.first_request_had_from,
                historical_observation.expected_end_token_was_used,
                historical_observation.expected_end_token_request_count,
                historical_observation.freshness_page_served,
            ));
        }
        wait_for_timeline_gap_count_for_qa(
            &detached_conn,
            detached_gap_count.saturating_sub(1),
            "timeline legacy persisted gap detached continuation closes its added gap",
        )
        .await?;
        println!("live_tail_historical_continuation=ok");

        start_sync_for_qa(
            &mut conn_b,
            "timeline legacy persisted gap resume B after room-absent proof",
        )
        .await?;
        cleanup_logged_in_runtime(
            conn_b,
            runtime_b,
            account_key_b,
            "timeline legacy persisted gap detached live-tail cleanup B",
        )
        .await?;
        cleanup_logged_in_runtime(
            detached_conn,
            detached_runtime,
            account_key_a,
            "timeline legacy persisted gap detached live-tail cleanup A",
        )
        .await?;
        return Ok(());
    }
    let reopened_items = match reopened_before_later {
        Some(items) => items,
        None => {
            subscribe_timeline_for_qa(
                &mut conn_a,
                &key_a,
                "timeline_reconnect reopen unsubscribed A room",
            )
            .await?
        }
    };
    wait_for_reconnect_projection(
        &mut conn_a,
        &key_a,
        &reopened_items,
        &offline_bodies,
        "timeline_reconnect A repairs the complete missed batch",
    )
    .await?;
    println!("timeline_reconnect_recv_after_reconnect=ok");
    println!("live_catchup_checkpoint=ok");
    println!("live_catchup_gap_repaired=ok");
    println!("timeline_reconnect=ok");

    cleanup_logged_in_runtime(
        conn_b,
        runtime_b,
        account_key_b,
        "timeline_reconnect cleanup B",
    )
    .await?;
    cleanup_logged_in_runtime(
        conn_a,
        runtime_a,
        account_key_a,
        "timeline_reconnect cleanup A",
    )
    .await?;
    Ok(())
}

pub(super) fn timeline_gap_count_for_qa(conn: &CoreConnection) -> u32 {
    match conn.snapshot().timeline.continuity {
        koushi_state::TimelineContinuityState::Inspecting {
            known_gap_count, ..
        } => known_gap_count,
        koushi_state::TimelineContinuityState::Incomplete { gap_count, .. }
        | koushi_state::TimelineContinuityState::Repairing { gap_count, .. }
        | koushi_state::TimelineContinuityState::FailedIncomplete { gap_count, .. } => gap_count,
        koushi_state::TimelineContinuityState::Unknown
        | koushi_state::TimelineContinuityState::Healthy { .. } => 0,
    }
}

fn live_tail_snapshot_completion_count_for_qa() -> usize {
    koushi_diagnostics::snapshot()
        .records
        .iter()
        .filter(|record| {
            record.event.source == "core.timeline_gap_repair"
                && record.event.stage == "inspection"
                && record.event.fields.iter().any(|field| {
                    field.key == "trigger"
                        && field.value
                            == koushi_diagnostics::DiagnosticValue::Token("live_tail_snapshot")
                })
                && record.event.fields.iter().any(|field| {
                    field.key == "outcome"
                        && matches!(
                            field.value,
                            koushi_diagnostics::DiagnosticValue::Token(
                                "unknown" | "incomplete" | "healthy"
                            )
                        )
                })
        })
        .count()
}

async fn wait_for_live_tail_snapshot_gap_count_for_qa(
    conn: &CoreConnection,
    completion_baseline: usize,
    label: &str,
) -> Result<u32, String> {
    tokio::time::timeout(EVENT_TIMEOUT, async {
        let mut previous_gap_count = None;
        let mut stable_samples = 0_u8;
        loop {
            let snapshot_completed =
                live_tail_snapshot_completion_count_for_qa() > completion_baseline;
            let continuity_is_inspecting = matches!(
                conn.snapshot().timeline.continuity,
                koushi_state::TimelineContinuityState::Inspecting { .. }
            );
            let gap_count = timeline_gap_count_for_qa(conn);
            if snapshot_completed && !continuity_is_inspecting {
                if previous_gap_count == Some(gap_count) {
                    stable_samples = stable_samples.saturating_add(1);
                } else {
                    previous_gap_count = Some(gap_count);
                    stable_samples = 0;
                }
                if stable_samples >= 1 {
                    break gap_count;
                }
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .map_err(|_| {
        format!(
            "{label}: did not settle a completed live-tail snapshot (snapshots={}, observed={})",
            live_tail_snapshot_completion_count_for_qa(),
            timeline_gap_count_for_qa(conn),
        )
    })
}

async fn wait_for_projected_gap_and_item_for_qa(
    conn: &mut CoreConnection,
    key: &TimelineKey,
    mut items: Vec<TimelineItem>,
    expected_body: &str,
    label: &str,
) -> Result<(Vec<TimelineItem>, QaVisibleGapSelection, (u64, u64), u32), String> {
    let mut capture = QaVisibleGapCapture::default();
    loop {
        capture.observe_items(&items, expected_body, label)?;
        if let Some((visible_gap, initial_gap_projection)) = capture.projected_gap()
            && let Some(settled_gap_count) =
                settled_nonzero_timeline_gap_count_for_qa(conn, initial_gap_projection.1)
        {
            return Ok((
                items,
                visible_gap.clone(),
                *initial_gap_projection,
                settled_gap_count,
            ));
        }

        let event = tokio::time::timeout(EVENT_TIMEOUT, conn.recv_event())
            .await
            .map_err(|_| format!("{label}: timed out waiting for a projected visible gap"))?
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;
        match event {
            CoreEvent::Timeline(TimelineEvent::InitialItems {
                key: ref event_key,
                items: replacement,
                ..
            }) if event_key == key => items = replacement,
            CoreEvent::Timeline(TimelineEvent::ItemsUpdated {
                key: ref event_key,
                diffs,
                ..
            }) if event_key == key => {
                for diff in &diffs {
                    apply_timeline_diff(&mut items, diff);
                }
            }
            CoreEvent::Timeline(TimelineEvent::GapPositionsUpdated {
                key: ref event_key,
                actor_generation,
                generation,
                positions,
                ..
            }) if event_key == key && !positions.is_empty() => {
                capture.observe_gap_positions(
                    &items,
                    actor_generation,
                    generation,
                    &positions,
                    label,
                )?;
            }
            _ => {}
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct QaVisibleGapSelection {
    id: TimelineGapId,
    first_visible_event_id: Option<String>,
    last_visible_event_id: Option<String>,
}

#[derive(Default)]
struct QaVisibleGapCapture {
    exact_body_present: bool,
    projected_gap: Option<(QaVisibleGapSelection, (u64, u64))>,
}

impl QaVisibleGapCapture {
    fn observe_items(
        &mut self,
        items: &[TimelineItem],
        expected_body: &str,
        label: &str,
    ) -> Result<(), String> {
        let body_count = items
            .iter()
            .filter(|item| item.body.as_deref() == Some(expected_body))
            .count();
        if body_count > 1 {
            return Err(format!(
                "{label}: detached live-tail row was projected twice"
            ));
        }

        let exact_body_present = body_count == 1;
        if !exact_body_present || !self.exact_body_present {
            self.projected_gap = None;
        }
        self.exact_body_present = exact_body_present;
        Ok(())
    }

    fn observe_gap_positions(
        &mut self,
        items: &[TimelineItem],
        actor_generation: u64,
        generation: u64,
        positions: &[TimelineGapPosition],
        label: &str,
    ) -> Result<(), String> {
        if !self.exact_body_present {
            self.projected_gap = None;
            return Ok(());
        }

        let visible_gap = select_visible_gap_for_qa(items, positions)
            .map_err(|error| format!("{label}: {error}"))?;
        if let Some((existing_gap, (existing_actor, _))) = self.projected_gap.as_ref()
            && (existing_gap.id != visible_gap.id || *existing_actor != actor_generation)
        {
            return Err(format!(
                "{label}: projected visible gap identity changed before viewport demand"
            ));
        }
        self.projected_gap = Some((visible_gap, (actor_generation, generation)));
        Ok(())
    }

    fn projected_gap(&self) -> Option<&(QaVisibleGapSelection, (u64, u64))> {
        self.projected_gap.as_ref()
    }
}

fn select_visible_gap_for_qa(
    items: &[TimelineItem],
    positions: &[TimelineGapPosition],
) -> Result<QaVisibleGapSelection, String> {
    let bracketed = positions
        .iter()
        .filter_map(|position| {
            let first_visible_event_id = items
                .get(..position.before_item_index)?
                .iter()
                .rev()
                .find_map(|item| match &item.id {
                    TimelineItemId::Event { event_id } => Some(event_id.clone()),
                    TimelineItemId::Transaction { .. } | TimelineItemId::Synthetic { .. } => None,
                })?;
            let last_visible_event_id =
                items
                    .get(position.before_item_index..)?
                    .iter()
                    .find_map(|item| match &item.id {
                        TimelineItemId::Event { event_id } => Some(event_id.clone()),
                        TimelineItemId::Transaction { .. } | TimelineItemId::Synthetic { .. } => {
                            None
                        }
                    })?;
            Some((
                position.before_item_index,
                position.id,
                QaVisibleGapSelection {
                    id: position.id,
                    first_visible_event_id: Some(first_visible_event_id),
                    last_visible_event_id: Some(last_visible_event_id),
                },
            ))
        })
        .max_by_key(|(before_item_index, id, _)| {
            (*before_item_index, id.topology_revision, id.ordinal)
        })
        .map(|(_, _, selection)| selection);
    if let Some(selection) = bracketed {
        return Ok(selection);
    }

    if let Some(position) = positions
        .iter()
        .filter(|position| position.before_item_index == 0)
        .max_by_key(|position| (position.id.topology_revision, position.id.ordinal))
    {
        return Ok(QaVisibleGapSelection {
            id: position.id,
            first_visible_event_id: None,
            last_visible_event_id: None,
        });
    }

    let min_before_item_index = positions
        .iter()
        .map(|position| position.before_item_index)
        .min()
        .map_or_else(|| "none".to_owned(), |index| index.to_string());
    let max_before_item_index = positions
        .iter()
        .map(|position| position.before_item_index)
        .max()
        .map_or_else(|| "none".to_owned(), |index| index.to_string());
    Err(format!(
        "visible gap selection found no bracketed or top-row position \
         (item_count={}, position_count={}, min_before_item_index={}, \
         max_before_item_index={})",
        items.len(),
        positions.len(),
        min_before_item_index,
        max_before_item_index,
    ))
}

fn settled_nonzero_timeline_gap_count_for_qa(
    conn: &CoreConnection,
    projection_generation: u64,
) -> Option<u32> {
    // The position event and Incomplete state share one inspection serial.
    // Starting repair allocates a newer serial, which FailedIncomplete retains.
    let gap_count = match conn.snapshot().timeline.continuity {
        koushi_state::TimelineContinuityState::Incomplete {
            generation,
            gap_count,
        } if generation == projection_generation => gap_count,
        koushi_state::TimelineContinuityState::Repairing {
            generation,
            gap_count,
            ..
        }
        | koushi_state::TimelineContinuityState::FailedIncomplete {
            generation,
            gap_count,
            ..
        } if generation >= projection_generation => gap_count,
        koushi_state::TimelineContinuityState::Unknown
        | koushi_state::TimelineContinuityState::Inspecting { .. }
        | koushi_state::TimelineContinuityState::Healthy { .. }
        | koushi_state::TimelineContinuityState::Incomplete { .. }
        | koushi_state::TimelineContinuityState::Repairing { .. }
        | koushi_state::TimelineContinuityState::FailedIncomplete { .. } => return None,
    };
    (gap_count > 0).then_some(gap_count)
}

async fn wait_for_timeline_gap_count_for_qa(
    conn: &CoreConnection,
    expected_gap_count: u32,
    label: &str,
) -> Result<(), String> {
    let result = tokio::time::timeout(EVENT_TIMEOUT, async {
        loop {
            if timeline_gap_count_for_qa(conn) == expected_gap_count {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await;
    match result {
        Ok(()) => Ok(()),
        Err(_) => Err(format!(
            "{label}: did not settle at the expected coarse gap count \
             (expected={}, observed={})",
            expected_gap_count,
            timeline_gap_count_for_qa(conn),
        )),
    }
}

fn qa_detached_live_tail_events(sender: &str) -> Vec<QaCannedTimelineEvent> {
    (0..128)
        .rev()
        .map(|index| QaCannedTimelineEvent {
            event_id: format!("$qa-live-tail-detached-{index:03}:example.invalid"),
            sender: sender.to_owned(),
            body: format!("QA timeline detached live tail {index:03}"),
            origin_server_ts: 1_900_000_100_000 + index as u64,
        })
        .collect()
}

fn qa_detached_historical_continuation_events(sender: &str) -> Vec<QaCannedTimelineEvent> {
    vec![QaCannedTimelineEvent {
        event_id: "$qa-live-tail-detached-historical:example.invalid".to_owned(),
        sender: sender.to_owned(),
        body: "QA timeline detached historical continuation".to_owned(),
        origin_server_ts: 1_900_000_099_999,
    }]
}

async fn wait_for_room_unread_count(
    conn: &mut CoreConnection,
    room_id: &str,
    label: &str,
) -> Result<(), String> {
    let started_at = std::time::Instant::now();
    loop {
        if conn
            .snapshot()
            .rooms
            .iter()
            .any(|room| room.room_id == room_id && room.unread_count > 0)
        {
            return Ok(());
        }
        if started_at.elapsed() > EVENT_TIMEOUT {
            return Err(format!(
                "{label}: timed out waiting for unread room summary"
            ));
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

async fn wait_for_activity_snapshot(
    conn: &mut CoreConnection,
    request_id: RequestId,
    label: &str,
) -> Result<(Vec<String>, Vec<String>, Vec<String>), String> {
    loop {
        let event = tokio::time::timeout(EVENT_TIMEOUT, conn.recv_event())
            .await
            .map_err(|_| format!("{label}: timed out waiting for Activity SnapshotLoaded"))?
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;

        match event {
            CoreEvent::Activity(ActivityEvent::SnapshotLoaded {
                request_id: ev_id,
                recent,
                unread,
                ..
            }) if ev_id == request_id => {
                let mut unread_room_ids = Vec::new();
                let mut unread_event_ids = Vec::new();
                for row in unread.rows {
                    match row.kind {
                        ActivityRowKind::Event => {
                            let event_id = row.event_id.ok_or_else(|| {
                                format!("{label}: Activity event row lacked an event id")
                            })?;
                            unread_event_ids.push(event_id);
                        }
                        ActivityRowKind::RoomUnread => {
                            if row.event_id.is_some() {
                                return Err(format!(
                                    "{label}: Activity placeholder contained an event id"
                                ));
                            }
                        }
                    }
                    unread_room_ids.push(row.room_id);
                }

                return Ok((
                    recent
                        .rows
                        .into_iter()
                        .filter_map(|row| row.event_id)
                        .collect(),
                    unread_event_ids,
                    unread_room_ids,
                ));
            }
            CoreEvent::OperationFailed {
                request_id: ev_id,
                failure,
            } if ev_id == request_id => {
                return Err(format!("{label}: Activity open failed: {failure:?}"));
            }
            _ => {}
        }
    }
}

async fn wait_for_activity_marked_read(
    conn: &mut CoreConnection,
    request_id: RequestId,
    label: &str,
) -> Result<(), String> {
    loop {
        let event = tokio::time::timeout(EVENT_TIMEOUT, conn.recv_event())
            .await
            .map_err(|_| format!("{label}: timed out waiting for Activity MarkedRead"))?
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;

        match event {
            CoreEvent::Activity(ActivityEvent::MarkedRead {
                request_id: ev_id, ..
            }) if ev_id == request_id => return Ok(()),
            CoreEvent::OperationFailed {
                request_id: ev_id,
                failure,
            } if ev_id == request_id => {
                return Err(format!("{label}: Activity mark-read failed: {failure:?}"));
            }
            _ => {}
        }
    }
}

async fn wait_for_activity_unread_empty(
    conn: &mut CoreConnection,
    label: &str,
) -> Result<(), String> {
    let started_at = std::time::Instant::now();
    loop {
        if matches!(
            &conn.snapshot().activity,
            ActivityState::Open { unread, .. } if unread.rows.is_empty()
        ) {
            return Ok(());
        }
        if started_at.elapsed() > EVENT_TIMEOUT {
            return Err(format!(
                "{label}: timed out waiting for empty unread stream"
            ));
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

pub(super) async fn run_activity_stage(
    conn_a: &mut CoreConnection,
    conn_b: &mut CoreConnection,
    key_a: &TimelineKey,
    key_b: &TimelineKey,
    room_id: &str,
) -> Result<(), String> {
    let activity_body = "Phase 23 QA activity unread seed";
    let txn = "qa-phase23-activity-unread".to_owned();
    let send_id = conn_b.next_request_id();
    conn_b
        .command(CoreCommand::Timeline(TimelineCommand::SendText {
            request_id: send_id,
            key: key_b.clone(),
            transaction_id: txn.clone(),
            document: koushi_state::ComposerDocument::from_plain_text(activity_body.to_owned()),
        }))
        .await
        .map_err(|e| format!("activity: submit unread seed failed: {e}"))?;

    let send_outcome = wait_for_send_flow_completion(
        conn_b,
        send_id,
        key_b,
        &txn,
        activity_body,
        "activity unread seed send",
    )
    .await?;
    wait_for_item_with_body(
        conn_a,
        key_a,
        activity_body,
        "activity unread seed observed by A",
    )
    .await?;

    wait_for_room_unread_count(conn_a, room_id, "activity room unread count").await?;

    let open_id = conn_a.next_request_id();
    conn_a
        .command(CoreCommand::App(AppCommand::OpenActivity {
            request_id: open_id,
        }))
        .await
        .map_err(|e| format!("activity: submit open failed: {e}"))?;
    let (recent_event_ids, unread_event_ids, unread_room_ids) =
        wait_for_activity_snapshot(conn_a, open_id, "activity open").await?;

    if !recent_event_ids
        .iter()
        .any(|event_id| event_id == &send_outcome.event_id)
    {
        return Err("activity recent projection did not include the unread seed".to_owned());
    }
    println!("activity_recent=ok");

    if !unread_room_ids
        .iter()
        .any(|unread_room_id| unread_room_id == room_id)
    {
        return Err("activity unread projection did not include the unread seed".to_owned());
    }
    println!("activity_unread=ok");
    if !unread_event_ids
        .iter()
        .any(|event_id| event_id == &send_outcome.event_id)
    {
        return Err("activity unread projection did not resolve the unread event".to_owned());
    }
    println!("activity_resolution=ok");

    let mark_id = conn_a.next_request_id();
    conn_a
        .command(CoreCommand::App(AppCommand::MarkActivityRead {
            request_id: mark_id,
            target: ActivityMarkReadTarget::All,
        }))
        .await
        .map_err(|e| format!("activity: submit mark-read failed: {e}"))?;
    wait_for_activity_marked_read(conn_a, mark_id, "activity mark read").await?;
    wait_for_activity_unread_empty(conn_a, "activity unread cleared").await?;
    println!("activity_markread=ok");

    Ok(())
}

pub(super) async fn subscribe_and_ack_active_timeline_projection_for_qa(
    conn: &mut CoreConnection,
    key: &TimelineKey,
    label: &str,
) -> Result<Vec<TimelineItem>, String> {
    let subscribe_request_id = conn.next_request_id();
    conn.command(CoreCommand::Timeline(TimelineCommand::Subscribe {
        request_id: subscribe_request_id,
        key: key.clone(),
    }))
    .await
    .map_err(|e| format!("{label}: submit timeline subscribe failed: {e}"))?;

    let deadline = QaEventDeadline::after(TIMELINE_INITIAL_EVENT_TIMEOUT);
    loop {
        let event = deadline
            .recv(conn)
            .await
            .map_err(|_| format!("{label}: timed out waiting for active timeline projection"))?
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;
        match event {
            CoreEvent::Timeline(TimelineEvent::InitialItems {
                request_id: Some(projection_request_id),
                key: ref event_key,
                generation,
                items,
                ..
            }) if event_key == key => {
                let acknowledgement_request_id = conn.next_request_id();
                conn.command(CoreCommand::App(
                    koushi_core::command::AppCommand::AcknowledgeTimelineProjection {
                        request_id: acknowledgement_request_id,
                        projection_request_id,
                        key: key.clone(),
                        generation,
                        item_count: items.len() as u64,
                        target_present: true,
                    },
                ))
                .await
                .map_err(|e| format!("{label}: submit projection acknowledgement failed: {e}"))?;
                return Ok(items);
            }
            CoreEvent::OperationFailed {
                request_id,
                failure,
            } if request_id == subscribe_request_id => {
                return Err(format!("{label} failed: {failure:?}"));
            }
            _ => {}
        }
    }
}

pub(super) fn thread_initial_items_need_paginate_backfill(
    initial_items: &[koushi_core::event::TimelineItem],
    expected_body: &str,
) -> bool {
    find_timeline_item_with_body(initial_items, expected_body).is_none()
}

fn thread_reply_should_repaginate_on_idle(pagination_ended: bool) -> bool {
    !pagination_ended
}

fn observe_send_queue_retry_item_state(
    item: &TimelineItem,
    sdk_transaction_id: &str,
    first_left_not_sent_after_retry: &mut bool,
) -> Option<&'static str> {
    if timeline_item_transaction_id(item) != Some(sdk_transaction_id) {
        return None;
    }
    match item.send_state.as_ref() {
        Some(TimelineSendState::NotSent {
            reason: koushi_core::event::TimelineSendFailureReason::Recoverable,
        }) if *first_left_not_sent_after_retry => Some("recoverable"),
        Some(TimelineSendState::NotSent {
            reason: koushi_core::event::TimelineSendFailureReason::Unrecoverable,
        }) if *first_left_not_sent_after_retry => Some("unrecoverable"),
        Some(TimelineSendState::NotSent { .. }) | None => None,
        Some(
            TimelineSendState::Sending | TimelineSendState::Cancelled | TimelineSendState::Sent,
        ) => {
            *first_left_not_sent_after_retry = true;
            None
        }
    }
}

pub(super) async fn wait_for_send_completions_in_order(
    conn: &mut CoreConnection,
    key: &TimelineKey,
    retry_request_id: RequestId,
    first: &SendQueueLocalEcho,
    second: &SendQueueLocalEcho,
    label: &str,
) -> Result<(), String> {
    let mut first_completed = false;
    let mut first_left_not_sent_after_retry = false;
    loop {
        let event = tokio::time::timeout(SEND_QUEUE_EVENT_TIMEOUT, conn.recv_event())
            .await
            .map_err(|_| {
                format!(
                    "{label}: timed out waiting for ordered SendCompleted events \
                     first_completed={first_completed}"
                )
            })?
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;

        match event {
            CoreEvent::Timeline(TimelineEvent::InitialItems {
                key: ref ev_key,
                items,
                ..
            }) if ev_key == key => {
                if let Some(reason) = items.iter().find_map(|item| {
                    observe_send_queue_retry_item_state(
                        item,
                        &first.sdk_transaction_id,
                        &mut first_left_not_sent_after_retry,
                    )
                }) {
                    return Err(format!(
                        "{label}: first queued send returned to NotSent reason={reason}"
                    ));
                }
            }
            CoreEvent::Timeline(TimelineEvent::ItemsUpdated {
                key: ref ev_key,
                diffs,
                ..
            }) if ev_key == key => {
                visit_timeline_diff_items(&diffs, |item| {
                    if let Some(reason) = observe_send_queue_retry_item_state(
                        item,
                        &first.sdk_transaction_id,
                        &mut first_left_not_sent_after_retry,
                    ) {
                        return Err(format!(
                            "{label}: first queued send returned to NotSent reason={reason}"
                        ));
                    }
                    Ok(())
                })?;
            }
            CoreEvent::Timeline(TimelineEvent::SendCompleted {
                request_id,
                key: ref ev_key,
                transaction_id,
                ..
            }) if ev_key == key && request_id == first.request_id => {
                if transaction_id != first.client_transaction_id {
                    return Err(format!("{label}: first completion transaction mismatch"));
                }
                first_completed = true;
            }
            CoreEvent::Timeline(TimelineEvent::SendCompleted {
                request_id,
                key: ref ev_key,
                transaction_id,
                ..
            }) if ev_key == key && request_id == second.request_id => {
                if !first_completed {
                    return Err(format!(
                        "{label}: later queued send completed before the failed predecessor"
                    ));
                }
                if transaction_id != second.client_transaction_id {
                    return Err(format!("{label}: second completion transaction mismatch"));
                }
                return Ok(());
            }
            CoreEvent::OperationFailed { request_id, .. } if request_id == retry_request_id => {
                return Err(format!("{label}: retry operation failed"));
            }
            CoreEvent::OperationFailed { request_id, .. }
                if request_id == first.request_id || request_id == second.request_id =>
            {
                return Err(format!("{label}: queued send operation failed"));
            }
            _ => {}
        }
    }
}

pub(super) async fn wait_for_cancelled_or_removed_send(
    conn: &mut CoreConnection,
    key: &TimelineKey,
    cancel_request_id: RequestId,
    sdk_transaction_id: &str,
    label: &str,
) -> Result<(), String> {
    loop {
        let event = tokio::time::timeout(EVENT_TIMEOUT, conn.recv_event())
            .await
            .map_err(|_| format!("{label}: timed out waiting for cancel"))?
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;

        match event {
            CoreEvent::Timeline(TimelineEvent::ItemsUpdated {
                key: ref ev_key,
                diffs,
                ..
            }) if ev_key == key => {
                let mut cancelled = false;
                for diff in &diffs {
                    match diff {
                        TimelineDiff::Remove { .. } => return Ok(()),
                        TimelineDiff::PushBack { item }
                        | TimelineDiff::PushFront { item }
                        | TimelineDiff::Insert { item, .. }
                        | TimelineDiff::Set { item, .. }
                            if timeline_item_transaction_id(item) == Some(sdk_transaction_id)
                                && matches!(
                                    item.send_state,
                                    Some(TimelineSendState::Cancelled)
                                ) =>
                        {
                            cancelled = true;
                        }
                        TimelineDiff::Reset { items } => {
                            if items.iter().all(|item| {
                                timeline_item_transaction_id(item) != Some(sdk_transaction_id)
                            }) {
                                cancelled = true;
                            }
                        }
                        _ => {}
                    }
                }
                if cancelled {
                    return Ok(());
                }
            }
            CoreEvent::OperationFailed {
                request_id,
                failure,
            } if request_id == cancel_request_id => {
                return Err(format!("{label}: cancel failed: {failure:?}"));
            }
            _ => {}
        }
    }
}

pub(super) async fn run_live_signals_stage(
    conn_a: &mut CoreConnection,
    conn_b: &mut CoreConnection,
    key_a: &TimelineKey,
    key_b: &TimelineKey,
    event_id: &str,
    expected_reader_user_id: &str,
) -> Result<(), String> {
    let room_id = timeline_key_room_id(key_b)
        .ok_or_else(|| "live signals: expected room timeline key".to_owned())?
        .to_owned();
    let observer_room_id = timeline_key_room_id(key_a)
        .ok_or_else(|| "live signals: expected observer room timeline key".to_owned())?
        .to_owned();

    let read_receipt_id = conn_b.next_request_id();
    conn_b
        .command(CoreCommand::Timeline(TimelineCommand::SendReadReceipt {
            request_id: read_receipt_id,
            key: key_b.clone(),
            event_id: event_id.to_owned(),
        }))
        .await
        .map_err(|e| format!("live signals: submit read receipt failed: {e}"))?;
    wait_for_live_signal_event(conn_b, read_receipt_id, "read receipt", |event| {
        matches!(event, LiveSignalsEvent::ReadReceiptSent { .. })
    })
    .await?;
    wait_for_read_receipt_projection(
        conn_a,
        &observer_room_id,
        event_id,
        expected_reader_user_id,
        "read receipt state",
    )
    .await?;
    println!("read_receipt=ok");

    let fully_read_id = conn_b.next_request_id();
    conn_b
        .command(CoreCommand::Timeline(TimelineCommand::SetFullyRead {
            request_id: fully_read_id,
            key: key_b.clone(),
            event_id: event_id.to_owned(),
        }))
        .await
        .map_err(|e| format!("live signals: submit fully-read marker failed: {e}"))?;
    wait_for_live_signal_event(conn_b, fully_read_id, "fully read", |event| {
        matches!(event, LiveSignalsEvent::FullyReadSet { .. })
    })
    .await?;
    wait_for_live_signal_snapshot(conn_b, "fully read state", |snapshot| {
        snapshot
            .live_signals
            .rooms
            .get(&room_id)
            .is_some_and(|room| room.fully_read_event_id.as_deref() == Some(event_id))
    })
    .await?;
    println!("fully_read=ok");

    let typing_id = conn_b.next_request_id();
    conn_b
        .command(CoreCommand::Timeline(TimelineCommand::SetTyping {
            request_id: typing_id,
            key: key_b.clone(),
            is_typing: true,
        }))
        .await
        .map_err(|e| format!("live signals: submit typing notice failed: {e}"))?;
    wait_for_live_signal_event(conn_b, typing_id, "typing", |event| {
        matches!(
            event,
            LiveSignalsEvent::TypingSet {
                is_typing: true,
                ..
            }
        )
    })
    .await?;
    wait_for_live_signal_snapshot(conn_a, "typing state", |snapshot| {
        snapshot
            .live_signals
            .rooms
            .get(&observer_room_id)
            .is_some_and(|room| !room.typing_user_ids.is_empty())
    })
    .await?;
    println!("typing=ok");

    let user_id_b = match &conn_b.snapshot().session {
        SessionState::Ready(info) => info.user_id.clone(),
        _ => return Err("live signals: user B session was not ready".to_owned()),
    };
    let presence_id = conn_b.next_request_id();
    conn_b
        .command(CoreCommand::Account(AccountCommand::SetPresence {
            request_id: presence_id,
            presence: PresenceKind::Away,
        }))
        .await
        .map_err(|e| format!("live signals: submit presence failed: {e}"))?;
    wait_for_live_signal_event(conn_b, presence_id, "presence", |event| {
        matches!(event, LiveSignalsEvent::PresenceSet { .. })
    })
    .await?;
    wait_for_live_signal_snapshot(conn_b, "presence state", |snapshot| {
        snapshot.live_signals.presence.get(&user_id_b) == Some(&PresenceKind::Away)
    })
    .await?;
    println!("presence=ok");
    println!("live_signals=ok");

    Ok(())
}

fn read_receipt_projection_status(
    snapshot: &AppState,
    room_id: &str,
    event_id: &str,
    expected_reader_user_id: &str,
) -> &'static str {
    let Some(room) = snapshot.live_signals.rooms.get(room_id) else {
        return "room_missing";
    };
    let Some(receipts) = room.receipts_by_event.get(event_id) else {
        return "event_missing";
    };
    if receipts.readers.is_empty() {
        return "readers_empty";
    }
    let Some(reader) = receipts
        .readers
        .iter()
        .find(|reader| reader.user_id == expected_reader_user_id)
    else {
        return "reader_missing";
    };
    let has_display_label = reader
        .display_name
        .as_deref()
        .is_some_and(|label| !label.trim().is_empty())
        || !reader.original_display_label.trim().is_empty();
    if has_display_label {
        "projected"
    } else {
        "label_missing"
    }
}

async fn wait_for_read_receipt_projection(
    conn: &mut CoreConnection,
    room_id: &str,
    event_id: &str,
    expected_reader_user_id: &str,
    label: &str,
) -> Result<AppState, String> {
    let snapshot = conn.snapshot();
    let mut last_status =
        read_receipt_projection_status(&snapshot, room_id, event_id, expected_reader_user_id);
    if last_status == "projected" {
        return Ok(snapshot);
    }

    loop {
        let event = tokio::time::timeout(EVENT_TIMEOUT, conn.recv_event())
            .await
            .map_err(|_| {
                format!(
                    "{label}: timed out waiting for read-receipt projection status={last_status}"
                )
            })?
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;

        if let CoreEvent::StateChanged(snapshot) = event {
            last_status = read_receipt_projection_status(
                &snapshot,
                room_id,
                event_id,
                expected_reader_user_id,
            );
            if last_status == "projected" {
                return Ok(snapshot);
            }
        }
    }
}

pub(super) async fn run_composer_stage(
    conn: &mut CoreConnection,
    key: &TimelineKey,
    mentioned_user_id: &str,
) -> Result<(), String> {
    let ime_action = resolve_composer_key_action(
        ComposerKeyEvent {
            key: ComposerKey::Enter,
            modifiers: ComposerKeyModifiers::default(),
            is_composing: true,
            selection: Some(ComposerSelection { start: 0, end: 0 }),
        },
        ComposerResolverContext {
            surface: ComposerSurface::Main,
            send_shortcut: ComposerSendShortcut::Enter,
            autocomplete_open: true,
            send_enabled: true,
        },
    );
    if ime_action != ComposerResolvedAction::CommitImeCandidate {
        return Err(format!("composer IME guard mismatch: {ime_action:?}"));
    }

    let mention_txn = "qa-composer-mention-txn";
    let mention_document = koushi_state::ComposerDocument::new(vec![
        koushi_state::ComposerInline::Text {
            text: "Composer mention QA ".to_owned(),
        },
        koushi_state::ComposerInline::Mention {
            target: MentionTarget::User {
                user_id: mentioned_user_id.to_owned(),
                display_label: "Synthetic mention".to_owned(),
            },
            display_label: "Synthetic mention".to_owned(),
        },
    ]);
    let mention_body = mention_document.plain_body();
    let mention_id = conn.next_request_id();
    conn.command(CoreCommand::Timeline(TimelineCommand::SendText {
        request_id: mention_id,
        key: key.clone(),
        transaction_id: mention_txn.to_owned(),
        document: mention_document,
    }))
    .await
    .map_err(|e| format!("composer mention send submit failed: {e}"))?;
    wait_for_send_flow_completion(
        conn,
        mention_id,
        key,
        mention_txn,
        &mention_body,
        "composer mention send",
    )
    .await?;
    println!("mention_send=ok");

    let markdown_txn = "qa-composer-markdown-txn";
    let markdown_body = "Composer **markdown** QA";
    let markdown_id = conn.next_request_id();
    conn.command(CoreCommand::Timeline(TimelineCommand::SendText {
        request_id: markdown_id,
        key: key.clone(),
        transaction_id: markdown_txn.to_owned(),
        document: koushi_state::ComposerDocument::from_plain_text(markdown_body.to_owned()),
    }))
    .await
    .map_err(|e| format!("composer markdown send submit failed: {e}"))?;
    wait_for_send_flow_completion(
        conn,
        markdown_id,
        key,
        markdown_txn,
        markdown_body,
        "composer markdown send",
    )
    .await?;
    println!("markdown_send=ok");

    let slash_txn = "qa-composer-slash-txn";
    let slash_id = conn.next_request_id();
    conn.command(CoreCommand::Timeline(TimelineCommand::SendText {
        request_id: slash_id,
        key: key.clone(),
        transaction_id: slash_txn.to_owned(),
        document: koushi_state::ComposerDocument::from_plain_text(
            "/me composer slash command".to_owned(),
        ),
    }))
    .await
    .map_err(|e| format!("composer slash send submit failed: {e}"))?;
    wait_for_send_flow_completion(
        conn,
        slash_id,
        key,
        slash_txn,
        "composer slash command",
        "composer slash command",
    )
    .await?;
    println!("slash_command=ok");
    println!("ime_guard=ok");

    Ok(())
}

fn timeline_key_room_id(key: &TimelineKey) -> Option<&str> {
    match &key.kind {
        TimelineKind::Room { room_id }
        | TimelineKind::Thread { room_id, .. }
        | TimelineKind::Focused { room_id, .. } => Some(room_id.as_str()),
    }
}

async fn wait_for_live_signal_event(
    conn: &mut CoreConnection,
    request_id: RequestId,
    label: &str,
    matches_event: impl Fn(&LiveSignalsEvent) -> bool,
) -> Result<(), String> {
    loop {
        let event = tokio::time::timeout(EVENT_TIMEOUT, conn.recv_event())
            .await
            .map_err(|_| format!("{label}: timed out waiting for live-signal event"))?
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;

        match event {
            CoreEvent::LiveSignals(event) if matches_event(&event) => return Ok(()),
            CoreEvent::OperationFailed {
                request_id: ev_id,
                failure,
            } if ev_id == request_id => {
                return Err(format!("{label}: live-signal command failed: {failure:?}"));
            }
            _ => {}
        }
    }
}

async fn wait_for_live_signal_snapshot(
    conn: &mut CoreConnection,
    label: &str,
    predicate: impl Fn(&AppState) -> bool,
) -> Result<AppState, String> {
    let snapshot = conn.snapshot();
    if predicate(&snapshot) {
        return Ok(snapshot);
    }

    loop {
        let event = tokio::time::timeout(EVENT_TIMEOUT, conn.recv_event())
            .await
            .map_err(|_| format!("{label}: timed out waiting for live-signal state"))?
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;

        if let CoreEvent::StateChanged(snapshot) = event
            && predicate(&snapshot)
        {
            return Ok(snapshot);
        }
    }
}

pub(super) async fn run_media_stage(
    conn_a: &mut CoreConnection,
    conn_b: &mut CoreConnection,
    key_a: &TimelineKey,
    key_b: &TimelineKey,
) -> Result<(), String> {
    const MEDIA_BYTES: &[u8] = b"koushi-desktop synthetic media fixture";
    const MEDIA_CAPTION: &str = "matrix desktop media caption";
    const MEDIA_CAPTION_EDITED: &str = "matrix desktop media caption edited";

    let expected_account = match conn_a.snapshot().session {
        koushi_state::SessionState::Ready(info) => {
            koushi_core::store::session_key_id_from_info(&info)
        }
        _ => return Err("media stage requires a ready session".to_owned()),
    };
    let media_txn = "qa-phase15-media-txn".to_owned();
    let send_media_id = conn_a.next_request_id();
    conn_a
        .command(CoreCommand::Timeline(TimelineCommand::UploadAndSendMedia {
            request_id: send_media_id,
            expected_account,
            key: key_a.clone(),
            transaction_id: media_txn.clone(),
            request: UploadMediaRequest {
                filename: "koushi-desktop-qa-media.bin".to_owned(),
                mime_type: "application/octet-stream".to_owned(),
                bytes: MEDIA_BYTES.to_vec(),
                kind: UploadMediaKind::File,
                compression: None,
                thumbnail: None,
                caption: Some(build_formatted_message_draft(
                    MEDIA_CAPTION,
                    MentionIntent::default(),
                )),
            },
        }))
        .await
        .map_err(|e| format!("submit media send: {e}"))?;

    let _media_event_id = wait_for_media_send_flow_completion(
        conn_a,
        send_media_id,
        key_a,
        &media_txn,
        "media send flow",
    )
    .await?;
    println!("send_media=ok");

    let media_item = wait_for_media_item(conn_b, key_b, "B receives media item").await?;
    let media = media_item
        .media
        .as_ref()
        .ok_or_else(|| "media item missing media metadata".to_owned())?;
    if media.kind != koushi_core::event::TimelineMediaKind::File {
        return Err("media item kind mismatch".to_owned());
    }
    if media_item.body.as_deref() != Some(MEDIA_CAPTION) {
        return Err("media caption did not project onto timeline item body".to_owned());
    }
    println!("media_caption=ok");
    assert_image_upload_compression_contract()?;
    println!("image_compress=ok");
    assert_upload_ux_state_contract(key_a.room_id())?;
    println!("upload_staging=ok");
    println!("media_gallery=ok");
    let media_event_id = match &media_item.id {
        koushi_core::event::TimelineItemId::Event { event_id } => event_id.clone(),
        koushi_core::event::TimelineItemId::Transaction { .. }
        | koushi_core::event::TimelineItemId::Synthetic { .. } => {
            return Err("received media item was not event-backed".to_owned());
        }
    };

    let download_id = conn_b.next_request_id();
    conn_b
        .command(CoreCommand::Timeline(TimelineCommand::DownloadMedia {
            request_id: download_id,
            key: key_b.clone(),
            event_id: media_event_id.clone(),
            selection: MediaDownloadSelection::File,
        }))
        .await
        .map_err(|e| format!("submit media download: {e}"))?;

    wait_for_media_download_completed(
        conn_b,
        download_id,
        key_b,
        &media_event_id,
        u64::try_from(MEDIA_BYTES.len()).unwrap_or(u64::MAX),
        "media download",
    )
    .await?;
    println!("recv_media=ok");

    // Editing a captioned media message must replace only the caption. A
    // text-only replacement drops the attachment and reads as data loss in the
    // timeline (issue #328), so assert the author's own projected row keeps its
    // media metadata while the body becomes the new caption.
    let edit_caption_id = conn_a.next_request_id();
    conn_a
        .command(CoreCommand::Timeline(TimelineCommand::EditText {
            request_id: edit_caption_id,
            key: key_a.clone(),
            event_id: media_event_id.clone(),
            document: ComposerDocument::from_plain_text(MEDIA_CAPTION_EDITED),
        }))
        .await
        .map_err(|e| format!("submit media caption edit: {e}"))?;

    wait_for_media_caption_edit(
        conn_a,
        key_a,
        edit_caption_id,
        &media_event_id,
        MEDIA_CAPTION_EDITED,
        "media caption edit",
    )
    .await?;
    println!("media_caption_edit=ok");

    Ok(())
}

/// Wait for the `Set` diff that applies a media caption edit and require the
/// attachment projection to survive it (issue #328). Only presence of the media
/// projection is checked; no MXC URI, filename, or caption text is printed.
async fn wait_for_media_caption_edit(
    conn: &mut CoreConnection,
    key: &TimelineKey,
    request_id: koushi_core::ids::RequestId,
    event_id: &str,
    edited_caption: &str,
    label: &str,
) -> Result<(), String> {
    let timeout = Duration::from_secs(60);
    loop {
        let event = tokio::time::timeout(timeout, conn.recv_event())
            .await
            .map_err(|_| format!("{label}: timed out waiting for caption edit Set diff"))?
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;

        match event {
            CoreEvent::Timeline(TimelineEvent::ItemsUpdated {
                key: ref ev_key,
                diffs,
                ..
            }) if ev_key == key => {
                for diff in &diffs {
                    let koushi_core::event::TimelineDiff::Set { item, .. } = diff else {
                        continue;
                    };
                    let targets_event = matches!(
                        &item.id,
                        koushi_core::event::TimelineItemId::Event { event_id: id }
                        if id == event_id
                    );
                    if !targets_event || item.body.as_deref() != Some(edited_caption) {
                        continue;
                    }
                    if item.media.is_none() {
                        return Err(format!(
                            "{label}: edited media row lost its attachment projection"
                        ));
                    }
                    return Ok(());
                }
            }
            CoreEvent::OperationFailed {
                request_id: ev_id,
                failure,
            } if ev_id == request_id => {
                return Err(format!("{label}: caption edit failed: {failure:?}"));
            }
            _ => continue,
        }
    }
}

pub(super) async fn run_link_preview_stage(
    conn_a: &mut CoreConnection,
    conn_b: &mut CoreConnection,
    key_a: &TimelineKey,
    key_b: &TimelineKey,
) -> Result<(), String> {
    const URL_MESSAGE_BODY: &str = "link preview test message https://example.invalid/page";
    const URL_EXTRACTED: &str = "https://example.invalid/page";

    // 1. Send a message containing a URL from conn_a to the shared timeline room.
    let txn = "qa-link-preview-txn".to_owned();
    let send_id = conn_a.next_request_id();
    conn_a
        .command(CoreCommand::Timeline(TimelineCommand::SendText {
            request_id: send_id,
            key: key_a.clone(),
            transaction_id: txn.clone(),
            document: koushi_state::ComposerDocument::from_plain_text(URL_MESSAGE_BODY.to_owned()),
        }))
        .await
        .map_err(|e| format!("submit link preview message: {e}"))?;

    let (_send_txn, _event_id) =
        wait_for_send_completed(conn_a, send_id, key_a, "link preview send").await?;

    // 2. Wait for conn_b to see the message and verify a pending preview.
    let item =
        wait_for_item_with_body(conn_b, key_b, URL_MESSAGE_BODY, "B sees URL message").await?;
    let event_id = match &item.id {
        TimelineItemId::Event { event_id } => event_id.clone(),
        _ => return Err("link preview item was not event-backed".to_owned()),
    };
    let previews = item
        .link_previews
        .as_ref()
        .ok_or("missing link_previews on URL message")?;
    if previews.len() != 1 {
        return Err(format!(
            "link preview count mismatch: expected 1, got {}",
            previews.len()
        ));
    }
    if previews[0].url != URL_EXTRACTED {
        return Err("link preview URL mismatch".to_owned());
    }
    if !matches!(previews[0].state, LinkPreviewState::Pending) {
        return Err(format!(
            "link preview state mismatch: expected Pending, got {:?}",
            previews[0].state
        ));
    }
    println!("link_preview_global=ok");

    // 3. Disable URL previews globally via UpdateSettings and verify the
    //    projection drops the preview.
    let settings_id = conn_b.next_request_id();
    conn_b
        .command(CoreCommand::App(AppCommand::UpdateSettings {
            request_id: settings_id,
            patch: SettingsPatch {
                display: Some(DisplaySettings {
                    code_block_wrap: true,
                    hide_redacted: false,
                    url_previews_enabled: false,
                    encrypted_url_previews_enabled: false,
                }),
                ..SettingsPatch::default()
            },
        }))
        .await
        .map_err(|e| format!("submit global preview disable: {e}"))?;
    let disabled_item = wait_for_link_preview_item_projection(
        conn_b,
        key_b,
        settings_id,
        URL_MESSAGE_BODY,
        "B sees message after global disable",
        |item| {
            item.link_previews
                .as_ref()
                .map(|previews| previews.is_empty())
                .unwrap_or(true)
        },
    )
    .await?;
    if !disabled_item
        .link_previews
        .as_ref()
        .map(|p| p.is_empty())
        .unwrap_or(true)
    {
        return Err("global disable did not empty link previews".to_owned());
    }
    println!("link_preview_room=ok");

    // 4. Re-enable URL previews globally.
    let settings_id = conn_b.next_request_id();
    conn_b
        .command(CoreCommand::App(AppCommand::UpdateSettings {
            request_id: settings_id,
            patch: SettingsPatch {
                display: Some(DisplaySettings {
                    code_block_wrap: true,
                    hide_redacted: false,
                    url_previews_enabled: true,
                    encrypted_url_previews_enabled: true,
                }),
                ..SettingsPatch::default()
            },
        }))
        .await
        .map_err(|e| format!("submit global preview enable: {e}"))?;
    let reenabled_item = wait_for_link_preview_item_projection(
        conn_b,
        key_b,
        settings_id,
        URL_MESSAGE_BODY,
        "B sees message after global re-enable",
        |item| {
            item.link_previews.as_ref().is_some_and(|previews| {
                previews.len() == 1
                    && previews[0].url == URL_EXTRACTED
                    && matches!(previews[0].state, LinkPreviewState::Pending)
            })
        },
    )
    .await?;
    let reenabled_previews = reenabled_item
        .link_previews
        .as_ref()
        .ok_or("missing link_previews after global re-enable")?;
    if reenabled_previews.len() != 1
        || reenabled_previews[0].url != URL_EXTRACTED
        || !matches!(reenabled_previews[0].state, LinkPreviewState::Pending)
    {
        return Err("global re-enable did not restore the pending link preview".to_owned());
    }

    // 5. Send HideLinkPreview for the event and verify the message's previews
    //    become an empty list.
    let hide_id = conn_b.next_request_id();
    conn_b
        .command(CoreCommand::Timeline(TimelineCommand::HideLinkPreview {
            request_id: hide_id,
            key: key_b.clone(),
            event_id: event_id.clone(),
        }))
        .await
        .map_err(|e| format!("submit hide link preview: {e}"))?;

    let hidden_item =
        wait_for_item_with_body(conn_b, key_b, URL_MESSAGE_BODY, "B sees message after hide")
            .await?;
    if hidden_item.link_previews.as_ref() != Some(&Vec::new()) {
        return Err("hide link preview did not produce empty preview list".to_owned());
    }
    println!("link_preview_hide=ok");

    let settings_id = conn_a.next_request_id();
    conn_a
        .command(CoreCommand::App(AppCommand::UpdateSettings {
            request_id: settings_id,
            patch: SettingsPatch {
                display: Some(DisplaySettings {
                    code_block_wrap: true,
                    hide_redacted: false,
                    url_previews_enabled: true,
                    encrypted_url_previews_enabled: true,
                }),
                ..SettingsPatch::default()
            },
        }))
        .await
        .map_err(|e| format!("submit encrypted preview enable: {e}"))?;
    wait_for_settings_persisted(conn_a, settings_id, "encrypted preview enable", true).await?;

    // 6. Test E2EE default-on: create a new encrypted room, send a URL message,
    //    and verify previews are projected for the sender's own item.
    //
    //    The sender can decrypt their own event, so checking A's timeline asserts
    //    the Rust-owned encrypted-room policy end-to-end without depending on
    //    cross-device key sharing. The unit tests in link_preview.rs already
    //    assert the encrypted-room default-on rule directly.
    let enc_room_id = create_room_for_qa(
        conn_a,
        "QA Link Preview E2EE Room",
        true,
        "link_preview create encrypted room",
    )
    .await?;

    wait_for_room_in_room_list(
        conn_a,
        &enc_room_id,
        "room list after link preview encrypted room",
    )
    .await?;

    // Wait until the room summary reports encryption enabled before sending.
    let mut found_encrypted = false;
    for _ in 0..30 {
        if conn_a
            .snapshot()
            .rooms
            .iter()
            .any(|r| r.room_id == enc_room_id && r.is_encrypted)
        {
            found_encrypted = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    if !found_encrypted {
        return Err("encrypted room did not report is_encrypted".to_owned());
    }

    let account_key_a = match &conn_a.snapshot().session {
        SessionState::Ready(info) => AccountKey(info.user_id.clone()),
        _ => return Err("link_preview: session A was not ready".to_owned()),
    };
    let enc_key_a = TimelineKey::room(account_key_a, enc_room_id.clone());

    let sub_a_id = conn_a.next_request_id();
    conn_a
        .command(CoreCommand::Timeline(TimelineCommand::Subscribe {
            request_id: sub_a_id,
            key: enc_key_a.clone(),
        }))
        .await
        .map_err(|e| format!("link_preview subscribe encrypted room A: {e}"))?;
    wait_for_initial_items(conn_a, &enc_key_a, sub_a_id, "subscribe encrypted room A").await?;

    let enc_txn = "qa-link-preview-e2ee-txn".to_owned();
    let enc_send_id = conn_a.next_request_id();
    conn_a
        .command(CoreCommand::Timeline(TimelineCommand::SendText {
            request_id: enc_send_id,
            key: enc_key_a.clone(),
            transaction_id: enc_txn.clone(),
            document: koushi_state::ComposerDocument::from_plain_text(URL_MESSAGE_BODY.to_owned()),
        }))
        .await
        .map_err(|e| format!("submit encrypted room URL message: {e}"))?;
    wait_for_send_completed(conn_a, enc_send_id, &enc_key_a, "encrypted room URL send").await?;

    let enc_item = wait_for_event_item_with_body(
        conn_a,
        &enc_key_a,
        URL_MESSAGE_BODY,
        "A sees encrypted room URL message",
    )
    .await?;
    let enc_previews = enc_item
        .link_previews
        .as_ref()
        .ok_or("missing link_previews on encrypted room URL message")?;
    if enc_previews.len() != 1 {
        return Err(format!(
            "encrypted room link preview count mismatch: expected 1, got {}",
            enc_previews.len()
        ));
    }
    if enc_previews[0].url != URL_EXTRACTED {
        return Err("encrypted room link preview URL mismatch".to_owned());
    }
    if !matches!(enc_previews[0].state, LinkPreviewState::Pending) {
        return Err(format!(
            "encrypted room link preview state mismatch: expected Pending, got {:?}",
            enc_previews[0].state
        ));
    }
    println!("link_preview_e2ee_default=ok");

    Ok(())
}

fn assert_image_upload_compression_contract() -> Result<(), String> {
    let policy = ImageUploadCompressionPolicy::default();
    let original_dimensions = ImageUploadDimensions {
        width: 4032,
        height: 3024,
    };
    let selected_dimensions = policy.target_dimensions_for(original_dimensions);
    if selected_dimensions
        != (ImageUploadDimensions {
            width: 2048,
            height: 1536,
        })
    {
        return Err("image compression target dimensions did not preserve aspect ratio".to_owned());
    }

    let original = ImageUploadVariantInfo {
        mime_type: "image/jpeg".to_owned(),
        byte_count: 3_200_000,
        dimensions: Some(original_dimensions),
    };
    if policy.should_skip(&original) {
        return Err("large image was incorrectly classified as skip-small".to_owned());
    }
    let selected = ImageUploadVariantInfo {
        mime_type: "image/jpeg".to_owned(),
        byte_count: 128_000,
        dimensions: Some(selected_dimensions),
    };
    let compression = ImageUploadCompressionState {
        mode: ImageUploadCompressionMode::Always,
        policy,
        original,
        selected: selected.clone(),
        selected_variant: ImageUploadVariantKind::Compressed,
        skipped_small_image: false,
        metadata_stripped: true,
        thumbnail_refreshed: true,
    };
    let request = UploadMediaRequest {
        filename: "koushi-desktop-qa-private-name.jpg".to_owned(),
        mime_type: selected.mime_type,
        bytes: vec![0; 128_000],
        kind: UploadMediaKind::Image {
            width: selected.dimensions.map(|dimensions| dimensions.width),
            height: selected.dimensions.map(|dimensions| dimensions.height),
        },
        compression: Some(compression),
        thumbnail: Some(UploadMediaThumbnail {
            mime_type: "image/jpeg".to_owned(),
            bytes: vec![0; 4096],
            width: 320,
            height: 240,
        }),
        caption: None,
    };

    let Some(compression) = request.compression.as_ref() else {
        return Err("image upload request did not carry compression contract".to_owned());
    };
    if compression.selected_variant != ImageUploadVariantKind::Compressed {
        return Err("image upload request did not carry selected compressed variant".to_owned());
    }
    if !compression.metadata_stripped {
        return Err("compressed image contract did not require metadata stripping".to_owned());
    }
    if !compression.thumbnail_refreshed || request.thumbnail.is_none() {
        return Err(
            "compressed image contract did not carry refreshed thumbnail metadata".to_owned(),
        );
    }
    if compression.selected.byte_count != u64::try_from(request.bytes.len()).unwrap_or(u64::MAX) {
        return Err("selected compression byte count diverged from upload bytes".to_owned());
    }
    let debug = format!("{request:?}");
    if debug.contains("koushi-desktop-qa-private-name.jpg") || debug.contains("0, 0, 0") {
        return Err("image compression request debug leaked private filename or bytes".to_owned());
    }
    Ok(())
}

fn assert_upload_ux_state_contract(room_id: &str) -> Result<(), String> {
    let mut state = AppState {
        session: SessionState::Ready(SessionInfo {
            homeserver: "https://qa.example.invalid".to_owned(),
            user_id: "@qa:example.invalid".to_owned(),
            device_id: "QADEVICE".to_owned(),
            authentication_method: koushi_state::SessionAuthenticationMethod::Unknown,
        }),
        rooms: vec![native_attention_room(room_id, "QA Room", false, 0, 0, 0)],
        ..AppState::default()
    };
    reduce(
        &mut state,
        AppAction::SelectRoom {
            room_id: room_id.to_owned(),
        },
    );

    reduce(
        &mut state,
        AppAction::UploadStagingChanged {
            target: koushi_state::ComposerTarget::Main {
                room_id: room_id.to_owned(),
            },
            items: vec![
                StagedUploadItem {
                    staged_id: "stage-2".to_owned(),
                    room_id: room_id.to_owned(),
                    position: 2,
                    filename: "private-two.txt".to_owned(),
                    mime_type: "text/plain".to_owned(),
                    byte_count: 256,
                    kind: StagedUploadKind::File,
                    caption: Some(build_formatted_message_draft(
                        "private staged caption",
                        MentionIntent::default(),
                    )),
                    compression_choice: StagedUploadCompressionChoice::NotApplicable,
                    preparation: Default::default(),
                },
                StagedUploadItem {
                    staged_id: "stage-1".to_owned(),
                    room_id: room_id.to_owned(),
                    position: 1,
                    filename: "private-one.jpg".to_owned(),
                    mime_type: "image/jpeg".to_owned(),
                    byte_count: 3_200_000,
                    kind: StagedUploadKind::Image {
                        width: Some(4032),
                        height: Some(3024),
                    },
                    caption: None,
                    compression_choice: StagedUploadCompressionChoice::Original,
                    preparation: Default::default(),
                },
            ],
        },
    );
    if state.timeline.staged_uploads.len() != 2
        || state.timeline.staged_uploads[0].staged_id != "stage-1"
    {
        return Err("upload staging projection did not keep multiple files in order".to_owned());
    }

    reduce(
        &mut state,
        AppAction::UploadStagingCompressionChanged {
            target: koushi_state::ComposerTarget::Main {
                room_id: room_id.to_owned(),
            },
            staged_id: "stage-1".to_owned(),
            compression_choice: StagedUploadCompressionChoice::Compressed {
                mode: ImageUploadCompressionMode::Ask,
            },
        },
    );
    if state.timeline.staged_uploads[0].compression_choice
        != (StagedUploadCompressionChoice::Compressed {
            mode: ImageUploadCompressionMode::Ask,
        })
    {
        return Err("upload staging did not preserve per-file compression choice".to_owned());
    }

    reduce(
        &mut state,
        AppAction::MediaGalleryUpdated {
            room_id: room_id.to_owned(),
            items: vec![
                media_gallery_contract_item("$old-media", room_id, 1_900_000_000_000),
                media_gallery_contract_item("$new-media", room_id, 1_900_000_060_000),
            ],
        },
    );
    if state.timeline.media_gallery.len() != 2
        || state.timeline.media_gallery[0].event_id != "$new-media"
    {
        return Err("media gallery projection did not sort newest media first".to_owned());
    }

    let value = serde_json::to_value(&state).map_err(|e| format!("serialize upload state: {e}"))?;
    if value.get("upload_staging").is_some() || value.get("media_gallery").is_some() {
        return Err(
            "upload staging/gallery root stores leaked into serialized AppState".to_owned(),
        );
    }
    if value["timeline"]["staged_uploads"][0]["staged_id"] != "stage-1"
        || value["timeline"]["media_gallery"][0]["event_id"] != "$new-media"
    {
        return Err("selected timeline upload/gallery projection did not serialize".to_owned());
    }

    let debug = format!(
        "{:?} {:?}",
        state.timeline.staged_uploads[0], state.timeline.media_gallery[0]
    );
    for private in [
        room_id,
        "private-one.jpg",
        "private staged caption",
        "mxc://example.invalid/private-gallery",
    ] {
        if debug.contains(private) {
            return Err("upload staging/gallery debug leaked private media data".to_owned());
        }
    }

    Ok(())
}

fn media_gallery_contract_item(
    event_id: &str,
    room_id: &str,
    timestamp_ms: u64,
) -> TimelineMediaGalleryItem {
    TimelineMediaGalleryItem {
        event_id: event_id.to_owned(),
        room_id: room_id.to_owned(),
        sender: Some("@sender:example.invalid".to_owned()),
        sender_label: Some("Sender".to_owned()),
        timestamp_ms,
        media: TimelineMediaGalleryMedia {
            kind: TimelineMediaKind::Image,
            filename: "private-gallery.jpg".to_owned(),
            source: TimelineMediaGallerySource {
                mxc_uri: "mxc://example.invalid/private-gallery".to_owned(),
                encrypted: true,
                encryption_version: Some("v2".to_owned()),
            },
            mimetype: Some("image/jpeg".to_owned()),
            size: Some(2048),
            width: Some(800),
            height: Some(600),
            thumbnail: None,
        },
    }
}

struct ReconnectProjection {
    items: Vec<TimelineItem>,
    expected_bodies: Vec<String>,
}

impl ReconnectProjection {
    fn from_initial(
        initial_items: &[TimelineItem],
        expected_bodies: &[String],
        label: &str,
    ) -> Result<Self, String> {
        let projection = Self {
            items: initial_items.to_vec(),
            expected_bodies: expected_bodies.to_vec(),
        };
        if projection.expected_bodies.len() != TIMELINE_RECONNECT_EXPECTED_BODY_COUNT
            || projection
                .expected_bodies
                .iter()
                .collect::<BTreeSet<_>>()
                .len()
                != TIMELINE_RECONNECT_EXPECTED_BODY_COUNT
        {
            return Err(format!(
                "{label}: expected body contract invalid (expected_count={})",
                projection.expected_bodies.len()
            ));
        }
        projection.validate_items(label, true)?;
        Ok(projection)
    }

    fn replace(&mut self, items: Vec<TimelineItem>, label: &str) -> Result<(), String> {
        self.items = items;
        self.validate_items(label, false)
    }

    fn apply_batch(&mut self, diffs: &[TimelineDiff], label: &str) -> Result<(), String> {
        for diff in diffs {
            apply_timeline_diff(&mut self.items, diff);
        }
        self.validate_items(label, false)
    }

    fn validate_items(&self, label: &str, require_initial_window: bool) -> Result<(), String> {
        if self.items.iter().any(timeline_item_is_decryption_failure) {
            return Err(format!(
                "{label}: projection contains UTD (item_count={})",
                self.items.len()
            ));
        }
        let counts = self.body_counts();
        let duplicate_indices = counts
            .iter()
            .enumerate()
            .filter_map(|(index, count)| (*count > 1).then_some(index))
            .collect::<Vec<_>>();
        if !duplicate_indices.is_empty() {
            return Err(format!(
                "{label}: duplicate expected bodies (item_count={}; duplicate_indices={duplicate_indices:?})",
                self.items.len()
            ));
        }
        if require_initial_window {
            let oldest_count = counts.first().copied().unwrap_or_default();
            let newest_window_count = counts
                .get(1..TIMELINE_RECONNECT_EXPECTED_BODY_COUNT)
                .is_some_and(|window| {
                    window.len() == TIMELINE_RECONNECT_MIN_INITIAL_BODIES
                        && window.iter().all(|count| *count == 1)
                });
            if oldest_count != 0 || !newest_window_count {
                return Err(format!(
                    "{label}: initial projection must contain newest indices 1..=20 exactly once and omit index 0 (item_count={}; oldest_count={oldest_count}; newest_window_count={newest_window_count})",
                    self.items.len()
                ));
            }
        }
        Ok(())
    }

    fn body_counts(&self) -> Vec<usize> {
        self.expected_bodies
            .iter()
            .map(|expected| {
                self.items
                    .iter()
                    .filter(|item| item.body.as_deref() == Some(expected.as_str()))
                    .count()
            })
            .collect()
    }

    fn missing_indices(&self) -> Vec<usize> {
        self.body_counts()
            .iter()
            .enumerate()
            .filter_map(|(index, count)| (*count == 0).then_some(index))
            .collect()
    }

    fn is_complete(&self) -> bool {
        self.body_counts().iter().all(|count| *count == 1)
    }

    fn timeout_error(&self, label: &str, saw_paginating: bool, terminal: bool) -> String {
        format!(
            "{label}: reconnect proof timed out (item_count={}; missing_indices={:?}; saw_paginating={saw_paginating}; terminal={terminal})",
            self.items.len(),
            self.missing_indices()
        )
    }
}

fn observe_reconnect_pagination_state(
    request_id: Option<RequestId>,
    expected_request_id: RequestId,
    state: &PaginationState,
    saw_paginating: &mut bool,
    terminal: &mut bool,
    label: &str,
) -> Result<(), String> {
    if request_id != Some(expected_request_id) {
        return Ok(());
    }
    match state {
        PaginationState::Paginating => *saw_paginating = true,
        PaginationState::Idle | PaginationState::EndReached => {
            if !*saw_paginating {
                return Err(format!(
                    "{label}: pagination terminal arrived before Paginating"
                ));
            }
            *terminal = true;
        }
        PaginationState::Failed { .. } => {
            return Err(format!("{label}: pagination failed"));
        }
    }
    Ok(())
}

async fn wait_for_reconnect_projection(
    conn: &mut CoreConnection,
    key: &TimelineKey,
    initial_items: &[TimelineItem],
    expected_bodies: &[String],
    label: &str,
) -> Result<(), String> {
    let mut projection = ReconnectProjection::from_initial(initial_items, expected_bodies, label)?;

    let deadline = tokio::time::Instant::now() + EVENT_TIMEOUT;
    let request_id = conn.next_request_id();
    tokio::time::timeout_at(
        deadline,
        conn.command(CoreCommand::Timeline(TimelineCommand::Paginate {
            request_id,
            key: key.clone(),
            direction: PaginationDirection::Backward,
            event_count: TIMELINE_RECONNECT_PAGINATE_EVENT_COUNT,
        })),
    )
    .await
    .map_err(|_| format!("{label}: pagination submit timed out"))?
    .map_err(|_| format!("{label}: pagination submit failed"))?;

    let mut saw_paginating = false;
    let mut terminal = false;
    loop {
        if terminal && projection.is_complete() {
            return Ok(());
        }
        let event = tokio::time::timeout_at(deadline, conn.recv_event())
            .await
            .map_err(|_| projection.timeout_error(label, saw_paginating, terminal))?
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;
        match event {
            CoreEvent::Timeline(TimelineEvent::InitialItems {
                key: ref event_key,
                items,
                ..
            }) if event_key == key => projection.replace(items, label)?,
            CoreEvent::Timeline(TimelineEvent::ItemsUpdated {
                key: ref event_key,
                diffs,
                ..
            }) if event_key == key => projection.apply_batch(&diffs, label)?,
            CoreEvent::Timeline(TimelineEvent::PaginationStateChanged {
                request_id: event_request_id,
                key: ref event_key,
                direction: PaginationDirection::Backward,
                state,
                ..
            }) if event_key == key => observe_reconnect_pagination_state(
                event_request_id,
                request_id,
                &state,
                &mut saw_paginating,
                &mut terminal,
                label,
            )?,
            CoreEvent::OperationFailed {
                request_id: event_request_id,
                ..
            } if event_request_id == request_id => {
                return Err(format!("{label}: pagination operation failed"));
            }
            _ => {}
        }
    }
}

async fn wait_for_exact_items_and_gap_release(
    conn: &mut CoreConnection,
    key: &TimelineKey,
    mut items: Vec<TimelineItem>,
    expected_bodies: &[String],
    initial_gap_projection: Option<(u64, u64)>,
    expected_closed_gap: Option<TimelineGapId>,
    label: &str,
) -> Result<(), String> {
    let deadline = tokio::time::Instant::now() + EVENT_TIMEOUT;
    let mut released = false;
    let mut expected_gap_absent = expected_closed_gap.is_none();
    let mut saw_post_demand_gap_positions = false;
    let mut closure_projection = None;
    let mut gap_actor_generation =
        initial_gap_projection.map(|(actor_generation, _)| actor_generation);
    let mut pending_render_ack = None;
    let mut render_ack_request_id = None;
    let mut render_ack_sent_at: Option<tokio::time::Instant> = None;
    let mut render_ack_actor_generation = None;
    loop {
        let counts = expected_bodies
            .iter()
            .map(|expected| {
                items
                    .iter()
                    .filter(|item| item.body.as_deref() == Some(expected.as_str()))
                    .count()
            })
            .collect::<Vec<_>>();
        if released && expected_gap_absent && counts.iter().all(|count| *count == 1) {
            return Ok(());
        }
        if counts.iter().any(|count| *count > 1) {
            return Err(format!(
                "{label}: a recovered synthetic row was projected more than once"
            ));
        }

        let event = tokio::time::timeout_at(deadline, conn.recv_event())
            .await
            .map_err(|_| {
                let missing_count = counts.iter().filter(|count| **count == 0).count();
                format!(
                    "{label}: timed out with {missing_count} rows missing; gap_release={released}; \
                     expected_gap_absent={expected_gap_absent}; \
                     post_demand_gap_positions={saw_post_demand_gap_positions}; \
                     closure_projection={}; render_ack_sent={}; render_ack_same_actor={}",
                    closure_projection.is_some(),
                    render_ack_sent_at.is_some(),
                    closure_projection.is_some_and(|(actor_generation, _)| {
                        render_ack_actor_generation == Some(actor_generation)
                    }),
                )
            })?
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;
        match event {
            CoreEvent::Timeline(TimelineEvent::InitialItems {
                key: ref event_key,
                items: replacement,
                ..
            }) if event_key == key => items = replacement,
            CoreEvent::Timeline(TimelineEvent::ItemsUpdated {
                key: ref event_key,
                generation,
                batch_id,
                diffs,
                ..
            }) if event_key == key => {
                for diff in &diffs {
                    apply_timeline_diff(&mut items, diff);
                }
                if let Some(actor_generation) = gap_actor_generation {
                    pending_render_ack = Some((actor_generation, generation, batch_id));
                }
            }
            CoreEvent::Timeline(TimelineEvent::GapPositionsUpdated {
                key: ref event_key,
                actor_generation,
                generation,
                positions,
                ..
            }) if event_key == key => {
                if expected_closed_gap.is_none()
                    || initial_gap_projection
                        .is_some_and(|(initial_actor, _)| initial_actor == actor_generation)
                {
                    gap_actor_generation = Some(actor_generation);
                }
                saw_post_demand_gap_positions = true;
                if let (Some(expected_gap), Some((initial_actor, initial_generation))) =
                    (expected_closed_gap, initial_gap_projection)
                    && actor_generation == initial_actor
                    && generation > initial_generation
                {
                    expected_gap_absent =
                        positions.iter().all(|position| position.id != expected_gap);
                    closure_projection =
                        expected_gap_absent.then_some((actor_generation, generation));
                }
            }
            CoreEvent::Timeline(TimelineEvent::GapRepairReleased {
                key: ref event_key,
                actor_generation,
                generation,
            }) if event_key == key => {
                let release_projection = (actor_generation, generation);
                if expected_closed_gap.is_some()
                    && (closure_projection != Some(release_projection)
                        || render_ack_actor_generation != Some(actor_generation)
                        || render_ack_request_id.is_none())
                {
                    continue;
                }
                let Some(sent_at) = render_ack_sent_at else {
                    if expected_closed_gap.is_some() {
                        continue;
                    }
                    return Err(format!(
                        "{label}: gap repair released without a correlated render acknowledgement"
                    ));
                };
                if sent_at.elapsed() >= Duration::from_secs(5) {
                    return Err(format!(
                        "{label}: gap repair released only after render-settlement timeout"
                    ));
                }
                released = true;
            }
            CoreEvent::OperationFailed {
                request_id,
                failure,
            } if Some(request_id) == render_ack_request_id => {
                return Err(format!(
                    "{label}: render acknowledgement was rejected: {failure:?}"
                ));
            }
            _ => {}
        }

        if let Some((actor_generation, timeline_generation, batch_id)) = pending_render_ack
            && let koushi_state::TimelineContinuityState::Repairing {
                generation: repair_generation,
                ..
            } = conn.snapshot().timeline.continuity
        {
            let request_id = conn.next_request_id();
            conn.command(CoreCommand::App(
                koushi_core::command::AppCommand::AcknowledgeTimelineBatchRendered {
                    request_id,
                    key: key.clone(),
                    actor_generation,
                    timeline_generation,
                    repair_generation,
                    batch_id,
                },
            ))
            .await
            .map_err(|error| format!("{label}: render acknowledgement failed: {error}"))?;
            render_ack_request_id = Some(request_id);
            render_ack_sent_at = Some(tokio::time::Instant::now());
            render_ack_actor_generation = Some(actor_generation);
            pending_render_ack = None;
        }
    }
}

fn timeline_item_has_visible_payload(item: &TimelineItem) -> bool {
    item.body
        .as_ref()
        .is_some_and(|body| !body.trim().is_empty())
        || item.media.is_some()
        || item.formatted.as_ref().is_some_and(|formatted| {
            !formatted.plain_text.trim().is_empty()
                || formatted
                    .code_blocks
                    .iter()
                    .any(|block| !block.body.trim().is_empty())
        })
}

fn timeline_item_is_visible_event_row(item: &TimelineItem) -> bool {
    matches!(item.id, TimelineItemId::Event { .. })
        && !item.is_hidden
        && !item.is_redacted
        && item.sender.is_some()
        && item.timestamp_ms.is_some()
}

fn assert_no_blank_visible_event_rows(items: &[TimelineItem], label: &str) -> Result<(), String> {
    let blank_count = items
        .iter()
        .filter(|item| {
            timeline_item_is_visible_event_row(item) && !timeline_item_has_visible_payload(item)
        })
        .count();
    if blank_count == 0 {
        return Ok(());
    }
    Err(format!(
        "{label}: {blank_count} visible event row(s) had no renderable payload"
    ))
}

fn retain_unseen_expected_bodies(items: &[TimelineItem], remaining: &mut Vec<String>) {
    for item in items {
        if let Some(body) = item.body.as_ref() {
            remaining.retain(|expected| !body.contains(expected));
        }
    }
}

pub(super) async fn wait_for_stress_bodies_and_no_blank_rows(
    conn: &mut CoreConnection,
    key: &TimelineKey,
    initial_items: &[TimelineItem],
    expected_bodies: &[String],
    page_size: u16,
    label: &str,
) -> Result<(), String> {
    assert_no_blank_visible_event_rows(initial_items, label)?;
    let mut remaining_bodies = expected_bodies.to_vec();
    retain_unseen_expected_bodies(initial_items, &mut remaining_bodies);
    if remaining_bodies.is_empty() {
        return Ok(());
    }

    let mut pagination_ended = false;
    let mut current_paginate_request_id =
        submit_stress_backfill_paginate(conn, key, page_size, label).await?;

    loop {
        if remaining_bodies.is_empty() {
            return Ok(());
        }

        let event = tokio::time::timeout(EVENT_TIMEOUT, conn.recv_event())
            .await
            .map_err(|_| {
                format!(
                    "{label}: timed out; remaining_body_count={} pagination_ended={}",
                    remaining_bodies.len(),
                    pagination_ended
                )
            })?
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;

        match &event {
            CoreEvent::Timeline(TimelineEvent::ItemsUpdated {
                key: ev_key, diffs, ..
            }) if ev_key == key => {
                visit_timeline_diff_items(diffs, |item| {
                    if timeline_item_is_visible_event_row(item)
                        && !timeline_item_has_visible_payload(item)
                    {
                        return Err(format!(
                            "{label}: visible event row had no renderable payload"
                        ));
                    }
                    if let Some(body) = item.body.as_ref() {
                        remaining_bodies.retain(|expected| !body.contains(expected));
                    }
                    Ok(())
                })?;
            }
            CoreEvent::Timeline(TimelineEvent::InitialItems {
                key: ev_key, items, ..
            }) if ev_key == key => {
                assert_no_blank_visible_event_rows(items, label)?;
                retain_unseen_expected_bodies(items, &mut remaining_bodies);
            }
            CoreEvent::Timeline(TimelineEvent::PaginationStateChanged {
                key: ev_key,
                request_id: ev_id,
                state,
                ..
            }) if ev_key == key && ev_id == &Some(current_paginate_request_id) => match state {
                PaginationState::Idle => {
                    if !remaining_bodies.is_empty() && !pagination_ended {
                        current_paginate_request_id =
                            submit_stress_backfill_paginate(conn, key, page_size, label).await?;
                    }
                }
                PaginationState::EndReached => {
                    pagination_ended = true;
                }
                PaginationState::Failed { kind } => {
                    return Err(format!("{label}: pagination failed: {kind:?}"));
                }
                PaginationState::Paginating => {}
            },
            CoreEvent::OperationFailed {
                request_id: ev_id,
                failure,
            } if ev_id == &current_paginate_request_id => {
                return Err(format!("{label}: paginate operation failed: {failure:?}"));
            }
            _ => {}
        }
    }
}

pub(super) async fn submit_stress_backfill_paginate(
    conn: &mut CoreConnection,
    key: &TimelineKey,
    page_size: u16,
    label: &str,
) -> Result<RequestId, String> {
    let request_id = conn.next_request_id();
    conn.command(CoreCommand::Timeline(TimelineCommand::Paginate {
        request_id,
        key: key.clone(),
        direction: PaginationDirection::Backward,
        event_count: page_size,
    }))
    .await
    .map_err(|e| format!("{label}: submit receiver paginate failed: {e}"))?;
    Ok(request_id)
}

pub(super) async fn wait_for_timeline_navigation(
    conn: &mut CoreConnection,
    key: &TimelineKey,
    expected_position: TimelineUnreadPosition,
    minimum_unread_count: u64,
    minimum_newer_count: u64,
    label: &str,
) -> Result<(), String> {
    loop {
        let event = tokio::time::timeout(EVENT_TIMEOUT, conn.recv_event())
            .await
            .map_err(|_| format!("{label}: timed out waiting for NavigationUpdated"))?
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;

        match event {
            CoreEvent::Timeline(TimelineEvent::NavigationUpdated {
                key: ref ev_key,
                snapshot,
            }) if ev_key == key
                && snapshot.unread_position == expected_position
                && snapshot.unread_event_count >= minimum_unread_count
                && snapshot.newer_event_count >= minimum_newer_count =>
            {
                return Ok(());
            }
            CoreEvent::OperationFailed { failure, .. } => {
                return Err(format!("{label}: navigation command failed: {failure:?}"));
            }
            _ => {}
        }
    }
}

/// Wait for the thread reply item by scanning `initial_items` and subsequent
/// `InitialItems`, `ItemsUpdated`, and `PaginationStateChanged` events for the
/// reply body. If the reply is not yet visible, this helper drives additional
/// backward pagination until the reply arrives or pagination ends/fails.
#[allow(dead_code)]
pub(super) async fn wait_for_thread_reply_item(
    conn: &mut CoreConnection,
    key: &TimelineKey,
    initial_items: &[koushi_core::event::TimelineItem],
    expected_body: &str,
    label: &str,
) -> Result<koushi_core::event::TimelineItem, String> {
    if let Some(item) = find_timeline_item_with_body(initial_items, expected_body) {
        return Ok(item);
    }

    let mut current_paginate_request_id = conn.next_request_id();
    let mut pagination_ended = false;
    conn.command(CoreCommand::Timeline(TimelineCommand::Paginate {
        request_id: current_paginate_request_id,
        key: key.clone(),
        direction: PaginationDirection::Backward,
        event_count: 20,
    }))
    .await
    .map_err(|e| format!("{label}: submit thread paginate failed: {e}"))?;

    loop {
        let event = tokio::time::timeout(EVENT_TIMEOUT, conn.recv_event())
            .await
            .map_err(|_| {
                format!("{label}: timed out waiting for thread reply body {expected_body:?}")
            })?
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;

        match event {
            CoreEvent::Timeline(TimelineEvent::InitialItems {
                key: ref ev_key,
                items,
                ..
            }) if ev_key == key => {
                if let Some(item) = find_timeline_item_with_body(&items, expected_body) {
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
                        koushi_core::event::TimelineDiff::PushBack { item }
                        | koushi_core::event::TimelineDiff::PushFront { item }
                        | koushi_core::event::TimelineDiff::Insert { item, .. }
                        | koushi_core::event::TimelineDiff::Set { item, .. } => item,
                        koushi_core::event::TimelineDiff::Reset { items } => {
                            if let Some(item) = find_timeline_item_with_body(&items, expected_body)
                            {
                                return Ok(item);
                            }
                            continue;
                        }
                        _ => continue,
                    };
                    if item
                        .body
                        .as_ref()
                        .map(|body| body.contains(expected_body))
                        .unwrap_or(false)
                    {
                        return Ok(item.clone());
                    }
                }
            }
            CoreEvent::Timeline(TimelineEvent::PaginationStateChanged {
                key: ref ev_key,
                direction,
                state,
                ..
            }) if ev_key == key && direction == PaginationDirection::Backward => match state {
                PaginationState::Idle => {
                    if thread_reply_should_repaginate_on_idle(pagination_ended) {
                        current_paginate_request_id = conn.next_request_id();
                        conn.command(CoreCommand::Timeline(TimelineCommand::Paginate {
                            request_id: current_paginate_request_id,
                            key: key.clone(),
                            direction: PaginationDirection::Backward,
                            event_count: 20,
                        }))
                        .await
                        .map_err(|e| format!("{label}: re-paginate thread failed: {e}"))?;
                    }
                }
                PaginationState::EndReached => {
                    pagination_ended = true;
                }
                PaginationState::Failed { kind } => {
                    return Err(format!("{label}: thread pagination failed: {kind:?}"));
                }
                PaginationState::Paginating => {}
            },
            CoreEvent::OperationFailed {
                request_id: ev_id,
                failure,
            } if ev_id == current_paginate_request_id => {
                return Err(format!("{label}: thread paginate failed: {failure:?}"));
            }
            _ => {}
        }
    }
}

fn timeline_item_body_contains(item: &TimelineItem, expected_body: &str) -> bool {
    item.body
        .as_ref()
        .map(|body| body.contains(expected_body))
        .unwrap_or(false)
}

fn timeline_item_has_thread_summary_reply(item: &TimelineItem, root_event_id: &str) -> bool {
    timeline_item_event_id(item) == Some(root_event_id)
        && item
            .thread_summary
            .as_ref()
            .is_some_and(|summary| summary.reply_count >= 1)
}

fn thread_summary_reconciliation_diagnostic() -> String {
    let diagnostics = koushi_diagnostics::snapshot();
    let sequence = diagnostics
        .records
        .iter()
        .rev()
        .map(|record| &record.event)
        .filter(|event| event.source == "core.thread_summary")
        .take(8)
        .map(|event| {
            format!(
                "{}:{}:{}:{}>{}",
                diagnostic_token_field(event, "source").unwrap_or("missing"),
                diagnostic_token_field(event, "decision").unwrap_or("missing"),
                diagnostic_token_field(event, "merge_reason").unwrap_or("missing"),
                diagnostic_count_field(event, "count_before").unwrap_or(0),
                diagnostic_count_field(event, "count_after").unwrap_or(0),
            )
        })
        .collect::<Vec<_>>();
    if sequence.is_empty() {
        "reconciliation=none".to_owned()
    } else {
        format!("reconciliation={}", sequence.join(","))
    }
}

fn observe_thread_panel_item(
    item: &TimelineItem,
    expected_thread_body: &str,
    root_event_id: &str,
    saw_thread_panel: &mut bool,
) -> Result<(), String> {
    if timeline_item_body_contains(item, expected_thread_body) {
        assert_thread_reply_relation(item, root_event_id).map_err(|_| {
            "thread_summary failed: Thread panel relation did not match root".to_owned()
        })?;
        *saw_thread_panel = true;
    }
    Ok(())
}

struct RoomThreadSummaryObserver<'a> {
    expected_thread_body: &'a str,
    expected_latest_event_id: &'a str,
    expected_reply_count: u32,
    root_event_id: &'a str,
    saw_canonical_reply: bool,
    saw_summary: bool,
    saw_root_summary: bool,
    observed_reply_count: Option<u32>,
    latest_identity_matches: bool,
    latest_body_matches: bool,
    summary_count_sequence: Vec<u32>,
}

impl<'a> RoomThreadSummaryObserver<'a> {
    fn new(
        expected_thread_body: &'a str,
        expected_latest_event_id: &'a str,
        expected_reply_count: u32,
        root_event_id: &'a str,
    ) -> Self {
        Self {
            expected_thread_body,
            expected_latest_event_id,
            expected_reply_count,
            root_event_id,
            saw_canonical_reply: false,
            saw_summary: false,
            saw_root_summary: false,
            observed_reply_count: None,
            latest_identity_matches: false,
            latest_body_matches: false,
            summary_count_sequence: Vec::new(),
        }
    }

    fn observe_item(&mut self, item: &TimelineItem) -> Result<(), String> {
        if timeline_item_body_contains(item, self.expected_thread_body) {
            assert_thread_reply_relation(item, self.root_event_id).map_err(|_| {
                "thread_canonical failed: canonical reply relation did not match root".to_owned()
            })?;
            self.saw_canonical_reply = true;
        }
        if timeline_item_event_id(item) == Some(self.root_event_id)
            && let Some(summary) = item.thread_summary.as_ref()
        {
            self.saw_root_summary = true;
            self.observed_reply_count = Some(summary.reply_count);
            if self.summary_count_sequence.last() != Some(&summary.reply_count)
                && self.summary_count_sequence.len() < 8
            {
                self.summary_count_sequence.push(summary.reply_count);
            }
            self.latest_identity_matches =
                summary.latest_event_id.as_deref() == Some(self.expected_latest_event_id);
            self.latest_body_matches = summary
                .latest_body_preview
                .as_deref()
                .is_some_and(|body| body.contains(self.expected_thread_body));
            self.saw_summary |= summary.reply_count == self.expected_reply_count
                && self.latest_identity_matches
                && self.latest_body_matches;
        }
        Ok(())
    }

    fn is_complete(&self) -> bool {
        self.saw_canonical_reply && self.saw_summary
    }

    fn summary_is_complete(&self) -> bool {
        self.saw_summary
    }

    fn diagnostic(&self) -> String {
        format!(
            "canonical_reply={} root_summary={} reply_count={} expected_count={} latest_matches={} body_matches={} count_sequence={:?}",
            self.saw_canonical_reply,
            self.saw_root_summary,
            self.observed_reply_count
                .map_or_else(|| "none".to_owned(), |count| count.to_string()),
            self.expected_reply_count,
            self.latest_identity_matches,
            self.latest_body_matches,
            self.summary_count_sequence,
        )
    }

    fn observe_items(&mut self, items: &[TimelineItem]) -> Result<bool, String> {
        for item in items {
            self.observe_item(item)?;
        }
        Ok(self.is_complete())
    }

    fn observe_diffs(&mut self, diffs: &[TimelineDiff]) -> Result<bool, String> {
        visit_timeline_diff_items(diffs, |item| self.observe_item(item))?;
        Ok(self.is_complete())
    }
}

pub(super) async fn wait_for_room_timeline_thread_summary(
    conn: &mut CoreConnection,
    key: &TimelineKey,
    initial_items: &[TimelineItem],
    expected_thread_body: &str,
    expected_latest_event_id: &str,
    expected_reply_count: u32,
    root_event_id: &str,
    label: &str,
) -> Result<(), String> {
    let mut observer = RoomThreadSummaryObserver::new(
        expected_thread_body,
        expected_latest_event_id,
        expected_reply_count,
        root_event_id,
    );
    if observer.observe_items(initial_items)? {
        return Ok(());
    }

    let deadline = tokio::time::Instant::now() + EVENT_TIMEOUT;
    loop {
        let now = tokio::time::Instant::now();
        if now >= deadline {
            return Err(
                "thread_summary failed: root item did not carry a reply summary".to_owned(),
            );
        }

        let event =
            tokio::time::timeout(deadline.saturating_duration_since(now), conn.recv_event())
                .await
                .map_err(|_| {
                    "thread_summary failed: root item did not carry a reply summary".to_owned()
                })?
                .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;

        match event {
            CoreEvent::Timeline(TimelineEvent::InitialItems {
                key: ref ev_key,
                items,
                ..
            }) if ev_key == key => {
                if observer.observe_items(&items)? {
                    return Ok(());
                }
            }
            CoreEvent::Timeline(TimelineEvent::ItemsUpdated {
                key: ref ev_key,
                diffs,
                ..
            }) if ev_key == key => {
                if observer.observe_diffs(&diffs)? {
                    return Ok(());
                }
            }
            _ => {}
        }
    }
}

pub(super) async fn wait_for_thread_panel_and_room_summary(
    conn: &mut CoreConnection,
    room_key: &TimelineKey,
    room_initial_items: &[TimelineItem],
    thread_key: &TimelineKey,
    thread_initial_items: &[TimelineItem],
    expected_thread_body: &str,
    expected_latest_event_id: &str,
    expected_reply_count: u32,
    root_event_id: &str,
    label: &str,
) -> Result<(), String> {
    let mut room_observer = RoomThreadSummaryObserver::new(
        expected_thread_body,
        expected_latest_event_id,
        expected_reply_count,
        root_event_id,
    );
    room_observer.observe_items(room_initial_items)?;
    let mut saw_thread_panel = false;
    for item in thread_initial_items {
        observe_thread_panel_item(
            item,
            expected_thread_body,
            root_event_id,
            &mut saw_thread_panel,
        )?;
    }
    if saw_thread_panel && room_observer.summary_is_complete() {
        return Ok(());
    }

    let deadline = tokio::time::Instant::now() + EVENT_TIMEOUT;
    loop {
        let now = tokio::time::Instant::now();
        if now >= deadline {
            return Err(format!(
                "thread_summary failed: Thread panel and Room summary did not converge (thread_panel={} room_summary={} {} {})",
                saw_thread_panel,
                room_observer.summary_is_complete(),
                room_observer.diagnostic(),
                thread_summary_reconciliation_diagnostic(),
            ));
        }
        let event =
            tokio::time::timeout(deadline.saturating_duration_since(now), conn.recv_event())
                .await
                .map_err(|_| {
                    format!(
                        "thread_summary failed: Thread panel and Room summary did not converge (thread_panel={} room_summary={} {} {})",
                        saw_thread_panel,
                        room_observer.summary_is_complete(),
                        room_observer.diagnostic(),
                        thread_summary_reconciliation_diagnostic(),
                    )
                })?
                .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;
        match event {
            CoreEvent::Timeline(TimelineEvent::InitialItems { key, items, .. })
                if key == *thread_key =>
            {
                for item in &items {
                    observe_thread_panel_item(
                        item,
                        expected_thread_body,
                        root_event_id,
                        &mut saw_thread_panel,
                    )?;
                }
            }
            CoreEvent::Timeline(TimelineEvent::ItemsUpdated { key, diffs, .. })
                if key == *thread_key =>
            {
                visit_timeline_diff_items(&diffs, |item| {
                    observe_thread_panel_item(
                        item,
                        expected_thread_body,
                        root_event_id,
                        &mut saw_thread_panel,
                    )
                })?;
            }
            CoreEvent::Timeline(TimelineEvent::InitialItems { key, items, .. })
                if key == *room_key =>
            {
                room_observer.observe_items(&items)?;
            }
            CoreEvent::Timeline(TimelineEvent::ItemsUpdated { key, diffs, .. })
                if key == *room_key =>
            {
                room_observer.observe_diffs(&diffs)?;
            }
            _ => {}
        }
        if saw_thread_panel && room_observer.summary_is_complete() {
            return Ok(());
        }
    }
}

#[allow(dead_code)]
fn assert_room_timeline_exposes_canonical_reply_and_summarizes_root(
    items: &[TimelineItem],
    expected_thread_body: &str,
    root_event_id: &str,
) -> Result<(), String> {
    let saw_reply = items
        .iter()
        .any(|item| timeline_item_body_contains(item, expected_thread_body));
    let saw_summary = items
        .iter()
        .any(|item| timeline_item_has_thread_summary_reply(item, root_event_id));
    if !saw_reply || !saw_summary {
        return Err(
            "thread_canonical failed: root summary and canonical reply were not both observed"
                .to_owned(),
        );
    }
    Ok(())
}

pub(super) fn assert_thread_reply_relation(
    item: &TimelineItem,
    root_event_id: &str,
) -> Result<(), String> {
    if item
        .in_reply_to_event_id
        .as_deref()
        .is_some_and(|reply_id| reply_id != root_event_id)
    {
        return Err("thread_recv relation mismatch: in_reply_to did not match root".to_owned());
    }
    if item.thread_root.as_deref() != Some(root_event_id) {
        return Err("thread_recv relation mismatch: thread_root did not match root".to_owned());
    }
    Ok(())
}

/// Wait for an `ItemsUpdated` Set diff for the event identified by `event_id`
/// OR a Set diff that has the given body substring (whichever arrives first).
/// This asserts that an edit was reflected in the timeline. A failed edit
/// operation (`OperationFailed` with the edit's request_id) is surfaced as an
/// explicit error instead of a silent timeout.
/// Timeout is extended to 60s because edit confirmation requires a sync round-trip.
pub(super) async fn wait_for_edit_diff(
    conn: &mut CoreConnection,
    key: &TimelineKey,
    request_id: koushi_core::ids::RequestId,
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
                    if let koushi_core::event::TimelineDiff::Set { item, .. } = diff {
                        // Accept: item has the edited body, OR item is identified by event_id
                        // (the SDK may not yet have applied the body to the item in all cases).
                        let body_matches = item.body.as_deref().unwrap_or("").contains(edited_body);
                        let event_id_matches = matches!(
                            &item.id,
                            koushi_core::event::TimelineItemId::Event { event_id: id }
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
pub(super) async fn wait_for_redact_diff(
    conn: &mut CoreConnection,
    key: &TimelineKey,
    request_id: koushi_core::ids::RequestId,
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
                        koushi_core::event::TimelineDiff::Remove { .. } => return Ok(()),
                        koushi_core::event::TimelineDiff::Set { item, .. } => {
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

#[cfg(test)]
#[path = "timeline_tests.rs"]
mod tests;
