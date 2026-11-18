//! Optional disposable encrypted fixture for the deterministic CLI QA lane.
use super::super::{
    CoreConnection, diagnostics::QaTcpProxy, event_wait, fixtures,
    participants::QaParticipantLoginOutcome, registry::ROOM_LIST_EVENT_TIMEOUT,
};
use koushi_protocol::{
    command::{CoreCommand, TimelineCommand},
    ids::TimelineKey,
};
use koushi_state::AppState;

/// The room name the CLI lane looks the fixture up by.
const ROOM_NAME: &str = "Synthetic CLI QA";

/// Seeds the encrypted room the CLI lane reads, and returns only once the
/// snapshot the bridge serves lists it by name as encrypted.
///
/// Room creation and the send each settle on their own HTTP response. The SDK
/// lists a created room at once and its encryption state is fetched on demand,
/// but the room name reaches the room list only through Sliding Sync, so
/// neither acknowledgement means `koushi list_rooms` can find the room yet.
/// Seeding withholds sync responses so that gap is open on every run rather
/// than only on a slow machine, then waits for the room-list projection.
pub(super) async fn seed(
    participant: &mut QaParticipantLoginOutcome,
    proxy: &QaTcpProxy,
) -> Result<(), String> {
    proxy.hold_sliding_sync_responses();
    let seeded = seed_room(participant).await;
    let held_sync = proxy.sliding_sync_responses_held_count();
    proxy.release_sliding_sync_responses();
    let room = seeded?;
    wait_for_room_listed(&mut participant.conn, &room, held_sync).await
}

async fn seed_room(participant: &mut QaParticipantLoginOutcome) -> Result<String, String> {
    let room =
        fixtures::create_room_for_qa(&mut participant.conn, ROOM_NAME, true, "cli fixture room")
            .await?;
    let key = TimelineKey::room(participant.account_key.clone(), room.clone());
    event_wait::subscribe_timeline_for_qa(&mut participant.conn, &key, "cli fixture subscribe")
        .await?;
    let body = "SYNTHETIC_CLI_SEARCH_PROOF";
    let transaction = "cli-fixture-send";
    let request_id = participant.conn.next_request_id();
    participant
        .conn
        .command(CoreCommand::Timeline(TimelineCommand::SendText {
            request_id,
            key: key.clone(),
            transaction_id: transaction.into(),
            document: koushi_state::ComposerDocument::from_plain_text(body),
        }))
        .await
        .map_err(|_| "cli fixture submission failed".to_owned())?;
    event_wait::wait_for_send_flow_completion(
        &mut participant.conn,
        request_id,
        &key,
        transaction,
        body,
        "cli fixture delivery",
    )
    .await?;
    Ok(room)
}

/// What the CLI's `rooms` call must already see once the host reports ready:
/// the seeded room, under its name, as encrypted.
fn room_listed(snapshot: &AppState, room_id: &str) -> bool {
    snapshot
        .rooms
        .iter()
        .any(|room| room.room_id == room_id && room.is_encrypted && room.display_label == ROOM_NAME)
}

async fn wait_for_room_listed(
    conn: &mut CoreConnection,
    room_id: &str,
    held_sync: usize,
) -> Result<(), String> {
    let deadline = tokio::time::Instant::now() + ROOM_LIST_EVENT_TIMEOUT;
    loop {
        if room_listed(&conn.snapshot(), room_id) {
            return Ok(());
        }
        match tokio::time::timeout_at(deadline, conn.recv_event()).await {
            Ok(Ok(_)) => {}
            Ok(Err(lag)) => {
                return Err(format!(
                    "cli fixture room list: event stream lagged (skipped={})",
                    lag.skipped
                ));
            }
            Err(_) => {
                let snapshot = conn.snapshot();
                let listed = snapshot.rooms.iter().find(|room| room.room_id == room_id);
                return Err(format!(
                    "cli fixture room not listed (held_sync={held_sync} listed={} encrypted={} named={})",
                    listed.is_some(),
                    listed.is_some_and(|room| room.is_encrypted),
                    listed.is_some_and(|room| room.display_label == ROOM_NAME),
                ));
            }
        }
    }
}

/// Credentials for the deterministic runner enter only through a private
/// FIFO. Other existing host callers retain their legacy configuration path.
pub(super) fn pipe_credentials() -> Result<Option<(String, String)>, String> {
    let Some(path) = std::env::var_os("KOUSHI_QA_LOGIN_PIPE") else {
        return Ok(None);
    };
    #[cfg(unix)]
    {
        use std::{
            io::Read,
            os::unix::fs::{FileTypeExt, PermissionsExt},
        };
        let invalid = || "cli fixture login pipe rejected".to_owned();
        let metadata = std::fs::symlink_metadata(&path).map_err(|_| invalid())?;
        if !metadata.file_type().is_fifo() || metadata.permissions().mode() & 0o077 != 0 {
            return Err(invalid());
        }
        let mut bytes = Vec::new();
        std::fs::File::open(path)
            .map_err(|_| invalid())?
            .take(16385)
            .read_to_end(&mut bytes)
            .map_err(|_| invalid())?;
        if bytes.len() > 16384 {
            return Err(invalid());
        }
        let value: serde_json::Value = serde_json::from_slice(&bytes).map_err(|_| invalid())?;
        let user = value["username"]
            .as_str()
            .filter(|s| !s.is_empty())
            .ok_or_else(invalid)?
            .to_owned();
        let password = value["password"]
            .as_str()
            .filter(|s| !s.is_empty())
            .ok_or_else(invalid)?
            .to_owned();
        Ok(Some((user, password)))
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        Err("cli fixture login pipe unsupported".to_owned())
    }
}
