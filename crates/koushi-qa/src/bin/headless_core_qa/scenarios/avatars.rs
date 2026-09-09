use super::cleanup::cleanup_logged_in_runtime;
use super::diagnostics::QaTcpProxy;
use super::event_wait::wait_for_room_in_room_list;
use super::participants::{QaParticipantLoginGate, login_synced_participant_for_qa, qa_data_dir};
use super::registry::EVENT_TIMEOUT;
use super::*;
use koushi_protocol::view::{
    ReaderWindow, ReaderWindowLimit, ReaderWindowRequest, ReaderWindowTarget, ReceiptSourceRef,
    TimelineViewSource, ViewDelivery, ViewModel,
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
    let result = run_window(&mut conn, &account_key, room, event, &proxy)
        .await
        .map_err(|error| {
            format!(
                "{error} media_http_requests={}",
                proxy.media_read_forwarded_count()
            )
        });
    proxy.release_media_responses();
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
    let limit = ReaderWindowLimit::try_from(32).unwrap();
    let mut reader = conn
        .subscribe_reader(source.clone(), 0, limit)
        .map_err(|_| "avatar reader admission failed".to_owned())?;
    let (mut revision, mut initial) = next_window(&mut reader, 0).await?;
    let mut first_targets: Vec<String> = Vec::new();
    for (sequence, phase, start, expected_requests) in [
        (1, "initial", 0, 16),
        (2, "scroll", 32, 32),
        (3, "return", 0, 32),
        (1, "reopen", 0, 32),
    ] {
        if phase == "reopen" {
            let closed_control = reader.control();
            reader.close_handle().close();
            drop(reader);
            if closed_control.observe_avatars(revision, u64::MAX, &[], &[])
                != Err(koushi_core::runtime::ScopeError::Closed)
            {
                return Err("avatar closed control accepted late observation".to_owned());
            }
            reader = conn
                .subscribe_reader(source.clone(), start, limit)
                .map_err(|_| "avatar reader reopen failed".to_owned())?;
            (revision, initial) = next_window(&mut reader, start).await?;
        } else if phase != "initial" {
            let previous_revision = revision;
            let previous_targets: Vec<_> = initial
                .rows
                .iter()
                .filter(|row| row.avatar.is_some())
                .take(16)
                .map(|row| row.user_id.clone())
                .collect();
            reader
                .update_window(ReaderWindowRequest {
                    installed_revision: revision,
                    sequence,
                    target: ReaderWindowTarget::Index { start },
                    limit,
                })
                .map_err(|_| "avatar window update rejected".to_owned())?;
            (revision, initial) = next_window(&mut reader, start).await?;
            // A rejected stale model must not consume even the largest host
            // sequence; the ordinary lower sequence below must still succeed.
            if reader.observe_avatars(
                previous_revision,
                u64::MAX,
                &previous_targets[..8],
                &previous_targets[8..],
            ) != Err(koushi_core::runtime::ScopeError::InvalidRevision)
            {
                return Err("avatar stale revision was not rejected".to_owned());
            }
        }
        if initial.total_count < 1500 || initial.rows.len() > 32 {
            return Err(format!(
                "avatar reader population/window mismatch total={} rows={}",
                initial.total_count,
                initial.rows.len()
            ));
        }
        let targets: Vec<String> = initial
            .rows
            .iter()
            .filter(|row| row.avatar.is_some())
            .take(16)
            .map(|row| row.user_id.clone())
            .collect();
        if targets.len() != 16 {
            return Err("avatar fixture visible metadata missing".to_owned());
        }
        match phase {
            "initial" => {
                first_targets = targets.clone();
                if proxy.media_read_forwarded_count() != 0 {
                    return Err("avatar images fetched before observation".to_owned());
                }
            }
            "scroll" if targets.iter().any(|target| first_targets.contains(target)) => {
                return Err("avatar scroll did not reach disjoint identities".to_owned());
            }
            "return" | "reopen" if targets != first_targets => {
                return Err("avatar return identities changed".to_owned());
            }
            _ => {}
        }
        if matches!(phase, "scroll" | "return")
            && reader.observe_avatars(revision, sequence - 1, &targets[..8], &targets[8..])
                != Err(koushi_core::runtime::ScopeError::InvalidRevision)
        {
            return Err("avatar stale sequence was not rejected".to_owned());
        }
        reader
            .observe_avatars(revision, sequence, &targets[..8], &targets[8..])
            .map_err(|_| "avatar observation rejected".to_owned())?;
        (revision, initial) = tokio::time::timeout(EVENT_TIMEOUT, async {
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
                    return Ok::<_, String>((revision, window));
                }
                (revision, window) = next_window(&mut reader, start).await?;
            }
        })
        .await
        .map_err(|_| "avatar window Ready timed out".to_owned())??;
        tokio::time::sleep(Duration::from_millis(250)).await;
        let requests = proxy.media_read_forwarded_count();
        if requests != expected_requests {
            return Err(format!(
                "avatar HTTP bound failed phase={phase} requests={requests}"
            ));
        }
        println!(
            "avatar_population=1500 visible=8 prefetch=8 phase={phase} media_http_requests={requests}"
        );
    }
    reader.close_handle().close();
    drop(reader);
    verify_shared_cancellation(conn, source, proxy).await
}

async fn verify_shared_cancellation(
    conn: &CoreConnection,
    source: ReceiptSourceRef,
    proxy: &QaTcpProxy,
) -> Result<(), String> {
    let limit = ReaderWindowLimit::try_from(16).unwrap();
    let mut first = conn
        .subscribe_reader(source.clone(), 64, limit)
        .map_err(|_| "avatar cancellation scope rejected".to_owned())?;
    let (revision, window) = next_window(&mut first, 64).await?;
    let targets: Vec<_> = window
        .rows
        .iter()
        .filter(|row| row.avatar.is_some())
        .map(|row| row.user_id.clone())
        .collect();
    if targets.len() != 16
        || window.rows.iter().any(|row| {
            matches!(
                row.avatar,
                Some(koushi_state::AvatarThumbnailState::Ready { .. })
            )
        })
    {
        return Err("avatar cancellation fixture is not cold".to_owned());
    }
    let before = proxy.media_read_forwarded_count();
    let closed_before = proxy.media_peer_closed_count();
    proxy.hold_media_responses();
    first
        .observe_avatars(revision, 1, &targets[..8], &targets[8..])
        .map_err(|_| "avatar cancellation observation rejected".to_owned())?;
    wait_media_connections(proxy, 6, closed_before).await?;
    if proxy.media_read_forwarded_count() != before + 6 {
        return Err("avatar active request bound exceeded".to_owned());
    }
    let mut shared = conn
        .subscribe_reader(source, 64, limit)
        .map_err(|_| "avatar shared cancellation scope rejected".to_owned())?;
    let (revision, window) = next_window(&mut shared, 64).await?;
    if window
        .rows
        .iter()
        .map(|row| &row.user_id)
        .ne(targets.iter())
    {
        return Err("avatar shared cancellation identities changed".to_owned());
    }
    shared
        .observe_avatars(revision, 1, &targets[..8], &targets[8..])
        .map_err(|_| "avatar shared cancellation observation rejected".to_owned())?;
    first.close_handle().close();
    drop(first);
    tokio::time::sleep(Duration::from_millis(250)).await;
    if proxy.media_read_forwarded_count() != before + 6
        || proxy.media_responses_held_count() != 6
        || proxy.media_peer_closed_count() != closed_before
    {
        return Err("avatar shared active work was interrupted or duplicated".to_owned());
    }
    shared.close_handle().close();
    drop(shared);
    wait_media_connections(proxy, 0, closed_before + 6).await?;
    proxy.release_media_responses();
    tokio::time::sleep(Duration::from_millis(250)).await;
    if proxy.media_read_forwarded_count() != before + 6 {
        return Err("avatar queued work escaped cancellation".to_owned());
    }
    println!(
        "avatar_shared_inflight=ok avatar_cancelled_connections=6 media_http_requests={}",
        proxy.media_read_forwarded_count()
    );
    Ok(())
}

async fn wait_media_connections(
    proxy: &QaTcpProxy,
    held: usize,
    closed: usize,
) -> Result<(), String> {
    tokio::time::timeout(EVENT_TIMEOUT, async {
        while proxy.media_responses_held_count() != held
            || proxy.media_peer_closed_count() != closed
        {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .map_err(|_| {
        format!(
            "avatar connection wait timed out held={} closed={}",
            proxy.media_responses_held_count(),
            proxy.media_peer_closed_count()
        )
    })
}

async fn next_window(
    reader: &mut koushi_core::runtime::ReaderSubscription,
    expected_start: u64,
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
                        if window.total_count >= 1500 && window.start == expected_start {
                            return Ok((revision, window));
                        }
                    }
                }
                Some(ViewDelivery::Retired { reason, .. }) => {
                    return Err(format!("avatar reader retired: {reason:?}"));
                }
                None => return Err("avatar reader closed".to_owned()),
            }
        }
    })
    .await
    .map_err(|_| format!("avatar model timed out observed_total={observed_total}"))?
}
