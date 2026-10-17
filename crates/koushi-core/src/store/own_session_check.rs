//! #1505: when this account's own sessions were last checked.
//!
//! Canon: `docs/architecture/state-machine.md`, "Own-session check". The
//! record makes the 24-hour check interval survive restarts. It names the
//! device it was written for, so a new login (a new device) is never mistaken
//! for a recently checked one. It is account-scoped, envelope-encrypted with
//! its own derived key, and never sent to a homeserver.

use super::{CoreFailure, StoreActor};
use koushi_key::LocalUnlockSecret;
use koushi_protocol::SessionKeyId;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

const OWN_SESSION_CHECK_FILE_MAGIC: &[u8] = b"KOUSHI-OWN-SESSION-CHECK-V1\0";

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
struct OwnSessionCheckRecord {
    device_id: String,
    last_checked_at_ms: u64,
}

impl StoreActor {
    /// The last check of this exact session, or `None` when it was never
    /// checked. An unreadable record counts as never checked: the cost is one
    /// extra check, never a missed one.
    pub fn load_own_session_last_checked_at(&self, key_id: &SessionKeyId) -> Option<u64> {
        let bytes = std::fs::read(self.account_own_session_check_file(key_id)).ok()?;
        let secret = self.load_unlock_secret(key_id).ok()?;
        let record = decrypt_record(&secret, &bytes).ok()?;
        (record.device_id == key_id.device_id).then_some(record.last_checked_at_ms)
    }

    pub fn save_own_session_last_checked_at(
        &self,
        key_id: &SessionKeyId,
        last_checked_at_ms: u64,
    ) -> Result<(), CoreFailure> {
        let path = self.account_own_session_check_file(key_id);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|_| CoreFailure::StoreUnavailable)?;
        }
        let record = OwnSessionCheckRecord {
            device_id: key_id.device_id.clone(),
            last_checked_at_ms,
        };
        let payload = encrypt_record(&self.load_or_create_unlock_secret(key_id)?, &record)?;
        koushi_store::atomic_replace_file(&path, &payload, false)
            .map_err(|_| CoreFailure::StoreUnavailable)
    }

    fn account_own_session_check_file(&self, key_id: &SessionKeyId) -> PathBuf {
        self.account_root_dir(key_id)
            .join("own-session-check")
            .join("check.v1.enc")
    }
}

fn encrypt_record(
    secret: &LocalUnlockSecret,
    record: &OwnSessionCheckRecord,
) -> Result<Vec<u8>, CoreFailure> {
    let plaintext = serde_json::to_vec(record).map_err(|_| CoreFailure::StoreUnavailable)?;
    let key = secret.derive_own_session_check_key();
    koushi_store::encrypt_envelope(
        OWN_SESSION_CHECK_FILE_MAGIC,
        key.as_bytes(),
        &plaintext,
        usize::MAX,
    )
    .map_err(|_| CoreFailure::StoreUnavailable)
}

fn decrypt_record(
    secret: &LocalUnlockSecret,
    payload: &[u8],
) -> Result<OwnSessionCheckRecord, CoreFailure> {
    let key = secret.derive_own_session_check_key();
    let plaintext = koushi_store::decrypt_envelope(
        OWN_SESSION_CHECK_FILE_MAGIC,
        key.as_bytes(),
        payload,
        usize::MAX,
    )
    .map_err(|_| CoreFailure::StoreUnavailable)?;
    serde_json::from_slice(&plaintext).map_err(|_| CoreFailure::StoreUnavailable)
}

#[cfg(test)]
mod tests {
    use super::super::test_support::{file_store_actor, make_key_id};
    use tempfile::tempdir;

    #[test]
    fn the_last_check_round_trips_encrypted_and_is_bound_to_the_device() {
        let data_dir = tempdir().expect("tempdir");
        let cred_dir = tempdir().expect("tempdir");
        let key_id = make_key_id();
        let actor = file_store_actor(&data_dir, &cred_dir);
        assert_eq!(actor.load_own_session_last_checked_at(&key_id), None);

        actor
            .save_own_session_last_checked_at(&key_id, 1_700_000_000_123)
            .expect("save last check");
        let bytes = std::fs::read(actor.account_own_session_check_file(&key_id))
            .expect("read encrypted record");
        let timestamp = 1_700_000_000_123u64.to_string();
        assert!(
            !bytes
                .windows(timestamp.len())
                .any(|window| window == timestamp.as_bytes())
        );
        assert_eq!(
            actor.load_own_session_last_checked_at(&key_id),
            Some(1_700_000_000_123)
        );

        // A new login on the same account is a new device: never checked.
        let mut new_login = key_id.clone();
        new_login.device_id = format!("{}-NEW", key_id.device_id);
        if actor.account_root_dir(&new_login) == actor.account_root_dir(&key_id) {
            assert_eq!(actor.load_own_session_last_checked_at(&new_login), None);
        }
    }

    #[test]
    fn a_corrupted_record_counts_as_never_checked() {
        let data_dir = tempdir().expect("tempdir");
        let cred_dir = tempdir().expect("tempdir");
        let key_id = make_key_id();
        let actor = file_store_actor(&data_dir, &cred_dir);
        actor
            .save_own_session_last_checked_at(&key_id, 42)
            .expect("save last check");
        let path = actor.account_own_session_check_file(&key_id);
        let mut bytes = std::fs::read(&path).expect("read");
        *bytes.last_mut().expect("non-empty") ^= 0x01;
        std::fs::write(&path, bytes).expect("corrupt");
        assert_eq!(actor.load_own_session_last_checked_at(&key_id), None);
    }
}
