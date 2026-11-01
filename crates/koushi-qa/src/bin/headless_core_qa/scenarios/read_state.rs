use super::cleanup::cleanup_logged_in_runtime;
use super::diagnostics::QaTcpProxy;
use super::event_wait::{
    stop_sync_for_qa, subscribe_timeline_for_qa, wait_for_invite_in_snapshot,
    wait_for_item_with_body, wait_for_ready_snapshot, wait_for_send_flow_completion,
    wait_for_session_restored, wait_for_sync_running_after_reconnect, wait_for_sync_started,
};
use super::fixtures::{accept_invite_for_qa, create_room_for_qa, invite_user_for_qa};
use super::participants::{QaParticipantLoginGate, login_synced_participant_for_qa, qa_data_dir};
use super::registry::{EVENT_TIMEOUT, QaConfig};
use super::{
    AccountCommand, AccountKey, CoreCommand, CoreConnection, CoreEvent, CoreRuntime,
    LiveSignalsEvent, SyncCommand, TimelineBottomArrival, TimelineCommand, TimelineEvent,
    TimelineItemId, TimelineKey, TimelineKind, TimelineReadStateSync, TimelineViewportObservation,
};

struct ReadStateRestartCheckpoint {
    key: TimelineKey,
    event_id: String,
}

pub(super) async fn run_read_state_convergence_scenario(config: &QaConfig) -> Result<(), String> {
    let proxy = QaTcpProxy::start(&config.homeserver)
        .map_err(|_| "read-state convergence proxy setup failed".to_owned())?;
    let proxied_config = config.with_homeserver(proxy.homeserver_url());
    let data_dir_a = qa_data_dir("read-state-convergence-a");
    let participant_a = login_synced_participant_for_qa(
        &proxied_config.homeserver,
        data_dir_a.clone(),
        &proxied_config.user_a,
        &proxied_config.password_a,
        "Koushi Read State QA A",
        "read-state convergence login A",
        "read-state convergence gate A",
        QaParticipantLoginGate::BootstrapNewIdentity,
    )
    .await?;
    let participant_b = login_synced_participant_for_qa(
        &proxied_config.homeserver,
        qa_data_dir("read-state-convergence-b"),
        &proxied_config.user_b,
        &proxied_config.password_b,
        "Koushi Read State QA B",
        "read-state convergence login B",
        "read-state convergence gate B",
        QaParticipantLoginGate::BootstrapNewIdentity,
    )
    .await?;
    let super::participants::QaParticipantLoginOutcome {
        runtime: runtime_a,
        conn: mut conn_a,
        account_key: account_key_a,
        ..
    } = participant_a;
    let super::participants::QaParticipantLoginOutcome {
        runtime: runtime_b,
        conn: mut conn_b,
        account_key: account_key_b,
        ..
    } = participant_b;
    let user_b_id = format!("@{}:{}", config.user_b, config.server_name);

    let checkpoint = match run_read_state_convergence_flow(
        &mut conn_a,
        &account_key_a,
        &mut conn_b,
        &account_key_b,
        &user_b_id,
        &proxy,
    )
    .await
    {
        Ok(checkpoint) => checkpoint,
        Err(flow) => {
            let _ = cleanup_logged_in_runtime(
                conn_b,
                runtime_b,
                account_key_b,
                "read-state convergence failed cleanup B",
            )
            .await;
            let _ = cleanup_logged_in_runtime(
                conn_a,
                runtime_a,
                account_key_a,
                "read-state convergence failed cleanup A",
            )
            .await;
            return Err(flow);
        }
    };

    let cleanup_b = cleanup_logged_in_runtime(
        conn_b,
        runtime_b,
        account_key_b,
        "read-state convergence cleanup B",
    )
    .await;
    drop(conn_a);
    tokio::time::timeout(EVENT_TIMEOUT, runtime_a.shutdown())
        .await
        .map_err(|_| "read-state convergence runtime shutdown timed out".to_owned())?;

    let restarted_runtime = CoreRuntime::start_with_data_dir(data_dir_a);
    let mut restarted_conn = restarted_runtime.attach();
    let restore_id = restarted_conn.next_request_id();
    restarted_conn
        .command(CoreCommand::Account(AccountCommand::RestoreSession {
            request_id: restore_id,
            account_key: account_key_a.clone(),
        }))
        .await
        .map_err(|_| "read-state convergence restore submission failed".to_owned())?;
    wait_for_session_restored(
        &mut restarted_conn,
        restore_id,
        &account_key_a,
        "read-state convergence restored session",
    )
    .await?;
    wait_for_ready_snapshot(&mut restarted_conn, "read-state convergence restored Ready").await?;
    let sync_id = restarted_conn.next_request_id();
    restarted_conn
        .command(CoreCommand::Sync(SyncCommand::Start {
            request_id: sync_id,
        }))
        .await
        .map_err(|_| "read-state convergence restarted sync submission failed".to_owned())?;
    wait_for_sync_started(
        &mut restarted_conn,
        sync_id,
        "read-state convergence restarted sync",
    )
    .await?;
    wait_for_sync_running_after_reconnect(
        &mut restarted_conn,
        "read-state convergence restarted sync running",
    )
    .await?;
    subscribe_timeline_for_qa(
        &mut restarted_conn,
        &checkpoint.key,
        "read-state convergence restored timeline",
    )
    .await?;
    let before_release = proxy
        .read_state_observation()
        .map_err(|_| "read-state convergence restart observation failed".to_owned())?
        .request_count;
    proxy.release_read_state_writes();
    let result = wait_for_navigation(&mut restarted_conn, &checkpoint.key, |snapshot| {
        snapshot.local_viewed_event_id.as_deref() == Some(checkpoint.event_id.as_str())
            && snapshot.server_confirmed_read_event_id.as_deref()
                == Some(checkpoint.event_id.as_str())
            && snapshot.read_state_sync == TimelineReadStateSync::Synced
    })
    .await
    .and_then(|_| {
        let after_release = proxy
            .read_state_observation()
            .map_err(|_| "read-state convergence drain observation failed".to_owned())?
            .request_count;
        let submitted = after_release.saturating_sub(before_release);
        if submitted == 0 || submitted > 2 {
            return Err(
                "read-state convergence restart submitted a non-newest read-state batch".to_owned(),
            );
        }
        Ok(())
    });
    let cleanup_a = cleanup_logged_in_runtime(
        restarted_conn,
        restarted_runtime,
        account_key_a,
        "read-state convergence cleanup restarted A",
    )
    .await;
    let cleanup = cleanup_b.and(cleanup_a);
    match (result, cleanup) {
        (Ok(()), Ok(())) => {
            println!("read_state_convergence=ok");
            Ok(())
        }
        (Err(flow), Ok(())) => Err(flow),
        (Ok(()), Err(cleanup)) => Err(cleanup),
        (Err(flow), Err(_)) => Err(flow),
    }
}

async fn run_read_state_convergence_flow(
    conn: &mut CoreConnection,
    account_key: &AccountKey,
    sender_conn: &mut CoreConnection,
    sender_account_key: &AccountKey,
    sender_user_id: &str,
    proxy: &QaTcpProxy,
) -> Result<ReadStateRestartCheckpoint, String> {
    let room_id = create_room_for_qa(
        conn,
        "Synthetic Read State Room",
        false,
        "read-state convergence room",
    )
    .await?;
    invite_user_for_qa(
        conn,
        &room_id,
        sender_user_id,
        "read-state convergence invite sender",
    )
    .await?;
    wait_for_invite_in_snapshot(
        sender_conn,
        &room_id,
        None,
        "read-state convergence sender invite",
    )
    .await?;
    accept_invite_for_qa(
        sender_conn,
        &room_id,
        "read-state convergence sender accepts invite",
    )
    .await?;
    let key = TimelineKey {
        account_key: account_key.clone(),
        kind: TimelineKind::Room {
            room_id: room_id.clone(),
        },
    };
    let sender_key = TimelineKey {
        account_key: sender_account_key.clone(),
        kind: TimelineKind::Room {
            room_id: room_id.clone(),
        },
    };
    let _initial_items =
        subscribe_timeline_for_qa(conn, &key, "read-state convergence subscribe reader").await?;
    let _sender_initial = subscribe_timeline_for_qa(
        sender_conn,
        &sender_key,
        "read-state convergence subscribe sender",
    )
    .await?;

    proxy.release_read_state_writes();
    let seed_body = "Synthetic read-state seed";
    let seed_event_id = send_text_and_wait_event(
        sender_conn,
        &sender_key,
        "read-state-seed",
        seed_body,
        "read-state convergence seed",
    )
    .await?;
    wait_for_remote_event(conn, &key, seed_body, &seed_event_id).await?;
    set_fully_read_and_wait(conn, &key, &seed_event_id).await?;

    let viewed_body = "Synthetic read-state viewed";
    let viewed_event_id = send_text_and_wait_event(
        sender_conn,
        &sender_key,
        "read-state-viewed",
        viewed_body,
        "read-state convergence viewed",
    )
    .await?;
    wait_for_remote_event(conn, &key, viewed_body, &viewed_event_id).await?;

    proxy.hold_read_state_writes();
    observe_viewport(conn, &key, &viewed_event_id).await?;
    let pending = wait_for_navigation(conn, &key, |snapshot| {
        snapshot.local_viewed_event_id.as_deref() == Some(viewed_event_id.as_str())
            && snapshot.server_confirmed_read_event_id.as_deref() == Some(seed_event_id.as_str())
            && snapshot.read_state_sync == TimelineReadStateSync::Pending
    })
    .await?;
    if pending.read_marker_event_id.as_deref() != Some(seed_event_id.as_str()) {
        return Err(
            "read-state convergence changed the server boundary while writes were held".to_owned(),
        );
    }
    let held = proxy
        .wait_for_held_read_state_writes(1, EVENT_TIMEOUT)
        .map_err(|_| "read-state convergence held-write evidence failed".to_owned())?;
    if held.held_request_count == 0 {
        return Err("read-state convergence observed no held read-state write".to_owned());
    }
    if held.max_inflight > 4 {
        return Err("read-state convergence exceeded the bounded write dispatcher".to_owned());
    }

    proxy.release_read_state_writes();
    wait_for_navigation(conn, &key, |snapshot| {
        snapshot.local_viewed_event_id.as_deref() == Some(viewed_event_id.as_str())
            && snapshot.server_confirmed_read_event_id.as_deref() == Some(viewed_event_id.as_str())
            && snapshot.read_state_sync == TimelineReadStateSync::Synced
    })
    .await?;

    let failed_body = "Synthetic read-state failure";
    let failed_event_id = send_text_and_wait_event(
        sender_conn,
        &sender_key,
        "read-state-failed",
        failed_body,
        "read-state convergence failure",
    )
    .await?;
    wait_for_remote_event(conn, &key, failed_body, &failed_event_id).await?;
    proxy.fail_read_state_writes();
    observe_viewport(conn, &key, &failed_event_id).await?;
    wait_for_navigation(conn, &key, |snapshot| {
        snapshot.local_viewed_event_id.as_deref() == Some(failed_event_id.as_str())
            && snapshot.read_state_sync.is_failed()
    })
    .await?;
    for _ in 0..100 {
        observe_viewport(conn, &key, &failed_event_id).await?;
    }
    let attempts = proxy.read_state_attempts()?;
    for (name, times) in ["public", "private", "fully_read"]
        .into_iter()
        .zip(attempts)
    {
        if times.is_empty() {
            return Err(format!(
                "read-state convergence observed no failed {name} write"
            ));
        }
        let elapsed: Vec<_> = times
            .iter()
            .map(|time| time.duration_since(times[0]))
            .collect();
        check_retry_intervals(&elapsed).map_err(|(index, gap, required)| {
            format!("repeated viewport checkpoints bypassed the read-state backoff: kind={name} attempt={} elapsed_ms={} gap_ms={} required_ms={}", index + 1, elapsed[index].as_millis(), gap.as_millis(), required.as_millis())
        })?;
    }

    stop_sync_for_qa(conn, "read-state convergence restart stop").await?;
    Ok(ReadStateRestartCheckpoint {
        key,
        event_id: failed_event_id,
    })
}

fn retry_delay(attempt: usize) -> std::time::Duration {
    std::time::Duration::from_secs(1_u64 << attempt.min(6)).min(std::time::Duration::from_secs(60))
}

fn check_retry_intervals(
    times: &[std::time::Duration],
) -> Result<(), (usize, std::time::Duration, std::time::Duration)> {
    for (index, pair) in times.windows(2).enumerate() {
        let gap = pair[1].saturating_sub(pair[0]);
        let required = retry_delay(index).saturating_sub(std::time::Duration::from_millis(50));
        if gap < required {
            return Err((index + 1, gap, required));
        }
    }
    Ok(())
}

async fn wait_for_remote_event(
    conn: &mut CoreConnection,
    key: &TimelineKey,
    body: &str,
    expected_event_id: &str,
) -> Result<(), String> {
    let item = wait_for_item_with_body(
        conn,
        key,
        body,
        "read-state convergence reader receives remote event",
    )
    .await?;
    match item.id {
        TimelineItemId::Event { event_id } if event_id == expected_event_id => Ok(()),
        TimelineItemId::Event { .. }
        | TimelineItemId::Transaction { .. }
        | TimelineItemId::Synthetic { .. } => {
            Err("read-state convergence remote event identity mismatch".to_owned())
        }
    }
}

pub(super) async fn send_text_and_wait_event(
    conn: &mut CoreConnection,
    key: &TimelineKey,
    transaction_id: &str,
    body: &str,
    label: &str,
) -> Result<String, String> {
    let request_id = conn.next_request_id();
    conn.command(CoreCommand::Timeline(TimelineCommand::SendText {
        request_id,
        key: key.clone(),
        transaction_id: transaction_id.to_owned(),
        document: koushi_state::ComposerDocument::from_plain_text(body.to_owned()),
    }))
    .await
    .map_err(|_| format!("{label}: send submission failed"))?;
    Ok(
        wait_for_send_flow_completion(conn, request_id, key, transaction_id, body, label)
            .await?
            .event_id,
    )
}

async fn set_fully_read_and_wait(
    conn: &mut CoreConnection,
    key: &TimelineKey,
    event_id: &str,
) -> Result<(), String> {
    let request_id = conn.next_request_id();
    conn.command(CoreCommand::Timeline(TimelineCommand::SetFullyRead {
        request_id,
        key: key.clone(),
        event_id: event_id.to_owned(),
    }))
    .await
    .map_err(|_| "read-state convergence fully-read submission failed".to_owned())?;
    loop {
        match tokio::time::timeout(EVENT_TIMEOUT, conn.recv_event())
            .await
            .map_err(|_| "read-state convergence fully-read timeout".to_owned())?
            .map_err(|_| "read-state convergence event stream closed".to_owned())?
        {
            CoreEvent::LiveSignals(LiveSignalsEvent::FullyReadSet {
                request_id: event_request,
                key: event_key,
                ..
            }) if event_request == request_id && event_key == *key => return Ok(()),
            CoreEvent::OperationFailed {
                request_id: event_request,
                ..
            } if event_request == request_id => {
                return Err("read-state convergence fully-read write failed".to_owned());
            }
            _ => {}
        }
    }
}

pub(super) async fn observe_viewport(
    conn: &mut CoreConnection,
    key: &TimelineKey,
    event_id: &str,
) -> Result<(), String> {
    let observation = super::viewport_observation::fresh_viewport_observation_for_qa(
        conn,
        key,
        TimelineViewportObservation {
            display_window_revision: None,
            first_visible_event_id: Some(event_id.to_owned()),
            last_visible_event_id: Some(event_id.to_owned()),
            visible_gap_ids: Vec::new(),
            at_bottom: true,
            bottom_arrival: TimelineBottomArrival::User,
        },
    )
    .await?;
    let request_id = conn.next_request_id();
    conn.command(CoreCommand::Timeline(TimelineCommand::ObserveViewport {
        request_id,
        key: key.clone(),
        observation,
    }))
    .await
    .map_err(|_| "viewport submission failed".to_owned())
}

async fn wait_for_navigation(
    conn: &mut CoreConnection,
    key: &TimelineKey,
    predicate: impl Fn(&koushi_protocol::event::TimelineNavigationSnapshot) -> bool,
) -> Result<koushi_protocol::event::TimelineNavigationSnapshot, String> {
    let deadline = tokio::time::Instant::now() + EVENT_TIMEOUT;
    loop {
        if let CoreEvent::Timeline(TimelineEvent::NavigationUpdated {
            key: event_key,
            snapshot,
        }) = tokio::time::timeout_at(deadline, conn.recv_event())
            .await
            .map_err(|_| "read-state convergence navigation timeout".to_owned())?
            .map_err(|_| "read-state convergence event stream closed".to_owned())?
            && event_key == *key
            && predicate(&snapshot)
        {
            return Ok(*snapshot);
        }
    }
}

trait TimelineReadStateSyncExt {
    fn is_failed(&self) -> bool;
}

impl TimelineReadStateSyncExt for TimelineReadStateSync {
    fn is_failed(&self) -> bool {
        matches!(self, Self::Failed { .. })
    }
}

#[cfg(test)]
mod retry_interval_tests {
    use super::{check_retry_intervals, retry_delay};
    use std::time::Duration;

    #[test]
    fn retry_delays_cap_at_sixty_seconds() {
        assert_eq!(
            (0..8).map(|i| retry_delay(i).as_secs()).collect::<Vec<_>>(),
            [1, 2, 4, 8, 16, 32, 60, 60]
        );
    }

    #[test]
    fn retry_intervals_allow_due_retry_and_reject_early_retry() {
        let sec = Duration::from_secs;
        assert!(
            check_retry_intervals(&[sec(0), sec(1), sec(3), sec(7), sec(15), sec(31), sec(63)])
                .is_ok()
        );
        assert!(check_retry_intervals(&[sec(0), sec(10), Duration::from_millis(10_100)]).is_err());
        assert!(check_retry_intervals(&[sec(0), Duration::from_millis(949)]).is_err());
        assert!(check_retry_intervals(&[sec(0), Duration::from_millis(950)]).is_ok());
    }
}
