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
        write_durably(&path, &payload).map_err(|_| CoreFailure::StoreUnavailable)
    }

    pub(crate) fn account_mention_log_file(&self, key_id: &SessionKeyId) -> PathBuf {
        self.account_root_dir(key_id)
            .join("mention-log")
            .join("mentions.v1.enc")
    }
}

/// Writes `payload` to a uniquely named sibling, flushes it to disk, renames it
/// over `path` and flushes the directory, so a committed row survives power
/// loss and two writers never share a temporary file.
pub(super) fn write_durably(path: &std::path::Path, payload: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let parent = path
        .parent()
        .ok_or_else(|| std::io::Error::other("log path has no parent"))?;
    let temporary = path.with_extension(format!(
        "{}-{}.tmp",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    let result = (|| {
        let mut file = std::fs::File::create(&temporary)?;
        file.write_all(payload)?;
        file.sync_all()?;
        drop(file);
        std::fs::rename(&temporary, path)?;
        sync_directory(parent)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result
}

/// Flushes a directory's entries (the rename) to disk. Windows cannot open a
/// directory for this; NTFS journals the rename, so it is skipped there.
pub(super) fn sync_directory(dir: &std::path::Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        std::fs::File::open(dir)?.sync_all()
    }
    #[cfg(not(unix))]
    {
        let _ = dir;
        Ok(())
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_durable_write_replaces_the_file_and_leaves_no_temporary() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("mentions.v1.enc");
        write_durably(&path, b"one").expect("first");
        write_durably(&path, b"two").expect("second");
        assert_eq!(std::fs::read(&path).expect("read"), b"two");
        let names: Vec<_> = std::fs::read_dir(dir.path())
            .expect("dir")
            .map(|entry| entry.expect("entry").file_name())
            .collect();
        assert_eq!(names, [std::ffi::OsString::from("mentions.v1.enc")]);
    }

    #[test]
    fn the_directory_is_synced_and_a_missing_one_is_an_error() {
        let dir = tempfile::tempdir().expect("tempdir");
        sync_directory(dir.path()).expect("sync");
        #[cfg(unix)]
        assert!(sync_directory(&dir.path().join("absent")).is_err());
        // A failed write cleans up after itself.
        assert!(write_durably(&dir.path().join("absent").join("f"), b"x").is_err());
    }

    #[test]
    fn concurrent_writers_do_not_share_a_temporary() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("mentions.v1.enc");
        std::thread::scope(|scope| {
            for index in 0..8u8 {
                let path = path.clone();
                scope.spawn(move || write_durably(&path, &[index; 64]).expect("write"));
            }
        });
        let bytes = std::fs::read(&path).expect("read");
        assert_eq!(bytes.len(), 64);
        assert!(bytes.iter().all(|byte| *byte == bytes[0]));
    }
}
