use super::*;
use koushi_protocol::view::{
    ReaderWindowLimit, ReceiptSourceRef, TimelineViewSource, ViewDelivery, ViewModel,
};

/// Exercise the public source/ACK/observation path against the live SDK data.
/// The seeded reader must progress from an observed identity to real scoped PNG bytes.
pub(super) async fn verify_live_reader_scope(
    conn: &mut CoreConnection,
    key: &TimelineKey,
    event_id: &str,
    expected_reader: &str,
    proxy: &QaTcpProxy,
) -> Result<(), String> {
    let media_before = proxy.media_read_forwarded_count();
    // A committed replay may omit its original projection request identity.
    // Start a fresh QA subscription rather than guessing that source identity.
    let unsubscribe_id = conn.next_request_id();
    conn.command(CoreCommand::Timeline(TimelineCommand::Unsubscribe {
        request_id: unsubscribe_id,
        key: key.clone(),
    }))
    .await
    .map_err(|_| "reader scope: unsubscribe failed".to_owned())?;
    let request_id = conn.next_request_id();
    conn.command(CoreCommand::Timeline(TimelineCommand::Subscribe {
        request_id,
        key: key.clone(),
        initial_backfill: koushi_protocol::command::InitialBackfillPolicy::Disabled,
    }))
    .await
    .map_err(|_| "reader scope: subscribe failed".to_owned())?;
    let mut initial_count = 0;
    let mut matching_cause_count = 0;
    let mut projection_count = 0;
    let timeline = tokio::time::timeout(EVENT_TIMEOUT, async {
        loop {
            let event = conn
                .recv_event()
                .await
                .map_err(|_| "reader scope: timeline stream lagged".to_owned())?;
            if let CoreEvent::Timeline(TimelineEvent::InitialItems { request_id: projection, cause_request_id: cause, key: observed_key, .. }) = &event {
                if observed_key == key {
                    initial_count += 1;
                    matching_cause_count += usize::from(*cause == Some(request_id));
                    projection_count += usize::from(projection.is_some());
                }
            }
            if let CoreEvent::Timeline(TimelineEvent::InitialItems {
                request_id: Some(projection_request_id),
                cause_request_id: Some(cause),
                key: observed_key,
                generation,
                ..
            }) = event
            {
                if cause == request_id && &observed_key == key {
                    return Ok::<_, String>(TimelineViewSource {
                        key: observed_key,
                        projection_request_id,
                        generation,
                    });
                }
            }
        }
    })
    .await
    .map_err(|_| format!("reader scope: source timed out initial={initial_count} cause_matches={matching_cause_count} projection_present={projection_count}"))??;
    let mut first_scope: Option<(
        koushi_core::runtime::ReaderSubscription,
        koushi_protocol::view::ViewRevision,
    )> = None;
    for phase in ["initial", "shared", "reopen"] {
        let mut reader = conn
            .subscribe_reader(
                ReceiptSourceRef {
                    timeline: timeline.clone(),
                    event_id: event_id.to_owned(),
                },
                0,
                ReaderWindowLimit::try_from(16).unwrap(),
            )
            .map_err(|_| "reader scope: admission failed".to_owned())?;
        let mut ready_resource = None;
        let revision = tokio::time::timeout(EVENT_TIMEOUT, async {
            loop {
                match reader.next_delivery().await {
                    Some(ViewDelivery::Model {
                        revision, model, ..
                    }) => {
                        reader
                            .ack_model(revision)
                            .map_err(|_| "reader scope: ACK failed".to_owned())?;
                        if let ViewModel::ReaderReady(window) = model {
                            if let Some(row) = window
                                .rows
                                .iter()
                                .find(|row| row.user_id == expected_reader)
                            {
                                if let Some(koushi_state::AvatarThumbnailState::Ready {
                                    source_ref,
                                    ..
                                }) = &row.avatar
                                {
                                    verify_png(&reader, revision, source_ref)?;
                                    ready_resource = Some((revision, source_ref.clone()));
                                }
                                return Ok::<_, String>(revision);
                            }
                        }
                    }
                    _ => return Err("reader scope: retired before matching model".to_owned()),
                }
            }
        })
        .await
        .map_err(|_| "reader scope: model timed out".to_owned())??;
        reader
            .observe_avatars(revision, 1, &[expected_reader.to_owned()], &[])
            .map_err(|_| "reader scope: observation rejected".to_owned())?;
        if ready_resource.is_none() {
            tokio::time::timeout(EVENT_TIMEOUT, async {
                loop {
                    match reader.next_delivery().await {
                        Some(ViewDelivery::Model {
                            revision, model, ..
                        }) => {
                            reader
                                .ack_model(revision)
                                .map_err(|_| "reader avatar: Ready ACK failed".to_owned())?;
                            if let ViewModel::ReaderReady(window) = model {
                                let ready = window
                                    .rows
                                    .iter()
                                    .find(|row| row.user_id == expected_reader)
                                    .and_then(|row| row.avatar.as_ref());
                                if let Some(koushi_state::AvatarThumbnailState::Ready {
                                    source_ref,
                                    ..
                                }) = ready
                                {
                                    verify_png(&reader, revision, source_ref)?;
                                    ready_resource = Some((revision, source_ref.clone()));
                                    return Ok::<_, String>(());
                                }
                            }
                        }
                        _ => return Err("reader avatar: scope retired before Ready".to_owned()),
                    }
                }
            })
            .await
            .map_err(|_| "reader avatar: Ready timed out".to_owned())??;
        }
        let (ready_revision, source_ref) =
            ready_resource.ok_or_else(|| "reader avatar: missing Ready resource".to_owned())?;
        if let Some((first, first_revision)) = first_scope.take() {
            verify_closed(&first, first_revision)?;
            // Drop the first subscription and its model/lease owners before
            // accessing the surviving scope's independent capability.
            drop(first);
            verify_png(&reader, ready_revision, &source_ref)?;
            reader
                .observe_avatars(ready_revision, 2, &[expected_reader.to_owned()], &[])
                .map_err(|_| "reader avatar: surviving demand rejected".to_owned())?;
        }
        // Keep demand live during a bounded no-additional-HTTP observation interval.
        tokio::time::sleep(Duration::from_millis(250)).await;
        let media_reads = proxy
            .media_read_forwarded_count()
            .saturating_sub(media_before);
        if media_reads != 1 {
            return Err(format!(
                "reader avatar: expected one total HTTP request phase={phase} observed={media_reads}"
            ));
        }
        println!("reader_avatar_phase={phase} media_http_requests={media_reads}");
        if phase == "initial" {
            first_scope = Some((reader, ready_revision));
        } else {
            verify_closed(&reader, ready_revision)?;
        }
    }
    Ok(())
}

fn verify_closed(
    reader: &koushi_core::runtime::ReaderSubscription,
    revision: koushi_protocol::view::ViewRevision,
) -> Result<(), String> {
    reader.close_handle().close();
    if reader.observe_avatars(revision, 3, &[], &[])
        != Err(koushi_core::runtime::ScopeError::Closed)
    {
        return Err("reader scope: closed observation was not rejected".to_owned());
    }
    Ok(())
}

fn verify_png(
    reader: &koushi_core::runtime::ReaderSubscription,
    revision: koushi_protocol::view::ViewRevision,
    source_ref: &str,
) -> Result<(), String> {
    let content = reader
        .resource_content(revision, source_ref)
        .map_err(|_| "reader avatar: scoped resource rejected".to_owned())?
        .ok_or_else(|| "reader avatar: Ready bytes missing".to_owned())?;
    if !content
        .bytes
        .starts_with(&[137, 80, 78, 71, 13, 10, 26, 10])
    {
        return Err("reader avatar: invalid PNG resource".to_owned());
    }
    Ok(())
}
