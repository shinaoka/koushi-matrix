use super::{CoreFailure, StoreActor};
use koushi_key::LocalUnlockSecret;
use koushi_protocol::SessionKeyId;
use koushi_state::NavigationState;
use std::io::Write;
use std::path::PathBuf;

const NAVIGATION_FILE_MAGIC: &[u8] = b"KOUSHI-NAVIGATION-V1\0";

fn atomic_replace(path: &std::path::Path, payload: &[u8]) -> Result<(), CoreFailure> {
    let temporary_path = path.with_extension("tmp");
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(&temporary_path)
        .map_err(|_| CoreFailure::StoreUnavailable)?;
    file.write_all(payload)
        .and_then(|_| file.sync_all())
        .map_err(|_| CoreFailure::StoreUnavailable)?;
    std::fs::rename(&temporary_path, path).map_err(|_| CoreFailure::StoreUnavailable)
}

impl StoreActor {
    pub fn load_navigation(&self, key_id: &SessionKeyId) -> Result<NavigationState, CoreFailure> {
        let path = self.account_navigation_file(key_id);
        let bytes = match std::fs::read(&path) {
            Ok(bytes) => bytes,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                return self.load_legacy_navigation(key_id);
            }
            Err(_) => return Err(CoreFailure::StoreUnavailable),
        };
        Ok(decrypt_navigation_payload(&self.load_unlock_secret(key_id)?, &bytes)?.persistence_view())
    }

    pub fn save_navigation(
        &self,
        key_id: &SessionKeyId,
        navigation: &NavigationState,
    ) -> Result<(), CoreFailure> {
        let path = self.account_navigation_file(key_id);
        let legacy_path = self.account_navigation_legacy_file(key_id);
        if navigation == &NavigationState::default() {
            match std::fs::remove_file(&path) {
                Ok(()) => {}
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
                Err(_) => return Err(CoreFailure::StoreUnavailable),
            }
            match std::fs::remove_file(&legacy_path) {
                Ok(()) => {}
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
                Err(_) => return Err(CoreFailure::StoreUnavailable),
            }
            return Ok(());
        }
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|_| CoreFailure::StoreUnavailable)?;
        }
        let payload = encrypt_navigation_payload(
            &self.load_or_create_unlock_secret(key_id)?,
            &navigation.persistence_view(),
        )?;
        atomic_replace(&path, &payload)?;
        match std::fs::remove_file(&legacy_path) {
            Ok(()) => Ok(()),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(_) => Err(CoreFailure::StoreUnavailable),
        }
    }

    fn load_legacy_navigation(
        &self,
        key_id: &SessionKeyId,
    ) -> Result<NavigationState, CoreFailure> {
        let path = self.account_navigation_legacy_file(key_id);
        let json = match std::fs::read_to_string(&path) {
            Ok(json) => json,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                return Ok(NavigationState::default());
            }
            Err(_) => return Err(CoreFailure::StoreUnavailable),
        };
        serde_json::from_str(&json)
            .map(|navigation: NavigationState| navigation.persistence_view())
            .map_err(|_| CoreFailure::StoreUnavailable)
    }

    fn account_navigation_file(&self, key_id: &SessionKeyId) -> PathBuf {
        self.account_root_dir(key_id)
            .join("navigation")
            .join("navigation.v1.enc")
    }

    fn account_navigation_legacy_file(&self, key_id: &SessionKeyId) -> PathBuf {
        self.account_root_dir(key_id)
            .join("navigation")
            .join("navigation.v1.json")
    }
}

fn encrypt_navigation_payload(
    secret: &LocalUnlockSecret,
    navigation: &NavigationState,
) -> Result<Vec<u8>, CoreFailure> {
    let plaintext = serde_json::to_vec(navigation).map_err(|_| CoreFailure::StoreUnavailable)?;
    let key = secret.derive_navigation_key();
    koushi_store::encrypt_envelope(
        NAVIGATION_FILE_MAGIC,
        key.as_bytes(),
        &plaintext,
        usize::MAX,
    )
    .map_err(|_| CoreFailure::StoreUnavailable)
}

fn decrypt_navigation_payload(
    secret: &LocalUnlockSecret,
    payload: &[u8],
) -> Result<NavigationState, CoreFailure> {
    let key = secret.derive_navigation_key();
    let plaintext =
        koushi_store::decrypt_envelope(NAVIGATION_FILE_MAGIC, key.as_bytes(), payload, usize::MAX)
            .map_err(|_| CoreFailure::StoreUnavailable)?;
    serde_json::from_slice(&plaintext).map_err(|_| CoreFailure::StoreUnavailable)
}

#[cfg(test)]
mod tests;
