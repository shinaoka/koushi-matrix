//! Local migration sources for #1871: detection and room-key staging.
//!
//! Platform capability only. The adapter asks Element Desktop for its own room-key
//! export and stages what it got into a private (`0600`) file. What Koushi keeps
//! from that file and what "imported" means belongs to Core, which consumes the
//! staging file through `AccountCommand::ImportMigratedRoomKeys` and deletes it
//! afterwards.
//!
//! Element X is detected and never imported: its crypto store is encrypted under a
//! key it keeps in its own keychain, so the migration path for those accounts is
//! verifying from the old device with secret sharing.

mod element_desktop;
mod element_x;

#[cfg(test)]
mod element_desktop_tests;
#[cfg(test)]
mod element_x_tests;
#[cfg(test)]
mod staging_tests;

use serde::Serialize;
use std::{fmt, path::PathBuf};

/// Staging layout accepted by `koushi_sdk::import_migrated_room_keys`.
pub(crate) const STAGING_VERSION: u32 = 1;

/// What the settings surface shows before the user picks a source. Carries
/// counts and stable codes only: no key material, no filesystem paths.
#[derive(Clone, Debug, Default, Serialize)]
pub(crate) struct MigrationSources {
    pub(crate) element_x: MigrationElementX,
    pub(crate) element_desktop: MigrationElementDesktop,
    /// Set when a source could not be inspected at all: the platform has no
    /// detection, or Element X's group container could not be listed. A count of
    /// zero is only trustworthy when this is `None`.
    pub(crate) unsupported_reason: Option<&'static str>,
}

/// Element X never offers an import; detection only says how much is there and
/// why it stays where it is.
#[derive(Clone, Debug, Default, Serialize)]
pub(crate) struct MigrationElementX {
    /// Session directories holding a crypto store. Zero means the account has no
    /// Element X data on this machine, unless `unsupported_reason` says the
    /// container could not be listed.
    pub(crate) sessions: u64,
    /// Stable code for the message catalog when `sessions > 0`.
    pub(crate) unavailable_reason: Option<&'static str>,
}

#[derive(Clone, Debug, Default, Serialize)]
pub(crate) struct MigrationElementDesktop {
    pub(crate) installed: bool,
    pub(crate) running: bool,
    /// Application-support directory name, for example `Element`.
    pub(crate) profile_hint: Option<String>,
    pub(crate) importable: bool,
}

/// Room keys read out of a local source and written to a staging file.
pub(crate) struct StagedRoomKeys {
    pub(crate) path: PathBuf,
    /// Keys handed to Core.
    pub(crate) keys: u64,
    /// Distinct rooms those keys cover, as far as the source could tell.
    pub(crate) rooms: u64,
}

#[derive(Debug)]
pub(crate) enum MigrationError {
    /// Detection runs on macOS only in this phase.
    UnsupportedPlatform,
    /// The source application reported its own failure.
    Source(element_desktop::ElementDesktopError),
    /// The staging file could not be written.
    Staging,
}

impl fmt::Display for MigrationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::UnsupportedPlatform => "migration_source_unsupported_platform",
            Self::Source(error) => error.code(),
            Self::Staging => "migration_staging_failed",
        })
    }
}

impl From<element_desktop::ElementDesktopError> for MigrationError {
    fn from(error: element_desktop::ElementDesktopError) -> Self {
        Self::Source(error)
    }
}

pub(crate) fn detect_sources() -> MigrationSources {
    sweep_stale_staging();
    if !cfg!(target_os = "macos") {
        return MigrationSources {
            unsupported_reason: Some("migration_platform_unsupported"),
            ..MigrationSources::default()
        };
    }
    let (element_x_sessions, element_x_unreadable) = element_x::detect_sessions();
    MigrationSources {
        element_x: MigrationElementX {
            sessions: element_x_sessions,
            unavailable_reason: (element_x_sessions > 0)
                .then_some("migration_element_x_encrypted_store"),
        },
        element_desktop: element_desktop::detect(),
        // A group container this process may not read looks exactly like "no
        // data" without this hint, and the UI would offer the wrong fallback.
        unsupported_reason: element_x_unreadable.then_some("migration_element_x_unreadable"),
    }
}

pub(crate) async fn stage_element_desktop() -> Result<StagedRoomKeys, MigrationError> {
    if !cfg!(target_os = "macos") {
        return Err(MigrationError::UnsupportedPlatform);
    }
    let keys = element_desktop::export_room_keys().await?;
    stage(keys)
}

/// A private staging file Core reads once and deletes. `tempfile` creates it
/// with owner-only permissions.
pub(crate) fn stage(keys: Vec<serde_json::Value>) -> Result<StagedRoomKeys, MigrationError> {
    sweep_stale_staging();
    #[derive(Serialize)]
    struct StagingFile<'a> {
        version: u32,
        keys: &'a [serde_json::Value],
    }
    let rooms = keys
        .iter()
        .filter_map(|key| key.get("room_id").and_then(|room| room.as_str()))
        .collect::<std::collections::BTreeSet<_>>()
        .len() as u64;
    let keys_staged = keys.len() as u64;
    let file = tempfile::Builder::new()
        .prefix("koushi-migrated-keys-")
        .suffix(".json")
        .tempfile()
        .map_err(|_| MigrationError::Staging)?;
    let staging = StagingFile {
        version: STAGING_VERSION,
        keys: &keys,
    };
    serde_json::to_writer(file.as_file(), &staging).map_err(|_| MigrationError::Staging)?;
    let path = file
        .into_temp_path()
        .keep()
        .map_err(|_| MigrationError::Staging)?;
    Ok(StagedRoomKeys {
        path,
        keys: keys_staged,
        rooms,
    })
}

/// Deletes every staging file at app start. Core consumes the file it is handed,
/// but a command rejected before it reaches the handler (for example a session
/// that ended between admission and dispatch) leaves it behind with no further
/// chance to be deleted.
pub(crate) fn sweep_stale_staging_at_startup() {
    sweep_staging_files(&std::env::temp_dir(), |_| true);
}

/// Deletes staging files an interrupted import left behind. Core removes each
/// file it consumes, so this only has work to do after a crash or a quit
/// between admission and the actor call; without it plaintext keys would stay in
/// the temp directory indefinitely. Files younger than an hour are left alone so
/// two windows importing at once cannot delete each other's staging.
fn sweep_stale_staging() {
    const MAX_AGE: std::time::Duration = std::time::Duration::from_secs(60 * 60);
    sweep_staging_files(&std::env::temp_dir(), |age| {
        age.is_none_or(|age| age > MAX_AGE)
    });
}

fn sweep_staging_files(
    root: &std::path::Path,
    mut delete: impl FnMut(Option<std::time::Duration>) -> bool,
) {
    const PREFIX: &str = "koushi-migrated-keys-";
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        if !entry.file_name().to_string_lossy().starts_with(PREFIX) {
            continue;
        }
        let age = entry
            .metadata()
            .and_then(|metadata| metadata.modified())
            .ok()
            .and_then(|modified| modified.elapsed().ok());
        if delete(age) {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}
