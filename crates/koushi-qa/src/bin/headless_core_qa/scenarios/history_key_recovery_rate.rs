//! Exercise the responder rate ledger through real encrypted requests and replies.
use super::*;
use koushi_state::HistoryKeyRecoveryFailure as Failure;

pub(super) async fn run(
    sender: &mut Participant,
    recipient: &mut Participant,
    responder: &mut CoreFixture,
    requester: &CoreFixture,
) -> Result<(), String> {
    expired(sender, recipient, responder, requester).await?;
    let history = public_history(sender).await?;
    join(recipient, &history.room_id).await?;
    responder.stop().await?;
    sender.restart_with_sync(false).await?;
    let client = sender.session().client();
    let mut saved = ledger(&client).await?;
    let original = saved["responded"].clone();
    let future = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_err(|_| "history recovery: clock unavailable")?
        .as_secs()
        + 300;
    let entries: serde_json::Map<String, serde_json::Value> = (0..512)
        .map(|n| {
            (
                format!("!fixture:example.invalid\n@member{n}:example.invalid"),
                future.into(),
            )
        })
        .collect();
    saved["responded"] = entries.into();
    client
        .encryption()
        .set_application_crypto_data(
            "chat.koushi.history_keys.v1",
            &serde_json::to_vec(&saved).map_err(|_| "history recovery: ledger encoding failed")?,
        )
        .await
        .map_err(|_| "history recovery: rate fixture write failed")?;
    *responder = CoreFixture::start(sender).await?;
    let result = async {
        let request_id = requester.conn().next_request_id();
        requester
            .conn()
            .command(CoreCommand::Room(RoomCommand::RequestHistoryKeys {
                request_id,
                room_id: history.room_id.to_string(),
                force_all: false,
            }))
            .await
            .map_err(|_| "history recovery: rate request rejected")?;
        wait_until("rate capacity refusal", || async {
            Ok(requester
                .conn()
                .snapshot()
                .room_management
                .history_keys
                .get(history.room_id.as_str())
                .is_some_and(|s| {
                    matches!(
                        s.status,
                        Status::Failed {
                            reason: Failure::RateLimited
                        }
                    )
                }))
        })
        .await?;
        requester
            .runtime
            .as_ref()
            .ok_or("history recovery: runtime missing")?
            .history_recovery_barrier_for_testing()
            .await
            .ok_or("history recovery: requester barrier failed")?;
        let pending = ledger(&recipient.session().client()).await?;
        let attempts = pending["rooms"][history.room_id.as_str()]["attempts"]
            .as_object()
            .ok_or("history recovery: rate attempts missing")?;
        if attempts.len() != 1 || attempts.values().any(|a| a["terminal"] != false) {
            return Err("history recovery: rate refusal ended the attempt".to_owned());
        }
        let responder_ledger = ledger(&client).await?;
        if responder_ledger["responded"].as_object().map(|v| v.len()) != Some(512) {
            return Err("history recovery: rate capacity changed".to_owned());
        }
        if responder_ledger["cached"]
            .as_object()
            .ok_or("history recovery: response cache missing")?
            .values()
            .any(|entry| entry["room"] == history.room_id.as_str())
        {
            return Err("history recovery: rate refusal exported keys".to_owned());
        }
        Ok(())
    }
    .await;
    drop(client);
    responder.stop().await?;
    sender.restart_with_sync(false).await?;
    let client = sender.session().client();
    let mut current = ledger(&client).await?;
    current["responded"] = original;
    client
        .encryption()
        .set_application_crypto_data(
            "chat.koushi.history_keys.v1",
            &serde_json::to_vec(&current)
                .map_err(|_| "history recovery: ledger encoding failed")?,
        )
        .await
        .map_err(|_| "history recovery: rate fixture restore failed")?;
    *responder = CoreFixture::start(sender).await?;
    result
}

async fn expired(
    sender: &mut Participant,
    recipient: &mut Participant,
    responder: &mut CoreFixture,
    requester: &CoreFixture,
) -> Result<(), String> {
    let same = public_history(sender).await?;
    let new = public_history(sender).await?;
    join(recipient, &same.room_id).await?;
    join(recipient, &new.room_id).await?;
    responder.stop().await?;
    sender.restart_with_sync(false).await?;
    let client = sender.session().client();
    let mut saved = ledger(&client).await?;
    let original = saved["responded"].clone();
    let old = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_err(|_| "history recovery: clock unavailable")?
        .as_secs()
        .saturating_sub(61);
    let mut entries: serde_json::Map<String, serde_json::Value> = (0..511)
        .map(|n| {
            (
                format!("!fixture:example.invalid\n@member{n}:example.invalid"),
                old.into(),
            )
        })
        .collect();
    let requester_client = recipient.session().client();
    let user = requester_client
        .user_id()
        .ok_or("history recovery: user unavailable")?;
    entries.insert(format!("{}\n{user}", same.room_id), old.into());
    saved["responded"] = entries.into();
    client
        .encryption()
        .set_application_crypto_data(
            "chat.koushi.history_keys.v1",
            &serde_json::to_vec(&saved).map_err(|_| "history recovery: ledger encoding failed")?,
        )
        .await
        .map_err(|_| "history recovery: rate fixture write failed")?;
    *responder = CoreFixture::start(sender).await?;
    let result = async {
        for history in [&same, &new] {
            let request_id = requester.conn().next_request_id();
            requester
                .conn()
                .command(CoreCommand::Room(RoomCommand::RequestHistoryKeys {
                    request_id,
                    room_id: history.room_id.to_string(),
                    force_all: false,
                }))
                .await
                .map_err(|_| "history recovery: expired rate request rejected")?;
            wait_until("expired rate request decryption", || {
                historical_messages_decrypt(&requester_client, history)
            })
            .await?;
        }
        Ok(())
    }
    .await;
    drop(client);
    responder.stop().await?;
    sender.restart_with_sync(false).await?;
    let client = sender.session().client();
    let mut current = ledger(&client).await?;
    current["responded"] = original;
    client
        .encryption()
        .set_application_crypto_data(
            "chat.koushi.history_keys.v1",
            &serde_json::to_vec(&current)
                .map_err(|_| "history recovery: ledger encoding failed")?,
        )
        .await
        .map_err(|_| "history recovery: rate fixture restore failed")?;
    *responder = CoreFixture::start(sender).await?;
    result
}
