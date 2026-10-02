use std::path::PathBuf;

use koushi_key::LocalUnlockSecret;
use koushi_protocol::SessionKeyId;
use koushi_state::AccountSettingsValues;
use serde::{Deserialize, Serialize};

use super::{CoreFailure, StoreActor};

const ACCOUNT_SETTINGS_FILE_MAGIC: &[u8] = b"KOUSHI-ACCOUNT-SETTINGS-V1\0";

#[cfg(test)]
mod tests;

#[derive(Serialize, Deserialize)]
struct AccountSettingsFile {
    version: u8,
    #[serde(default)]
    legacy_migration_complete: bool,
    #[serde(default)]
    values: AccountSettingsValues,
}

impl StoreActor {
    pub fn load_account_settings(
        &self,
        key_id: &SessionKeyId,
    ) -> Result<AccountSettingsValues, CoreFailure> {
        let path = self.account_settings_file(key_id);
        let payload = match std::fs::read(path) {
            Ok(payload) => payload,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(AccountSettingsValues::default());
            }
            Err(_) => return Err(CoreFailure::StoreUnavailable),
        };
        read_account_settings(&self.load_unlock_secret(key_id)?, &payload).map(|file| file.values)
    }

    pub fn save_account_settings(
        &self,
        key_id: &SessionKeyId,
        values: &AccountSettingsValues,
    ) -> Result<(), CoreFailure> {
        write_account_settings(
            &self.load_or_create_unlock_secret(key_id)?,
            &self.account_settings_file(key_id),
            values,
        )
    }

    /// Seed existing accounts once from the old device-wide settings file. The
    /// encrypted file and marker are committed atomically, so retries never
    /// overwrite a later account-specific edit.
    pub fn seed_account_settings_from_legacy(
        &self,
        key_id: &SessionKeyId,
        legacy: &AccountSettingsValues,
    ) -> Result<(), CoreFailure> {
        let path = self.account_settings_file(key_id);
        match std::fs::read(&path) {
            Ok(payload) => {
                let file = read_account_settings(&self.load_unlock_secret(key_id)?, &payload)?;
                if file.legacy_migration_complete {
                    return Ok(());
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err(CoreFailure::StoreUnavailable),
        }
        write_account_settings(&self.load_or_create_unlock_secret(key_id)?, &path, legacy)
    }

    fn account_settings_file(&self, key_id: &SessionKeyId) -> PathBuf {
        self.account_root_dir(key_id)
            .join("settings")
            .join("account-settings.v1.enc")
    }
}

fn read_account_settings(
    secret: &LocalUnlockSecret,
    encrypted: &[u8],
) -> Result<AccountSettingsFile, CoreFailure> {
    let key = secret.derive_account_settings_key();
    let plaintext = koushi_store::decrypt_envelope(
        ACCOUNT_SETTINGS_FILE_MAGIC,
        key.as_bytes(),
        encrypted,
        usize::MAX,
    )
    .map_err(|_| CoreFailure::StoreUnavailable)?;
    let file: AccountSettingsFile =
        serde_json::from_slice(&plaintext).map_err(|_| CoreFailure::StoreUnavailable)?;
    if file.version != 1 {
        return Err(CoreFailure::StoreUnavailable);
    }
    Ok(file)
}

fn write_account_settings(
    secret: &LocalUnlockSecret,
    path: &std::path::Path,
    values: &AccountSettingsValues,
) -> Result<(), CoreFailure> {
    let file = AccountSettingsFile {
        version: 1,
        legacy_migration_complete: true,
        values: values.clone(),
    };
    let plaintext = serde_json::to_vec(&file).map_err(|_| CoreFailure::StoreUnavailable)?;
    let key = secret.derive_account_settings_key();
    let encrypted = koushi_store::encrypt_envelope(
        ACCOUNT_SETTINGS_FILE_MAGIC,
        key.as_bytes(),
        &plaintext,
        usize::MAX,
    )
    .map_err(|_| CoreFailure::StoreUnavailable)?;
    koushi_store::atomic_replace_file(path, &encrypted, false)
        .map_err(|_| CoreFailure::StoreUnavailable)
}
