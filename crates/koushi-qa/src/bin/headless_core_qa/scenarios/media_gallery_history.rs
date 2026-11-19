//! #1969: the room media gallery reaches media older than the loaded window.
//!
//! The primary device sends one attachment and then more text than the SDK's
//! default room-subscription timeline limit (20). A fresh device of the same
//! user therefore syncs a window without the attachment, so the
//! window-derived gallery cannot contain it. `PaginateMediaGallery` must page
//! the room's media browser until the attachment appears and the pagination
//! reaches `ReachedStart`, and the off-window item must still download.

use koushi_state::{AppState, AuthSecret, MediaGalleryPagination};

use super::cleanup::cleanup_logged_in_runtime;
use super::event_wait::{
    subscribe_timeline_for_qa, wait_for_media_download_completed,
    wait_for_media_send_flow_completion, wait_for_room_in_room_list, wait_for_send_completed,
};
use super::fixtures::create_room_for_qa;
use super::participants::{
    QaParticipantLoginGate, QaParticipantLoginOutcome, login_synced_participant_on_runtime_for_qa,
    start_isolated_qa_runtime,
};
use super::registry::{EVENT_TIMEOUT, QaConfig};
use super::{
    ComposerDocument, CoreCommand, CoreConnection, MediaDownloadSelection, RoomCommand,
    TimelineCommand, TimelineKey, UploadMediaKind, UploadMediaRequest,
};

const MEDIA_BYTES: &[u8] = b"koushi synthetic gallery history fixture";
/// More than the SDK's default room-subscription timeline limit (20), so the
/// fresh device's synced window no longer holds the attachment.
const FILLER_COUNT: usize = 25;
/// Each request pages one media-browser batch; the room is small, so a
/// handful reaches its start. The cap only bounds a regression.
const MAX_PAGINATE_REQUESTS: usize = 40;

pub(super) async fn run_media_gallery_history_stage(
    config: &QaConfig,
    recovery_secret: &AuthSecret,
    conn_a: &mut CoreConnection,
    key_a: &TimelineKey,
) -> Result<(), String> {
    let (room_id, media_event_id) = seed_room(conn_a, key_a).await?;

    let QaParticipantLoginOutcome {
        runtime,
        mut conn,
        account_key,
        ..
    } = login_synced_participant_on_runtime_for_qa(
        // Same-user second device: must not restore the primary session.
        start_isolated_qa_runtime("media-gallery-history-fresh")?,
        &config.homeserver,
        &config.user_a,
        &config.password_a,
        "Koushi Media Gallery History QA fresh",
        "media gallery history login fresh",
        "media gallery history gate fresh",
        QaParticipantLoginGate::RecoverExistingIdentity(recovery_secret),
    )
    .await?;
    let flow = run_fresh_device_flow(
        &mut conn,
        TimelineKey::room(account_key.clone(), room_id.clone()),
        &room_id,
        &media_event_id,
    )
    .await;
    let cleanup = cleanup_logged_in_runtime(
        conn,
        runtime,
        account_key,
        "media gallery history cleanup fresh",
    )
    .await;
    flow?;
    cleanup?;
    println!("media_gallery_history=ok");
    Ok(())
}

async fn seed_room(
    conn: &mut CoreConnection,
    primary_key: &TimelineKey,
) -> Result<(String, String), String> {
    let room_id = create_room_for_qa(
        conn,
        "Synthetic Media Gallery History Room",
        false,
        "media gallery history room",
    )
    .await?;
    wait_for_room_in_room_list(conn, &room_id, "media gallery history room list").await?;
    let key = TimelineKey::room(primary_key.account_key.clone(), room_id.clone());
    subscribe_timeline_for_qa(conn, &key, "media gallery history subscribe").await?;

    let expected_account = match conn.snapshot().session {
        koushi_state::SessionState::Ready(info) => {
            koushi_core::store::session_key_id_from_info(&info)
        }
        _ => return Err("media gallery history requires a ready session".to_owned()),
    };
    let transaction_id = "qa-media-gallery-history-media".to_owned();
    let request_id = conn.next_request_id();
    conn.command(CoreCommand::Timeline(TimelineCommand::UploadAndSendMedia {
        request_id,
        expected_account,
        key: key.clone(),
        transaction_id: transaction_id.clone(),
        request: UploadMediaRequest {
            filename: "fixture-gallery-history.bin".to_owned(),
            mime_type: "application/octet-stream".to_owned(),
            bytes: MEDIA_BYTES.to_vec(),
            kind: UploadMediaKind::File,
            compression: None,
            thumbnail: None,
            caption: None,
        },
    }))
    .await
    .map_err(|e| format!("media gallery history: submit media: {e}"))?;
    let media_event_id = wait_for_media_send_flow_completion(
        conn,
        request_id,
        &key,
        &transaction_id,
        "media gallery history media send",
    )
    .await?;

    for index in 0..FILLER_COUNT {
        let request_id = conn.next_request_id();
        conn.command(CoreCommand::Timeline(TimelineCommand::SendText {
            request_id,
            key: key.clone(),
            transaction_id: format!("qa-media-gallery-history-filler-{index}"),
            document: ComposerDocument::from_plain_text(format!(
                "Synthetic gallery history filler {index}"
            )),
        }))
        .await
        .map_err(|e| format!("media gallery history: submit filler {index}: {e}"))?;
        wait_for_send_completed(conn, request_id, &key, "media gallery history filler").await?;
    }
    Ok((room_id, media_event_id))
}

fn gallery_has(state: &AppState, event_id: &str) -> bool {
    state
        .timeline
        .media_gallery
        .iter()
        .any(|item| item.event_id == event_id)
}

async fn run_fresh_device_flow(
    conn: &mut CoreConnection,
    key: TimelineKey,
    room_id: &str,
    media_event_id: &str,
) -> Result<(), String> {
    wait_for_room_in_room_list(conn, room_id, "media gallery history fresh room list").await?;
    let select_id = conn.next_request_id();
    conn.command(CoreCommand::Room(RoomCommand::SelectRoom {
        request_id: select_id,
        room_id: room_id.to_owned(),
    }))
    .await
    .map_err(|e| format!("media gallery history: submit room select: {e}"))?;
    let deadline = tokio::time::Instant::now() + EVENT_TIMEOUT;
    loop {
        let state = conn.snapshot();
        if state.timeline.room_id.as_deref() == Some(room_id) && state.timeline.is_subscribed {
            break;
        }
        next_event(conn, deadline, "media gallery history fresh subscribe").await?;
    }

    // Precondition: the fresh window does not reach the attachment, so the
    // gallery can only find it through the media browser.
    if gallery_has(&conn.snapshot(), media_event_id) {
        return Err(
            "media gallery history precondition: the fresh window already holds the attachment"
                .to_owned(),
        );
    }
    println!("media_gallery_history_window_excludes_media=ok");

    let mut requests = 0;
    let deadline = tokio::time::Instant::now() + EVENT_TIMEOUT * 4;
    loop {
        let state = conn.snapshot();
        match state.timeline.media_gallery_pagination {
            MediaGalleryPagination::ReachedStart => break,
            MediaGalleryPagination::Failed => {
                return Err("media gallery history: media browser page failed".to_owned());
            }
            MediaGalleryPagination::Loading => {}
            MediaGalleryPagination::Idle => {
                if requests == MAX_PAGINATE_REQUESTS {
                    return Err(format!(
                        "media gallery history: no room start after {requests} page request(s)"
                    ));
                }
                requests += 1;
                // A request racing an in-flight page coalesces in the actor.
                let request_id = conn.next_request_id();
                conn.command(CoreCommand::Timeline(
                    TimelineCommand::PaginateMediaGallery {
                        request_id,
                        key: key.clone(),
                    },
                ))
                .await
                .map_err(|e| format!("media gallery history: submit paginate: {e}"))?;
            }
        }
        next_event(conn, deadline, "media gallery history pagination").await?;
    }
    if !gallery_has(&conn.snapshot(), media_event_id) {
        return Err("media gallery history: room start reached without the attachment".to_owned());
    }
    println!("media_gallery_history_requests={requests}");
    println!("media_gallery_history_reached_start=ok");

    let download_id = conn.next_request_id();
    conn.command(CoreCommand::Timeline(TimelineCommand::DownloadMedia {
        request_id: download_id,
        key: key.clone(),
        event_id: media_event_id.to_owned(),
        selection: MediaDownloadSelection::File,
    }))
    .await
    .map_err(|e| format!("media gallery history: submit download: {e}"))?;
    wait_for_media_download_completed(
        conn,
        download_id,
        &key,
        media_event_id,
        u64::try_from(MEDIA_BYTES.len()).unwrap_or(u64::MAX),
        "media gallery history off-window download",
    )
    .await?;
    println!("media_gallery_history_download=ok");
    Ok(())
}

async fn next_event(
    conn: &mut CoreConnection,
    deadline: tokio::time::Instant,
    label: &str,
) -> Result<(), String> {
    tokio::time::timeout_at(deadline, conn.recv_event())
        .await
        .map_err(|_| format!("{label}: timed out waiting for state"))?
        .map_err(|lag| format!("{label}: event stream lagged ({})", lag.skipped))?;
    Ok(())
}
