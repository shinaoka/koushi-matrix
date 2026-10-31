//! Same-device master rotations must invalidate paused historical recovery.
use super::*;
use matrix_sdk::encryption::CrossSigningResetAuthType;

async fn rotate_and_refresh(
    observer: &Client,
    peer: &Client,
    password: &str,
) -> Result<(), String> {
    let user = peer.user_id().ok_or("history recovery: peer missing")?;
    let device_id = peer.device_id().ok_or("history recovery: device missing")?;
    let old_identity = observer
        .encryption()
        .request_user_identity(user)
        .await
        .map_err(|_| "history recovery: old identity query failed")?
        .ok_or("history recovery: old identity missing")?;
    let old_device = observer
        .encryption()
        .get_device(user, device_id)
        .await
        .map_err(|_| "history recovery: old device query failed")?
        .ok_or("history recovery: old device missing")?;
    if let Some(handle) = peer
        .encryption()
        .reset_cross_signing()
        .await
        .map_err(|_| "history recovery: identity reset failed")?
    {
        let CrossSigningResetAuthType::Uiaa(info) = handle.auth_type() else {
            return Err("history recovery: unsupported reset authentication".to_owned());
        };
        handle
            .auth(info.as_password_auth_data(user.to_owned().into(), password.to_owned()))
            .await
            .map_err(|_| "history recovery: reset authentication failed")?;
    }
    let current = observer
        .encryption()
        .request_user_identity(user)
        .await
        .map_err(|_| "history recovery: new identity query failed")?
        .ok_or("history recovery: new identity missing")?;
    let device = observer
        .encryption()
        .get_device(user, device_id)
        .await
        .map_err(|_| "history recovery: new device query failed")?
        .ok_or("history recovery: new device missing")?;
    if old_identity.master_key().get_first_key() == current.master_key().get_first_key()
        || old_device.curve25519_key() != device.curve25519_key()
        || old_device.ed25519_key() != device.ed25519_key()
    {
        return Err("history recovery: expected master-only identity rotation".to_owned());
    }
    assert_unverified_peer(observer, peer).await
}

pub(super) async fn run(
    config: &QaConfig,
    sender: &Participant,
    recipient: &Participant,
    a: &CoreFixture,
    b: &CoreFixture,
) -> Result<(), String> {
    // The responder has admitted an exact owner pin, then the requester changes
    // only its cross-signing identity while disclosure is paused.
    let disclosure = public_history(sender).await?;
    let (reached, resume) = a
        .runtime
        .as_ref()
        .ok_or("history recovery: runtime missing")?
        .pause_next_history_key_disclosure_for_testing()
        .await
        .ok_or("history recovery: disclosure pause unavailable")?;
    join(recipient, &disclosure.room_id).await?;
    b.request(&disclosure, false, 1).await?;
    tokio::time::timeout(WAIT, reached)
        .await
        .map_err(|_| "history recovery: disclosure identity pause timed out")?
        .map_err(|_| "history recovery: disclosure identity pause cancelled")?;
    rotate_and_refresh(
        &sender.session().client(),
        &recipient.session().client(),
        &config.password_b,
    )
    .await?;
    let _ = resume.send(());
    rejected(a, &disclosure, 0).await?;
    if !no_keys(recipient, &disclosure).await? {
        return Err("history recovery: keys disclosed after master replacement".to_owned());
    }

    // The requester already has an admitted chunk when the forwarder's master
    // changes. A matching device key alone cannot authorize the pending import.
    let import = public_history(sender).await?;
    let (reached, resume) = b
        .runtime
        .as_ref()
        .ok_or("history recovery: runtime missing")?
        .pause_next_history_key_import_for_testing()
        .await
        .ok_or("history recovery: import pause unavailable")?;
    join(recipient, &import.room_id).await?;
    b.request(&import, false, 1).await?;
    tokio::time::timeout(WAIT, reached)
        .await
        .map_err(|_| "history recovery: import identity pause timed out")?
        .map_err(|_| "history recovery: import identity pause cancelled")?;
    rotate_and_refresh(
        &recipient.session().client(),
        &sender.session().client(),
        &config.password_a,
    )
    .await?;
    let _ = resume.send(());
    rejected(b, &import, 0).await?;
    if !no_keys(recipient, &import).await? {
        return Err("history recovery: keys imported after master replacement".to_owned());
    }
    println!("history_recovery_identity_race_guard=ok");
    Ok(())
}
