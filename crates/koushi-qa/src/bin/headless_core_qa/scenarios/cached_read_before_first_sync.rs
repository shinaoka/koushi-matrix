//! #1737 Phase A: a restored session reads its cached conversations before the
//! first committed sync response, and writes nothing until Secure Backup
//! admission.
//!
//! The fixture seeds a disposable encrypted room with one synthetic message,
//! restarts the runtime on the same encrypted stores, and withholds every
//! Simplified Sliding Sync response, so the restored session cannot have heard
//! anything from the server. It then proves, in order:
//!
//! 1. the cached room list and the selected room's cached message are projected
//!    while the room list is still provisional (never an authoritative empty
//!    account, #409),
//! 2. the projected read access is readable-but-not-sendable, and a message send
//!    inside that window is refused before it reaches the network,
//! 3. a complete network cut keeps the cached content visible,
//! 4. releasing the held response promotes the list to authoritative and returns
//!    the session to its normal send admission.
//!
//! No real identifiers, bodies, tokens, or local paths are printed; the tokens
//! below are count-free success markers only.

use super::cleanup::cleanup_logged_in_runtime;
use super::diagnostics::QaTcpProxy;
use super::event_wait::{
    find_timeline_item_with_body, send_text_expect_local_echo, stop_sync_for_qa,
    subscribe_timeline_for_qa, timeline_item_is_decryption_failure, wait_for_event_item_with_body,
    wait_for_logged_in, wait_for_ready_snapshot, wait_for_room_in_room_list,
    wait_for_send_flow_completion, wait_for_session_restored,
    wait_for_sync_running_after_reconnect, wait_for_sync_started,
    wait_for_sync_started_and_running,
};
use super::fixtures::create_room_for_qa;
use super::participants::{complete_new_identity_gate_for_qa, qa_data_dir};
use super::registry::{EVENT_TIMEOUT, QaConfig};
use super::{
    AccountCommand, AccountKey, AuthSecret, CoreCommand, CoreConnection, CoreEvent, CoreRuntime,
    SyncCommand, TimelineCommand, TimelineKey,
};
use koushi_protocol::failure::{CoreFailure, TimelineFailureKind};
use koushi_state::RoomListReadiness;

const ROOM_NAME: &str = "Synthetic Saved Conversation";
const CACHED_BODY: &str = "Synthetic cached conversation body";
const POST_ADMISSION_BODY: &str = "Synthetic post-admission body";

pub(super) async fn run_cached_read_before_first_sync_scenario(
    config: &QaConfig,
) -> Result<(), String> {
    let proxy = QaTcpProxy::start(&config.homeserver)
        .map_err(|_| "cached read: proxy setup failed".to_owned())?;
    let proxied_config = config.with_homeserver(proxy.homeserver_url());
    let data_dir = qa_data_dir("cached-read-before-first-sync");
    let runtime = CoreRuntime::start_with_data_dir(data_dir.clone());
    let mut conn = runtime.attach();

    let login_id = conn.next_request_id();
    conn.command(CoreCommand::Account(AccountCommand::LoginPassword {
        request_id: login_id,
        request: koushi_state::LoginRequest {
            homeserver: proxied_config.homeserver.clone(),
            username: proxied_config.user_a.clone(),
            password: AuthSecret::new(proxied_config.password_a.clone()),
            device_display_name: Some("Koushi Core QA Cached Read".to_owned()),
        },
        platform: koushi_state::DisplayPlatform::Linux,
    }))
    .await
    .map_err(|e| format!("cached read: submit login failed: {e}"))?;
    complete_new_identity_gate_for_qa(&mut conn, &proxied_config.password_a).await?;
    let account_key: AccountKey =
        wait_for_logged_in(&mut conn, login_id, "cached read login").await?;
    wait_for_ready_snapshot(&mut conn, "cached read session Ready").await?;

    let sync_id = conn.next_request_id();
    conn.command(CoreCommand::Sync(SyncCommand::Start {
        request_id: sync_id,
    }))
    .await
    .map_err(|e| format!("cached read: submit sync start failed: {e}"))?;
    wait_for_sync_started_and_running(&mut conn, sync_id, "cached read sync start").await?;
    // The identity bootstrap completes Secure Backup; sending is admitted only
    // through that inspection, so the seed waits for the same admission the
    // restored session will have to earn again.
    wait_for_send_admission(&mut conn, "cached read seed admission").await?;

    let room_id = create_room_for_qa(&mut conn, ROOM_NAME, true, "cached read create room").await?;
    let key = TimelineKey::room(account_key.clone(), room_id.clone());
    subscribe_timeline_for_qa(&mut conn, &key, "cached read seed subscribe").await?;
    send_text_expect_local_echo(
        &mut conn,
        &key,
        "cached-read-seed",
        CACHED_BODY,
        "cached read seed send",
    )
    .await?;
    let seeded =
        wait_for_event_item_with_body(&mut conn, &key, CACHED_BODY, "cached read seed body")
            .await?;
    if timeline_item_is_decryption_failure(&seeded) {
        return Err("cached read: the seeded body never decrypted".to_owned());
    }
    println!("cached_read_seed=ok");

    // Restart on the same encrypted stores with every new sync response held.
    stop_sync_for_qa(&mut conn, "cached read stop before restart").await?;
    drop(conn);
    tokio::time::timeout(EVENT_TIMEOUT, runtime.shutdown())
        .await
        .map_err(|_| "cached read: timed out shutting down the seeding runtime".to_owned())?;
    proxy.hold_sliding_sync_responses();

    let restored_runtime = CoreRuntime::start_with_data_dir(data_dir);
    let mut restored = restored_runtime.attach();
    let restore_id = restored.next_request_id();
    restored
        .command(CoreCommand::Account(AccountCommand::RestoreSession {
            request_id: restore_id,
            account_key: account_key.clone(),
        }))
        .await
        .map_err(|e| format!("cached read: submit restore failed: {e}"))?;
    wait_for_session_restored(
        &mut restored,
        restore_id,
        &account_key,
        "cached read restore",
    )
    .await?;
    wait_for_ready_snapshot(&mut restored, "cached read restored session Ready").await?;
    let restored_sync_id = restored.next_request_id();
    restored
        .command(CoreCommand::Sync(SyncCommand::Start {
            request_id: restored_sync_id,
        }))
        .await
        .map_err(|e| format!("cached read: submit restored sync start failed: {e}"))?;
    wait_for_sync_started(
        &mut restored,
        restored_sync_id,
        "cached read restored sync start",
    )
    .await?;
    wait_for_held_sync_response(&proxy).await?;
    // The seeding phase already forwarded sends through this proxy; the
    // read-only window is judged against what happens from here on.
    let send_forwarded_baseline = proxy.room_send_forwarded_count();
    println!("cached_read_first_response_withheld=ok");

    // 1. Cut the network completely before asserting anything: the cached room
    // list and the cached message must come from the local stores, not from a
    // request this proxy could still serve. The withheld sync response above
    // already proves the restored session issued its request and received
    // nothing.
    proxy.disable();

    let provisional =
        wait_for_room_in_room_list(&mut restored, &room_id, "cached read room list").await?;
    if matches!(
        provisional.room_list.readiness,
        RoomListReadiness::Ready { .. }
    ) {
        return Err(
            "cached read: the restored room list claimed authority before any committed response"
                .to_owned(),
        );
    }
    let access = provisional.conversation_access();
    if !access.readable {
        return Err("cached read: the restored session is not readable".to_owned());
    }
    if access.list_authoritative {
        return Err(
            "cached read: a provisional cache projection claimed list authority".to_owned(),
        );
    }
    if access.send_admitted {
        return Err("cached read: sending was admitted before Secure Backup admission".to_owned());
    }
    println!("cached_read_list=ok");

    let items =
        subscribe_timeline_for_qa(&mut restored, &key, "cached read timeline subscribe").await?;
    let Some(cached_item) = find_timeline_item_with_body(&items, CACHED_BODY) else {
        return Err("cached read: the selected room did not project its cached message".to_owned());
    };
    if timeline_item_is_decryption_failure(&cached_item) {
        return Err("cached read: the cached message was not decryptable locally".to_owned());
    }
    println!("cached_read_timeline=ok");
    println!("cached_read_network_blocked=ok");

    // 2. Sending is refused inside the read-only window, before the network.
    let blocked_send_id = restored.next_request_id();
    restored
        .command(CoreCommand::Timeline(TimelineCommand::SendText {
            request_id: blocked_send_id,
            key: key.clone(),
            transaction_id: "cached-read-blocked".to_owned(),
            document: koushi_state::ComposerDocument::from_plain_text("synthetic".to_owned()),
        }))
        .await
        .map_err(|e| format!("cached read: submit blocked send failed: {e}"))?;
    wait_for_secure_backup_required(&mut restored, blocked_send_id).await?;
    if proxy.room_send_forwarded_count() != send_forwarded_baseline {
        return Err("cached read: a send reached the network before admission".to_owned());
    }
    println!("cached_read_send_blocked=ok");

    // A viewport report is what makes Rust write read state, and the run admits
    // it only once it owns a committed response. The held-write behaviour itself
    // is covered by the read-state worker tests.
    let cached_event_id = super::event_wait::timeline_item_event_id(&cached_item)
        .ok_or_else(|| "cached read: the cached message carries no event id".to_owned())?
        .to_owned();

    // 3. Releasing the held response promotes the list and re-admits sending.
    proxy.enable();
    proxy.release_sliding_sync_responses();
    wait_for_sync_running_after_reconnect(&mut restored, "cached read promoted sync").await?;
    wait_for_list_authority(&mut restored).await?;
    println!("cached_read_promoted=ok");

    // The confirmed list now admits automatic read state.
    let promoted_viewport = super::viewport_observation::fresh_viewport_observation_for_qa(
        &mut restored,
        &key,
        koushi_protocol::event::TimelineViewportObservation {
            display_window_revision: None,
            first_visible_event_id: Some(cached_event_id.clone()),
            last_visible_event_id: Some(cached_event_id),
            visible_gap_ids: Vec::new(),
            at_bottom: true,
            bottom_arrival: koushi_protocol::event::TimelineBottomArrival::User,
        },
    )
    .await?;
    let promoted_viewport_id = restored.next_request_id();
    restored
        .command(CoreCommand::Timeline(TimelineCommand::ObserveViewport {
            request_id: promoted_viewport_id,
            key: key.clone(),
            observation: promoted_viewport,
        }))
        .await
        .map_err(|e| format!("cached read: submit promoted viewport failed: {e}"))?;
    wait_for_read_state_write(&proxy).await?;
    println!("cached_read_read_state_admitted=ok");
    wait_for_send_admission(&mut restored, "cached read post-promotion admission").await?;
    // The token requires the whole send flow, not a local echo: a permanently
    // paused queue would never produce the server event.
    let send_id = restored.next_request_id();
    restored
        .command(CoreCommand::Timeline(TimelineCommand::SendText {
            request_id: send_id,
            key: key.clone(),
            transaction_id: "cached-read-after-admission".to_owned(),
            document: koushi_state::ComposerDocument::from_plain_text(
                POST_ADMISSION_BODY.to_owned(),
            ),
        }))
        .await
        .map_err(|e| format!("cached read: submit post-admission send failed: {e}"))?;
    wait_for_send_flow_completion(
        &mut restored,
        send_id,
        &key,
        "cached-read-after-admission",
        POST_ADMISSION_BODY,
        "cached read post-admission send",
    )
    .await?;
    println!("cached_read_post_admission_send=ok");
    println!("cached_read_before_first_sync=ok");

    cleanup_logged_in_runtime(
        restored,
        restored_runtime,
        account_key,
        "cached read cleanup",
    )
    .await?;
    Ok(())
}

/// Bounded wait for the read-state write the committed response admits. The
/// provisional window wrote none (asserted before the network cut), so any
/// attempt here is owned by the confirmed list.
async fn wait_for_read_state_write(proxy: &QaTcpProxy) -> Result<(), String> {
    let deadline = tokio::time::Instant::now() + EVENT_TIMEOUT;
    loop {
        if proxy
            .read_state_observation()
            .is_ok_and(|observation| observation.request_count > 0)
        {
            return Ok(());
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(
                "cached read: no read-state write followed the committed-response promotion"
                    .to_owned(),
            );
        }
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
    }
}

/// Bounded wait for the projected send admission of a live session.
async fn wait_for_send_admission(conn: &mut CoreConnection, label: &str) -> Result<(), String> {
    let deadline = tokio::time::Instant::now() + EVENT_TIMEOUT;
    loop {
        if conn.snapshot().conversation_access().send_admitted {
            return Ok(());
        }
        let event = tokio::time::timeout_at(deadline, conn.recv_event())
            .await
            .map_err(|_| format!("{label}: timed out waiting for send admission"))?
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;
        if matches!(
            event,
            CoreEvent::OperationFailed { .. } | CoreEvent::Sync(super::SyncEvent::Failed)
        ) {
            return Err(format!(
                "{label}: the session failed before admission: {event:?}"
            ));
        }
    }
}

/// Bounded wait for a withheld Simplified Sliding Sync response: the restored
/// session must have attempted a request that never came back.
async fn wait_for_held_sync_response(proxy: &QaTcpProxy) -> Result<(), String> {
    let deadline = tokio::time::Instant::now() + EVENT_TIMEOUT;
    loop {
        if proxy.sliding_sync_responses_held_count() > 0 {
            return Ok(());
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(
                "cached read: no Simplified Sliding Sync response was withheld; the restored \
                 session never issued a sync request"
                    .to_owned(),
            );
        }
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
    }
}

/// Bounded wait for the committed-response authority promotion.
async fn wait_for_list_authority(
    conn: &mut CoreConnection,
) -> Result<koushi_state::AppState, String> {
    let deadline = tokio::time::Instant::now() + EVENT_TIMEOUT;
    loop {
        let snapshot = conn.snapshot();
        if snapshot.conversation_access().list_authoritative {
            return Ok(snapshot);
        }
        tokio::time::timeout_at(deadline, conn.recv_event())
            .await
            .map_err(|_| {
                format!(
                    "cached read: timed out waiting for committed-response authority (readiness={:?})",
                    snapshot.room_list.readiness
                )
            })?
            .map_err(|lag| format!("cached read: event stream lagged (skipped={})", lag.skipped))?;
    }
}

async fn wait_for_secure_backup_required(
    conn: &mut CoreConnection,
    request_id: koushi_protocol::ids::RequestId,
) -> Result<(), String> {
    let deadline = tokio::time::Instant::now() + EVENT_TIMEOUT;
    loop {
        let event = tokio::time::timeout_at(deadline, conn.recv_event())
            .await
            .map_err(|_| "cached read: timed out waiting for the send refusal".to_owned())?
            .map_err(|lag| format!("cached read: event stream lagged (skipped={})", lag.skipped))?;
        match event {
            CoreEvent::OperationFailed {
                request_id: failed_id,
                failure:
                    CoreFailure::TimelineOperationFailed {
                        kind: TimelineFailureKind::SecureBackupRequired,
                    },
            } if failed_id == request_id => return Ok(()),
            CoreEvent::OperationFailed {
                request_id: failed_id,
                failure,
            } if failed_id == request_id => {
                return Err(format!(
                    "cached read: the read-only send was refused with the wrong failure: {failure:?}"
                ));
            }
            _ => continue,
        }
    }
}
