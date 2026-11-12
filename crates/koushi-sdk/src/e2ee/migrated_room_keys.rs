//! Import of decryption keys exported from another local Matrix app (#1871).
//!
//! The desktop adapter cannot use SDK types: it asks Element Desktop for its own
//! `exportRoomKeys()` JSON and stages that verbatim. Everything needing SDK
//! knowledge — producing the Element-compatible export envelope and the
//! crypto-store import — stays in this module.
//!
//! Staging file (written by the adapter, `0600`, deleted by the caller after the
//! import) is private to this workspace:
//!
//! ```json
//! {"version":1,"keys":[<ExportedRoomKey>]}
//! ```

use super::{E2eeTrustError, MatrixClientSession};
use koushi_state::AuthSecret;
use matrix_sdk_base::crypto::olm::ExportedRoomKey;
use serde::Deserialize;
use std::{collections::BTreeSet, path::PathBuf};

/// Staging layout this importer accepts. The desktop adapter writes the same
/// number in `migration::STAGING_VERSION`; the two move together.
pub(crate) const MIGRATED_ROOM_KEYS_STAGING_VERSION: u32 = 1;

/// PBKDF2 rounds for the throwaway export envelope between the staged keys and
/// the crypto store. The passphrase is 256 random bits that never leave this
/// module and the file lives milliseconds, so the only cost of a high round
/// count here is import latency.
const STAGING_EXPORT_ROUNDS: u32 = 10_000;

/// What a local-app key import did.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct MigratedRoomKeysSummary {
    pub imported_keys: u64,
    /// Keys the crypto store considered, imported or not.
    pub attempted_keys: u64,
    pub imported_rooms: u64,
}

#[derive(Deserialize)]
struct MigratedRoomKeysStaging {
    version: u32,
    #[serde(default)]
    keys: Vec<ExportedRoomKey>,
}

pub async fn import_migrated_room_keys(
    session: &MatrixClientSession,
    staging_path: PathBuf,
) -> Result<MigratedRoomKeysSummary, E2eeTrustError> {
    let staging = read_staging(&staging_path)?;
    import_staged(session, staging).await
}

fn read_staging(path: &std::path::Path) -> Result<MigratedRoomKeysStaging, E2eeTrustError> {
    let bytes = std::fs::read(path)
        .map_err(|error| E2eeTrustError::Sdk(format!("migrated key staging: {error}")))?;
    let staging: MigratedRoomKeysStaging = serde_json::from_slice(&bytes)
        .map_err(|error| E2eeTrustError::Sdk(format!("migrated key staging: {error}")))?;
    if staging.version != MIGRATED_ROOM_KEYS_STAGING_VERSION {
        return Err(E2eeTrustError::Sdk(
            "migrated key staging: unsupported version".to_owned(),
        ));
    }
    Ok(staging)
}

async fn import_staged(
    session: &MatrixClientSession,
    staging: MigratedRoomKeysStaging,
) -> Result<MigratedRoomKeysSummary, E2eeTrustError> {
    let keys = staging.keys;
    if keys.is_empty() {
        return Ok(MigratedRoomKeysSummary::default());
    }

    let rooms = keys
        .iter()
        .map(|key| key.room_id.clone())
        .collect::<BTreeSet<_>>();
    let passphrase = AuthSecret::new(random_passphrase());
    let export = matrix_sdk_base::crypto::encrypt_room_key_export(
        &keys,
        passphrase.expose_secret(),
        STAGING_EXPORT_ROUNDS,
    )
    .map_err(|error| E2eeTrustError::Sdk(format!("migrated key staging: {error}")))?;
    let staged_export = tempfile::NamedTempFile::new()
        .map_err(|error| E2eeTrustError::Sdk(format!("migrated key staging: {error}")))?;
    std::fs::write(staged_export.path(), export.as_bytes())
        .map_err(|error| E2eeTrustError::Sdk(format!("migrated key staging: {error}")))?;

    // Reuses the manual import path so a migrated key lands exactly like an
    // Element "Export E2E room keys" file the user picked themselves.
    let imported =
        super::import_room_keys_from_file(session, staged_export.path().to_path_buf(), &passphrase)
            .await?;

    Ok(MigratedRoomKeysSummary {
        imported_keys: imported.imported_count,
        attempted_keys: imported.total_count,
        imported_rooms: rooms.len() as u64,
    })
}

fn random_passphrase() -> String {
    rand::random::<[u8; 32]>()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}
