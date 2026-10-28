//! Optional disposable encrypted fixture for the deterministic CLI QA lane.
use super::super::{event_wait, fixtures, participants::QaParticipantLoginOutcome};
use koushi_protocol::{
    command::{CoreCommand, TimelineCommand},
    ids::TimelineKey,
};

pub(super) async fn seed(participant: &mut QaParticipantLoginOutcome) -> Result<(), String> {
    let room = fixtures::create_room_for_qa(
        &mut participant.conn,
        "Synthetic CLI QA",
        true,
        "cli fixture room",
    )
    .await?;
    let key = TimelineKey::room(participant.account_key.clone(), room);
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
    Ok(())
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
