use super::cleanup::cleanup_logged_in_runtime;
use super::diagnostics::QaTcpProxy;
use super::event_wait::wait_for_room_in_room_list;
use super::participants::{QaParticipantLoginGate, login_synced_participant_for_qa, qa_data_dir};
use super::registry::EVENT_TIMEOUT;
use super::*;
use koushi_protocol::view::{
    ReaderWindow, ReaderWindowLimit, ReceiptSourceRef, TimelineViewSource, ViewDelivery, ViewModel,
};

pub(super) async fn run_avatar_demand_scenario(config: &QaConfig) -> Result<(), String> {
    let metadata: serde_json::Value = serde_json::from_str(
        &std::env::var("KOUSHI_QA_AVATAR_FIXTURE")
            .map_err(|_| "avatar fixture metadata missing".to_owned())?,
    )
    .map_err(|_| "avatar fixture metadata invalid".to_owned())?;
    if metadata["readerCount"].as_u64() != Some(1500) {
        return Err("avatar fixture population mismatch".to_owned());
    }
    let room = metadata["roomId"]
        .as_str()
        .ok_or("avatar fixture room missing")?;
    let event = metadata["eventId"]
        .as_str()
        .ok_or("avatar fixture event missing")?;
    let proxy = QaTcpProxy::start(&config.homeserver)?;
    let participant = login_synced_participant_for_qa(
        &proxy.homeserver_url(),
        qa_data_dir("avatar-demand"),
        &config.user_a,
        &config.password_a,
        "Koushi Avatar QA",
        "avatar login",
        "avatar gate",
        QaParticipantLoginGate::BootstrapNewIdentity,
    )
    .await?;
    let super::participants::QaParticipantLoginOutcome {
        runtime,
        mut conn,
        account_key,
        ..
    } = participant;
    let result = run_window(&mut conn, &account_key, room, event, &proxy).await;
    let cleanup = cleanup_logged_in_runtime(conn, runtime, account_key, "avatar cleanup").await;
    result?;
    cleanup?;
    println!("avatar_window_requests=ok");
    Ok(())
}

async fn run_window(
    conn: &mut CoreConnection,
    account: &AccountKey,
    room: &str,
    event: &str,
    proxy: &QaTcpProxy,
) -> Result<(), String> {
    wait_for_room_in_room_list(conn, room, "avatar room").await?;
    // Explicit fixture metadata preparation through a normal product command;
    // viewport resolution itself must not acquire all member images.
    let settings_id = conn.next_request_id();
    conn.command(CoreCommand::Room(RoomCommand::LoadRoomSettings {
        request_id: settings_id,
        room_id: room.to_owned(),
    }))
    .await
    .map_err(|_| "avatar settings submission failed".to_owned())?;
    tokio::time::timeout(EVENT_TIMEOUT, async {
        loop {
            match conn
                .recv_event()
                .await
                .map_err(|_| "avatar settings stream lagged".to_owned())?
            {
                CoreEvent::Room(RoomEvent::RoomSettingsLoaded { request_id, .. })
                    if request_id == settings_id =>
                {
                    return Ok::<_, String>(());
                }
                CoreEvent::OperationFailed { request_id, .. } if request_id == settings_id => {
                    return Err("avatar settings failed".to_owned());
                }
                _ => {}
            }
        }
    })
    .await
    .map_err(|_| "avatar settings timed out".to_owned())??;
    let key = TimelineKey::room(account.clone(), room.to_owned());
    // As in the live-signals reader check, a committed replay can omit the
    // original projection identity. Acquire a fresh source, never guess it.
    let unsubscribe = conn.next_request_id();
    conn.command(CoreCommand::Timeline(TimelineCommand::Unsubscribe {
        request_id: unsubscribe,
        key: key.clone(),
    }))
    .await
    .map_err(|_| "avatar timeline retirement failed".to_owned())?;
    let subscribe = conn.next_request_id();
    conn.command(CoreCommand::Timeline(TimelineCommand::Subscribe {
        request_id: subscribe,
        key: key.clone(),
        initial_backfill: koushi_protocol::command::InitialBackfillPolicy::Disabled,
    }))
    .await
    .map_err(|_| "avatar timeline submission failed".to_owned())?;
    let source = tokio::time::timeout(EVENT_TIMEOUT, async {
        loop {
            if let CoreEvent::Timeline(TimelineEvent::InitialItems {
                request_id: Some(projection_request_id),
                cause_request_id: Some(cause),
                key: actual_key,
                generation,
                ..
            }) = conn
                .recv_event()
                .await
                .map_err(|_| "avatar timeline stream lagged".to_owned())?
            {
                if cause == subscribe && actual_key == key {
                    return Ok::<_, String>(ReceiptSourceRef {
                        timeline: TimelineViewSource {
                            key: actual_key,
                            projection_request_id,
                            generation,
                        },
                        event_id: event.to_owned(),
                    });
                }
            }
        }
    })
    .await
    .map_err(|_| "avatar timeline timed out".to_owned())??;
    let mut targets: Vec<String> = Vec::new();
    for phase in ["initial", "reopen"] {
        let mut reader = conn
            .subscribe_reader(source.clone(), 0, ReaderWindowLimit::try_from(32).unwrap())
            .map_err(|_| "avatar reader admission failed".to_owned())?;
        let (revision, initial) = next_window(&mut reader).await?;
        if initial.total_count < 1500 || initial.rows.len() > 32 {
            return Err(format!(
                "avatar reader population/window mismatch total={} rows={}",
                initial.total_count,
                initial.rows.len()
            ));
        }
        if targets.is_empty() {
            targets = initial
                .rows
                .iter()
                .filter(|row| row.avatar.is_some())
                .take(16)
                .map(|row| row.user_id.clone())
                .collect();
            if targets.len() != 16 {
                return Err("avatar fixture visible metadata missing".to_owned());
            }
            if proxy.media_read_forwarded_count() != 0 {
                return Err("avatar images fetched before observation".to_owned());
            }
        }
        reader
            .observe_avatars(revision, 1, &targets[..8], &targets[8..])
            .map_err(|_| "avatar observation rejected".to_owned())?;
        tokio::time::timeout(EVENT_TIMEOUT, async {
            let mut window = initial;
            let mut revision = revision;
            loop {
                let mut ready = 0;
                for user in &targets {
                    if let Some(koushi_state::AvatarThumbnailState::Ready { source_ref, .. }) =
                        window
                            .rows
                            .iter()
                            .find(|row| &row.user_id == user)
                            .and_then(|row| row.avatar.as_ref())
                    {
                        let bytes = reader
                            .resource_content(revision, source_ref)
                            .map_err(|_| "avatar bytes rejected".to_owned())?
                            .ok_or("avatar bytes missing")?;
                        if !bytes.bytes.starts_with(&[137, 80, 78, 71, 13, 10, 26, 10]) {
                            return Err("avatar PNG invalid".to_owned());
                        }
                        ready += 1;
                    }
                }
                if ready == targets.len() {
                    return Ok::<_, String>(());
                }
                (revision, window) = next_window(&mut reader).await?;
            }
        })
        .await
        .map_err(|_| "avatar window Ready timed out".to_owned())??;
        tokio::time::sleep(Duration::from_millis(250)).await;
        let requests = proxy.media_read_forwarded_count();
        if requests != 16 {
            return Err(format!(
                "avatar HTTP bound failed phase={phase} requests={requests}"
            ));
        }
        println!(
            "avatar_population=1500 visible=8 prefetch=8 phase={phase} media_http_requests={requests}"
        );
        reader.close_handle().close();
    }
    Ok(())
}

async fn next_window(
    reader: &mut koushi_core::runtime::ReaderSubscription,
) -> Result<(koushi_protocol::view::ViewRevision, ReaderWindow), String> {
    let mut observed_total = 0;
    tokio::time::timeout(EVENT_TIMEOUT, async {
        loop {
            match reader.next_delivery().await {
                Some(ViewDelivery::Model {
                    revision, model, ..
                }) => {
                    reader
                        .ack_model(revision)
                        .map_err(|_| "avatar model ACK failed".to_owned())?;
                    if let ViewModel::ReaderReady(window) = model {
                        observed_total = window.total_count;
                        if window.total_count >= 1500 {
                            return Ok((revision, window));
                        }
                    }
                }
                _ => return Err("avatar reader retired".to_owned()),
            }
        }
    })
    .await
    .map_err(|_| format!("avatar model timed out observed_total={observed_total}"))?
}
