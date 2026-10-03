use std::time::Duration;

use koushi_protocol::{AccountKey, CoreCommand, SessionKeyId, command::AppCommand};
use koushi_state::{AppearanceSettings, SettingsPatch, ThemePreference};
use koushi_store::{CredentialStoreBackend, FileCredentialStore};

use super::*;

fn session(user_id: &str) -> SessionInfo {
    SessionInfo {
        homeserver: "https://example.invalid".to_owned(),
        user_id: user_id.to_owned(),
        device_id: format!("DEVICE-{}", user_id.trim_start_matches('@')),
        authentication_method: koushi_state::SessionAuthenticationMethod::Unknown,
    }
}

async fn wait_for(
    connection: &mut CoreConnection,
    predicate: impl Fn(&koushi_state::AppState) -> bool,
) {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if predicate(&connection.snapshot()) {
                return;
            }
            assert!(connection.next_versioned_snapshot().await.is_some());
        }
    })
    .await
    .expect("account runtime state should settle");
}

#[tokio::test]
async fn shutdown_waits_for_a_child_owned_by_an_in_flight_lifecycle_operation() {
    let data_dir = tempfile::tempdir().expect("data directory");
    let manager = AccountRuntimeManager::new(
        StoreActor::new(data_dir.path()),
        SettingsStore::new(data_dir.path()),
        Arc::new(|| Arc::new(crate::native_artifact::NativeArtifactRegistry::new())),
    );
    // A removal owns this child until cleanup finishes, despite the empty tab list.
    let operation = manager.operation_gate.lock().await;
    let retiring_runtime = manager.state.lock().unwrap().tabs.remove(0).runtime;
    let mut shutdown = std::pin::pin!(manager.shutdown_all_checked());
    let waits_for_operation =
        std::future::poll_fn(|cx| std::task::Poll::Ready(shutdown.as_mut().poll(cx).is_pending()))
            .await;
    retiring_runtime
        .shutdown_checked()
        .await
        .expect("child cleanup");
    drop(operation);
    if waits_for_operation {
        shutdown.await.expect("manager shutdown");
    }
    assert!(
        waits_for_operation,
        "shutdown must wait for the lifecycle owner"
    );
}

#[tokio::test]
async fn shutdown_is_terminal_for_runtime_creation_and_restoration() {
    let data_dir = tempfile::tempdir().expect("data directory");
    let manager = AccountRuntimeManager::new(
        StoreActor::new(data_dir.path()),
        SettingsStore::new(data_dir.path()),
        Arc::new(|| Arc::new(crate::native_artifact::NativeArtifactRegistry::new())),
    );
    manager.shutdown_all_checked().await.expect("shutdown");
    assert_eq!(
        manager.add_account_tab().await,
        Err(koushi_protocol::CoreFailure::ShutdownFailed)
    );
    assert_eq!(
        manager.restore_saved_accounts().await,
        Err(koushi_protocol::CoreFailure::ShutdownFailed)
    );
    assert!(manager.tab_descriptors().is_empty());
    manager
        .shutdown_all_checked()
        .await
        .expect("repeated shutdown");
}

#[tokio::test]
async fn repeated_shutdown_preserves_incomplete_child_cleanup() {
    let data_dir = tempfile::tempdir().expect("data directory");
    let manager = AccountRuntimeManager::new(
        StoreActor::new(data_dir.path()),
        SettingsStore::new(data_dir.path()),
        Arc::new(|| Arc::new(crate::native_artifact::NativeArtifactRegistry::new())),
    );
    manager.state.lock().unwrap().tabs[0]
        .runtime
        .shutdown_handle()
        .abort();
    assert_eq!(
        manager.shutdown_all_checked().await,
        Err(CoreShutdownError::Incomplete)
    );
    assert_eq!(
        manager.shutdown_all_checked().await,
        Err(CoreShutdownError::Incomplete)
    );
}

#[tokio::test]
async fn restored_account_media_cache_directories_are_tab_scoped() {
    let data_dir = tempfile::tempdir().expect("data directory");
    let credential_dir = tempfile::tempdir().expect("credential directory");
    let store = StoreActor::with_backend(
        CredentialStoreBackend::FileDir(FileCredentialStore::new(credential_dir.path())),
        data_dir.path().to_path_buf(),
    );
    let alice = SessionKeyId {
        homeserver: "https://example.invalid".to_owned(),
        user_id: "@alice:example.invalid".to_owned(),
        device_id: "DEVICE-alice".to_owned(),
    };
    let bob = SessionKeyId {
        homeserver: "https://example.invalid".to_owned(),
        user_id: "@bob:example.invalid".to_owned(),
        device_id: "DEVICE-bob".to_owned(),
    };
    let mut index = store.load_saved_session_index().expect("session index");
    index.upsert(alice.clone());
    index.upsert(bob.clone());
    assert!(index.select_account(&AccountKey(alice.user_id.clone())));
    store
        .save_saved_session_index(&index)
        .expect("save signed-in accounts");

    let native_artifact_factory: Arc<dyn Fn() -> Arc<dyn NativeArtifactPort> + Send + Sync> =
        Arc::new(|| Arc::new(crate::native_artifact::NativeArtifactRegistry::new()));
    let manager = AccountRuntimeManager::new(
        store,
        SettingsStore::new(data_dir.path()),
        native_artifact_factory,
    );
    manager
        .restore_saved_accounts()
        .await
        .expect("restore account tabs");
    let tabs = manager.tab_descriptors();
    let alice_tab = tabs
        .iter()
        .find(|tab| tab.account_key.as_ref() == Some(&AccountKey(alice.user_id.clone())))
        .expect("Alice tab");
    let bob_tab = tabs
        .iter()
        .find(|tab| tab.account_key.as_ref() == Some(&AccountKey(bob.user_id.clone())))
        .expect("Bob tab");
    let alice_cache = manager
        .media_cache_dir_for_tab(&alice_tab.id)
        .expect("Alice media cache");
    let bob_cache = manager
        .media_cache_dir_for_tab(&bob_tab.id)
        .expect("Bob media cache");
    assert_ne!(alice_cache, bob_cache);
    assert!(manager.media_cache_dirs().contains(&alice_cache));
    assert!(manager.media_cache_dirs().contains(&bob_cache));
    manager.shutdown_all().await;
}

#[tokio::test]
async fn binding_selected_account_creates_and_selects_its_saved_tab_before_session_persist() {
    let data_dir = tempfile::tempdir().expect("data directory");
    let credential_dir = tempfile::tempdir().expect("credential directory");
    let store = StoreActor::with_backend(
        CredentialStoreBackend::FileDir(FileCredentialStore::new(credential_dir.path())),
        data_dir.path().to_path_buf(),
    );
    let alice = AccountKey("@alice:example.invalid".to_owned());
    let bob = AccountKey("@bob:example.invalid".to_owned());
    let mut index = store.load_saved_session_index().expect("session index");
    index.ensure_account_tab(alice.clone(), "https://example.invalid");
    assert!(index.select_account(&alice));
    store
        .save_saved_session_index(&index)
        .expect("save initial account tab");

    let native_artifact_factory: Arc<dyn Fn() -> Arc<dyn NativeArtifactPort> + Send + Sync> =
        Arc::new(|| Arc::new(crate::native_artifact::NativeArtifactRegistry::new()));
    let manager = AccountRuntimeManager::new(
        store.clone(),
        SettingsStore::new(data_dir.path()),
        native_artifact_factory,
    );
    manager
        .restore_saved_accounts()
        .await
        .expect("restore initial account tab");
    let bob_tab = manager.add_account_tab().await.expect("add account tab");
    assert_eq!(
        manager
            .bind_authenticated_session(&bob_tab, &session(&bob.0))
            .await
            .expect("bind new account"),
        BindAccountOutcome::Bound(bob_tab.clone())
    );

    let index = store
        .load_saved_session_index()
        .expect("reload account tabs");
    assert_eq!(index.tabs().len(), 2);
    assert_eq!(index.selected_account(), Some(&bob));
    assert!(
        index.sessions().is_empty(),
        "binding precedes credential persist"
    );
    manager.shutdown_all().await;
}

#[tokio::test]
async fn rebinding_a_retained_tab_refreshes_a_changed_device_identity() {
    let data_dir = tempfile::tempdir().expect("data directory");
    let credential_dir = tempfile::tempdir().expect("credential directory");
    let store = StoreActor::with_backend(
        CredentialStoreBackend::FileDir(FileCredentialStore::new(credential_dir.path())),
        data_dir.path().to_path_buf(),
    );
    let native_artifact_factory: Arc<dyn Fn() -> Arc<dyn NativeArtifactPort> + Send + Sync> =
        Arc::new(|| Arc::new(crate::native_artifact::NativeArtifactRegistry::new()));
    let manager = AccountRuntimeManager::new(
        store,
        SettingsStore::new(data_dir.path()),
        native_artifact_factory,
    );
    let tab_id = manager.selected_tab_id();
    let first_session = session("@alice:example.invalid");

    assert_eq!(
        manager
            .bind_authenticated_session(&tab_id, &first_session)
            .await
            .expect("bind the first device"),
        BindAccountOutcome::Bound(tab_id.clone())
    );
    assert!(manager.is_session_binding_current(&tab_id, &first_session));

    let changed_device = SessionInfo {
        device_id: "ALICE-SECOND-DEVICE".to_owned(),
        ..first_session.clone()
    };
    assert!(!manager.is_session_binding_current(&tab_id, &changed_device));
    assert_eq!(
        manager
            .bind_authenticated_session(&tab_id, &changed_device)
            .await
            .expect("refresh the retained account tab"),
        BindAccountOutcome::Bound(tab_id.clone())
    );
    assert!(manager.is_session_binding_current(&tab_id, &changed_device));

    manager.shutdown_all().await;
}

#[tokio::test]
async fn signed_out_account_tab_rejects_a_different_authenticated_identity() {
    let data_dir = tempfile::tempdir().expect("data directory");
    let credential_dir = tempfile::tempdir().expect("credential directory");
    let store = StoreActor::with_backend(
        CredentialStoreBackend::FileDir(FileCredentialStore::new(credential_dir.path())),
        data_dir.path().to_path_buf(),
    );
    let alice = AccountKey("@alice:example.invalid".to_owned());
    let bob = AccountKey("@bob:example.invalid".to_owned());
    let mut index = store.load_saved_session_index().expect("session index");
    index.ensure_account_tab(alice.clone(), "https://example.invalid");
    assert!(index.select_account(&alice));
    store
        .save_saved_session_index(&index)
        .expect("save Alice tab");

    let native_artifact_factory: Arc<dyn Fn() -> Arc<dyn NativeArtifactPort> + Send + Sync> =
        Arc::new(|| Arc::new(crate::native_artifact::NativeArtifactRegistry::new()));
    let manager = AccountRuntimeManager::new(
        store.clone(),
        SettingsStore::new(data_dir.path()),
        native_artifact_factory,
    );
    manager
        .restore_saved_accounts()
        .await
        .expect("restore Alice tab");
    let alice_tab = manager
        .tab_descriptors()
        .into_iter()
        .find(|tab| tab.account_key.as_ref() == Some(&alice))
        .expect("Alice tab");

    assert_eq!(
        manager
            .bind_authenticated_session(&alice_tab.id, &session(&bob.0))
            .await
            .expect("different identity is rejected without replacing the tab"),
        BindAccountOutcome::IdentityMismatch {
            expected: alice.clone(),
            actual: bob.clone(),
        }
    );
    assert_eq!(
        manager
            .tab_descriptors()
            .into_iter()
            .find(|tab| tab.id == alice_tab.id)
            .and_then(|tab| tab.account_key),
        Some(alice.clone())
    );
    let index = store
        .load_saved_session_index()
        .expect("reload account tabs");
    assert_eq!(index.tabs().len(), 1);
    assert_eq!(index.selected_account(), Some(&alice));
    manager.shutdown_all().await;
}

#[tokio::test]
async fn duplicate_password_login_reuses_the_existing_account_tab() {
    let data_dir = tempfile::tempdir().expect("data directory");
    let credential_dir = tempfile::tempdir().expect("credential directory");
    let store = StoreActor::with_backend(
        CredentialStoreBackend::FileDir(FileCredentialStore::new(credential_dir.path())),
        data_dir.path().to_path_buf(),
    );
    let native_artifact_factory: Arc<dyn Fn() -> Arc<dyn NativeArtifactPort> + Send + Sync> =
        Arc::new(|| Arc::new(crate::native_artifact::NativeArtifactRegistry::new()));
    let manager = AccountRuntimeManager::new(
        store,
        SettingsStore::new(data_dir.path()),
        native_artifact_factory,
    );
    let bob_tab = manager.selected_tab_id();
    manager
        .bind_authenticated_session(&bob_tab, &session("@bob:example.invalid"))
        .await
        .expect("bind Bob");
    let add_tab = manager.add_account_tab().await.expect("add account tab");

    assert_eq!(
        manager.account_tab_for_existing_password_login(
            &add_tab,
            "https://example.invalid/",
            "@bob:example.invalid"
        ),
        Some(bob_tab.clone())
    );
    assert_eq!(
        manager.account_tab_for_existing_password_login(&add_tab, "https://example.invalid", "bob"),
        Some(bob_tab.clone())
    );
    assert_eq!(
        manager.account_tab_for_existing_password_login(&bob_tab, "https://example.invalid", "bob"),
        None,
        "the tab that owns the saved device may reauthenticate it"
    );
    assert_eq!(
        manager.account_tab_for_existing_password_login(&add_tab, "https://other.invalid", "bob"),
        None
    );
    manager.shutdown_all().await;
}

#[tokio::test]
async fn account_runtimes_isolate_account_settings_and_share_app_settings() {
    let data_dir = tempfile::tempdir().expect("data directory");
    let credential_dir = tempfile::tempdir().expect("credential directory");
    let store = StoreActor::with_backend(
        CredentialStoreBackend::FileDir(FileCredentialStore::new(credential_dir.path())),
        data_dir.path().to_path_buf(),
    );
    let mut index = store.load_saved_session_index().expect("session index");
    let alice = AccountKey("@alice:example.invalid".to_owned());
    let bob = AccountKey("@bob:example.invalid".to_owned());
    index.ensure_account_tab(alice.clone(), "https://example.invalid");
    index.ensure_account_tab(bob.clone(), "https://example.invalid");
    assert!(index.select_account(&alice));
    store
        .save_saved_session_index(&index)
        .expect("save tab order");

    let settings = SettingsStore::new(data_dir.path());
    let native_artifact_factory: Arc<dyn Fn() -> Arc<dyn NativeArtifactPort> + Send + Sync> =
        Arc::new(|| Arc::new(crate::native_artifact::NativeArtifactRegistry::new()));
    let manager =
        AccountRuntimeManager::new(store.clone(), settings, native_artifact_factory.clone());
    manager
        .restore_saved_accounts()
        .await
        .expect("restore account tabs");

    let tabs = manager.tab_descriptors();
    assert_eq!(tabs.len(), 2);
    let alice_tab = tabs
        .iter()
        .find(|tab| tab.account_key.as_ref() == Some(&alice))
        .expect("Alice runtime")
        .clone();
    let bob_tab = tabs
        .iter()
        .find(|tab| tab.account_key.as_ref() == Some(&bob))
        .expect("Bob runtime")
        .clone();
    assert_ne!(alice_tab.id, bob_tab.id);
    let mut alice_connection = manager
        .tab_connection(&alice_tab.id)
        .expect("Alice connection");
    let mut bob_connection = manager.tab_connection(&bob_tab.id).expect("Bob connection");

    // The index above retains signed-out tabs, not authenticated sessions.
    // Install synthetic session projections before exercising account writes.
    // Move the owned tabs out only to avoid holding the manager mutex on await.
    let runtime_tabs = std::mem::take(&mut manager.state.lock().unwrap().tabs);
    for tab in &runtime_tabs {
        tab.runtime
            .inject_actions(vec![
                koushi_state::AppAction::RestoreSessionRequested,
                koushi_state::AppAction::RestoreSessionSucceeded(session(
                    &tab.descriptor.account_key.as_ref().unwrap().0,
                )),
                koushi_state::AppAction::CurrentDeviceTrustChanged(
                    koushi_state::CurrentDeviceTrustState::Verified,
                ),
            ])
            .await;
    }
    manager.state.lock().unwrap().tabs = runtime_tabs;
    for connection in [&mut alice_connection, &mut bob_connection] {
        wait_for(connection, |snapshot| {
            matches!(snapshot.session, SessionState::Ready(_))
        })
        .await;
    }

    let mut alice_notifications = alice_connection.snapshot().settings.values.notifications;
    alice_notifications.send_typing_notifications = false;
    alice_connection
        .command(CoreCommand::App(AppCommand::UpdateSettings {
            request_id: alice_connection.next_request_id(),
            patch: SettingsPatch {
                scope: Some(koushi_state::SettingsPatchScope::Account),
                notifications: Some(alice_notifications),
                ..SettingsPatch::default()
            },
        }))
        .await
        .expect("update Alice account settings");
    wait_for(&mut alice_connection, |snapshot| {
        !snapshot
            .settings
            .values
            .notifications
            .send_typing_notifications
            && snapshot.settings.persistence == koushi_state::SettingsPersistenceState::Idle
    })
    .await;
    assert!(
        alice_connection.snapshot().errors.is_empty(),
        "account settings must persist successfully"
    );
    assert!(
        !store
            .load_account_settings(&crate::store::session_key_id_from_info(&session(&alice.0)))
            .expect("persisted Alice settings")
            .notifications
            .send_typing_notifications
    );
    assert!(
        !alice_connection
            .snapshot()
            .settings
            .values
            .notifications
            .send_typing_notifications
    );
    assert!(
        bob_connection
            .snapshot()
            .settings
            .values
            .notifications
            .send_typing_notifications
    );

    let appearance = AppearanceSettings {
        theme: ThemePreference::Light,
        ..AppearanceSettings::default()
    };
    let mut crawler_settings = alice_connection.snapshot().settings.values.search_crawler;
    crawler_settings.speed = koushi_state::SearchCrawlerSpeed::Slow;
    alice_connection
        .command(CoreCommand::App(AppCommand::UpdateSettings {
            request_id: alice_connection.next_request_id(),
            patch: SettingsPatch {
                scope: Some(koushi_state::SettingsPatchScope::App),
                appearance: Some(appearance),
                search_crawler: Some(crawler_settings),
                ..SettingsPatch::default()
            },
        }))
        .await
        .expect("update shared app settings");
    wait_for(&mut bob_connection, |snapshot| {
        snapshot.settings.values.appearance.theme == ThemePreference::Light
            && snapshot.settings.values.search_crawler.speed
                == koushi_state::SearchCrawlerSpeed::Slow
    })
    .await;
    assert_eq!(
        alice_connection.snapshot().settings.values.appearance.theme,
        ThemePreference::Light
    );
    assert_eq!(
        bob_connection.snapshot().settings.values.appearance.theme,
        ThemePreference::Light
    );
    assert_eq!(
        alice_connection
            .snapshot()
            .settings
            .values
            .search_crawler
            .speed,
        koushi_state::SearchCrawlerSpeed::Slow
    );
    assert_eq!(
        bob_connection
            .snapshot()
            .settings
            .values
            .search_crawler
            .speed,
        koushi_state::SearchCrawlerSpeed::Slow
    );
    let alice_work = manager.account_work.for_account(alice_tab.id.as_str());
    let bob_work = manager.account_work.for_account(bob_tab.id.as_str());
    drop(
        alice_work
            .acquire(crate::account_work::AccountWorkKind::SearchCrawl)
            .await,
    );
    assert!(
        tokio::time::timeout(
            std::time::Duration::from_millis(250),
            bob_work.acquire(crate::account_work::AccountWorkKind::MediaPrefetch),
        )
        .await
        .is_err(),
        "App Settings speed must rate-limit background work across accounts"
    );

    let selected = manager.selected_tab_id();
    assert_eq!(selected.as_str(), alice_tab.id.as_str());
    assert!(manager.select_tab(&bob_tab.id).await.expect("select Bob"));
    assert_eq!(manager.selected_tab_id(), bob_tab.id);
    assert_eq!(
        store
            .load_saved_session_index()
            .expect("reload tab order")
            .selected_account(),
        Some(&bob)
    );
    drop(alice_connection);
    drop(bob_connection);
    manager.shutdown_all().await;

    let manager = AccountRuntimeManager::new(
        store.clone(),
        SettingsStore::new(data_dir.path()),
        native_artifact_factory,
    );
    manager
        .restore_saved_accounts()
        .await
        .expect("restore selected account tab");
    assert_eq!(manager.selected_tab_id(), bob_tab.id);
    let restored_bob_tab = manager
        .tab_descriptors()
        .into_iter()
        .find(|tab| tab.account_key.as_ref() == Some(&bob))
        .expect("restored Bob tab");

    let add_tab = manager.add_account_tab().await.expect("add account tab");
    assert_eq!(
        manager
            .add_account_tab()
            .await
            .expect("reuse unfinished tab"),
        add_tab
    );
    assert_eq!(
        manager
            .bind_authenticated_session(&add_tab, &session(&bob.0))
            .await
            .expect("duplicate account binding"),
        BindAccountOutcome::Duplicate(bob_tab.id.clone())
    );
    assert!(
        manager
            .remove_signed_out_tab(&restored_bob_tab.id)
            .await
            .expect("remove Bob")
    );
    assert_eq!(manager.selected_tab_id(), add_tab);
    manager.shutdown_all().await;
}
