use super::{CoreFailure, StoreActor};
use crate::mention_feed::MentionLogData;
use koushi_key::LocalUnlockSecret;
use koushi_protocol::SessionKeyId;
use std::path::PathBuf;

/// The magic carries the format version: a future shape change bumps it and
/// ships a reader for `V1`, so an older log is migrated rather than discarded.
const MENTION_LOG_FILE_MAGIC: &[u8] = b"KOUSHI-MENTION-LOG-V1\0";

impl StoreActor {
    /// `None` when the account has no log yet.
    pub fn load_mention_log(
        &self,
        key_id: &SessionKeyId,
    ) -> Result<Option<MentionLogData>, CoreFailure> {
        let bytes = match std::fs::read(self.account_mention_log_file(key_id)) {
            Ok(bytes) => bytes,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(_) => return Err(CoreFailure::StoreUnavailable),
        };
        decrypt_mention_log(&self.load_unlock_secret(key_id)?, &bytes).map(Some)
    }

    /// Writes through a temporary file and renames it, so a crash leaves the
    /// previous committed log rather than a torn one.
    pub fn save_mention_log(
        &self,
        key_id: &SessionKeyId,
        log: &MentionLogData,
    ) -> Result<(), CoreFailure> {
        let path = self.account_mention_log_file(key_id);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|_| CoreFailure::StoreUnavailable)?;
        }
        let payload = encrypt_mention_log(&self.load_or_create_unlock_secret(key_id)?, log)?;
        let temporary = path.with_extension("tmp");
        std::fs::write(&temporary, payload).map_err(|_| CoreFailure::StoreUnavailable)?;
        std::fs::rename(&temporary, &path).map_err(|_| CoreFailure::StoreUnavailable)
    }

    pub(crate) fn account_mention_log_file(&self, key_id: &SessionKeyId) -> PathBuf {
        self.account_root_dir(key_id)
            .join("mention-log")
            .join("mentions.v1.enc")
    }
}

fn encrypt_mention_log(
    secret: &LocalUnlockSecret,
    log: &MentionLogData,
) -> Result<Vec<u8>, CoreFailure> {
    let plaintext = serde_json::to_vec(log).map_err(|_| CoreFailure::StoreUnavailable)?;
    let key = secret.derive_mention_log_key();
    koushi_store::encrypt_envelope(
        MENTION_LOG_FILE_MAGIC,
        key.as_bytes(),
        &plaintext,
        usize::MAX,
    )
    .map_err(|_| CoreFailure::StoreUnavailable)
}

fn decrypt_mention_log(
    secret: &LocalUnlockSecret,
    payload: &[u8],
) -> Result<MentionLogData, CoreFailure> {
    let key = secret.derive_mention_log_key();
    let plaintext =
        koushi_store::decrypt_envelope(MENTION_LOG_FILE_MAGIC, key.as_bytes(), payload, usize::MAX)
            .map_err(|_| CoreFailure::StoreUnavailable)?;
    serde_json::from_slice(&plaintext).map_err(|_| CoreFailure::StoreUnavailable)
}
