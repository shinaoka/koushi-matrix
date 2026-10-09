//! #1023: rooms created from a public Space on a real homeserver.
//!
//! The create-room defaults select a public room in a public Space, an unnamed
//! room omits `m.room.name` and shows the SDK-calculated name, an unnamed
//! public room is created without an address unless one is entered (then the
//! canonical alias names it), a conflict on that address is reported, the room
//! can be named later, and explicitly choosing private still creates a
//! restricted Space room.

use super::*;
use koushi_core::{CreateRoomOptions, CreateRoomParentSpace, CreateRoomVisibility};
use koushi_state::{RoomJoinRule, SpaceChildLinkOutcome, compose_sidebar_for_state};

pub(super) async fn verify(conn_a: &mut CoreConnection) -> Result<(), String> {
    let space_id = create_space_for_qa(conn_a, "Koushi Open Space", "unnamed space create").await?;
    wait_for_space_in_space_list(conn_a, &space_id, "unnamed space list").await?;
    let update_id = conn_a.next_request_id();
    conn_a
        .command(CoreCommand::Room(RoomCommand::UpdateRoomSetting {
            request_id: update_id,
            room_id: space_id.clone(),
            change: RoomSettingChange::JoinRule(RoomJoinRule::Public),
        }))
        .await
        .map_err(|_| "unnamed: Space join rule submission failed")?;
    wait_for_room_setting_updated(conn_a, update_id, "unnamed Space public").await?;
    wait_for_state(conn_a, "unnamed: public Space rule sync", |state| {
        state.spaces.iter().any(|space| {
            space.space_id == space_id && space.join_rule == Some(RoomJoinRule::Public)
        })
    })
    .await?;
    select_space(conn_a, Some(&space_id)).await?;
    let defaults = compose_sidebar_for_state(&conn_a.snapshot()).create_room_defaults;
    if defaults.visibility != CreateRoomVisibility::Public || !defaults.encrypted {
        return Err("unnamed: a public Space did not default to a public room".into());
    }
    println!("room_public_space_default=ok");

    // Unnamed, no address: public, listed, linked, calculated name.
    let preview = conn_a.preview_room_address("", None);
    if preview.error.is_some() || !preview.without_address {
        return Err("unnamed: an unnamed room was not allowed without an address".into());
    }
    let options = |alias_localpart: Option<String>| CreateRoomOptions {
        name: String::new(),
        topic: None,
        alias_localpart,
        encrypted: false,
        invited_only: false,
        visibility: CreateRoomVisibility::Public,
        parent_space: Some(CreateRoomParentSpace {
            space_id: space_id.clone(),
        }),
        access_policy: None,
        history: None,
    };
    let unnamed_id = create(conn_a, options(None), "unnamed public").await?;
    wait_for_linked(conn_a, &space_id, &unnamed_id).await?;
    let settings = load_room_settings_for_qa(conn_a, &unnamed_id, "unnamed settings").await?;
    if settings.join_rule != RoomJoinRule::Public
        || settings.canonical_alias.is_some()
        || settings.name.is_some()
    {
        return Err(format!(
            "unnamed: unexpected state (join_rule={:?}, has_alias={}, has_name={})",
            settings.join_rule,
            settings.canonical_alias.is_some(),
            settings.name.is_some()
        ));
    }
    let calculated =
        wait_for_display_name(conn_a, &unnamed_id, "unnamed calculated name", |name| {
            !name.trim().is_empty() && name != unnamed_id
        })
        .await?;
    // Link acknowledgement and the room name can precede the Space's synced
    // child projection. Wait for the exact sidebar row, not a fixed delay.
    wait_for_state(conn_a, "unnamed: Space calculated-name row", |state| {
        compose_sidebar_for_state(state)
            .space_rooms
            .iter()
            .any(|row| row.room_id == unnamed_id && row.display_name == calculated)
    })
    .await?;
    println!("room_unnamed_public_space=ok");

    // An entered address becomes the canonical alias, which names an unnamed
    // room ahead of any member-derived name (the SDK shows its localpart).
    let alias_localpart = format!("koushi-unnamed-{}", std::process::id());
    let aliased_id = create(
        conn_a,
        options(Some(alias_localpart.clone())),
        "unnamed alias",
    )
    .await?;
    wait_for_linked(conn_a, &space_id, &aliased_id).await?;
    wait_for_display_name(conn_a, &aliased_id, "unnamed alias name", |name| {
        name == alias_localpart
    })
    .await?;
    println!("room_unnamed_alias_name=ok");

    let conflict_id = conn_a.next_request_id();
    conn_a
        .command(CoreCommand::Room(RoomCommand::CreateRoom {
            request_id: conflict_id,
            options: options(Some(alias_localpart.clone())),
        }))
        .await
        .map_err(|_| "unnamed: conflict submission failed")?;
    let deadline = QaEventDeadline::after(EVENT_TIMEOUT);
    loop {
        match deadline
            .recv(conn_a)
            .await
            .map_err(|_| "unnamed: conflict outcome timeout")?
            .map_err(|_| "unnamed: conflict event stream lagged")?
        {
            CoreEvent::OperationFailed {
                request_id,
                failure:
                    CoreFailure::RoomOperationFailed {
                        kind: RoomFailureKind::AliasInUse,
                    },
            } if request_id == conflict_id => break,
            CoreEvent::OperationFailed { request_id, .. } if request_id == conflict_id => {
                return Err("unnamed: the conflict did not report AliasInUse".into());
            }
            CoreEvent::Room(RoomEvent::RoomCreated { request_id, .. })
                if request_id == conflict_id =>
            {
                return Err("unnamed: a duplicate address was accepted".into());
            }
            _ => {}
        }
    }
    println!("room_unnamed_alias_conflict=ok");

    // Naming the room later replaces the calculated name.
    let rename_id = conn_a.next_request_id();
    conn_a
        .command(CoreCommand::Room(RoomCommand::UpdateRoomSetting {
            request_id: rename_id,
            room_id: unnamed_id.clone(),
            change: RoomSettingChange::Name(Some("Koushi Named Later".to_owned())),
        }))
        .await
        .map_err(|_| "unnamed: rename submission failed")?;
    wait_for_room_setting_updated(conn_a, rename_id, "unnamed rename").await?;
    wait_for_display_name(conn_a, &unnamed_id, "unnamed renamed", |name| {
        name == "Koushi Named Later"
    })
    .await?;
    println!("room_unnamed_rename=ok");

    // Choosing private in a public Space keeps the Space-member default.
    let private_id = create(
        conn_a,
        CreateRoomOptions {
            name: "Koushi Private In Open Space".to_owned(),
            encrypted: true,
            visibility: CreateRoomVisibility::Private,
            ..options(None)
        },
        "public Space private choice",
    )
    .await?;
    wait_for_linked(conn_a, &space_id, &private_id).await?;
    let private = load_room_settings_for_qa(conn_a, &private_id, "private choice settings").await?;
    if private.join_rule != RoomJoinRule::Restricted || private.canonical_alias.is_some() {
        return Err(format!(
            "unnamed: the private choice was not a restricted room (join_rule={:?})",
            private.join_rule
        ));
    }
    println!("room_public_space_private_choice=ok");
    select_space(conn_a, None).await
}

async fn create(
    conn: &mut CoreConnection,
    options: CreateRoomOptions,
    label: &str,
) -> Result<String, String> {
    let request_id = conn.next_request_id();
    conn.command(CoreCommand::Room(RoomCommand::CreateRoom {
        request_id,
        options,
    }))
    .await
    .map_err(|_| format!("{label}: create submission failed"))?;
    let room_id = wait_for_room_created(conn, request_id, label).await?;
    wait_for_room_in_room_list(conn, &room_id, label).await?;
    Ok(room_id)
}

async fn wait_for_linked(
    conn: &mut CoreConnection,
    space_id: &str,
    room_id: &str,
) -> Result<(), String> {
    let mut outcome = None;
    wait_for_state(conn, "unnamed: Space link settlement", |state| {
        outcome = state
            .space_child_links
            .latest(space_id, room_id)
            .map(|result| result.outcome);
        outcome.is_some()
    })
    .await?;
    match outcome {
        Some(SpaceChildLinkOutcome::Linked) => Ok(()),
        _ => Err("unnamed: the room was not linked to its Space".into()),
    }
}

async fn wait_for_display_name(
    conn: &mut CoreConnection,
    room_id: &str,
    label: &str,
    accept: impl Fn(&str) -> bool,
) -> Result<String, String> {
    let mut found = None;
    wait_for_state(conn, label, |state| {
        found = state
            .rooms
            .iter()
            .find(|room| room.room_id == room_id)
            .map(|room| room.display_name.clone())
            .filter(|name| accept(name));
        found.is_some()
    })
    .await?;
    found.ok_or_else(|| format!("{label}: display name missing"))
}

async fn wait_for_state(
    conn: &mut CoreConnection,
    label: &str,
    mut ready: impl FnMut(&AppState) -> bool,
) -> Result<(), String> {
    let deadline = tokio::time::Instant::now() + ROOM_LIST_EVENT_TIMEOUT;
    while !ready(&conn.snapshot()) {
        tokio::time::timeout_at(deadline, conn.recv_event())
            .await
            .map_err(|_| format!("{label}: timeout"))?
            .map_err(|_| format!("{label}: event stream lagged"))?;
    }
    Ok(())
}

async fn select_space(conn: &mut CoreConnection, space_id: Option<&str>) -> Result<(), String> {
    let request_id = conn.next_request_id();
    conn.command(CoreCommand::Room(RoomCommand::SelectSpace {
        request_id,
        space_id: space_id.map(str::to_owned),
    }))
    .await
    .map_err(|_| "unnamed: select Space submission failed")?;
    wait_for_state(conn, "unnamed: select Space", |state| {
        state.navigation.active_space_id.as_deref() == space_id
    })
    .await
}
