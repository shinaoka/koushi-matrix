use super::*;
use koushi_core::{CreateRoomOptions, CreateRoomVisibility};
use matrix_sdk::ruma::{MatrixToUri, matrix_uri::MatrixId};

pub(super) async fn verify(
    config: &QaConfig,
    conn_a: &mut CoreConnection,
    conn_b: &mut CoreConnection,
) -> Result<(), String> {
    let name = format!("Koushi Address QA {}", std::process::id());
    let preview = conn_a.preview_room_address(&name, None);
    let expected_alias = preview
        .full_alias
        .ok_or("address: suggestion was invalid")?;
    let options = CreateRoomOptions {
        name,
        topic: None,
        alias_localpart: Some(preview.localpart),
        encrypted: false,
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
    Ok(())
}
