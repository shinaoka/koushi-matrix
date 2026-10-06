use std::sync::{Arc, Mutex};

use super::StoreActor;
use koushi_protocol::SessionKeyId;
use koushi_store::{CredentialStoreBackend, FileCredentialStore};

pub(super) fn make_key_id() -> SessionKeyId {
    SessionKeyId {
        homeserver: "https://test.example.com".to_owned(),
        user_id: "@alice:test.example.com".to_owned(),
        device_id: "DEVICE1".to_owned(),
    }
}

pub(super) fn file_store_actor(
    data_dir: &tempfile::TempDir,
    cred_dir: &tempfile::TempDir,
) -> StoreActor {
    StoreActor {
        credential_store: CredentialStoreBackend::FileDir(FileCredentialStore::new(
            cred_dir.path(),
        )),
        data_dir: data_dir.path().to_path_buf(),
        exclusive_store_root: false,
        composer_draft_io_probe: Arc::new(Mutex::new(None)),
        composer_draft_replace_fault: Arc::new(std::sync::atomic::AtomicBool::new(false)),
    }
}
