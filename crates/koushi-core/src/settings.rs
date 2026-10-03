use std::path::{Path, PathBuf};

use koushi_state::SettingsValues;
use koushi_store::atomic_replace_file;

/// Version of the on-disk `settings/settings.json` schema.
///
/// Files written before this field existed are version 0. Version 1 (#1034)
/// resets `display.encrypted_url_previews_enabled`: version-0 files persisted
/// the whole `SettingsValues`, including the retired `true` default, whenever
/// the user saved any unrelated setting, so they cannot distinguish an
/// explicit opt-in from that default. After migration only an opt-in written
/// with version 1 or later enables encrypted-room link previews.
///
/// Version 2 (#1054) resets `notifications.message_previews` for the same
/// reason: version-0 and version-1 files persisted the retired ON default, so
/// after migration only an opt-in written with version 2 or later shows
/// message content in OS notifications.
pub const SETTINGS_SCHEMA_VERSION: u32 = 2;

const SCHEMA_VERSION_KEY: &str = "schema_version";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SettingsStoreErrorKind {
    Io,
    Corrupt,
}

#[derive(Debug)]
pub struct SettingsStoreError {
    kind: SettingsStoreErrorKind,
}

impl SettingsStoreError {
    pub fn kind(&self) -> SettingsStoreErrorKind {
        self.kind
    }

    fn corrupt() -> Self {
        Self {
            kind: SettingsStoreErrorKind::Corrupt,
        }
    }

    fn io() -> Self {
        Self {
            kind: SettingsStoreErrorKind::Io,
        }
    }
}

#[derive(Clone)]
pub struct SettingsStore {
    path: PathBuf,
}

impl SettingsStore {
    pub fn new(data_dir: impl AsRef<Path>) -> Self {
        Self {
            path: data_dir.as_ref().join("settings").join("settings.json"),
        }
    }

    /// Loads persisted settings, migrating older schema versions.
    ///
    /// A migrated file is rewritten with [`SETTINGS_SCHEMA_VERSION`]. If that
    /// rewrite fails the migrated values are still returned; the next load
    /// migrates the unchanged file again, which is idempotent.
    pub fn load(&self) -> Result<SettingsValues, SettingsStoreError> {
        let json = match std::fs::read_to_string(&self.path) {
            Ok(json) => json,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                return Ok(SettingsValues::default());
            }
            Err(_) => return Err(SettingsStoreError::io()),
        };
        let mut document: serde_json::Value =
            serde_json::from_str(&json).map_err(|_| SettingsStoreError::corrupt())?;
        let object = document
            .as_object_mut()
            .ok_or_else(SettingsStoreError::corrupt)?;
        let version = match object.remove(SCHEMA_VERSION_KEY) {
            None => 0,
            Some(value) => value
                .as_u64()
                .and_then(|version| u32::try_from(version).ok())
                .ok_or_else(SettingsStoreError::corrupt)?,
        };
        let mut values: SettingsValues =
            serde_json::from_value(document).map_err(|_| SettingsStoreError::corrupt())?;
        if version < SETTINGS_SCHEMA_VERSION {
            migrate(&mut values, version);
            let _ = self.save(&values);
        }
        Ok(values)
    }

    pub fn save(&self, values: &SettingsValues) -> Result<(), SettingsStoreError> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent).map_err(|_| SettingsStoreError::io())?;
        }
        let mut document =
            serde_json::to_value(values).map_err(|_| SettingsStoreError::corrupt())?;
        document
            .as_object_mut()
            .ok_or_else(SettingsStoreError::corrupt)?
            .insert(
                SCHEMA_VERSION_KEY.to_owned(),
                serde_json::Value::from(SETTINGS_SCHEMA_VERSION),
            );
        let json =
            serde_json::to_string_pretty(&document).map_err(|_| SettingsStoreError::corrupt())?;
        atomic_replace_file(&self.path, format!("{json}\n").as_bytes(), false)
            .map_err(|_| SettingsStoreError::io())
    }
}

fn migrate(values: &mut SettingsValues, from_version: u32) {
    if from_version < 1 {
        // #1034: keep the privacy-conservative value for encrypted-room link
        // previews; the user can opt in again from Settings.
        values.display.encrypted_url_previews_enabled = false;
    }
    if from_version < 2 {
        // #1054: OS notifications carry counts only until the user opts in to
        // message previews again from Settings.
        values.notifications.message_previews = false;
    }
}
