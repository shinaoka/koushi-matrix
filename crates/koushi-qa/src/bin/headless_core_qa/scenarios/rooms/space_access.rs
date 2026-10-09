//! #935: a Space's access mode on a real homeserver.
//!
//! An administrator switches the Space between invite-only and public and back;
//! a second member observes each change through sync — both in the room-list
//! projection and in the settings snapshot they already have open — and is
//! refused when they try the change themselves. The Space's child room keeps
//! its own join rule throughout.

use super::super::event_wait::wait_for_space_child_projection;
use super::super::fixtures::set_space_child_for_qa;
use super::*;
use koushi_state::{RestrictedConditions, RoomAccessPolicy, RoomHistoryVisibility, RoomJoinRule};

pub(super) async fn verify(
    config: &QaConfig,
    conn_a: &mut CoreConnection,
    conn_b: &mut CoreConnection,
) -> Result<(), String> {
    let space_id = create_space_for_qa(conn_a, "QA Space Access", "space_access create").await?;
    wait_for_space_in_space_list(conn_a, &space_id, "space_access A space list").await?;
    let child_id =
        create_room_for_qa(conn_a, "QA Space Access Child", false, "space_access child").await?;
    set_space_child_for_qa(conn_a, &space_id, &child_id, "space_access set child").await?;
    wait_for_space_child_projection(
        conn_a,
        &space_id,
        std::slice::from_ref(&child_id),
        "space_access child projection",
    )
    .await?;
    let child_rule = load_room_settings_for_qa(conn_a, &child_id, "space_access child settings")
        .await?
        .join_rule;

    let user_b = format!("@{}:{}", config.user_b, config.server_name);
    invite_user_for_qa(conn_a, &space_id, &user_b, "space_access invite B").await?;
    wait_for_invite_in_snapshot(conn_b, &space_id, None, "space_access B invite").await?;
    let join_id = conn_b.next_request_id();
    conn_b
        .command(CoreCommand::Room(RoomCommand::JoinRoom {
            request_id: join_id,
            room_id: space_id.clone(),
        }))
        .await
        .map_err(|e| format!("space_access: submit B join failed: {e}"))?;
    wait_for_room_joined(conn_b, join_id, &space_id, "space_access B joins").await?;
    wait_for_space_in_space_list(conn_b, &space_id, "space_access B space list").await?;

    let settings_a =
        load_room_settings_for_qa(conn_a, &space_id, "space_access A settings").await?;
    if settings_a.join_rule != RoomJoinRule::Invite {
        return Err(format!(
            "space_access: a new Space was not invite-only (got {:?})",
            settings_a.join_rule
        ));
    }
    if !settings_a.permissions.can_change_join_rule {
        return Err("space_access: the creator cannot change the join rule".to_owned());
    }
    // B keeps these settings open for the rest of the run, as Space Info would.
    let settings_b =
        load_room_settings_for_qa(conn_b, &space_id, "space_access B settings").await?;
    if settings_b.permissions.can_change_join_rule {
        return Err("space_access: an ordinary member may change the join rule".to_owned());
    }

    let guard_id = conn_b.next_request_id();
    conn_b
        .command(CoreCommand::Room(RoomCommand::UpdateRoomSetting {
            request_id: guard_id,
            room_id: space_id.clone(),
            change: RoomSettingChange::JoinRule(RoomJoinRule::Public),
        }))
        .await
        .map_err(|e| format!("space_access: submit forbidden change failed: {e}"))?;
    wait_for_room_management_forbidden_operation(
        conn_b,
        guard_id,
        RoomManagementOperationKind::Settings,
        "space_access member guard",
    )
    .await?;

    for target in [RoomJoinRule::Public, RoomJoinRule::Invite] {
        let update_id = conn_a.next_request_id();
        conn_a
            .command(CoreCommand::Room(RoomCommand::UpdateRoomSetting {
                request_id: update_id,
                room_id: space_id.clone(),
                change: RoomSettingChange::JoinRule(target),
            }))
            .await
            .map_err(|e| format!("space_access: submit {target:?} failed: {e}"))?;
        let updated =
            wait_for_room_setting_updated(conn_a, update_id, "space_access A update").await?;
        if updated.join_rule != target {
            return Err(format!(
                "space_access: the saved snapshot carries {:?}, not {target:?}",
                updated.join_rule
            ));
        }
        wait_for_observed_join_rule(conn_b, &space_id, target, "space_access B observes").await?;
        // The reload below reads the SDK's synced room state, and
        // `send_state_event` does not apply the local write, so wait for A's own
        // projection before reloading. Otherwise a busy sync loop can still
        // report the previous rule (#1098).
        wait_for_observed_join_rule(conn_a, &space_id, target, "space_access A observes").await?;
        let persisted =
            load_room_settings_for_qa(conn_a, &space_id, "space_access A reload").await?;
        if persisted.join_rule != target {
            return Err(format!(
                "space_access: a reload after saving {target:?} reads {:?}",
                persisted.join_rule
            ));
        }
    }

    let child_after = load_room_settings_for_qa(conn_a, &child_id, "space_access child after")
        .await?
        .join_rule;
    if child_after != child_rule {
        return Err(format!(
            "space_access: the child room's join rule moved from {child_rule:?} to {child_after:?}"
        ));
    }

    // #1177: an explicit membership policy at creation is honoured, and its
    // allow content and history are readable after sync.
    let membership_room_id = conn_a.next_request_id();
    conn_a
        .command(CoreCommand::Room(RoomCommand::CreateRoom {
            request_id: membership_room_id,
            options: koushi_protocol::CreateRoomOptions {
                name: "QA Space Access Membership".to_owned(),
                topic: None,
                alias_localpart: None,
                encrypted: false,
                invited_only: false,
                visibility: Default::default(),
                parent_space: Some(koushi_protocol::CreateRoomParentSpace {
                    space_id: space_id.clone(),
                }),
                access_policy: Some(RoomAccessPolicy::new(
                    RoomJoinRule::Restricted,
                    vec![space_id.clone()],
                )),
                history: Some(RoomHistoryVisibility::Joined),
            },
        }))
        .await
        .map_err(|e| format!("space_access: submit membership room create failed: {e}"))?;
    let membership_room_id =
        wait_for_room_created(conn_a, membership_room_id, "space_access membership create").await?;
    wait_for_room_access_projection(
        conn_a,
        &membership_room_id,
        RoomJoinRule::Restricted,
        "space_access membership projection",
    )
    .await?;
    let membership = load_room_settings_for_qa(
        conn_a,
        &membership_room_id,
        "space_access membership settings",
    )
    .await?;
    if membership.access.restricted != Some(RestrictedConditions::MembershipOnly) {
        return Err(format!(
            "space_access: the created membership room reports {:?}",
            membership.access.restricted
        ));
    }
    if !membership
        .access
        .allow_targets
        .iter()
        .any(|target| target.room_id == space_id)
    {
        return Err("space_access: the created membership room does not name the Space".to_owned());
    }
    if membership.history_visibility != RoomHistoryVisibility::Joined {
        return Err(format!(
            "space_access: the created room's history is {:?}, not Joined",
            membership.history_visibility
        ));
    }
    println!("space_access_create_membership=ok");
    println!("space_access_history=ok");

    // #1177: restore a restricted rule through the real update command and read
    // back the synced allow content; B observes it without reloading.
    let restore_id = conn_a.next_request_id();
    conn_a
        .command(CoreCommand::Room(RoomCommand::UpdateRoomSetting {
            request_id: restore_id,
            room_id: membership_room_id.clone(),
            change: RoomSettingChange::AccessPolicy(RoomAccessPolicy::new(
                RoomJoinRule::Restricted,
                vec![space_id.clone()],
            )),
        }))
        .await
        .map_err(|e| format!("space_access: submit restricted restore failed: {e}"))?;
    wait_for_room_setting_updated(conn_a, restore_id, "space_access restore").await?;
    wait_for_room_access_projection(
        conn_a,
        &membership_room_id,
        RoomJoinRule::Restricted,
        "space_access restore projection",
    )
    .await?;
    let restored = load_room_settings_for_qa(
        conn_a,
        &membership_room_id,
        "space_access restored settings",
    )
    .await?;
    if restored.access.restricted != Some(RestrictedConditions::MembershipOnly)
        || !restored
            .access
            .allow_targets
            .iter()
            .any(|target| target.room_id == space_id)
    {
        return Err(
            "space_access: a restored restricted rule lost its synced allow content".to_owned(),
        );
    }
    println!("space_access_restricted_restore=ok");
    println!("space_access=ok");
    Ok(())
}

/// Wait until `conn` sees `expected` both on the synced Space and in the
/// settings snapshot it has open — the second without reloading, which is the
/// path Space Info relies on when another client changes the rule.
async fn wait_for_observed_join_rule(
    conn: &mut CoreConnection,
    space_id: &str,
    expected: RoomJoinRule,
    label: &str,
) -> Result<(), String> {
    let observed = |snapshot: &AppState| {
        let synced = snapshot
            .spaces
            .iter()
            .find(|space| space.space_id == space_id)
            .and_then(|space| space.join_rule);
        let open = snapshot
            .room_management
            .settings
            .as_ref()
            .filter(|settings| settings.room_id == space_id)
            .map(|settings| settings.join_rule);
        (synced, open)
    };
    let deadline = tokio::time::Instant::now() + ROOM_LIST_EVENT_TIMEOUT;
    loop {
        if observed(&conn.snapshot()) == (Some(expected), Some(expected)) {
            return Ok(());
        }
        match tokio::time::timeout_at(deadline, conn.recv_event()).await {
            Ok(Ok(_)) => continue,
            Ok(Err(lag)) => {
                return Err(format!(
                    "{label}: event stream lagged (skipped={})",
                    lag.skipped
                ));
            }
            Err(_) => {
                let (synced, open) = observed(&conn.snapshot());
                return Err(format!(
                    "{label}: timed out waiting for {expected:?} (synced={synced:?} open={open:?})"
                ));
            }
        }
    }
}

/// Wait until the shared room-access projection reports `expected` for a room
/// (not a Space), without reloading its settings.
async fn wait_for_room_access_projection(
    conn: &mut CoreConnection,
    room_id: &str,
    expected: RoomJoinRule,
    label: &str,
) -> Result<(), String> {
    let observed = |snapshot: &AppState| {
        snapshot
            .room_access
            .get(room_id)
            .and_then(|condition| condition.join_rule)
    };
    let deadline = tokio::time::Instant::now() + ROOM_LIST_EVENT_TIMEOUT;
    loop {
        if observed(&conn.snapshot()) == Some(expected) {
            return Ok(());
        }
        match tokio::time::timeout_at(deadline, conn.recv_event()).await {
            Ok(Ok(_)) => continue,
            Ok(Err(lag)) => {
                return Err(format!(
                    "{label}: event stream lagged (skipped={})",
                    lag.skipped
                ));
            }
            Err(_) => {
                return Err(format!(
                    "{label}: timed out waiting for {expected:?} (observed={:?})",
                    observed(&conn.snapshot())
                ));
            }
        }
    }
}
