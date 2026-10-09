//! #935: a Space's access mode on a real homeserver.
//!
//! An administrator switches the Space between invite-only and public and back;
//! a second member observes each change through sync — both in the room-list
//! projection and in the settings snapshot they already have open — and is
//! refused when they try the change themselves. The Space's child room keeps
//! its own join rule throughout.

use super::super::event_wait::wait_for_space_child_projection;
use super::super::fixtures::set_space_child_for_qa;
use super::super::scenario_identity::cleanup_qa_auditor_device;
use super::*;
use koushi_state::{RestrictedConditions, RoomAccessPolicy, RoomHistoryVisibility, RoomJoinRule};

pub(super) async fn verify(
    config: &QaConfig,
    conn_a: &mut CoreConnection,
    conn_b: &mut CoreConnection,
) -> Result<(), String> {
    // A disposable auditor device of account A reads the homeserver's own state:
    // the initial history event count and the persisted parent links are
    // asserted from the server, not from the local projection.
    let auditor = koushi_sdk::login_with_password(&koushi_state::LoginRequest {
        homeserver: config.homeserver.clone(),
        username: config.user_a.clone(),
        password: super::super::AuthSecret::new(config.password_a.clone()),
        device_display_name: Some("Koushi Space Access Auditor".to_owned()),
    })
    .await
    .map_err(|_| "space_access: auditor login failed".to_owned())?;
    let result = verify_with_auditor(config, conn_a, conn_b, &auditor).await;
    let cleanup = cleanup_qa_auditor_device(&auditor, &config.password_a).await;
    let _ = koushi_sdk::close_session_stores(&auditor).await;
    drop(auditor);
    result?;
    cleanup.map_err(|error| format!("space_access: {error}"))
}

async fn verify_with_auditor(
    config: &QaConfig,
    conn_a: &mut CoreConnection,
    conn_b: &mut CoreConnection,
    auditor: &koushi_sdk::MatrixClientSession,
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

    // #1177: the allow selection names a Space DISTINCT from the attachment, and
    // one A does not administer (B owns it and invites A), so the membership
    // route never depends on permission over the named Space.
    let allow_space_id =
        create_space_for_qa(conn_b, "QA Space Access Allow", "space_access allow space").await?;
    wait_for_space_in_space_list(conn_b, &allow_space_id, "space_access B allow space list")
        .await?;
    let user_a = format!("@{}:{}", config.user_a, config.server_name);
    invite_user_for_qa(
        conn_b,
        &allow_space_id,
        &user_a,
        "space_access invite A to the allow Space",
    )
    .await?;
    wait_for_invite_in_snapshot(conn_a, &allow_space_id, None, "space_access A allow invite")
        .await?;
    let join_allow_id = conn_a.next_request_id();
    conn_a
        .command(CoreCommand::Room(RoomCommand::JoinRoom {
            request_id: join_allow_id,
            room_id: allow_space_id.clone(),
        }))
        .await
        .map_err(|e| format!("space_access: submit A join allow Space failed: {e}"))?;
    wait_for_room_joined(
        conn_a,
        join_allow_id,
        &allow_space_id,
        "space_access A joins allow Space",
    )
    .await?;
    wait_for_space_in_space_list(conn_a, &allow_space_id, "space_access A allow space list")
        .await?;
    // Force a client-side read of the allow Space so its create event is in
    // A's state store before Create validates the selected target.
    load_room_settings_for_qa(
        conn_a,
        &allow_space_id,
        "space_access A allow space settings",
    )
    .await?;

    // #1177: an explicit membership policy at creation is honoured, its allow
    // content is EXACTLY the selected target, and its history is read back.
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
                    vec![allow_space_id.clone()],
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
    let membership = wait_for_room_settings_exact_targets(
        conn_a,
        &membership_room_id,
        RoomJoinRule::Restricted,
        &[allow_space_id.as_str()],
        "space_access membership settings",
    )
    .await?;
    if membership.history_visibility != RoomHistoryVisibility::Joined {
        return Err(format!(
            "space_access: the created room's history is {:?}, not Joined",
            membership.history_visibility
        ));
    }
    // The explicit `history` must send exactly one initial-state event, read from
    // the homeserver by the auditor rather than trusted from the success
    // snapshot.
    let history_event_count =
        count_room_state_events(auditor, &membership_room_id, "m.room.history_visibility").await?;
    if history_event_count != 1 {
        return Err(format!(
            "space_access: the created room has {history_event_count} \
             m.room.history_visibility events, expected exactly 1"
        ));
    }
    // The selected target Space is owned by B; A, the creator, has no edit
    // permission there, so the membership route provably does not depend on
    // permission over the named target.
    let allow_space_settings = load_room_settings_for_qa(
        conn_a,
        &allow_space_id,
        "space_access A allow target settings",
    )
    .await?;
    if allow_space_settings.permissions.can_edit_settings {
        return Err("space_access: the creator can edit the selected target Space".to_owned());
    }
    println!("space_access_create_membership=ok");
    println!("space_access_target_without_permission=ok");
    println!("space_access_history=ok");

    // #1177: all four history values through the real Room Info update command.
    for target in [
        RoomHistoryVisibility::Shared,
        RoomHistoryVisibility::Invited,
        RoomHistoryVisibility::Joined,
        RoomHistoryVisibility::WorldReadable,
    ] {
        let update_id = conn_a.next_request_id();
        conn_a
            .command(CoreCommand::Room(RoomCommand::UpdateRoomSetting {
                request_id: update_id,
                room_id: membership_room_id.clone(),
                change: RoomSettingChange::HistoryVisibility(target),
            }))
            .await
            .map_err(|e| format!("space_access: submit history {target:?} failed: {e}"))?;
        wait_for_room_setting_updated(conn_a, update_id, "space_access history update").await?;
        // Read the synced value back independently instead of trusting the
        // success snapshot.
        wait_for_room_settings_history(
            conn_a,
            &membership_room_id,
            target,
            "space_access history read-back",
        )
        .await?;
    }
    println!("space_access_history_values=ok");

    // #1177: B keeps the membership room's settings open; A's history change
    // reaches B's open settings without a reload.
    // #1177: B is a member of the allow Space, so the restricted room admits B;
    // joining lets B observe A's later allow-target and history changes.
    let b_join_id = conn_b.next_request_id();
    conn_b
        .command(CoreCommand::Room(RoomCommand::JoinRoom {
            request_id: b_join_id,
            room_id: membership_room_id.clone(),
        }))
        .await
        .map_err(|e| format!("space_access: submit B join membership room failed: {e}"))?;
    wait_for_room_joined(
        conn_b,
        b_join_id,
        &membership_room_id,
        "space_access B joins membership room",
    )
    .await?;
    load_room_settings_for_qa(
        conn_b,
        &membership_room_id,
        "space_access B membership settings",
    )
    .await?;
    let history_id = conn_a.next_request_id();
    conn_a
        .command(CoreCommand::Room(RoomCommand::UpdateRoomSetting {
            request_id: history_id,
            room_id: membership_room_id.clone(),
            change: RoomSettingChange::HistoryVisibility(RoomHistoryVisibility::Shared),
        }))
        .await
        .map_err(|e| format!("space_access: submit observed history change failed: {e}"))?;
    wait_for_room_setting_updated(conn_a, history_id, "space_access observed history change")
        .await?;
    wait_for_observed_history(
        conn_b,
        &membership_room_id,
        RoomHistoryVisibility::Shared,
        "space_access B observes history",
    )
    .await?;

    // #1177: a second client observes an allow-target change without a reload.
    let allow_id = conn_a.next_request_id();
    conn_a
        .command(CoreCommand::Room(RoomCommand::UpdateRoomSetting {
            request_id: allow_id,
            room_id: membership_room_id.clone(),
            change: RoomSettingChange::AccessPolicy(RoomAccessPolicy::new(
                RoomJoinRule::Restricted,
                vec![space_id.clone()],
            )),
        }))
        .await
        .map_err(|e| format!("space_access: submit observed allow change failed: {e}"))?;
    wait_for_room_setting_updated(conn_a, allow_id, "space_access observed allow change").await?;
    // Inspect B's OPEN settings and assert the EXACT canonical target set, not
    // containment.
    wait_for_open_settings_exact_targets(
        conn_b,
        &membership_room_id,
        RoomJoinRule::Restricted,
        &[space_id.as_str()],
        "space_access B observes allow target",
    )
    .await?;
    println!("space_access_second_client=ok");

    // #1177: move a genuinely different policy (Public) first, then restore the
    // restricted allow list and read back the exact synced allow content.
    let public_id = conn_a.next_request_id();
    conn_a
        .command(CoreCommand::Room(RoomCommand::UpdateRoomSetting {
            request_id: public_id,
            room_id: membership_room_id.clone(),
            change: RoomSettingChange::JoinRule(RoomJoinRule::Public),
        }))
        .await
        .map_err(|e| format!("space_access: submit public move failed: {e}"))?;
    wait_for_room_setting_updated(conn_a, public_id, "space_access public move").await?;
    wait_for_room_access_projection(
        conn_a,
        &membership_room_id,
        RoomJoinRule::Public,
        "space_access public projection",
    )
    .await?;

    let restore_id = conn_a.next_request_id();
    conn_a
        .command(CoreCommand::Room(RoomCommand::UpdateRoomSetting {
            request_id: restore_id,
            room_id: membership_room_id.clone(),
            change: RoomSettingChange::AccessPolicy(RoomAccessPolicy::new(
                RoomJoinRule::Restricted,
                vec![allow_space_id.clone()],
            )),
        }))
        .await
        .map_err(|e| format!("space_access: submit restricted restore failed: {e}"))?;
    wait_for_room_setting_updated(conn_a, restore_id, "space_access restore").await?;
    // Wait for the synced allow content, not just the rule: the SDK can publish
    // the rule before its membership entries are parsed.
    wait_for_room_access_allow_target(
        conn_a,
        &membership_room_id,
        RoomJoinRule::Restricted,
        &allow_space_id,
        "space_access restore projection",
    )
    .await?;
    wait_for_room_settings_exact_targets(
        conn_a,
        &membership_room_id,
        RoomJoinRule::Restricted,
        &[allow_space_id.as_str()],
        "space_access restored settings",
    )
    .await?;
    println!("space_access_restricted_restore=ok");

    // #1177: a second parent attachment, added through the existing link
    // command, leaves the allow content exactly the selected target. Both
    // attachments must be projected AND persisted on the homeserver before the
    // token is awarded.
    let second_parent_id = create_space_for_qa(
        conn_a,
        "QA Space Access Second Parent",
        "space_access second parent",
    )
    .await?;
    set_space_child_for_qa(
        conn_a,
        &second_parent_id,
        &membership_room_id,
        "space_access second parent link",
    )
    .await?;
    wait_for_space_child_projection(
        conn_a,
        &space_id,
        std::slice::from_ref(&membership_room_id),
        "space_access first parent projection",
    )
    .await?;
    wait_for_space_child_projection(
        conn_a,
        &second_parent_id,
        std::slice::from_ref(&membership_room_id),
        "space_access second parent projection",
    )
    .await?;
    assert_server_space_child(auditor, &space_id, &membership_room_id, "first parent").await?;
    assert_server_space_child(
        auditor,
        &second_parent_id,
        &membership_room_id,
        "second parent",
    )
    .await?;
    wait_for_room_settings_exact_targets(
        conn_a,
        &membership_room_id,
        RoomJoinRule::Restricted,
        &[allow_space_id.as_str()],
        "space_access two parent settings",
    )
    .await?;
    println!("space_access_two_parent=ok");

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

fn room_settings_carry_exact_targets(
    settings: &koushi_state::RoomSettingsSnapshot,
    expected_rule: RoomJoinRule,
    expected_targets: &[&str],
) -> bool {
    settings.access.join_rule == Some(expected_rule)
        && settings.access.restricted == Some(RestrictedConditions::MembershipOnly)
        && settings.access.allow_targets.len() == expected_targets.len()
        && expected_targets.iter().all(|target| {
            settings
                .access
                .allow_targets
                .iter()
                .any(|observed| observed.room_id == *target)
        })
}

/// Reload a room's settings until the synced allow content is EXACTLY
/// `expected_targets` (#1177). The SDK's settings read can lag the list
/// observer's synced content, so the next observation is awaited between reads.
async fn wait_for_room_settings_exact_targets(
    conn: &mut CoreConnection,
    room_id: &str,
    expected_rule: RoomJoinRule,
    expected_targets: &[&str],
    label: &str,
) -> Result<koushi_state::RoomSettingsSnapshot, String> {
    let deadline = tokio::time::Instant::now() + ROOM_LIST_EVENT_TIMEOUT;
    let mut latest = load_room_settings_for_qa(conn, room_id, label).await?;
    while !room_settings_carry_exact_targets(&latest, expected_rule, expected_targets) {
        match tokio::time::timeout_at(deadline, conn.recv_event()).await {
            Ok(Ok(_)) => {
                latest = load_room_settings_for_qa(conn, room_id, label).await?;
            }
            Ok(Err(lag)) => {
                return Err(format!(
                    "{label}: event stream lagged (skipped={})",
                    lag.skipped
                ));
            }
            Err(_) => break,
        }
    }
    if !room_settings_carry_exact_targets(&latest, expected_rule, expected_targets) {
        return Err(format!(
            "{label}: expected exactly {expected_targets:?}, got {:?}",
            latest
                .access
                .allow_targets
                .iter()
                .map(|target| &target.room_id)
                .collect::<Vec<_>>()
        ));
    }
    Ok(latest)
}

/// Wait until the open settings for `room_id` carry `expected` history without
/// a reload, the path Room Info relies on when another client changes it.
async fn wait_for_observed_history(
    conn: &mut CoreConnection,
    room_id: &str,
    expected: RoomHistoryVisibility,
    label: &str,
) -> Result<(), String> {
    let observed = |snapshot: &AppState| {
        snapshot
            .room_management
            .settings
            .as_ref()
            .filter(|settings| settings.room_id == room_id)
            .map(|settings| settings.history_visibility)
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
                    "{label}: timed out waiting for history {expected:?} (open={:?})",
                    observed(&conn.snapshot())
                ));
            }
        }
    }
}

/// Wait until the shared room-access projection reports a restricted rule with
/// `target_id` as a membership allow target, without reloading its settings.
async fn wait_for_room_access_allow_target(
    conn: &mut CoreConnection,
    room_id: &str,
    expected_rule: RoomJoinRule,
    target_id: &str,
    label: &str,
) -> Result<(), String> {
    let observed = |snapshot: &AppState| {
        snapshot.room_access.get(room_id).is_some_and(|condition| {
            condition.join_rule == Some(expected_rule)
                && condition.restricted == Some(RestrictedConditions::MembershipOnly)
                && condition
                    .allow_targets
                    .iter()
                    .any(|target| target.room_id == target_id)
        })
    };
    let deadline = tokio::time::Instant::now() + ROOM_LIST_EVENT_TIMEOUT;
    loop {
        if observed(&conn.snapshot()) {
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
                    "{label}: timed out waiting for the membership allow target"
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

/// Reload a room's settings until the synced history visibility matches
/// `expected` (#1177). A fresh SDK read, not the success snapshot.
async fn wait_for_room_settings_history(
    conn: &mut CoreConnection,
    room_id: &str,
    expected: RoomHistoryVisibility,
    label: &str,
) -> Result<(), String> {
    let deadline = tokio::time::Instant::now() + ROOM_LIST_EVENT_TIMEOUT;
    let mut latest = load_room_settings_for_qa(conn, room_id, label).await?;
    while latest.history_visibility != expected {
        match tokio::time::timeout_at(deadline, conn.recv_event()).await {
            Ok(Ok(_)) => {
                latest = load_room_settings_for_qa(conn, room_id, label).await?;
            }
            Ok(Err(lag)) => {
                return Err(format!(
                    "{label}: event stream lagged (skipped={})",
                    lag.skipped
                ));
            }
            Err(_) => break,
        }
    }
    if latest.history_visibility != expected {
        return Err(format!(
            "{label}: expected history {expected:?}, got {:?}",
            latest.history_visibility
        ));
    }
    Ok(())
}

/// Wait until `conn`'s OPEN settings for `room_id` carry EXACTLY
/// `expected_targets`, without a reload (#1177).
async fn wait_for_open_settings_exact_targets(
    conn: &mut CoreConnection,
    room_id: &str,
    expected_rule: RoomJoinRule,
    expected_targets: &[&str],
    label: &str,
) -> Result<(), String> {
    let observed = |snapshot: &AppState| {
        snapshot
            .room_management
            .settings
            .as_ref()
            .filter(|settings| settings.room_id == room_id)
            .is_some_and(|settings| {
                room_settings_carry_exact_targets(settings, expected_rule, expected_targets)
            })
    };
    let deadline = tokio::time::Instant::now() + ROOM_LIST_EVENT_TIMEOUT;
    loop {
        if observed(&conn.snapshot()) {
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
                    "{label}: timed out waiting for the open settings' exact allow set"
                ));
            }
        }
    }
}

/// Count the `event_type` state events the homeserver holds for a room, read by
/// the auditor SDK session (#1177). Used to prove an explicit create `history`
/// sends its initial state event exactly once.
async fn count_room_state_events(
    auditor: &koushi_sdk::MatrixClientSession,
    room_id: &str,
    event_type: &str,
) -> Result<usize, String> {
    use matrix_sdk::room::MessagesOptions;
    let owned: matrix_sdk::ruma::OwnedRoomId = room_id
        .try_into()
        .map_err(|_| "space_access: invalid room id for the auditor".to_owned())?;
    // The auditor did not create this room, so wait for its own sync to see it.
    let mut room = auditor.client().get_room(&owned);
    let deadline = tokio::time::Instant::now() + ROOM_LIST_EVENT_TIMEOUT;
    while room.is_none() {
        if tokio::time::timeout_at(deadline, koushi_sdk::sync_once(auditor))
            .await
            .is_err()
        {
            break;
        }
        room = auditor.client().get_room(&owned);
    }
    let room = room.ok_or_else(|| "space_access: the auditor cannot see the room".to_owned())?;
    let mut options = MessagesOptions::backward();
    options.limit = 256u32.into();
    let messages = room
        .messages(options)
        .await
        .map_err(|_| "space_access: the auditor could not read the room timeline".to_owned())?;
    let mut seen = std::collections::BTreeSet::new();
    let mut count = 0usize;
    let mut count_event = |value: &serde_json::Value| {
        if value.get("type").and_then(serde_json::Value::as_str) != Some(event_type) {
            return;
        }
        // The `/messages` `state` field can repeat a timeline event; count each
        // event id once.
        let dedupe_key = value
            .get("event_id")
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned)
            .unwrap_or_else(|| value.to_string());
        if seen.insert(dedupe_key) {
            count += 1;
        }
    };
    for event in &messages.chunk {
        let value: serde_json::Value = serde_json::from_str(event.raw().json().get())
            .map_err(|_| "space_access: unreadable timeline event".to_owned())?;
        count_event(&value);
    }
    for state in &messages.state {
        let value: serde_json::Value = serde_json::from_str(state.json().get())
            .map_err(|_| "space_access: unreadable state event".to_owned())?;
        count_event(&value);
    }
    Ok(count)
}

/// Read one Space's `m.space.child` for `child_room_id` from the homeserver and
/// require nonempty routing (#1177).
async fn assert_server_space_child(
    auditor: &koushi_sdk::MatrixClientSession,
    space_id: &str,
    child_room_id: &str,
    label: &str,
) -> Result<(), String> {
    use matrix_sdk::ruma::api::client::state::get_state_event_for_key;
    use matrix_sdk::ruma::events::StateEventType;
    let space: matrix_sdk::ruma::OwnedRoomId = space_id
        .try_into()
        .map_err(|_| "space_access: invalid Space id".to_owned())?;
    let request = get_state_event_for_key::v3::Request::new(
        space,
        StateEventType::SpaceChild,
        child_room_id.to_owned(),
    );
    let response = auditor
        .client()
        .send(request)
        .await
        .map_err(|_| format!("space_access: {label} has no server-side m.space.child"))?;
    let content: serde_json::Value = serde_json::from_str(response.event_or_content.get())
        .map_err(|_| format!("space_access: {label} m.space.child is unreadable"))?;
    let has_route = content
        .get("via")
        .and_then(serde_json::Value::as_array)
        .is_some_and(|via| {
            via.iter()
                .any(|server| server.as_str().is_some_and(|s| !s.is_empty()))
        });
    if has_route {
        Ok(())
    } else {
        Err(format!(
            "space_access: {label} m.space.child has no routing"
        ))
    }
}
