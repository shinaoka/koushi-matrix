use super::event_wait::{
    QaEventDeadline, space_has_expected_children, wait_for_room_created, wait_for_space_child_set,
    wait_for_space_created,
};
use super::registry::{EVENT_TIMEOUT, ROOM_LIST_EVENT_TIMEOUT};
use super::{
    AppCommand, AppState, BTreeSet, CoreCommand, CoreConnection, CoreEvent, CreateRoomOptions,
    CreateRoomVisibility, RequestId, RoomCommand, RoomEvent, RoomListFilter, RoomSettingsSnapshot,
    RoomSummary, RoomTags,
};

pub(super) fn private_room_options(name: impl Into<String>, encrypted: bool) -> CreateRoomOptions {
    CreateRoomOptions {
        name: name.into(),
        topic: None,
        alias_localpart: None,
        encrypted,
        invited_only: false,
        visibility: CreateRoomVisibility::Private,
        parent_space: None,
    }
}

pub(super) async fn create_room_for_qa(
    conn: &mut CoreConnection,
    name: &str,
    encrypted: bool,
    label: &str,
) -> Result<String, String> {
    let request_id = conn.next_request_id();
    conn.command(CoreCommand::Room(RoomCommand::CreateRoom {
        request_id,
        options: private_room_options(name, encrypted),
    }))
    .await
    .map_err(|e| format!("{label}: submit room create failed: {e}"))?;
    wait_for_room_created(conn, request_id, label).await
}

pub(super) async fn create_space_for_qa(
    conn: &mut CoreConnection,
    name: &str,
    label: &str,
) -> Result<String, String> {
    let request_id = conn.next_request_id();
    conn.command(CoreCommand::Room(RoomCommand::CreateSpace {
        request_id,
        name: name.to_owned(),
    }))
    .await
    .map_err(|e| format!("{label}: submit space create failed: {e}"))?;
    wait_for_space_created(conn, request_id, label).await
}

pub(super) async fn invite_user_for_qa(
    conn: &mut CoreConnection,
    room_id: &str,
    user_id: &str,
    label: &str,
) -> Result<(), String> {
    let request_id = conn.next_request_id();
    conn.command(CoreCommand::Room(RoomCommand::InviteUser {
        request_id,
        room_id: room_id.to_owned(),
        user_id: user_id.to_owned(),
    }))
    .await
    .map_err(|e| format!("{label}: submit invite failed: {e}"))?;
    wait_for_user_invited_ack(conn, request_id, label).await
}

pub(super) async fn load_room_settings_for_qa(
    conn: &mut CoreConnection,
    room_id: &str,
    label: &str,
) -> Result<RoomSettingsSnapshot, String> {
    let request_id = conn.next_request_id();
    conn.command(CoreCommand::Room(RoomCommand::LoadRoomSettings {
        request_id,
        room_id: room_id.to_owned(),
    }))
    .await
    .map_err(|e| format!("{label}: submit load settings failed: {e}"))?;
    wait_for_room_settings_loaded(conn, request_id, label).await
}

pub(super) fn assert_room_settings_contains_members(
    settings: &RoomSettingsSnapshot,
    expected_user_ids: &[&str],
    label: &str,
) -> Result<(), String> {
    let observed_user_ids = settings
        .members
        .iter()
        .map(|member| member.user_id.as_str())
        .collect::<BTreeSet<_>>();
    let missing_count = expected_user_ids
        .iter()
        .filter(|user_id| !observed_user_ids.contains(**user_id))
        .count();
    if missing_count > 0 {
        return Err(format!(
            "{label}: member list missing expected users \
             (expected={}, observed={}, missing={missing_count})",
            expected_user_ids.len(),
            observed_user_ids.len()
        ));
    }
    Ok(())
}

pub(super) async fn accept_invite_for_qa(
    conn: &mut CoreConnection,
    room_id: &str,
    label: &str,
) -> Result<(), String> {
    let request_id = conn.next_request_id();
    conn.command(CoreCommand::Room(RoomCommand::AcceptInvite {
        request_id,
        room_id: room_id.to_owned(),
    }))
    .await
    .map_err(|e| format!("{label}: submit accept invite failed: {e}"))?;
    wait_for_invite_accepted(conn, request_id, room_id, label).await
}

pub(super) async fn start_direct_message_for_qa(
    conn: &mut CoreConnection,
    user_id: &str,
    label: &str,
) -> Result<String, String> {
    let request_id = conn.next_request_id();
    conn.command(CoreCommand::Room(RoomCommand::StartDirectMessage {
        request_id,
        user_id: user_id.to_owned(),
    }))
    .await
    .map_err(|e| format!("{label}: submit start DM failed: {e}"))?;
    wait_for_direct_message_started(conn, request_id, label).await
}

pub(super) async fn set_space_child_for_qa(
    conn: &mut CoreConnection,
    space_id: &str,
    child_room_id: &str,
    label: &str,
) -> Result<(), String> {
    let request_id = conn.next_request_id();
    conn.command(CoreCommand::Room(RoomCommand::SetSpaceChild {
        request_id,
        space_id: space_id.to_owned(),
        child_room_id: child_room_id.to_owned(),
    }))
    .await
    .map_err(|e| format!("{label}: submit set space child failed: {e}"))?;
    wait_for_space_child_set(conn, request_id, space_id, child_room_id, label).await
}

async fn wait_for_room_settings_loaded(
    conn: &mut CoreConnection,
    request_id: RequestId,
    label: &str,
) -> Result<RoomSettingsSnapshot, String> {
    loop {
        let event = tokio::time::timeout(EVENT_TIMEOUT, conn.recv_event())
            .await
            .map_err(|_| format!("{label}: timed out waiting for RoomSettingsLoaded"))?
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;

        match event {
            CoreEvent::Room(RoomEvent::RoomSettingsLoaded {
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

/// Wait for `RoomEvent::UserInvited` by request_id without exposing IDs in
/// failure text. Used by private-data-free invite QA.
async fn wait_for_user_invited_ack(
    conn: &mut CoreConnection,
    request_id: koushi_protocol::ids::RequestId,
    label: &str,
) -> Result<(), String> {
    loop {
        let event = tokio::time::timeout(EVENT_TIMEOUT, conn.recv_event())
            .await
            .map_err(|_| format!("{label}: timed out waiting for RoomEvent::UserInvited"))?
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;

        match event {
            CoreEvent::Room(RoomEvent::UserInvited {
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

async fn wait_for_invite_accepted(
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
            .map_err(|_| format!("{label}: timed out waiting for RoomEvent::InviteAccepted"))?
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;

        match event {
            CoreEvent::Room(RoomEvent::InviteAccepted {
                request_id: ev_id,
                room_id,
            }) if ev_id == request_id => {
                if room_id != expected_room_id {
                    return Err(format!("{label}: accepted invite room mismatch"));
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

async fn wait_for_direct_message_started(
    conn: &mut CoreConnection,
    request_id: koushi_protocol::ids::RequestId,
    label: &str,
) -> Result<String, String> {
    let deadline = QaEventDeadline::after(EVENT_TIMEOUT);
    loop {
        let event = deadline
            .recv(conn)
            .await
            .map_err(|_| format!("{label}: timed out waiting for RoomEvent::DirectMessageStarted"))?
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;

        match event {
            CoreEvent::Room(RoomEvent::DirectMessageStarted {
                request_id: ev_id,
                room_id,
            }) if ev_id == request_id => return Ok(room_id),
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

pub(super) async fn select_space_and_wait_for_room_scope(
    conn: &mut CoreConnection,
    space_id: &str,
    expected_room_ids: &[String],
    label: &str,
) -> Result<AppState, String> {
    select_room_list_filter_for_qa(conn, RoomListFilter::Rooms, label).await?;
    let request_id = conn.next_request_id();
    conn.command(CoreCommand::Room(RoomCommand::SelectSpace {
        request_id,
        space_id: Some(space_id.to_owned()),
    }))
    .await
    .map_err(|e| format!("{label}: submit select space failed: {e}"))?;

    let matches_scope = |snapshot: &AppState| {
        room_list_matches_selected_space(snapshot, space_id, expected_room_ids)
    };
    let snapshot = conn.snapshot();
    if matches_scope(&snapshot) {
        return Ok(snapshot);
    }

    let deadline = tokio::time::Instant::now() + ROOM_LIST_EVENT_TIMEOUT;
    loop {
        let event = tokio::time::timeout_at(deadline, conn.recv_event())
            .await
            .map_err(|_| {
                let snapshot = conn.snapshot();
                format!(
                    "{label}: timed out waiting for selected-space room scope \
                     (expected_rooms={}, projected_items={}, total_rooms={}, active_space={})",
                    expected_room_ids.len(),
                    snapshot.room_list.items.len(),
                    snapshot.rooms.len(),
                    snapshot.navigation.active_space_id.is_some()
                )
            })?
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;

        match event {
            CoreEvent::Room(RoomEvent::RoomListUpdated) | CoreEvent::StateDelta(_) => {
                let snapshot = conn.snapshot();
                if matches_scope(&snapshot) {
                    return Ok(snapshot);
                }
            }
            CoreEvent::OperationFailed {
                request_id: ev_id,
                failure,
            } if ev_id == request_id => {
                return Err(format!("{label}: select space failed: {failure:?}"));
            }
            _ => continue,
        }
    }
}

async fn select_room_list_filter_for_qa(
    conn: &mut CoreConnection,
    filter: RoomListFilter,
    label: &str,
) -> Result<(), String> {
    if conn.snapshot().room_list.active_filter == filter {
        return Ok(());
    }

    let request_id = conn.next_request_id();
    conn.command(CoreCommand::App(AppCommand::SelectRoomListFilter {
        request_id,
        filter,
    }))
    .await
    .map_err(|e| format!("{label}: submit room-list filter failed: {e}"))?;

    loop {
        let event = tokio::time::timeout(EVENT_TIMEOUT, conn.recv_event())
            .await
            .map_err(|_| format!("{label}: timed out waiting for room-list filter"))?
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;

        match event {
            CoreEvent::StateDelta(_) if conn.snapshot().room_list.active_filter == filter => {
                return Ok(());
            }
            CoreEvent::Room(RoomEvent::RoomListUpdated)
                if conn.snapshot().room_list.active_filter == filter =>
            {
                return Ok(());
            }
            CoreEvent::OperationFailed {
                request_id: ev_id,
                failure,
            } if ev_id == request_id => {
                return Err(format!("{label}: room-list filter failed: {failure:?}"));
            }
            _ if conn.snapshot().room_list.active_filter == filter => return Ok(()),
            _ => continue,
        }
    }
}

fn room_list_matches_selected_space(
    snapshot: &AppState,
    space_id: &str,
    expected_room_ids: &[String],
) -> bool {
    if snapshot.navigation.active_space_id.as_deref() != Some(space_id)
        || snapshot.room_list.active_filter != RoomListFilter::Rooms
        || !space_has_expected_children(snapshot, space_id, expected_room_ids)
    {
        return false;
    }
    let expected = expected_room_ids.iter().collect::<BTreeSet<_>>();
    let projected = snapshot
        .room_list
        .items
        .iter()
        .filter(|item| matches!(item.kind, koushi_state::RoomListEntryKind::Room))
        .map(|item| &item.room_id)
        .collect::<BTreeSet<_>>();
    projected == expected
}

pub(super) fn native_attention_room(
    room_id: &str,
    display_name: &str,
    is_dm: bool,
    unread_count: u64,
    notification_count: u64,
    highlight_count: u64,
) -> RoomSummary {
    RoomSummary {
        room_id: room_id.to_owned(),
        display_name: display_name.to_owned(),
        display_label: display_name.to_owned(),
        original_display_label: display_name.to_owned(),
        avatar: None,
        is_dm,
        dm_user_ids: Vec::new(),
        tags: RoomTags::default(),
        unread_count,
        notification_count,
        highlight_count,
        marked_unread: false,
        recency_stamp: None,
        conversation_activity: None,
        latest_event: None,
        parent_space_ids: Vec::new(),
        dm_space_ids: Vec::new(),
        is_encrypted: false,
        joined_members: 0,
    }
}
