#[path = "rooms/address.rs"]
mod address;

use super::event_wait::{
    QaEventDeadline, wait_for_dm_room_in_room_list, wait_for_initial_items,
    wait_for_invite_in_snapshot, wait_for_item_with_body, wait_for_room_created,
    wait_for_room_in_room_list, wait_for_room_joined, wait_for_send_flow_completion,
    wait_for_space_in_space_list, wait_for_user_invited,
};
use super::fixtures::{
    accept_invite_for_qa, assert_room_settings_contains_members, create_room_for_qa,
    create_space_for_qa, invite_user_for_qa, load_room_settings_for_qa,
    start_direct_message_for_qa,
};
use super::registry::{EVENT_TIMEOUT, QaConfig, ROOM_LIST_EVENT_TIMEOUT};
use super::{
    AccountCommand, AccountEvent, AccountKey, AppState, BTreeSet, CoreCommand, CoreConnection,
    CoreEvent, CoreFailure, DirectoryQuery, DirectoryRoomSummary, Duration,
    MentionCandidatesCompleteness, MentionCandidatesTarget, MentionSurface, MentionTarget,
    OperationFailureKind, RequestId, RoomCommand, RoomEvent, RoomFailureKind,
    RoomManagementOperationKind, RoomManagementOperationState, RoomMentionPermission,
    RoomModerationAction, RoomSettingChange, RoomSettingsSnapshot, TimelineCommand, TimelineEvent,
    TimelineItemId, TimelineKey, compose_sidebar,
};

pub(super) async fn run_invites_dm_stage(
    config: &QaConfig,
    conn_a: &mut CoreConnection,
    conn_b: &mut CoreConnection,
) -> Result<(), String> {
    let user_b_full_id = format!("@{}:{}", config.user_b, config.server_name);
    let user_a_full_id = format!("@{}:{}", config.user_a, config.server_name);

    let accept_room_id = create_room_for_qa(
        conn_a,
        "QA Invite Accept Room",
        false,
        "invites_dm create accept room",
    )
    .await?;
    invite_user_for_qa(
        conn_a,
        &accept_room_id,
        &user_b_full_id,
        "invites_dm invite B to room",
    )
    .await?;
    wait_for_invite_in_snapshot(
        conn_b,
        &accept_room_id,
        Some(false),
        "invites_dm wait for room invite",
    )
    .await?;
    println!("invite_recv=ok");

    accept_invite_for_qa(conn_b, &accept_room_id, "invites_dm accept room invite").await?;
    wait_for_room_in_room_list(
        conn_b,
        &accept_room_id,
        "invites_dm room list after room accept",
    )
    .await?;
    let accept_room_settings =
        load_room_settings_for_qa(conn_b, &accept_room_id, "invites_dm accepted room members")
            .await?;
    assert_room_settings_contains_members(
        &accept_room_settings,
        &[user_a_full_id.as_str(), user_b_full_id.as_str()],
        "invites_dm accepted room members",
    )?;

    let accept_space_id = create_space_for_qa(
        conn_a,
        "QA Invite Accept Space",
        "invites_dm create accept space",
    )
    .await?;
    invite_user_for_qa(
        conn_a,
        &accept_space_id,
        &user_b_full_id,
        "invites_dm invite B to space",
    )
    .await?;
    wait_for_invite_in_snapshot(
        conn_b,
        &accept_space_id,
        Some(false),
        "invites_dm wait for space invite",
    )
    .await?;
    accept_invite_for_qa(conn_b, &accept_space_id, "invites_dm accept space invite").await?;
    wait_for_space_in_space_list(
        conn_b,
        &accept_space_id,
        "invites_dm space list after space accept",
    )
    .await?;
    let accept_space_settings = load_room_settings_for_qa(
        conn_b,
        &accept_space_id,
        "invites_dm accepted space members",
    )
    .await?;
    assert_room_settings_contains_members(
        &accept_space_settings,
        &[user_a_full_id.as_str(), user_b_full_id.as_str()],
        "invites_dm accepted space members",
    )?;
    let accept_space_settings_a = load_room_settings_for_qa(
        conn_a,
        &accept_space_id,
        "invites_dm creator observes accepted space member",
    )
    .await?;
    assert_room_settings_contains_members(
        &accept_space_settings_a,
        &[user_a_full_id.as_str(), user_b_full_id.as_str()],
        "invites_dm creator observes accepted space member",
    )?;
    println!("invite_accept=ok");
    println!("member_list=ok");

    let decline_room_id = create_room_for_qa(
        conn_a,
        "QA Invite Decline Room",
        false,
        "invites_dm create decline room",
    )
    .await?;
    invite_user_for_qa(
        conn_a,
        &decline_room_id,
        &user_b_full_id,
        "invites_dm invite B to decline room",
    )
    .await?;
    wait_for_invite_in_snapshot(
        conn_b,
        &decline_room_id,
        Some(false),
        "invites_dm wait for decline invite",
    )
    .await?;
    decline_invite_for_qa(conn_b, &decline_room_id, "invites_dm decline room invite").await?;
    wait_for_invite_absent(
        conn_b,
        &decline_room_id,
        "invites_dm wait for declined invite removal",
    )
    .await?;
    println!("invite_decline=ok");

    let dm_room_id =
        start_direct_message_for_qa(conn_a, &user_b_full_id, "invites_dm start direct message")
            .await?;
    wait_for_dm_room_in_room_list(conn_a, &dm_room_id, "invites_dm A room list after DM start")
        .await?;
    wait_for_invite_in_snapshot(
        conn_b,
        &dm_room_id,
        Some(true),
        "invites_dm wait for DM invite",
    )
    .await?;
    println!("dm_start=ok");

    let user_c_full_id = config.dm_scope_control_user_id()?;
    let control_dm_room_id = start_direct_message_for_qa(
        conn_a,
        &user_c_full_id,
        "invites_dm start control direct message",
    )
    .await?;
    wait_for_dm_room_in_room_list(
        conn_a,
        &control_dm_room_id,
        "invites_dm A room list after control DM start",
    )
    .await?;
    assert_dm_space_scope_for_qa(conn_a, &accept_space_id, &dm_room_id, &control_dm_room_id)
        .await?;
    println!("dm_space_scope=ok");

    Ok(())
}

pub(super) async fn run_directory_stage(
    config: &QaConfig,
    conn_a: &mut CoreConnection,
    conn_b: &mut CoreConnection,
) -> Result<(), String> {
    let directory_room_name = "Koushi Directory QA";
    let alias_localpart = format!("koushi-desktop-directory-qa-{}", std::process::id());
    let expected_alias = format!("#{alias_localpart}:{}", config.server_name);
    let public_room_id = create_public_directory_room_for_qa(
        conn_a,
        directory_room_name,
        &alias_localpart,
        "directory create public room",
    )
    .await?;

    let query = DirectoryQuery {
        term: Some(directory_room_name.to_owned()),
        server_name: Some(config.server_name.clone()),
        limit: Some(10),
        since: None,
    };
    let rooms = query_directory_until_room_visible(
        conn_a,
        query,
        &public_room_id,
        &expected_alias,
        "directory query public room",
    )
    .await?;
    if rooms.is_empty() {
        return Err("directory query unexpectedly returned no rooms".to_owned());
    }
    println!("directory_query=ok");

    join_directory_room_for_qa(
        conn_b,
        &expected_alias,
        &config.server_name,
        &public_room_id,
        "directory B joins public room",
    )
    .await?;
    println!("directory_join=ok");
    address::verify(config, conn_a, conn_b).await?;

    Ok(())
}

pub(super) async fn join_directory_room_for_qa(
    conn_b: &mut CoreConnection,
    expected_alias: &str,
    via_server: &str,
    public_room_id: &str,
    label: &str,
) -> Result<(), String> {
    let join_id = conn_b.next_request_id();
    conn_b
        .command(CoreCommand::Room(RoomCommand::JoinDirectoryRoom {
            request_id: join_id,
            room_id_or_alias: expected_alias.to_owned(),
            via_servers: vec![via_server.to_owned()],
        }))
        .await
        .map_err(|e| format!("{label}: submit join by alias failed: {e}"))?;
    wait_for_room_joined(conn_b, join_id, public_room_id, label).await
}

pub(super) async fn run_room_people_projection_stage(
    config: &QaConfig,
    conn_a: &mut CoreConnection,
    conn_b: &mut CoreConnection,
    account_key_a: &AccountKey,
    account_key_b: &AccountKey,
    room_id: &str,
) -> Result<(), String> {
    let user_a_id = format!("@{}:{}", config.user_a, config.server_name);
    let user_b_id = format!("@{}:{}", config.user_b, config.server_name);
    let user_c_id = config.dm_scope_control_user_id()?;

    let profile_id = conn_a.next_request_id();
    conn_a
        .command(CoreCommand::Account(AccountCommand::SetDisplayName {
            request_id: profile_id,
            display_name: Some("Room People Known".to_owned()),
        }))
        .await
        .map_err(|e| format!("room people: submit display-name update failed: {e}"))?;
    wait_for_profile_updated(conn_a, profile_id, "room people display-name update").await?;

    let main_target = query_mention_candidates(
        conn_a,
        account_key_a,
        room_id,
        MentionSurface::Main,
        "",
        "room people main candidates",
    )
    .await?;
    assert_joined_candidate_scope(&main_target, [&user_a_id, &user_b_id], &user_c_id)?;
    if main_target.room_mention_allowed != RoomMentionPermission::Allowed {
        return Err(
            "room people: room mention permission was not allowed for the room creator".to_owned(),
        );
    }
    if !main_target.candidates.iter().any(|candidate| {
        candidate.user_id == user_a_id
            && candidate.display_label.as_deref() == Some("Room People Known")
    }) {
        return Err("room people: known room display label was not projected".to_owned());
    }
    if !main_target
        .candidates
        .iter()
        .any(|candidate| candidate.user_id == user_b_id)
    {
        return Err(
            "room people: joined member with an optional label was not projected".to_owned(),
        );
    }
    println!("room_people_joined_scope=ok");

    let alias_id = conn_a.next_request_id();
    conn_a
        .command(CoreCommand::Account(AccountCommand::SetLocalUserAlias {
            request_id: alias_id,
            user_id: user_b_id.clone(),
            alias: Some("Room People Personal Alias".to_owned()),
        }))
        .await
        .map_err(|e| format!("room people: submit local alias update failed: {e}"))?;
    wait_for_local_alias(
        conn_a,
        alias_id,
        &user_b_id,
        "Room People Personal Alias",
        "room people local alias update",
    )
    .await?;
    let aliased_target = query_mention_candidates(
        conn_a,
        account_key_a,
        room_id,
        MentionSurface::Main,
        "personal alias",
        "room people alias candidates",
    )
    .await?;
    if aliased_target.candidates.len() != 1
        || aliased_target.candidates[0].user_id != user_b_id
        || aliased_target.candidates[0].display_label.as_deref()
            != Some("Room People Personal Alias")
    {
        return Err(
            "room people: local alias did not drive candidate matching and label".to_owned(),
        );
    }
    println!("room_people_alias_search=ok");

    let _main_target = query_mention_candidates(
        conn_a,
        account_key_a,
        room_id,
        MentionSurface::Main,
        "",
        "room people restored main candidates",
    )
    .await?;
    let thread_target = query_mention_candidates(
        conn_a,
        account_key_a,
        room_id,
        MentionSurface::Thread,
        &config.user_b,
        "room people thread candidates",
    )
    .await?;
    if thread_target.candidates.len() != 1 || thread_target.candidates[0].user_id != user_b_id {
        return Err("room people: thread target did not retain its independent query".to_owned());
    }
    let retained_main = conn_a
        .snapshot()
        .mention_candidates
        .target(room_id, MentionSurface::Main)
        .cloned()
        .ok_or_else(|| "room people: main target disappeared after thread query".to_owned())?;
    assert_joined_candidate_scope(&retained_main, [&user_a_id, &user_b_id], &user_c_id)?;
    println!("room_people_surface_isolation=ok");

    let leave_id = conn_b.next_request_id();
    conn_b
        .command(CoreCommand::Room(RoomCommand::LeaveRoom {
            request_id: leave_id,
            room_id: room_id.to_owned(),
        }))
        .await
        .map_err(|e| format!("room people: submit leave failed: {e}"))?;
    wait_for_room_left(conn_b, leave_id, room_id, "room people member leave").await?;
    wait_for_mention_candidate_ids(
        conn_a,
        room_id,
        MentionSurface::Main,
        [&user_a_id],
        "room people candidates after leave",
    )
    .await?;

    let invite_id = conn_a.next_request_id();
    conn_a
        .command(CoreCommand::Room(RoomCommand::InviteUser {
            request_id: invite_id,
            room_id: room_id.to_owned(),
            user_id: user_b_id.clone(),
        }))
        .await
        .map_err(|e| format!("room people: submit reinvite failed: {e}"))?;
    wait_for_user_invited(
        conn_a,
        invite_id,
        room_id,
        &user_b_id,
        "room people reinvite",
    )
    .await?;
    let rejoin_id = conn_b.next_request_id();
    conn_b
        .command(CoreCommand::Room(RoomCommand::JoinRoom {
            request_id: rejoin_id,
            room_id: room_id.to_owned(),
        }))
        .await
        .map_err(|e| format!("room people: submit rejoin failed: {e}"))?;
    wait_for_room_joined(conn_b, rejoin_id, room_id, "room people member rejoin").await?;
    wait_for_mention_candidate_ids(
        conn_a,
        room_id,
        MentionSurface::Main,
        [&user_a_id, &user_b_id],
        "room people candidates after rejoin",
    )
    .await?;
    println!("room_people_membership_refresh=ok");

    let key = TimelineKey::room(account_key_a.clone(), room_id.to_owned());
    let subscribe_id = conn_a.next_request_id();
    conn_a
        .command(CoreCommand::Timeline(TimelineCommand::Subscribe {
            request_id: subscribe_id,
            key: key.clone(),
            initial_backfill: koushi_protocol::command::InitialBackfillPolicy::Disabled,
        }))
        .await
        .map_err(|e| format!("room people: submit timeline subscribe failed: {e}"))?;
    wait_for_initial_items(conn_a, &key, subscribe_id, "room people timeline subscribe").await?;
    let key_b = TimelineKey::room(account_key_b.clone(), room_id.to_owned());
    let subscribe_b_id = conn_b.next_request_id();
    conn_b
        .command(CoreCommand::Timeline(TimelineCommand::Subscribe {
            request_id: subscribe_b_id,
            key: key_b.clone(),
            initial_backfill: koushi_protocol::command::InitialBackfillPolicy::Disabled,
        }))
        .await
        .map_err(|e| format!("room people: submit receiver timeline subscribe failed: {e}"))?;
    wait_for_initial_items(
        conn_b,
        &key_b,
        subscribe_b_id,
        "room people receiver timeline subscribe",
    )
    .await?;

    let display_label = thread_target.candidates[0]
        .display_label
        .clone()
        .unwrap_or_else(|| "Unknown user".to_owned());
    let document = koushi_state::ComposerDocument::new(vec![
        koushi_state::ComposerInline::Text {
            text: "Room people structured mention QA ".to_owned(),
        },
        koushi_state::ComposerInline::Mention {
            target: MentionTarget::User {
                user_id: user_b_id.clone(),
                display_label: display_label.clone(),
            },
            display_label,
        },
    ]);
    let body = document.plain_body();
    let transaction_id = "qa-room-people-mention";
    let send_id = conn_a.next_request_id();
    conn_a
        .command(CoreCommand::Timeline(TimelineCommand::SendText {
            request_id: send_id,
            key: key.clone(),
            transaction_id: transaction_id.to_owned(),
            document,
        }))
        .await
        .map_err(|e| format!("room people: submit structured mention failed: {e}"))?;
    wait_for_send_flow_completion(
        conn_a,
        send_id,
        &key,
        transaction_id,
        &body,
        "room people structured mention",
    )
    .await?;
    let received = wait_for_item_with_body(
        conn_b,
        &key_b,
        &body,
        "room people receiver structured mention",
    )
    .await?;
    let received_event_id = match received.id {
        TimelineItemId::Event { event_id } => event_id,
        TimelineItemId::Transaction { .. } | TimelineItemId::Synthetic { .. } => {
            return Err("room people: receiver mention did not have a remote event id".to_owned());
        }
    };
    let source_id = conn_b.next_request_id();
    conn_b
        .command(CoreCommand::Timeline(TimelineCommand::LoadMessageSource {
            request_id: source_id,
            key: key_b.clone(),
            event_id: received_event_id,
        }))
        .await
        .map_err(|e| format!("room people: submit message-source load failed: {e}"))?;
    wait_for_structured_mention_source(
        conn_b,
        source_id,
        &user_b_id,
        "room people structured mention source",
    )
    .await?;
    let unsubscribe_id = conn_a.next_request_id();
    conn_a
        .command(CoreCommand::Timeline(TimelineCommand::Unsubscribe {
            request_id: unsubscribe_id,
            key,
        }))
        .await
        .map_err(|e| format!("room people: submit timeline unsubscribe failed: {e}"))?;
    let unsubscribe_b_id = conn_b.next_request_id();
    conn_b
        .command(CoreCommand::Timeline(TimelineCommand::Unsubscribe {
            request_id: unsubscribe_b_id,
            key: key_b,
        }))
        .await
        .map_err(|e| format!("room people: submit receiver timeline unsubscribe failed: {e}"))?;
    println!("room_people_mentions_content=ok");
    println!("room_people_projection=ok");
    Ok(())
}

fn assert_joined_candidate_scope<const N: usize>(
    target: &MentionCandidatesTarget,
    expected_user_ids: [&String; N],
    excluded_user_id: &str,
) -> Result<(), String> {
    if target.completeness != MentionCandidatesCompleteness::Complete {
        return Err("room people: candidate target was not complete".to_owned());
    }
    let actual = target
        .candidates
        .iter()
        .map(|candidate| candidate.user_id.as_str())
        .collect::<BTreeSet<_>>();
    let expected = expected_user_ids
        .into_iter()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    if actual != expected || actual.contains(excluded_user_id) {
        return Err("room people: candidate target did not match joined-room scope".to_owned());
    }
    Ok(())
}

async fn query_mention_candidates(
    conn: &mut CoreConnection,
    account_key: &AccountKey,
    room_id: &str,
    surface: MentionSurface,
    query: &str,
    label: &str,
) -> Result<MentionCandidatesTarget, String> {
    let request_id = conn.next_request_id();
    conn.command(CoreCommand::Room(RoomCommand::QueryMentionCandidates {
        request_id,
        account_key: account_key.clone(),
        room_id: room_id.to_owned(),
        surface,
        query: query.to_owned(),
    }))
    .await
    .map_err(|e| format!("{label}: submit failed: {e}"))?;
    wait_for_mention_target(conn, room_id, surface, Some(request_id), label, |target| {
        target.completeness == MentionCandidatesCompleteness::Complete
    })
    .await
}

async fn wait_for_mention_candidate_ids<const N: usize>(
    conn: &mut CoreConnection,
    room_id: &str,
    surface: MentionSurface,
    expected_user_ids: [&String; N],
    label: &str,
) -> Result<MentionCandidatesTarget, String> {
    let expected = expected_user_ids
        .into_iter()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    wait_for_mention_target(conn, room_id, surface, None, label, |target| {
        target.completeness == MentionCandidatesCompleteness::Complete
            && target
                .candidates
                .iter()
                .map(|candidate| candidate.user_id.as_str())
                .collect::<BTreeSet<_>>()
                == expected
    })
    .await
}

async fn wait_for_mention_target(
    conn: &mut CoreConnection,
    room_id: &str,
    surface: MentionSurface,
    request_id: Option<RequestId>,
    label: &str,
    predicate: impl Fn(&MentionCandidatesTarget) -> bool,
) -> Result<MentionCandidatesTarget, String> {
    let deadline = QaEventDeadline::after(ROOM_LIST_EVENT_TIMEOUT);
    loop {
        if let Some(target) = conn
            .snapshot()
            .mention_candidates
            .target(room_id, surface)
            .filter(|target| {
                request_id.is_none_or(|request_id| target.request_id == request_id.sequence)
                    && predicate(target)
            })
            .cloned()
        {
            return Ok(target);
        }
        let event = deadline
            .recv(conn)
            .await
            .map_err(|_| format!("{label}: timed out waiting for mention candidate projection"))?
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;
        if let CoreEvent::OperationFailed {
            request_id: event_request_id,
            failure,
        } = event
            && request_id == Some(event_request_id)
        {
            return Err(format!(
                "{label}: mention candidate command failed: {failure:?}"
            ));
        }
    }
}

async fn wait_for_profile_updated(
    conn: &mut CoreConnection,
    request_id: RequestId,
    label: &str,
) -> Result<(), String> {
    let deadline = QaEventDeadline::after(EVENT_TIMEOUT);
    loop {
        let event = deadline
            .recv(conn)
            .await
            .map_err(|_| format!("{label}: timed out waiting for profile update"))?
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;
        match event {
            CoreEvent::Account(AccountEvent::ProfileUpdated {
                request_id: event_request_id,
                ..
            }) if event_request_id == request_id => return Ok(()),
            CoreEvent::OperationFailed {
                request_id: event_request_id,
                failure,
            } if event_request_id == request_id => {
                return Err(format!("{label}: profile update failed: {failure:?}"));
            }
            _ => {}
        }
    }
}

async fn wait_for_local_alias(
    conn: &mut CoreConnection,
    request_id: RequestId,
    user_id: &str,
    expected_alias: &str,
    label: &str,
) -> Result<(), String> {
    let deadline = QaEventDeadline::after(EVENT_TIMEOUT);
    loop {
        if conn
            .snapshot()
            .profile
            .local_aliases
            .get(user_id)
            .is_some_and(|alias| alias == expected_alias)
        {
            return Ok(());
        }
        let event = deadline
            .recv(conn)
            .await
            .map_err(|_| format!("{label}: timed out waiting for local alias projection"))?
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;
        if let CoreEvent::OperationFailed {
            request_id: event_request_id,
            failure,
        } = event
            && event_request_id == request_id
        {
            return Err(format!("{label}: local alias update failed: {failure:?}"));
        }
    }
}

async fn wait_for_room_left(
    conn: &mut CoreConnection,
    request_id: RequestId,
    room_id: &str,
    label: &str,
) -> Result<(), String> {
    let deadline = QaEventDeadline::after(EVENT_TIMEOUT);
    loop {
        let event = deadline
            .recv(conn)
            .await
            .map_err(|_| format!("{label}: timed out waiting for room leave"))?
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;
        match event {
            CoreEvent::Room(RoomEvent::RoomLeft {
                request_id: event_request_id,
                room_id: event_room_id,
            }) if event_request_id == request_id => {
                if event_room_id != room_id {
                    return Err(format!("{label}: room leave target mismatch"));
                }
                return Ok(());
            }
            CoreEvent::OperationFailed {
                request_id: event_request_id,
                failure,
            } if event_request_id == request_id => {
                return Err(format!("{label}: room leave failed: {failure:?}"));
            }
            _ => {}
        }
    }
}

async fn wait_for_structured_mention_source(
    conn: &mut CoreConnection,
    request_id: RequestId,
    mentioned_user_id: &str,
    label: &str,
) -> Result<(), String> {
    let deadline = QaEventDeadline::after(EVENT_TIMEOUT);
    loop {
        let event = deadline
            .recv(conn)
            .await
            .map_err(|_| format!("{label}: timed out waiting for message source"))?
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;
        match event {
            CoreEvent::Timeline(TimelineEvent::MessageSourceLoaded {
                request_id: event_request_id,
                source,
                ..
            }) if event_request_id == request_id => {
                let user_ids = source
                    .original_json
                    .as_ref()
                    .and_then(|json| json.pointer("/content/m.mentions/user_ids"))
                    .and_then(serde_json::Value::as_array)
                    .ok_or_else(|| format!("{label}: source lacked structured mention user ids"))?;
                if user_ids.len() != 1 || user_ids[0].as_str() != Some(mentioned_user_id) {
                    return Err(format!("{label}: structured mention user ids mismatch"));
                }
                return Ok(());
            }
            CoreEvent::OperationFailed {
                request_id: event_request_id,
                failure,
            } if event_request_id == request_id => {
                return Err(format!("{label}: message-source load failed: {failure:?}"));
            }
            _ => {}
        }
    }
}

pub(super) async fn run_room_management_stage(
    config: &QaConfig,
    conn_a: &mut CoreConnection,
    conn_b: &mut CoreConnection,
    account_key_a: &AccountKey,
    account_key_b: &AccountKey,
) -> Result<(), String> {
    let room_id = create_room_for_qa(
        conn_a,
        "QA Room Management",
        false,
        "room_management create room",
    )
    .await?;
    wait_for_room_in_room_list(conn_a, &room_id, "room_management A room list").await?;

    let user_b_full_id = format!("@{}:{}", config.user_b, config.server_name);
    invite_user_for_qa(
        conn_a,
        &room_id,
        &user_b_full_id,
        "room_management invite B",
    )
    .await?;
    wait_for_invite_in_snapshot(
        conn_b,
        &room_id,
        Some(false),
        "room_management wait for B invite",
    )
    .await?;

    let join_b_id = conn_b.next_request_id();
    conn_b
        .command(CoreCommand::Room(RoomCommand::JoinRoom {
            request_id: join_b_id,
            room_id: room_id.clone(),
        }))
        .await
        .map_err(|e| format!("room_management: submit B join failed: {e}"))?;
    wait_for_room_joined(conn_b, join_b_id, &room_id, "room_management B joins").await?;
    let settings_a =
        load_room_settings_for_qa(conn_a, &room_id, "room_management load settings A").await?;
    assert_room_settings_contains_members(
        &settings_a,
        &[account_key_a.0.as_str(), account_key_b.0.as_str()],
        "room_management A observes joined members",
    )?;
    if !settings_a.permissions.can_edit_settings || !settings_a.permissions.can_kick {
        return Err("room_management: creator permissions were not projected".to_owned());
    }

    let update_id = conn_a.next_request_id();
    conn_a
        .command(CoreCommand::Room(RoomCommand::UpdateRoomSetting {
            request_id: update_id,
            room_id: room_id.clone(),
            change: RoomSettingChange::Topic(Some("QA room management topic".to_owned())),
        }))
        .await
        .map_err(|e| format!("room_management: submit topic update failed: {e}"))?;
    let updated =
        wait_for_room_setting_updated(conn_a, update_id, "room_management topic update").await?;
    if updated.topic.as_deref() != Some("QA room management topic") {
        return Err("room_management: updated settings snapshot did not carry topic".to_owned());
    }
    println!("room_settings=ok");

    let settings_b =
        load_room_settings_for_qa(conn_b, &room_id, "room_management load settings B").await?;
    assert_room_settings_contains_members(
        &settings_b,
        &[account_key_a.0.as_str(), account_key_b.0.as_str()],
        "room_management B observes joined members",
    )?;
    if settings_b.permissions.can_kick {
        return Err("room_management: normal member unexpectedly has kick permission".to_owned());
    }

    let guard_id = conn_b.next_request_id();
    conn_b
        .command(CoreCommand::Room(RoomCommand::ModerateRoomMember {
            request_id: guard_id,
            room_id: room_id.clone(),
            target_user_id: account_key_a.0.clone(),
            action: RoomModerationAction::Kick,
            reason: None,
        }))
        .await
        .map_err(|e| format!("room_management: submit forbidden moderation failed: {e}"))?;
    wait_for_room_management_forbidden(conn_b, guard_id, "room_management permission guard")
        .await?;
    println!("permission_guard=ok");

    let kick_id = conn_a.next_request_id();
    conn_a
        .command(CoreCommand::Room(RoomCommand::ModerateRoomMember {
            request_id: kick_id,
            room_id,
            target_user_id: account_key_b.0.clone(),
            action: RoomModerationAction::Kick,
            reason: Some("QA moderation".to_owned()),
        }))
        .await
        .map_err(|e| format!("room_management: submit kick failed: {e}"))?;
    wait_for_room_member_moderated(conn_a, kick_id, "room_management member kick").await?;
    println!("moderation=ok");

    Ok(())
}

async fn create_public_directory_room_for_qa(
    conn: &mut CoreConnection,
    name: &str,
    alias_localpart: &str,
    label: &str,
) -> Result<String, String> {
    let request_id = conn.next_request_id();
    conn.command(CoreCommand::Room(RoomCommand::CreatePublicDirectoryRoom {
        request_id,
        name: name.to_owned(),
        alias_localpart: alias_localpart.to_owned(),
    }))
    .await
    .map_err(|e| format!("{label}: submit public directory room create failed: {e}"))?;
    wait_for_room_created(conn, request_id, label).await
}

async fn decline_invite_for_qa(
    conn: &mut CoreConnection,
    room_id: &str,
    label: &str,
) -> Result<(), String> {
    let request_id = conn.next_request_id();
    conn.command(CoreCommand::Room(RoomCommand::DeclineInvite {
        request_id,
        room_id: room_id.to_owned(),
    }))
    .await
    .map_err(|e| format!("{label}: submit decline invite failed: {e}"))?;
    wait_for_invite_declined(conn, request_id, room_id, label).await
}

async fn query_directory_until_room_visible(
    conn: &mut CoreConnection,
    query: DirectoryQuery,
    room_id: &str,
    alias: &str,
    label: &str,
) -> Result<Vec<DirectoryRoomSummary>, String> {
    for attempt in 1..=6 {
        let request_id = conn.next_request_id();
        conn.command(CoreCommand::Room(RoomCommand::QueryDirectory {
            request_id,
            query: query.clone(),
        }))
        .await
        .map_err(|e| format!("{label}: submit directory query failed: {e}"))?;
        let rooms = wait_for_directory_query_completed(conn, request_id, label).await?;
        if rooms
            .iter()
            .any(|room| room.room_id == room_id || room.canonical_alias.as_deref() == Some(alias))
        {
            return Ok(rooms);
        }
        if attempt < 6 {
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    }

    Err(format!(
        "{label}: public directory did not return the created room after bounded retries"
    ))
}

async fn wait_for_directory_query_completed(
    conn: &mut CoreConnection,
    request_id: koushi_protocol::ids::RequestId,
    label: &str,
) -> Result<Vec<DirectoryRoomSummary>, String> {
    loop {
        let event = tokio::time::timeout(EVENT_TIMEOUT, conn.recv_event())
            .await
            .map_err(|_| format!("{label}: timed out waiting for DirectoryQueryCompleted"))?
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;

        match event {
            CoreEvent::Room(RoomEvent::DirectoryQueryCompleted {
                request_id: ev_id,
                rooms,
                ..
            }) if ev_id == request_id => {
                return Ok(rooms);
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

async fn wait_for_room_setting_updated(
    conn: &mut CoreConnection,
    request_id: RequestId,
    label: &str,
) -> Result<RoomSettingsSnapshot, String> {
    loop {
        let event = tokio::time::timeout(EVENT_TIMEOUT, conn.recv_event())
            .await
            .map_err(|_| format!("{label}: timed out waiting for RoomSettingUpdated"))?
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;

        match event {
            CoreEvent::Room(RoomEvent::RoomSettingUpdated {
                request_id: ev_id,
                settings,
            }) if ev_id == request_id => return Ok(settings),
            CoreEvent::OperationFailed {
                request_id: ev_id,
                failure,
            } if ev_id == request_id => return Err(format!("{label} failed: {failure:?}")),
            _ => continue,
        }
    }
}

async fn wait_for_room_member_moderated(
    conn: &mut CoreConnection,
    request_id: RequestId,
    label: &str,
) -> Result<(), String> {
    loop {
        let event = tokio::time::timeout(EVENT_TIMEOUT, conn.recv_event())
            .await
            .map_err(|_| format!("{label}: timed out waiting for RoomMemberModerated"))?
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;

        match event {
            CoreEvent::Room(RoomEvent::RoomMemberModerated {
                request_id: ev_id, ..
            }) if ev_id == request_id => return Ok(()),
            CoreEvent::OperationFailed {
                request_id: ev_id,
                failure,
            } if ev_id == request_id => return Err(format!("{label} failed: {failure:?}")),
            _ => continue,
        }
    }
}

async fn wait_for_room_management_forbidden(
    conn: &mut CoreConnection,
    request_id: RequestId,
    label: &str,
) -> Result<(), String> {
    let deadline = tokio::time::Instant::now() + EVENT_TIMEOUT;
    let mut saw_forbidden_failure = false;

    loop {
        if saw_forbidden_failure && room_management_forbidden_recorded(&conn.snapshot(), request_id)
        {
            return Ok(());
        }

        let event = tokio::time::timeout_at(deadline, conn.recv_event())
            .await
            .map_err(|_| format!("{label}: timed out waiting for forbidden room-management state"))?
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;

        match event {
            CoreEvent::OperationFailed {
                request_id: ev_id,
                failure:
                    CoreFailure::RoomOperationFailed {
                        kind: RoomFailureKind::Forbidden,
                    },
            } if ev_id == request_id => {
                saw_forbidden_failure = true;
            }
            CoreEvent::OperationFailed {
                request_id: ev_id,
                failure,
            } if ev_id == request_id => {
                return Err(format!(
                    "{label}: expected forbidden room-management failure, got {failure:?}"
                ));
            }
            CoreEvent::StateDelta(_)
                if room_management_forbidden_recorded(&conn.snapshot(), request_id) =>
            {
                if saw_forbidden_failure {
                    return Ok(());
                }
            }
            _ => {}
        }
    }
}

fn room_management_forbidden_recorded(snapshot: &AppState, request_id: RequestId) -> bool {
    matches!(
        &snapshot.room_management.operation,
        RoomManagementOperationState::Failed {
            request_id: state_request_id,
            operation,
            kind,
            ..
        } if *state_request_id == request_id.sequence
            && *operation == RoomManagementOperationKind::Moderation
            && *kind == OperationFailureKind::Forbidden
    )
}

async fn wait_for_invite_declined(
    conn: &mut CoreConnection,
    request_id: koushi_protocol::ids::RequestId,
    expected_room_id: &str,
    label: &str,
) -> Result<(), String> {
    let deadline = QaEventDeadline::after(EVENT_TIMEOUT);
    loop {
        let event = deadline
            .recv(conn)
            .await
            .map_err(|_| format!("{label}: timed out waiting for RoomEvent::InviteDeclined"))?
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;

        match event {
            CoreEvent::Room(RoomEvent::InviteDeclined {
                request_id: ev_id,
                room_id,
            }) if ev_id == request_id => {
                if room_id != expected_room_id {
                    return Err(format!("{label}: declined invite room mismatch"));
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

pub(super) async fn wait_for_pin_event_completed(
    conn: &mut CoreConnection,
    request_id: koushi_protocol::ids::RequestId,
    label: &str,
) -> Result<(), String> {
    loop {
        let event = tokio::time::timeout(EVENT_TIMEOUT, conn.recv_event())
            .await
            .map_err(|_| format!("{label}: timed out waiting for RoomEvent::PinEventCompleted"))?
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;

        match event {
            CoreEvent::Room(RoomEvent::PinEventCompleted {
                request_id: ev_id, ..
            }) if ev_id == request_id => return Ok(()),
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

pub(super) async fn wait_for_unpin_event_completed(
    conn: &mut CoreConnection,
    request_id: koushi_protocol::ids::RequestId,
    label: &str,
) -> Result<(), String> {
    loop {
        let event = tokio::time::timeout(EVENT_TIMEOUT, conn.recv_event())
            .await
            .map_err(|_| format!("{label}: timed out waiting for RoomEvent::UnpinEventCompleted"))?
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;

        match event {
            CoreEvent::Room(RoomEvent::UnpinEventCompleted {
                request_id: ev_id, ..
            }) if ev_id == request_id => return Ok(()),
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

pub(super) async fn wait_for_pinned_state(
    conn: &mut CoreConnection,
    room_id: &str,
    event_id: &str,
    expected_present: bool,
    label: &str,
) -> Result<(), String> {
    if snapshot_has_pinned_event(&conn.snapshot(), room_id, event_id) == expected_present {
        return Ok(());
    }

    loop {
        let event = tokio::time::timeout(EVENT_TIMEOUT, conn.recv_event())
            .await
            .map_err(|_| format!("{label}: timed out waiting for pinned state"))?
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;

        match event {
            CoreEvent::StateDelta(_) => {
                if snapshot_has_pinned_event(&conn.snapshot(), room_id, event_id)
                    == expected_present
                {
                    return Ok(());
                }
            }
            CoreEvent::Room(RoomEvent::PinnedEventsUpdated {
                room_id: ev_room_id,
                pinned,
                ..
            }) if ev_room_id == room_id => {
                let has_event = pinned.iter().any(|event| event.event_id == event_id);
                if has_event == expected_present {
                    return Ok(());
                }
            }
            _ => {}
        }
    }
}

fn snapshot_has_pinned_event(snapshot: &AppState, room_id: &str, event_id: &str) -> bool {
    snapshot
        .room_interactions
        .get(room_id)
        .map(|state| {
            state
                .pinned_events
                .iter()
                .any(|event| event.event_id == event_id)
        })
        .unwrap_or(false)
}

/// Wait (event-driven on `RoomListUpdated`/`StateDelta`, bounded by
/// `ROOM_LIST_EVENT_TIMEOUT`) until the snapshot's room list contains the
/// expected room in `rooms` AND the expected space in `spaces`. Returns the matching
/// snapshot. Waiting for "any non-empty list" is not enough: spaces only
/// classify as spaces after the create reaches the client via sync, so the
/// list can be momentarily rooms-only.
pub(super) async fn wait_for_room_list_containing(
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

    let deadline = tokio::time::Instant::now() + ROOM_LIST_EVENT_TIMEOUT;
    loop {
        let event = tokio::time::timeout_at(deadline, conn.recv_event())
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
            CoreEvent::Room(RoomEvent::RoomListUpdated) | CoreEvent::StateDelta(_) => {
                // The discrete event may arrive before the reducer projection;
                // check the latest snapshot and keep waiting otherwise.
                let snapshot = conn.snapshot();
                if contains_expected(&snapshot) {
                    return Ok(snapshot);
                }
            }
            _ => continue,
        }
    }
}

async fn assert_dm_space_scope_for_qa(
    conn: &mut CoreConnection,
    member_space_id: &str,
    member_dm_room_id: &str,
    control_dm_room_id: &str,
) -> Result<(), String> {
    select_space_scope_for_qa(conn, None, "invites_dm select Home for DM scope").await?;
    wait_for_sidebar_dm_room_ids(
        conn,
        &[member_dm_room_id, control_dm_room_id],
        "invites_dm Home DM scope",
    )
    .await?;

    select_space_scope_for_qa(
        conn,
        Some(member_space_id),
        "invites_dm select member Space for DM scope",
    )
    .await?;
    wait_for_sidebar_dm_room_ids(conn, &[member_dm_room_id], "invites_dm Space DM scope").await
}

async fn select_space_scope_for_qa(
    conn: &mut CoreConnection,
    space_id: Option<&str>,
    label: &str,
) -> Result<(), String> {
    let matches_scope =
        |snapshot: &AppState| snapshot.navigation.active_space_id.as_deref() == space_id;
    if matches_scope(&conn.snapshot()) {
        return Ok(());
    }

    let request_id = conn.next_request_id();
    conn.command(CoreCommand::Room(RoomCommand::SelectSpace {
        request_id,
        space_id: space_id.map(str::to_owned),
    }))
    .await
    .map_err(|e| format!("{label}: submit select space failed: {e}"))?;

    let deadline = tokio::time::Instant::now() + ROOM_LIST_EVENT_TIMEOUT;
    loop {
        let event = tokio::time::timeout_at(deadline, conn.recv_event())
            .await
            .map_err(|_| {
                let snapshot = conn.snapshot();
                format!(
                    "{label}: timed out waiting for space selection \
                     (expected_active={}, observed_active={})",
                    space_id.is_some(),
                    snapshot.navigation.active_space_id.is_some()
                )
            })?
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;

        match event {
            CoreEvent::StateDelta(_) if matches_scope(&conn.snapshot()) => return Ok(()),
            CoreEvent::Room(RoomEvent::RoomListUpdated) if matches_scope(&conn.snapshot()) => {
                return Ok(());
            }
            CoreEvent::OperationFailed {
                request_id: ev_id,
                failure,
            } if ev_id == request_id => {
                return Err(format!("{label}: select space failed: {failure:?}"));
            }
            _ if matches_scope(&conn.snapshot()) => return Ok(()),
            _ => continue,
        }
    }
}

async fn wait_for_sidebar_dm_room_ids(
    conn: &mut CoreConnection,
    expected_room_ids: &[&str],
    label: &str,
) -> Result<(), String> {
    let expected = expected_room_ids
        .iter()
        .map(|room_id| (*room_id).to_owned())
        .collect::<BTreeSet<_>>();
    let matches_expected = |snapshot: &AppState| sidebar_dm_room_ids(snapshot) == expected;
    if matches_expected(&conn.snapshot()) {
        return Ok(());
    }

    let deadline = tokio::time::Instant::now() + ROOM_LIST_EVENT_TIMEOUT;
    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            let snapshot = conn.snapshot();
            return Err(format!(
                "{label}: DM section scope mismatch \
                 (expected_count={}, observed_count={}, active_space={})",
                expected.len(),
                sidebar_dm_room_ids(&snapshot).len(),
                snapshot.navigation.active_space_id.is_some()
            ));
        }

        let event = tokio::time::timeout(remaining, conn.recv_event())
            .await
            .map_err(|_| {
                let snapshot = conn.snapshot();
                format!(
                    "{label}: DM section scope mismatch \
                     (expected_count={}, observed_count={}, active_space={})",
                    expected.len(),
                    sidebar_dm_room_ids(&snapshot).len(),
                    snapshot.navigation.active_space_id.is_some()
                )
            })?
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;

        match event {
            CoreEvent::StateDelta(_) if matches_expected(&conn.snapshot()) => return Ok(()),
            CoreEvent::Room(RoomEvent::RoomListUpdated) if matches_expected(&conn.snapshot()) => {
                return Ok(());
            }
            _ if matches_expected(&conn.snapshot()) => return Ok(()),
            _ => {}
        }
    }
}

fn sidebar_dm_room_ids(snapshot: &AppState) -> BTreeSet<String> {
    compose_sidebar(
        snapshot.navigation.active_space_id.as_deref(),
        &snapshot.spaces,
        &snapshot.rooms,
    )
    .global_dms
    .into_iter()
    .map(|room| room.room_id)
    .collect()
}

pub(super) async fn wait_for_invite_absent(
    conn: &mut CoreConnection,
    expected_room_id: &str,
    label: &str,
) -> Result<AppState, String> {
    let is_absent = |snapshot: &AppState| {
        !snapshot
            .invites
            .iter()
            .any(|invite| invite.room_id == expected_room_id)
    };

    let snapshot = conn.snapshot();
    if is_absent(&snapshot) {
        return Ok(snapshot);
    }

    let deadline = tokio::time::Instant::now() + ROOM_LIST_EVENT_TIMEOUT;
    loop {
        let event = tokio::time::timeout_at(deadline, conn.recv_event())
            .await
            .map_err(|_| {
                let snapshot = conn.snapshot();
                format!(
                    "{label}: timed out waiting for invite removal \
                     (have {} invites)",
                    snapshot.invites.len()
                )
            })?
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;

        match event {
            CoreEvent::Room(RoomEvent::RoomListUpdated) | CoreEvent::StateDelta(_) => {
                let snapshot = conn.snapshot();
                if is_absent(&snapshot) {
                    return Ok(snapshot);
                }
            }
            _ => continue,
        }
    }
}

#[cfg(test)]
#[path = "rooms_tests.rs"]
mod tests;
