//! Which accounts the bridge exposes (#1536 follow-up): every tab bound to an
//! account, under a stable `account:<user id>` id, including an account that
//! signed in during this run and still sits in its original sign-in tab.

use std::sync::Arc;

use koushi_core::account_runtime_manager::AccountRuntimeManager;
use koushi_core::native_artifact::NativeArtifactRegistry;
use koushi_core::settings::SettingsStore;
use koushi_core::store::StoreActor;
use koushi_key::InMemoryCredentialBackend;
use koushi_mcp_host::BridgeAccounts;
use koushi_state::{SessionAuthenticationMethod, SessionInfo, SessionState};

fn manager(data: &std::path::Path) -> AccountRuntimeManager {
    AccountRuntimeManager::new(
        StoreActor::with_os_backend(data, Arc::new(InMemoryCredentialBackend::default())),
        SettingsStore::new(data),
        Arc::new(|| Arc::new(NativeArtifactRegistry::new())),
    )
}

fn info() -> SessionInfo {
    SessionInfo {
        homeserver: "https://example.invalid".into(),
        user_id: "@member1:example.invalid".into(),
        device_id: "SYNTHETIC".into(),
        authentication_method: SessionAuthenticationMethod::Password,
    }
}

#[tokio::test]
async fn an_account_signed_in_this_run_is_exposed_under_its_stable_id() {
    let data = tempfile::tempdir().expect("tempdir");
    let manager = manager(data.path());
    assert!(
        manager.accounts().is_empty(),
        "a sign-in tab with no account is not exposed"
    );

    let sign_in_tab = manager.selected_tab_id();
    manager
        .bind_authenticated_session(&sign_in_tab, &info())
        .await
        .expect("bind");
    assert!(
        !sign_in_tab.is_account(),
        "precondition: the tab keeps its sign-in id until the next launch"
    );

    let ids: Vec<String> = manager
        .accounts()
        .into_iter()
        .map(|account| account.id)
        .collect();
    assert_eq!(ids, vec!["account:@member1:example.invalid".to_owned()]);
    // The tab keeps its account key, but its runtime never signed in: the
    // bridge lists the account for `list_accounts` and refuses every tool that
    // needs it (#1580).
    assert!(
        !matches!(
            manager.accounts()[0].connection.snapshot().session,
            SessionState::Ready(_)
        ),
        "a signed-out runtime is not ready"
    );

    manager.shutdown_all_checked().await.expect("shutdown");
}
