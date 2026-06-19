use std::path::{Path, PathBuf};

use koushi_state::SettingsValues;

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
}

pub struct SettingsStore {
    path: PathBuf,
}

impl SettingsStore {
    pub fn new(data_dir: impl AsRef<Path>) -> Self {
        Self {
            path: data_dir.as_ref().join("settings").join("settings.json"),
        }
    }

    pub fn load(&self) -> Result<SettingsValues, SettingsStoreError> {
        match std::fs::read_to_string(&self.path) {
            Ok(json) => serde_json::from_str(&json).map_err(|_| SettingsStoreError {
                kind: SettingsStoreErrorKind::Corrupt,
            }),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(SettingsValues::default()),
            Err(_) => Err(SettingsStoreError {
                kind: SettingsStoreErrorKind::Io,
            }),
        }
    }

    pub fn save(&self, values: &SettingsValues) -> Result<(), SettingsStoreError> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent).map_err(|_| SettingsStoreError {
                kind: SettingsStoreErrorKind::Io,
            })?;
        }
        let json = serde_json::to_string_pretty(values).map_err(|_| SettingsStoreError {
            kind: SettingsStoreErrorKind::Corrupt,
        })?;
        std::fs::write(&self.path, format!("{json}\n")).map_err(|_| SettingsStoreError {
            kind: SettingsStoreErrorKind::Io,
        })
    }
}
