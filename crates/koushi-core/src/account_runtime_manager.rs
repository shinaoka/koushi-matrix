use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
};

use crate::{
    NativeArtifactPort, NativeStillImageDecoder,
    account_work::AccountWorkScheduler,
    executor,
    runtime::{CoreConnection, CoreRuntime, CoreShutdownError},
    settings::SettingsStore,
    store::StoreActor,
};
use koushi_diagnostics::{DiagnosticEvent, DiagnosticLevel, record};
use koushi_protocol::{AccountKey, CoreCommand, SessionKeyId, command::AccountCommand};
use koushi_state::{SessionInfo, SessionState};

mod sign_in_attempts;

pub use sign_in_attempts::{OidcCallbackCorrelation, PendingSignInAttempts};

#[derive(
    Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, serde::Serialize, serde::Deserialize,
)]
#[serde(transparent)]
pub struct AccountTabId(String);

impl AccountTabId {
    fn add(sequence: u64) -> Self {
        Self(format!("add:{sequence}"))
    }

    fn account(account_key: &AccountKey) -> Self {
        Self(format!("account:{}", account_key.0))
    }

    pub fn from_string(value: String) -> Self {
        Self(value)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountTabDescriptor {
    pub id: AccountTabId,
    pub account_key: Option<AccountKey>,
    pub homeserver: Option<String>,
}

struct ManagedTab {
    descriptor: AccountTabDescriptor,
    session_key_id: Option<SessionKeyId>,
    runtime: CoreRuntime,
}

struct ManagerState {
    tabs: Vec<ManagedTab>,
    selected: AccountTabId,
    next_add_id: u64,
    /// The tab that was selected when the unbound add-account tab was opened,
    /// so cancelling it returns the user to where they came from.
    add_return_to: Option<AccountTabId>,
    /// Cleanup joins of cancelled/removed children. Each child joins only after
    /// every adapter connection to it drops, so the lifecycle gate is never held
    /// across these joins; manager shutdown drains them instead.
    retiring: Vec<executor::JoinHandle<Result<(), CoreShutdownError>>>,
    shutdown_result: Option<Result<(), CoreShutdownError>>,
}

/// Owns one persistent `CoreRuntime` per account tab. All account runtimes
/// share the credential and app-settings stores, but keep their actor trees,
/// snapshots, and account-local stores independent.
pub struct AccountRuntimeManager {
    state: Mutex<ManagerState>,
    operation_gate: tokio::sync::Mutex<()>,
    sign_in_attempts: Mutex<PendingSignInAttempts>,
    store: StoreActor,
    settings: SettingsStore,
    account_work: AccountWorkScheduler,
    native_artifact_factory: Arc<dyn Fn() -> Arc<dyn NativeArtifactPort> + Send + Sync>,
    native_image_decoder: Option<Arc<dyn NativeStillImageDecoder>>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum BindAccountOutcome {
    Bound(AccountTabId),
    Duplicate(AccountTabId),
    IdentityMismatch {
        expected: AccountKey,
        actual: AccountKey,
    },
}

impl AccountRuntimeManager {
    pub fn new(
        store: StoreActor,
        settings: SettingsStore,
        native_artifact_factory: Arc<dyn Fn() -> Arc<dyn NativeArtifactPort> + Send + Sync>,
    ) -> Self {
        Self::new_with_native_image_decoder(store, settings, native_artifact_factory, None)
    }

    /// Like [`Self::new`], with a platform still-image decoder shared by every
    /// account runtime's media preparation.
    pub fn new_with_native_image_decoder(
        store: StoreActor,
        settings: SettingsStore,
        native_artifact_factory: Arc<dyn Fn() -> Arc<dyn NativeArtifactPort> + Send + Sync>,
        native_image_decoder: Option<Arc<dyn NativeStillImageDecoder>>,
    ) -> Self {
        let first_add = AccountTabId::add(1);
        let account_work = AccountWorkScheduler::default();
        account_work.set_selected_account(Some(first_add.as_str()));
        let runtime = start_runtime(
            &store,
            &settings,
            &native_artifact_factory,
            &native_image_decoder,
            &account_work,
            &first_add,
        );
        Self {
            state: Mutex::new(ManagerState {
                tabs: vec![ManagedTab {
                    descriptor: AccountTabDescriptor {
                        id: first_add.clone(),
                        account_key: None,
                        homeserver: None,
                    },
                    session_key_id: None,
                    runtime,
                }],
                selected: first_add,
                next_add_id: 2,
                add_return_to: None,
                retiring: Vec::new(),
                shutdown_result: None,
            }),
            operation_gate: tokio::sync::Mutex::new(()),
            sign_in_attempts: Mutex::new(PendingSignInAttempts::default()),
            store,
            settings,
            account_work,
            native_artifact_factory,
            native_image_decoder,
        }
    }

    /// Load tab order/selection and restore every saved session concurrently.
    /// Legacy account settings are seeded before any account runtime starts, so
    /// a partially completed migration can be retried without overwriting edits.
    pub async fn restore_saved_accounts(
        &self,
    ) -> Result<(), koushi_protocol::failure::CoreFailure> {
        let _operation = self.operation_gate.lock().await;
        if self
            .state
            .lock()
            .expect("account runtime manager mutex")
            .shutdown_result
            .is_some()
        {
            return Err(koushi_protocol::failure::CoreFailure::ShutdownFailed);
        }
        let store = self.store.clone();
        let settings = self.settings.clone();
        let index = executor::spawn_blocking(move || {
            let mut index = store.load_saved_session_index()?;
            let sessions = index.sessions().to_vec();
            if index.selected_account().is_none() {
                let last_session = store
                    .credential_store_backend()
                    .load_last_session()
                    .ok()
                    .flatten()
                    .map(|session| AccountKey(session.user_id));
                let first = index.tabs().first().map(|tab| tab.account_key.clone());
                if !last_session
                    .as_ref()
                    .is_some_and(|account_key| index.select_account(account_key))
                    && let Some(first) = first
                {
                    let _ = index.select_account(&first);
                }
            }
            if let Some(legacy) = settings
                .legacy_account_settings()
                .map_err(|_| koushi_protocol::failure::CoreFailure::StoreUnavailable)?
            {
                for session in &sessions {
                    store
                        .seed_account_settings_from_legacy(session, &legacy)
                        .map_err(|_| koushi_protocol::failure::CoreFailure::StoreUnavailable)?;
                }
                settings
                    .complete_legacy_account_migration()
                    .map_err(|_| koushi_protocol::failure::CoreFailure::StoreUnavailable)?;
            }
            store.save_saved_session_index(&index)?;
            Ok::<_, koushi_protocol::failure::CoreFailure>(index)
        })
        .await
        .map_err(|_| koushi_protocol::failure::CoreFailure::StoreUnavailable)??;
        let session_accounts: std::collections::HashSet<String> = index
            .sessions()
            .iter()
            .map(|session| session.user_id.clone())
            .collect();
        let selected_key = index.selected_account().cloned();
        if let Some(key) = &selected_key {
            self.account_work
                .set_selected_account(Some(AccountTabId::account(key).as_str()));
        }
        let existing = self
            .state
            .lock()
            .expect("account runtime manager mutex")
            .tabs
            .iter()
            .find(|tab| tab.descriptor.account_key.is_none())
            .map(|tab| tab.descriptor.id.clone());
        let add_runtime_in_use = existing.as_ref().is_some_and(|id| {
            self.tab_connection(id).is_some_and(|connection| {
                !matches!(connection.snapshot().session, SessionState::SignedOut)
            })
        });

        let mut restored = Vec::new();
        for tab in index.tabs() {
            let key = tab.account_key.clone();
            let descriptor = AccountTabDescriptor {
                id: AccountTabId::account(&key),
                account_key: Some(key.clone()),
                homeserver: Some(tab.homeserver.clone()),
            };
            let runtime = start_runtime(
                &self.store,
                &self.settings,
                &self.native_artifact_factory,
                &self.native_image_decoder,
                &self.account_work,
                &descriptor.id,
            );
            let should_restore = session_accounts.contains(&key.0);
            let session_key_id = index
                .sessions()
                .iter()
                .find(|session| session.user_id == key.0)
                .cloned();
            restored.push((descriptor, session_key_id, runtime, should_restore, key));
        }

        let mut old_runtimes = Vec::new();
        let selected_tab_id = {
            let mut state = self.state.lock().expect("account runtime manager mutex");
            let mut keep_add = None;
            if add_runtime_in_use
                && let Some(add_id) = existing
                && let Some(position) = state
                    .tabs
                    .iter()
                    .position(|tab| tab.descriptor.id == add_id)
            {
                keep_add = Some(state.tabs.remove(position));
            }
            old_runtimes.extend(state.tabs.drain(..).map(|tab| tab.runtime));
            let mut new_tabs = Vec::new();
            if let Some(add_tab) = keep_add {
                new_tabs.push(add_tab);
            }
            for (descriptor, session_key_id, runtime, should_restore, key) in restored {
                new_tabs.push(ManagedTab {
                    descriptor,
                    session_key_id,
                    runtime,
                });
                if should_restore {
                    let connection = new_tabs.last().expect("just inserted tab").runtime.attach();
                    let request_id = connection.next_request_id();
                    let command = CoreCommand::Account(AccountCommand::RestoreSession {
                        request_id,
                        account_key: key,
                    });
                    executor::spawn(async move {
                        let _ = connection.command(command).await;
                    });
                }
            }
            if new_tabs.is_empty() {
                let id = AccountTabId::add(state.next_add_id);
                state.next_add_id = state.next_add_id.saturating_add(1);
                let runtime = start_runtime(
                    &self.store,
                    &self.settings,
                    &self.native_artifact_factory,
                    &self.native_image_decoder,
                    &self.account_work,
                    &id,
                );
                new_tabs.push(ManagedTab {
                    descriptor: AccountTabDescriptor {
                        id: id.clone(),
                        account_key: None,
                        homeserver: None,
                    },
                    session_key_id: None,
                    runtime,
                });
                state.selected = id;
            } else {
                state.selected = selected_key
                    .as_ref()
                    .and_then(|key| {
                        new_tabs
                            .iter()
                            .find(|tab| tab.descriptor.account_key.as_ref() == Some(key))
                    })
                    .map(|tab| tab.descriptor.id.clone())
                    .unwrap_or_else(|| new_tabs[0].descriptor.id.clone());
            }
            state.tabs = new_tabs;
            state.selected.clone()
        };
        self.account_work
            .set_selected_account(Some(selected_tab_id.as_str()));
        let mut shutdown_failed = false;
        for runtime in old_runtimes {
            shutdown_failed |= runtime.shutdown_checked().await.is_err();
        }
        if shutdown_failed {
            return Err(koushi_protocol::failure::CoreFailure::StoreUnavailable);
        }
        Ok(())
    }

    pub fn tab_descriptors(&self) -> Vec<AccountTabDescriptor> {
        self.state
            .lock()
            .expect("account runtime manager mutex")
            .tabs
            .iter()
            .map(|tab| tab.descriptor.clone())
            .collect()
    }

    pub fn subscribe_app_settings_updates(
        &self,
    ) -> tokio::sync::watch::Receiver<koushi_state::AppSettingsValues> {
        self.settings.subscribe()
    }

    pub fn selected_tab_id(&self) -> AccountTabId {
        self.state
            .lock()
            .expect("account runtime manager mutex")
            .selected
            .clone()
    }

    pub fn attach(&self) -> CoreConnection {
        self.selected_connection()
    }

    pub fn selected_connection(&self) -> CoreConnection {
        self.selected_binding().1
    }

    pub fn selected_binding(&self) -> (AccountTabId, CoreConnection) {
        let state = self.state.lock().expect("account runtime manager mutex");
        let tab = state
            .tabs
            .iter()
            .find(|tab| tab.descriptor.id == state.selected)
            .expect("selected account tab exists");
        (tab.descriptor.id.clone(), tab.runtime.attach())
    }

    pub fn media_preparation(&self) -> Arc<crate::media_preparation::MediaPreparationService> {
        let state = self.state.lock().expect("account runtime manager mutex");
        state
            .tabs
            .iter()
            .find(|tab| tab.descriptor.id == state.selected)
            .expect("selected account tab exists")
            .runtime
            .media_preparation()
    }

    pub fn sliding_sync_diagnostics(&self) -> crate::SlidingSyncDiagnosticsSnapshot {
        let state = self.state.lock().expect("account runtime manager mutex");
        state
            .tabs
            .iter()
            .find(|tab| tab.descriptor.id == state.selected)
            .expect("selected account tab exists")
            .runtime
            .sliding_sync_diagnostics()
    }

    pub fn tab_connection(&self, id: &AccountTabId) -> Option<CoreConnection> {
        self.state
            .lock()
            .expect("account runtime manager mutex")
            .tabs
            .iter()
            .find(|tab| &tab.descriptor.id == id)
            .map(|tab| tab.runtime.attach())
    }

    pub fn is_session_binding_current(&self, id: &AccountTabId, info: &SessionInfo) -> bool {
        let account_key = AccountKey(info.user_id.clone());
        let session_key_id = crate::store::session_key_id_from_info(info);
        self.state
            .lock()
            .expect("account runtime manager mutex")
            .tabs
            .iter()
            .find(|tab| &tab.descriptor.id == id)
            .is_some_and(|tab| {
                tab.descriptor.account_key.as_ref() == Some(&account_key)
                    && tab.descriptor.homeserver.as_deref() == Some(&info.homeserver)
                    && tab.session_key_id.as_ref() == Some(&session_key_id)
            })
    }

    pub fn tab_connections(&self) -> Vec<(AccountTabDescriptor, CoreConnection)> {
        self.state
            .lock()
            .expect("account runtime manager mutex")
            .tabs
            .iter()
            .map(|tab| (tab.descriptor.clone(), tab.runtime.attach()))
            .collect()
    }

    /// Register the pending sign-in attempt a browser callback for this tab must
    /// repeat. `state` is the SDK-minted OAuth CSRF state, or empty on the
    /// legacy `m.login.sso` fallback that carries no state (#1266).
    pub fn register_oidc_attempt(&self, id: &AccountTabId, state: String) {
        self.sign_in_attempts
            .lock()
            .expect("sign-in attempt mutex")
            .register(id, state);
    }

    /// The tab a callback with this correlation belongs to, without consuming
    /// the attempt. Returns `None` for a mismatch, an unsolicited callback, or
    /// an ambiguous legacy callback with more than one pending attempt.
    pub fn oidc_attempt_tab(&self, correlation: &OidcCallbackCorrelation) -> Option<AccountTabId> {
        self.sign_in_attempts
            .lock()
            .expect("sign-in attempt mutex")
            .tab(correlation)
    }

    /// Consume the attempt a callback with this correlation belongs to, so a
    /// replayed callback is rejected.
    pub fn take_oidc_attempt(&self, correlation: OidcCallbackCorrelation) -> Option<AccountTabId> {
        self.sign_in_attempts
            .lock()
            .expect("sign-in attempt mutex")
            .take(&correlation)
    }

    /// Forget the adapter's one-shot mapping for a tab (#1267). After an
    /// explicit cancel, a late callback for the retired attempt no longer
    /// matches a pending login on any tab, so it is rejected instead of
    /// completing a replaced attempt.
    pub fn forget_oidc_attempts_for_tab(&self, id: &AccountTabId) {
        self.oidc_attempts
            .lock()
            .expect("OIDC attempt mutex")
            .retain(|_, tab_id| tab_id != id);
    }

    pub fn media_cache_dir_for_tab(&self, id: &AccountTabId) -> Option<PathBuf> {
        let state = self.state.lock().expect("account runtime manager mutex");
        let key_id = state
            .tabs
            .iter()
            .find(|tab| &tab.descriptor.id == id)?
            .session_key_id
            .as_ref()?;
        Some(
            self.store
                .account_local_data_dir(key_id)
                .join("media-downloads"),
        )
    }

    pub fn media_cache_dirs(&self) -> Vec<PathBuf> {
        self.state
            .lock()
            .expect("account runtime manager mutex")
            .tabs
            .iter()
            .filter_map(|tab| {
                tab.session_key_id.as_ref().map(|key_id| {
                    self.store
                        .account_local_data_dir(key_id)
                        .join("media-downloads")
                })
            })
            .collect()
    }

    pub async fn select_tab(
        &self,
        id: &AccountTabId,
    ) -> Result<bool, koushi_protocol::failure::CoreFailure> {
        let _operation = self.operation_gate.lock().await;
        let account_key = {
            let state = self.state.lock().expect("account runtime manager mutex");
            let Some(tab) = state.tabs.iter().find(|tab| &tab.descriptor.id == id) else {
                return Ok(false);
            };
            tab.descriptor.account_key.clone()
        };
        if let Some(account_key) = &account_key {
            let store = self.store.clone();
            let account_key = account_key.clone();
            executor::spawn_blocking(move || store.select_saved_account(&account_key))
                .await
                .map_err(|_| koushi_protocol::failure::CoreFailure::StoreUnavailable)??;
        }
        let mut state = self.state.lock().expect("account runtime manager mutex");
        if state.tabs.iter().any(|tab| &tab.descriptor.id == id) {
            state.selected = id.clone();
            self.account_work.set_selected_account(Some(id.as_str()));
            Ok(true)
        } else {
            Ok(false)
        }
    }

    pub async fn add_account_tab(
        &self,
    ) -> Result<AccountTabId, koushi_protocol::failure::CoreFailure> {
        let _operation = self.operation_gate.lock().await;
        self.add_account_tab_locked()
    }

    fn add_account_tab_locked(
        &self,
    ) -> Result<AccountTabId, koushi_protocol::failure::CoreFailure> {
        let mut state = self.state.lock().expect("account runtime manager mutex");
        if state.shutdown_result.is_some() {
            return Err(koushi_protocol::failure::CoreFailure::ShutdownFailed);
        }
        if let Some(tab) = state
            .tabs
            .iter()
            .find(|tab| tab.descriptor.account_key.is_none())
        {
            let id = tab.descriptor.id.clone();
            if state.selected != id {
                state.add_return_to = Some(state.selected.clone());
            }
            state.selected = id.clone();
            self.account_work.set_selected_account(Some(id.as_str()));
            return Ok(id);
        }
        let id = AccountTabId::add(state.next_add_id);
        state.next_add_id = state.next_add_id.saturating_add(1);
        let runtime = start_runtime(
            &self.store,
            &self.settings,
            &self.native_artifact_factory,
            &self.native_image_decoder,
            &self.account_work,
            &id,
        );
        state.tabs.push(ManagedTab {
            descriptor: AccountTabDescriptor {
                id: id.clone(),
                account_key: None,
                homeserver: None,
            },
            session_key_id: None,
            runtime,
        });
        state.add_return_to = Some(state.selected.clone());
        state.selected = id.clone();
        self.account_work.set_selected_account(Some(id.as_str()));
        Ok(id)
    }

    /// Close an add-account tab that never bound an account. Only a signed-out
    /// tab qualifies (an in-flight password login must settle first), and the
    /// last tab is kept so the window always has a sign-in surface. Existing
    /// accounts and the credential store are never touched.
    pub async fn cancel_add_account_tab(
        &self,
        id: &AccountTabId,
    ) -> Result<bool, koushi_protocol::failure::CoreFailure> {
        let _operation = self.operation_gate.lock().await;
        let selected_tab_id = {
            let mut state = self.state.lock().expect("account runtime manager mutex");
            let Some(position) = state.tabs.iter().position(|tab| &tab.descriptor.id == id) else {
                return Ok(false);
            };
            let tab = &state.tabs[position];
            let signed_out = matches!(
                tab.runtime.attach().snapshot().session,
                SessionState::SignedOut
            );
            if tab.descriptor.account_key.is_some() || !signed_out || state.tabs.len() < 2 {
                return Ok(false);
            }
            let runtime = state.tabs.remove(position).runtime;
            let return_to = state.add_return_to.take();
            if state.selected == *id {
                state.selected = return_to
                    .filter(|target| state.tabs.iter().any(|tab| &tab.descriptor.id == target))
                    .unwrap_or_else(|| {
                        state.tabs[position.min(state.tabs.len() - 1)]
                            .descriptor
                            .id
                            .clone()
                    });
            }
            state
                .retiring
                .push(executor::spawn(runtime.shutdown_checked()));
            state.selected.clone()
        };
        self.account_work
            .set_selected_account(Some(selected_tab_id.as_str()));
        self.sign_in_attempts
            .lock()
            .expect("sign-in attempt mutex")
            .forget(id);
        Ok(true)
    }

    /// Find another retained tab that owns the saved device this login would reuse.
    pub fn account_tab_for_existing_password_login(
        &self,
        current_tab: &AccountTabId,
        homeserver: &str,
        username: &str,
    ) -> Option<AccountTabId> {
        let username = username.trim();
        // A full Matrix ID names the account regardless of which homeserver
        // URL its domain delegates to.
        let exact = username.starts_with('@') && username.contains(':');
        let homeserver = if exact {
            None
        } else {
            Some(koushi_sdk::Homeserver::parse(homeserver).ok()?.normalized())
        };
        self.state
            .lock()
            .expect("account runtime manager mutex")
            .tabs
            .iter()
            .find(|tab| {
                tab.descriptor.id != *current_tab
                    && tab.descriptor.account_key.as_ref().is_some_and(|key| {
                        if exact {
                            key.0 == username
                        } else {
                            tab.descriptor.homeserver.as_deref() == homeserver.as_deref()
                                && key
                                    .0
                                    .strip_prefix('@')
                                    .and_then(|user| user.split_once(':'))
                                    .is_some_and(|(localpart, _)| localpart == username)
                        }
                    })
            })
            .map(|tab| tab.descriptor.id.clone())
    }

    pub async fn bind_authenticated_session(
        &self,
        id: &AccountTabId,
        info: &SessionInfo,
    ) -> Result<BindAccountOutcome, koushi_protocol::failure::CoreFailure> {
        let _operation = self.operation_gate.lock().await;
        let account_key = AccountKey(info.user_id.clone());
        let selected = {
            let state = self.state.lock().expect("account runtime manager mutex");
            if let Some(existing) = state.tabs.iter().find(|tab| {
                tab.descriptor.account_key.as_ref() == Some(&account_key)
                    && tab.descriptor.id != *id
            }) {
                return Ok(BindAccountOutcome::Duplicate(
                    existing.descriptor.id.clone(),
                ));
            }
            let Some(tab) = state.tabs.iter().find(|tab| &tab.descriptor.id == id) else {
                return Err(koushi_protocol::failure::CoreFailure::StoreUnavailable);
            };
            if let Some(expected) = &tab.descriptor.account_key
                && expected != &account_key
            {
                return Ok(BindAccountOutcome::IdentityMismatch {
                    expected: expected.clone(),
                    actual: account_key.clone(),
                });
            }
            state.selected == *id
        };
        let store = self.store.clone();
        let account_key_for_store = account_key.clone();
        let homeserver = info.homeserver.clone();
        executor::spawn_blocking(move || {
            store.ensure_saved_account_tab(&account_key_for_store, &homeserver, selected)
        })
        .await
        .map_err(|_| koushi_protocol::failure::CoreFailure::StoreUnavailable)??;
        let mut state = self.state.lock().expect("account runtime manager mutex");
        let Some(tab) = state.tabs.iter_mut().find(|tab| &tab.descriptor.id == id) else {
            return Err(koushi_protocol::failure::CoreFailure::StoreUnavailable);
        };
        tab.descriptor.account_key = Some(account_key.clone());
        tab.descriptor.homeserver = Some(info.homeserver.clone());
        tab.session_key_id = Some(crate::store::session_key_id_from_info(info));
        Ok(BindAccountOutcome::Bound(id.clone()))
    }

    pub async fn remove_signed_out_tab(
        &self,
        id: &AccountTabId,
    ) -> Result<bool, koushi_protocol::failure::CoreFailure> {
        let _operation = self.operation_gate.lock().await;
        let (account_key, runtime_is_signed_out) = {
            let state = self.state.lock().expect("account runtime manager mutex");
            let Some(tab) = state.tabs.iter().find(|tab| &tab.descriptor.id == id) else {
                return Ok(false);
            };
            let signed_out = matches!(
                tab.runtime.attach().snapshot().session,
                SessionState::SignedOut
            );
            (tab.descriptor.account_key.clone(), signed_out)
        };
        let Some(account_key) = account_key.filter(|_| runtime_is_signed_out) else {
            return Ok(false);
        };
        let store = self.store.clone();
        let account_key_for_store = account_key.clone();
        let removed = executor::spawn_blocking(move || {
            store.remove_saved_account_tab(&account_key_for_store)
        })
        .await
        .map_err(|_| koushi_protocol::failure::CoreFailure::StoreUnavailable)??;
        if !removed {
            return Ok(false);
        }
        let selected_tab_id = {
            let mut state = self.state.lock().expect("account runtime manager mutex");
            let Some(position) = state.tabs.iter().position(|tab| &tab.descriptor.id == id) else {
                return Ok(false);
            };
            let runtime = state.tabs.remove(position).runtime;
            if state.selected == *id
                && let Some(tab) = state
                    .tabs
                    .get(position.min(state.tabs.len().saturating_sub(1)))
            {
                state.selected = tab.descriptor.id.clone();
            }
            state
                .retiring
                .push(executor::spawn(runtime.shutdown_checked()));
            state.selected.clone()
        };
        self.account_work
            .set_selected_account(Some(selected_tab_id.as_str()));
        self.sign_in_attempts
            .lock()
            .expect("sign-in attempt mutex")
            .forget(id);
        if self.tab_descriptors().is_empty() {
            self.add_account_tab_locked()?;
        }
        Ok(true)
    }

    pub async fn shutdown_all(&self) {
        if self.shutdown_all_checked().await.is_err() {
            record(DiagnosticEvent::new(
                DiagnosticLevel::Warn,
                "core.account_runtime_manager",
                "shutdown_incomplete",
            ));
        }
    }

    pub async fn shutdown_all_checked(&self) -> Result<(), CoreShutdownError> {
        let _operation = self.operation_gate.lock().await;
        self.account_work.set_selected_account(None);
        let (runtimes, retiring) = {
            let mut state = self.state.lock().expect("account runtime manager mutex");
            if let Some(result) = state.shutdown_result {
                return result;
            }
            // Fail closed if the owning shutdown future is cancelled before completion.
            state.shutdown_result = Some(Err(CoreShutdownError::Incomplete));
            let runtimes = std::mem::take(&mut state.tabs)
                .into_iter()
                .map(|tab| tab.runtime)
                .collect::<Vec<_>>();
            (runtimes, std::mem::take(&mut state.retiring))
        };
        let mut result = Ok(());
        for runtime in runtimes {
            if let Err(error) = runtime.shutdown_checked().await {
                result = Err(error);
            }
        }
        for retiring in retiring {
            match retiring.await {
                Ok(Ok(())) => {}
                Ok(Err(error)) => result = Err(error),
                Err(_) => result = Err(CoreShutdownError::Incomplete),
            }
        }
        self.state
            .lock()
            .expect("account runtime manager mutex")
            .shutdown_result = Some(result);
        result
    }
}

fn start_runtime(
    store: &StoreActor,
    settings: &SettingsStore,
    native_artifact_factory: &Arc<dyn Fn() -> Arc<dyn NativeArtifactPort> + Send + Sync>,
    native_image_decoder: &Option<Arc<dyn NativeStillImageDecoder>>,
    account_work: &AccountWorkScheduler,
    account_tab_id: &AccountTabId,
) -> CoreRuntime {
    CoreRuntime::start_with_shared_stores(
        store.clone(),
        settings.clone(),
        native_artifact_factory(),
        account_work.for_account(account_tab_id.as_str()),
        native_image_decoder.clone(),
    )
}

#[cfg(test)]
mod tests;
