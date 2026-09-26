use super::*;
use koushi_core::{CreateRoomOptions, CreateRoomParentSpace, CreateRoomVisibility};
use koushi_state::{RoomAddressAvailability, RoomAddressAvailabilityState, SpaceChildLinkOutcome};
use matrix_sdk::ruma::{MatrixToUri, matrix_uri::MatrixId};

pub(super) async fn verify(
    config: &QaConfig,
    conn_a: &mut CoreConnection,
    conn_b: &mut CoreConnection,
) -> Result<(), String> {
    let name = format!("Koushi Address QA {}", std::process::id());
    // Suggest from Home: no Space prefix for this unrelated room.
    select_space_for_address_qa(conn_a, None).await?;
    let preview = conn_a.preview_room_address(&name, None);
    let expected_alias = preview
        .full_alias
        .ok_or("address: suggestion was invalid")?;
    let options = CreateRoomOptions {
        name: name.clone(),
        topic: None,
        alias_localpart: Some(preview.localpart),
        encrypted: false,
        invited_only: false,
        visibility: CreateRoomVisibility::Public,
        parent_space: None,
    };
    let request_id = conn_a.next_request_id();
    conn_a
        .command(CoreCommand::Room(RoomCommand::CreateRoom {
            request_id,
            options: options.clone(),
        }))
        .await
        .map_err(|_| "address: create submission failed")?;
    let room_id = wait_for_room_created(conn_a, request_id, "address create").await?;
    wait_for_room_in_room_list(conn_a, &room_id, "address room list").await?;
    // RoomCreated/list insertion can precede the first canonical-alias sync on
    // Synapse. Observe that state, rather than treating command completion as sync.
    let settings = tokio::time::timeout(EVENT_TIMEOUT, async {
        for _ in 0..6 {
            let settings = load_room_settings_for_qa(conn_a, &room_id, "address settings").await?;
            if settings.canonical_alias.is_some() {
                return Ok::<_, String>(settings);
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
        Err("address: canonical alias missing after bounded sync observation".into())
    })
    .await
    .map_err(|_| "address: canonical alias sync timeout")??;
    if settings.canonical_alias.as_deref() != Some(expected_alias.as_str()) {
        return Err("address: created canonical alias differs from preview".into());
    }
    let link = settings.share_link.ok_or("address: share link missing")?;
    let uri = MatrixToUri::parse(&link).map_err(|_| "address: invalid share URI")?;
    let MatrixId::RoomAlias(alias) = uri.id() else {
        return Err("address: public room link does not contain its alias".into());
    };
    if alias.as_str() != expected_alias {
        return Err("address: share link targets the wrong alias".into());
    }
    join_directory_room_for_qa(
        conn_b,
        alias.as_str(),
        &config.server_name,
        &room_id,
        "address share join",
    )
    .await?;
    println!("room_address_preview_create_share=ok");

    let collision_id = conn_a.next_request_id();
    conn_a
        .command(CoreCommand::Room(RoomCommand::CreateRoom {
            request_id: collision_id,
            options,
        }))
        .await
        .map_err(|_| "address: collision submission failed")?;
    let deadline = QaEventDeadline::after(EVENT_TIMEOUT);
    loop {
        let event = deadline
            .recv(conn_a)
            .await
            .map_err(|_| "address: collision outcome timeout")?
            .map_err(|_| "address: collision event stream lagged")?;
        match event {
            CoreEvent::OperationFailed {
                request_id,
                failure:
                    CoreFailure::RoomOperationFailed {
                        kind: RoomFailureKind::AliasInUse,
                    },
            } if request_id == collision_id => break,
            CoreEvent::OperationFailed { request_id, .. } if request_id == collision_id => {
                return Err("address: collision did not retain AliasInUse".into());
            }
            CoreEvent::Room(RoomEvent::RoomCreated { request_id, .. })
                if request_id == collision_id =>
            {
                return Err("address: duplicate alias unexpectedly succeeded".into());
            }
            _ => {}
        }
    }
    println!("room_address_collision=ok");

    verify_advisory_availability(conn_a, &expected_alias).await?;
    println!("room_address_availability=ok");

    verify_space_prefixed_address(conn_a, &name, &expected_alias).await?;
    println!("room_address_space_prefix=ok");
    Ok(())
}

/// #1006: the unrelated room above owns the room-only address. Creating a
/// public room with the same display name from a Space conflicts at that
/// address (Spaces share the server's alias namespace), while the default
/// `<space>-<room>` suggestion creates it, keeps the display name, and links
/// it to the Space.
async fn verify_space_prefixed_address(
    conn_a: &mut CoreConnection,
    name: &str,
    taken_alias: &str,
) -> Result<(), String> {
    let space_id =
        create_space_for_qa(conn_a, "Koushi Address Space", "address space create").await?;
    wait_for_space_in_space_list(conn_a, &space_id, "address space list").await?;
    select_space_for_address_qa(conn_a, Some(&space_id)).await?;

    let prefixed = conn_a.preview_room_address(name, None);
    let prefixed_alias = prefixed
        .full_alias
        .clone()
        .ok_or("address: Space suggestion was invalid")?;
    if !prefixed.localpart.starts_with("koushi-address-space-") || prefixed_alias == taken_alias {
        return Err("address: Space suggestion did not carry the Space prefix".into());
    }
    let taken_localpart = taken_alias
        .trim_start_matches('#')
        .split(':')
        .next()
        .unwrap_or_default()
        .to_owned();
    let options = |alias_localpart: String| CreateRoomOptions {
        name: name.to_owned(),
        topic: None,
        alias_localpart: Some(alias_localpart),
        encrypted: false,
        invited_only: false,
        visibility: CreateRoomVisibility::Public,
        parent_space: Some(CreateRoomParentSpace {
            space_id: space_id.clone(),
        }),
    };

    let conflict_id = conn_a.next_request_id();
    conn_a
        .command(CoreCommand::Room(RoomCommand::CreateRoom {
            request_id: conflict_id,
            options: options(taken_localpart),
        }))
        .await
        .map_err(|_| "address: Space conflict submission failed")?;
    let deadline = QaEventDeadline::after(EVENT_TIMEOUT);
    loop {
        match deadline
            .recv(conn_a)
            .await
            .map_err(|_| "address: Space conflict outcome timeout")?
            .map_err(|_| "address: Space conflict event stream lagged")?
        {
            CoreEvent::OperationFailed {
                request_id,
                failure:
                    CoreFailure::RoomOperationFailed {
                        kind: RoomFailureKind::AliasInUse,
                    },
            } if request_id == conflict_id => break,
            CoreEvent::OperationFailed { request_id, .. } if request_id == conflict_id => {
                return Err("address: Space conflict did not report AliasInUse".into());
            }
            CoreEvent::Room(RoomEvent::RoomCreated { request_id, .. })
                if request_id == conflict_id =>
            {
                return Err("address: an address taken outside the Space was reused".into());
            }
            _ => {}
        }
    }

    let create_id = conn_a.next_request_id();
    conn_a
        .command(CoreCommand::Room(RoomCommand::CreateRoom {
            request_id: create_id,
            options: options(prefixed.localpart.clone()),
        }))
        .await
        .map_err(|_| "address: Space room submission failed")?;
    let room_id = wait_for_room_created(conn_a, create_id, "address Space room").await?;
    wait_for_room_in_room_list(conn_a, &room_id, "address Space room list").await?;
    // The room list can show a computed name until the name event syncs.
    let named = |state: &AppState| {
        state
            .rooms
            .iter()
            .any(|room| room.room_id == room_id && room.display_name == name)
    };
    let deadline = tokio::time::Instant::now() + EVENT_TIMEOUT;
    while !named(&conn_a.snapshot()) {
        tokio::time::timeout_at(deadline, conn_a.recv_event())
            .await
            .map_err(|_| "address: the Space room did not keep its display name")?
            .map_err(|_| "address: Space room name event stream lagged")?;
    }
    let deadline = tokio::time::Instant::now() + EVENT_TIMEOUT;
    loop {
        let linked = conn_a
            .snapshot()
            .space_child_links
            .latest(&space_id, &room_id)
            .map(|result| result.outcome);
        match linked {
            Some(SpaceChildLinkOutcome::Linked) => break,
            Some(SpaceChildLinkOutcome::Failed { .. }) => {
                return Err("address: the Space room was not linked to its Space".into());
            }
            None => {
                tokio::time::timeout_at(deadline, conn_a.recv_event())
                    .await
                    .map_err(|_| "address: Space link settlement timeout")?
                    .map_err(|_| "address: Space link event stream lagged")?;
            }
        }
    }
    let settings =
        load_room_settings_for_qa(conn_a, &room_id, "address Space room settings").await?;
    if settings
        .canonical_alias
        .as_deref()
        .is_some_and(|alias| alias != prefixed_alias)
    {
        return Err("address: the Space room's alias differs from its preview".into());
    }
    select_space_for_address_qa(conn_a, None).await
}

/// #1006: the advisory check reports the address taken above as in use with
/// an unchecked alternative, and an unused address as available.
async fn verify_advisory_availability(
    conn_a: &mut CoreConnection,
    taken_alias: &str,
) -> Result<(), String> {
    let taken_localpart = taken_alias
        .trim_start_matches('#')
        .split(':')
        .next()
        .unwrap_or_default()
        .to_owned();
    let taken = check_address_for_qa(conn_a, &taken_localpart).await?;
    match taken {
        RoomAddressAvailabilityState::Checked {
            availability: RoomAddressAvailability::InUse,
            suggestion: Some(suggestion),
            ..
        } if suggestion.localpart != taken_localpart => {}
        _ => {
            return Err(
                "address: a taken address was not reported in use with a suggestion".into(),
            );
        }
    }
    let free_localpart = format!("koushi-free-{}", std::process::id());
    match check_address_for_qa(conn_a, &free_localpart).await? {
        RoomAddressAvailabilityState::Checked {
            availability: RoomAddressAvailability::Available,
            suggestion: None,
            ..
        } => {}
        _ => return Err("address: an unused address was not reported available".into()),
    }
    let clear_id = conn_a.next_request_id();
    conn_a
        .command(CoreCommand::Room(
            RoomCommand::ClearRoomAddressAvailability {
                request_id: clear_id,
            },
        ))
        .await
        .map_err(|_| "address: clear availability submission failed")?;
    let deadline = tokio::time::Instant::now() + EVENT_TIMEOUT;
    while conn_a.snapshot().room_address_availability != RoomAddressAvailabilityState::Idle {
        tokio::time::timeout_at(deadline, conn_a.recv_event())
            .await
            .map_err(|_| "address: clear availability timeout")?
            .map_err(|_| "address: clear availability event stream lagged")?;
    }
    Ok(())
}

async fn check_address_for_qa(
    conn: &mut CoreConnection,
    localpart: &str,
) -> Result<RoomAddressAvailabilityState, String> {
    let request_id = conn.next_request_id();
    conn.command(CoreCommand::Room(
        RoomCommand::CheckRoomAddressAvailability {
            request_id,
            alias_localpart: localpart.to_owned(),
        },
    ))
    .await
    .map_err(|_| "address: availability submission failed")?;
    let settled = |state: &AppState| match &state.room_address_availability {
        checked @ RoomAddressAvailabilityState::Checked {
            request_id: settled_id,
            ..
        } if *settled_id == request_id.sequence => Some(checked.clone()),
        _ => None,
    };
    let deadline = tokio::time::Instant::now() + EVENT_TIMEOUT;
    loop {
        if let Some(state) = settled(&conn.snapshot()) {
            return Ok(state);
        }
        tokio::time::timeout_at(deadline, conn.recv_event())
            .await
            .map_err(|_| "address: availability settlement timeout")?
            .map_err(|_| "address: availability event stream lagged")?;
    }
}

async fn select_space_for_address_qa(
    conn: &mut CoreConnection,
    space_id: Option<&str>,
) -> Result<(), String> {
    let request_id = conn.next_request_id();
    conn.command(CoreCommand::Room(RoomCommand::SelectSpace {
        request_id,
        space_id: space_id.map(str::to_owned),
    }))
    .await
    .map_err(|_| "address: select Space submission failed")?;
    let selected = |state: &AppState| state.navigation.active_space_id.as_deref() == space_id;
    let deadline = tokio::time::Instant::now() + EVENT_TIMEOUT;
    while !selected(&conn.snapshot()) {
        tokio::time::timeout_at(deadline, conn.recv_event())
            .await
            .map_err(|_| "address: select Space timeout")?
            .map_err(|_| "address: select Space event stream lagged")?;
    }
    Ok(())
}
