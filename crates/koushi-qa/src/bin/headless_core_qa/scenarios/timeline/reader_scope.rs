use super::*;
use koushi_protocol::view::{
    ReaderWindowLimit, ReceiptSourceRef, TimelineViewSource, ViewDelivery, ViewModel,
};

/// Exercise the public source/ACK/observation path against the live SDK data.
/// Avatar HTTP bounds are a separate assertion; this also accepts placeholders.
pub(super) async fn verify_live_reader_scope(
    conn: &mut CoreConnection,
    key: &TimelineKey,
    event_id: &str,
    expected_reader: &str,
) -> Result<(), String> {
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
    let mut reader = conn
        .subscribe_reader(
            ReceiptSourceRef {
                timeline,
                event_id: event_id.to_owned(),
            },
            0,
            ReaderWindowLimit::try_from(16).unwrap(),
        )
        .map_err(|_| "reader scope: admission failed".to_owned())?;
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
                        if window.rows.iter().any(|row| row.user_id == expected_reader) {
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
    reader.close_handle().close();
    if reader.observe_avatars(revision, 2, &[], &[])
        != Err(koushi_core::runtime::ScopeError::Closed)
    {
        return Err("reader scope: closed observation was not rejected".to_owned());
    }
    Ok(())
}
