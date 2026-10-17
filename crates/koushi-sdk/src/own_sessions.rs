//! #1505: read-only inventory of the account's own sessions.
//!
//! The server's device list supplies names and last-seen times; the crypto
//! store supplies trust. Only reads: renaming and signing out other sessions
//! stay with the account's server destination ("Manage account & devices").

use koushi_state::{DeviceTrustLevel, DeviceTrustSummary};

use crate::MatrixClientSession;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OwnSessionsInventory {
    /// Every session of the account, the current one included.
    pub devices: Vec<DeviceTrustSummary>,
    /// Whether the current session is verified. Without that, the trust of the
    /// other sessions is not meaningful.
    pub current_session_verified: bool,
}

/// The inventory could not be read; the caller retries later.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct OwnSessionsInventoryError;

pub async fn inspect_own_sessions(
    session: &MatrixClientSession,
) -> Result<OwnSessionsInventory, OwnSessionsInventoryError> {
    let client = &session.client;
    let user_id = client
        .user_id()
        .ok_or(OwnSessionsInventoryError)?
        .to_owned();
    let current_device_id = client
        .device_id()
        .ok_or(OwnSessionsInventoryError)?
        .to_owned();
    let server_devices = client
        .devices()
        .await
        .map_err(|_| OwnSessionsInventoryError)?
        .devices;
    let crypto_devices = client
        .encryption()
        .get_user_devices(&user_id)
        .await
        .map_err(|_| OwnSessionsInventoryError)?;

    let mut devices = Vec::with_capacity(server_devices.len());
    for device in server_devices {
        let crypto = crypto_devices.get(&device.device_id);
        // A dehydrated device is a key-delivery mailbox, not a signed-in
        // session (Element X hides it too).
        if crypto.as_ref().is_some_and(|crypto| crypto.is_dehydrated()) {
            continue;
        }
        devices.push(DeviceTrustSummary {
            user_id: user_id.to_string(),
            device_id: device.device_id.to_string(),
            trust_level: own_session_trust_level(
                crypto
                    .as_ref()
                    .map(|crypto| (crypto.is_verified(), crypto.is_blacklisted())),
            ),
            display_name: device.display_name,
            last_seen_ts: device.last_seen_ts.map(|ts| u64::from(ts.get())),
            is_current: device.device_id == current_device_id,
        });
    }
    let current_session_verified = crypto_devices
        .get(&current_device_id)
        .is_some_and(|device| device.is_verified());
    Ok(OwnSessionsInventory {
        devices,
        current_session_verified,
    })
}

/// A session the crypto store has no keys for (it never set up encryption, or
/// its keys have not been fetched yet) is unknown, which the alert counts as
/// not verified.
fn own_session_trust_level(crypto: Option<(bool, bool)>) -> DeviceTrustLevel {
    match crypto {
        None => DeviceTrustLevel::Unknown,
        Some((_, true)) => DeviceTrustLevel::Blocked,
        Some((true, false)) => DeviceTrustLevel::Verified,
        Some((false, false)) => DeviceTrustLevel::Unverified,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trust_level_follows_the_crypto_store() {
        assert_eq!(own_session_trust_level(None), DeviceTrustLevel::Unknown);
        assert_eq!(
            own_session_trust_level(Some((true, false))),
            DeviceTrustLevel::Verified
        );
        assert_eq!(
            own_session_trust_level(Some((false, false))),
            DeviceTrustLevel::Unverified
        );
        assert_eq!(
            own_session_trust_level(Some((true, true))),
            DeviceTrustLevel::Blocked
        );
    }
}
