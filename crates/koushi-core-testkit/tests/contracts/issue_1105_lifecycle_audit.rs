//! Audit probes: retained adapter connections must not wedge tab lifecycle.
use koushi_core::{
    account_runtime_manager::AccountRuntimeManager, native_artifact::NativeArtifactRegistry,
    settings::SettingsStore, store::StoreActor,
};
use koushi_key::InMemoryCredentialBackend;
use koushi_state::SessionAuthenticationMethod;
use koushi_state::SessionInfo;
use std::{sync::Arc, time::Duration};

fn manager() -> (tempfile::TempDir, tempfile::TempDir, AccountRuntimeManager) {
    let data = tempfile::tempdir().unwrap();
    let credentials = tempfile::tempdir().unwrap();
    let store =
        StoreActor::with_os_backend(data.path(), Arc::new(InMemoryCredentialBackend::default()));
    let manager = AccountRuntimeManager::new(
        store,
        SettingsStore::new(data.path()),
        Arc::new(|| Arc::new(NativeArtifactRegistry::new())),
    );
    (data, credentials, manager)
}

fn info() -> SessionInfo {
    SessionInfo {
        homeserver: "https://example.invalid".into(),
        user_id: "@member:example.invalid".into(),
        device_id: "SYNTHETIC".into(),
        authentication_method: SessionAuthenticationMethod::Password,
    }
}

#[tokio::test]
async fn cancel_finishes_with_adapter_connection_retained() {
    let (_data, _credentials, manager) = manager();
    let original = manager.selected_tab_id();
    manager
        .bind_authenticated_session(&original, &info())
        .await
        .unwrap();
    let add = manager.add_account_tab().await.unwrap();
    let connection = manager.tab_connection(&add).unwrap();
    let mut cancel = Box::pin(manager.cancel_add_account_tab(&add));
    let completed = tokio::time::timeout(Duration::from_millis(200), &mut cancel)
        .await
        .is_ok();
    assert_eq!(manager.selected_tab_id(), original);
    drop(connection);
    if !completed {
        cancel.await.unwrap();
    }
    manager.shutdown_all_checked().await.unwrap();
    assert!(
        completed,
        "Cancel waited for adapter connection drop; adapter drops only after Cancel returns"
    );
}

#[tokio::test]
async fn stalled_cancel_does_not_block_selecting_original_account() {
    let (_data, _credentials, manager) = manager();
    let original = manager.selected_tab_id();
    manager
        .bind_authenticated_session(&original, &info())
        .await
        .unwrap();
    let add = manager.add_account_tab().await.unwrap();
    let connection = manager.tab_connection(&add).unwrap();
    let mut cancel = Box::pin(manager.cancel_add_account_tab(&add));
    let completed = tokio::time::timeout(Duration::from_millis(200), &mut cancel)
        .await
        .is_ok();
    let selected = tokio::time::timeout(Duration::from_millis(200), manager.select_tab(&original))
        .await
        .is_ok();
    drop(connection);
    if !completed {
        cancel.await.unwrap();
    }
    manager.shutdown_all_checked().await.unwrap();
    assert!(
        selected,
        "Cancel retained operation_gate while waiting for external connection; select_tab blocked"
    );
}

#[tokio::test]
async fn signed_out_removal_does_not_block_unrelated_selection_with_retained_connection() {
    let (_data, _credentials, manager) = manager();
    let original = manager.selected_tab_id();
    manager
        .bind_authenticated_session(&original, &info())
        .await
        .unwrap();
    let other = manager.add_account_tab().await.unwrap();
    let connection = manager.tab_connection(&original).unwrap();
    let mut removal = Box::pin(manager.remove_signed_out_tab(&original));
    let completed = tokio::time::timeout(Duration::from_millis(200), &mut removal)
        .await
        .is_ok();
    let selected = tokio::time::timeout(Duration::from_millis(200), manager.select_tab(&other))
        .await
        .is_ok();
    drop(connection);
    if !completed {
        removal.await.unwrap();
    }
    manager.shutdown_all_checked().await.unwrap();
    assert!(
        selected,
        "signed-out removal also retains operation_gate while shutdown waits for connection drop"
    );
}
