use std::{
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use koushi_state::{
    AccountSettingsValues, AppSettingsValues, SettingsPatch, SettingsPatchScope, SettingsValues,
};
use koushi_store::atomic_replace_file;
use tokio::sync::watch;

/// Version of the on-disk `settings/settings.json` schema.
///
/// Version 3 moves account-owned preferences into each account's encrypted
/// settings file. Legacy account values remain here until the account manager
/// has seeded every saved account.
pub const SETTINGS_SCHEMA_VERSION: u32 = 3;

const SCHEMA_VERSION_KEY: &str = "schema_version";
const LEGACY_ACCOUNT_SETTINGS_KEY: &str = "legacy_account_settings";

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

struct SettingsStoreState {
    loaded: bool,
    values: AppSettingsValues,
    legacy_account_settings: Option<AccountSettingsValues>,
}

#[derive(Clone)]
pub struct SettingsStore {
    path: PathBuf,
    state: Arc<Mutex<SettingsStoreState>>,
    updates: watch::Sender<AppSettingsValues>,
}

impl SettingsStore {
    pub fn new(data_dir: impl AsRef<Path>) -> Self {
        let (updates, _) = watch::channel(AppSettingsValues::default());
        Self {
            path: data_dir.as_ref().join("settings").join("settings.json"),
            state: Arc::new(Mutex::new(SettingsStoreState {
                loaded: false,
                values: AppSettingsValues::default(),
                legacy_account_settings: None,
            })),
            updates,
        }
    }

    /// Loads app-wide preferences, migrating older schema versions. Account
    /// preferences are deliberately returned as defaults; callers load them
    /// from the selected account's encrypted store.
    pub fn load(&self) -> Result<SettingsValues, SettingsStoreError> {
        let mut state = self.state.lock().expect("settings store poisoned");
        ensure_loaded(&self.path, &mut state, &self.updates)?;
        let mut values = SettingsValues::default();
        values.apply_app_settings(&state.values);
        Ok(values)
    }

    pub fn subscribe(&self) -> watch::Receiver<AppSettingsValues> {
        self.updates.subscribe()
    }

    /// Legacy values are retained until the account manager has seeded every
    /// saved account. Seeding each account is idempotent, so startup can retry
    /// after interruption.
    pub fn legacy_account_settings(
        &self,
    ) -> Result<Option<AccountSettingsValues>, SettingsStoreError> {
        let mut state = self.state.lock().expect("settings store poisoned");
        ensure_loaded(&self.path, &mut state, &self.updates)?;
        Ok(state.legacy_account_settings.clone())
    }

    pub fn complete_legacy_account_migration(&self) -> Result<(), SettingsStoreError> {
        let mut state = self.state.lock().expect("settings store poisoned");
        ensure_loaded(&self.path, &mut state, &self.updates)?;
        if state.legacy_account_settings.is_none() {
            return Ok(());
        }
        write_settings_file(&self.path, &state.values, None)?;
        state.legacy_account_settings = None;
        Ok(())
    }

    /// Atomically merges only the app-owned fields from this patch. This keeps
    /// concurrent account runtimes from overwriting one another's settings.
    pub fn save_patch(&self, patch: &SettingsPatch) -> Result<(), SettingsStoreError> {
        if patch.scope == Some(SettingsPatchScope::Account) {
            return Ok(());
        }
        let mut state = self.state.lock().expect("settings store poisoned");
        ensure_loaded(&self.path, &mut state, &self.updates)?;
        let mut updated = state.values.clone();
        updated.apply_patch(patch);
        if updated == state.values {
            return Ok(());
        }
        write_settings_file(&self.path, &updated, state.legacy_account_settings.as_ref())?;
        state.values = updated.clone();
        self.updates.send_replace(updated);
        Ok(())
    }

    /// Save a complete projected state for the legacy front-end import path.
    /// Account-owned values are not written to the device-wide file.
    pub fn save(&self, values: &SettingsValues) -> Result<(), SettingsStoreError> {
        let mut state = self.state.lock().expect("settings store poisoned");
        ensure_loaded(&self.path, &mut state, &self.updates)?;
        let updated = values.app_settings();
        if updated == state.values {
            return Ok(());
        }
        write_settings_file(&self.path, &updated, state.legacy_account_settings.as_ref())?;
        state.values = updated.clone();
        self.updates.send_replace(updated);
        Ok(())
    }
}

fn ensure_loaded(
    path: &Path,
    state: &mut SettingsStoreState,
    updates: &watch::Sender<AppSettingsValues>,
) -> Result<(), SettingsStoreError> {
    if state.loaded {
        return Ok(());
    }
    let json = match std::fs::read_to_string(path) {
        Ok(json) => json,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            state.loaded = true;
            return Ok(());
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
    if version > SETTINGS_SCHEMA_VERSION {
        return Err(SettingsStoreError::corrupt());
    }

    let legacy = object.remove(LEGACY_ACCOUNT_SETTINGS_KEY);
    let (values, legacy_account_settings) = if version < SETTINGS_SCHEMA_VERSION {
        let mut old_values: SettingsValues =
            serde_json::from_value(document).map_err(|_| SettingsStoreError::corrupt())?;
        migrate(&mut old_values, version);
        (
            old_values.app_settings(),
            Some(old_values.account_settings()),
        )
    } else {
        let values = serde_json::from_value(document).map_err(|_| SettingsStoreError::corrupt())?;
        let legacy = legacy
            .map(serde_json::from_value)
            .transpose()
            .map_err(|_| SettingsStoreError::corrupt())?;
        (values, legacy)
    };

    state.values = values;
    state.legacy_account_settings = legacy_account_settings;
    state.loaded = true;
    updates.send_replace(state.values.clone());
    if version < SETTINGS_SCHEMA_VERSION {
        // A failed rewrite is safe: the source remains intact and this
        // migration is idempotent on the next launch.
        let _ = write_settings_file(path, &state.values, state.legacy_account_settings.as_ref());
    }
    Ok(())
}

fn write_settings_file(
    path: &Path,
    values: &AppSettingsValues,
    legacy_account_settings: Option<&AccountSettingsValues>,
) -> Result<(), SettingsStoreError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|_| SettingsStoreError::io())?;
    }
    let mut document = serde_json::to_value(values).map_err(|_| SettingsStoreError::corrupt())?;
    let object = document
        .as_object_mut()
        .ok_or_else(SettingsStoreError::corrupt)?;
    object.insert(
        SCHEMA_VERSION_KEY.to_owned(),
        serde_json::Value::from(SETTINGS_SCHEMA_VERSION),
    );
    if let Some(legacy) = legacy_account_settings {
        object.insert(
            LEGACY_ACCOUNT_SETTINGS_KEY.to_owned(),
            serde_json::to_value(legacy).map_err(|_| SettingsStoreError::corrupt())?,
        );
    }
    let json =
        serde_json::to_string_pretty(&document).map_err(|_| SettingsStoreError::corrupt())?;
    atomic_replace_file(path, format!("{json}\n").as_bytes(), false)
        .map_err(|_| SettingsStoreError::io())
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
