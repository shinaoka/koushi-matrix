//! `account_management` ownership for AccountActor.

use koushi_state::{AccountManagementOperation, AppAction, AuthFailureKind};

use crate::failure::CoreFailure;
use crate::ids::RequestId;

use super::actor::AccountActor;

pub(super) struct PendingUiaOperation {
    operation: AccountManagementOperation,
    new_password: Option<koushi_state::AuthSecret>,
    erase_data: bool,
    uiaa_session: Option<String>,
}

impl AccountActor {
    pub(super) async fn start_active_session_account_management_discovery(
        &mut self,
        session: std::sync::Arc<koushi_sdk::MatrixClientSession>,
    ) {
        self.stop_active_session_account_management_discovery()
            .await;
        self.account_management_discovery_generation =
            self.account_management_discovery_generation.wrapping_add(1);
        let generation = self.account_management_discovery_generation;
        let info = session.info.clone();
        let tx = self.self_tx.clone();
        #[cfg(test)]
        let override_result = self
            .account_management_discovery_override
            .lock()
            .expect("account-management discovery override lock")
            .take();
        self.account_management_discovery_task = Some(crate::executor::spawn(async move {
            #[cfg(test)]
            let url = match override_result {
                Some(result) => result.await.unwrap_or(None),
                None => koushi_sdk::resolve_active_session_account_management_url(&session).await,
            };
            #[cfg(not(test))]
            let url = koushi_sdk::resolve_active_session_account_management_url(&session).await;
            let _ = tx
                .send(
                    super::actor::AccountMessage::ActiveSessionAccountManagementUrlResolved {
                        generation,
                        info,
                        url,
                    },
                )
                .await;
        }));
    }

    pub(super) async fn stop_active_session_account_management_discovery(&mut self) {
        self.account_management_discovery_generation =
            self.account_management_discovery_generation.wrapping_add(1);
        if let Some(task) = self.account_management_discovery_task.take() {
            task.abort();
            let _ = task.await;
        }
    }

    pub(super) async fn handle_active_session_account_management_url_resolved(
        &mut self,
        generation: u64,
        info: koushi_state::SessionInfo,
        url: Option<String>,
    ) {
        if generation != self.account_management_discovery_generation
            || !self.session_promoted
            || !matches!(self.session.as_deref(), Some(session) if session.info == info)
        {
            return;
        }
        if let Some(task) = self.account_management_discovery_task.take() {
            let _ = task.await;
        }
        self.send_actions(vec![AppAction::ActiveSessionAccountManagementUrlResolved {
            info,
            url: url.map(koushi_state::AccountManagementUrl::from_validated),
        }])
        .await;
    }

    pub(super) async fn handle_load_account_management_capabilities(
        &mut self,
        request_id: RequestId,
    ) {
        let session = match &self.session {
            Some(session) => session.clone(),
            None => {
                self.send_actions(vec![AppAction::AccountManagementCapabilitiesLoadFailed])
                    .await;
                self.emit_failure(request_id, CoreFailure::SessionRequired);
                return;
            }
        };

        let capabilities = koushi_sdk::account_management_capabilities(&session).await;
        self.send_actions(vec![AppAction::AccountManagementCapabilitiesLoaded {
            change_password: capabilities.change_password,
        }])
        .await;
    }

    pub(super) async fn handle_change_password(
        &mut self,
        request_id: RequestId,
        new_password: koushi_state::AuthSecret,
    ) {
        let operation = AccountManagementOperation::ChangePassword;
        let session = match &self.session {
            Some(session) => session.clone(),
            None => {
                self.project_account_management_failure(
                    request_id,
                    operation,
                    AuthFailureKind::Sdk,
                    CoreFailure::SessionRequired,
                )
                .await;
                return;
            }
        };

        let result = koushi_sdk::change_password(&session, &new_password, None, None).await;
        match result {
            Ok(()) => {
                self.send_actions(vec![AppAction::AccountManagementSucceeded {
                    request_id: request_id.sequence,
                    operation,
                }])
                .await;
            }
            Err(koushi_sdk::AccountManagementError::UiaaChallenge { session }) => {
                let flow_id = request_id.sequence;
                self.pending_uia_operations.insert(
                    flow_id,
                    PendingUiaOperation {
                        operation,
                        new_password: Some(new_password),
                        erase_data: false,
                        uiaa_session: session,
                    },
                );
                self.send_actions(vec![AppAction::AccountManagementUiaRequired {
                    request_id: request_id.sequence,
                    flow_id,
                    operation,
                }])
                .await;
            }
            Err(koushi_sdk::AccountManagementError::Sdk(_)) => {
                drop(new_password);
                self.project_account_management_failure(
                    request_id,
                    operation,
                    AuthFailureKind::Sdk,
                    CoreFailure::AccountOperationFailed {
                        kind: AuthFailureKind::Sdk,
                    },
                )
                .await;
            }
        }
    }

    pub(super) async fn handle_deactivate_account(
        &mut self,
        request_id: RequestId,
        erase_data: bool,
    ) {
        let operation = AccountManagementOperation::DeactivateAccount;
        let session = match &self.session {
            Some(session) => session.clone(),
            None => {
                self.project_account_management_failure(
                    request_id,
                    operation,
                    AuthFailureKind::Sdk,
                    CoreFailure::SessionRequired,
                )
                .await;
                return;
            }
        };

        let result = koushi_sdk::deactivate_account(&session, erase_data, None, None).await;
        match result {
            Ok(()) => {
                self.pending_uia_operations.remove(&request_id.sequence);
                self.send_actions(vec![AppAction::AccountManagementSucceeded {
                    request_id: request_id.sequence,
                    operation,
                }])
                .await;
                // Deactivation ends the account on the server. Perform local
                // sign-out cleanup without sending a second /logout request.
                self.perform_logout(request_id, false, false).await;
            }
            Err(koushi_sdk::AccountManagementError::UiaaChallenge { session }) => {
                let flow_id = request_id.sequence;
                self.pending_uia_operations.insert(
                    flow_id,
                    PendingUiaOperation {
                        operation,
                        new_password: None,
                        erase_data,
                        uiaa_session: session,
                    },
                );
                self.send_actions(vec![AppAction::AccountManagementUiaRequired {
                    request_id: request_id.sequence,
                    flow_id,
                    operation,
                }])
                .await;
            }
            Err(koushi_sdk::AccountManagementError::Sdk(_)) => {
                self.project_account_management_failure(
                    request_id,
                    operation,
                    AuthFailureKind::Sdk,
                    CoreFailure::AccountOperationFailed {
                        kind: AuthFailureKind::Sdk,
                    },
                )
                .await;
            }
        }
    }

    pub(super) async fn handle_submit_account_management_uia(
        &mut self,
        request_id: RequestId,
        flow_id: u64,
        auth: koushi_state::IdentityResetAuthRequest,
    ) {
        let Some(mut pending) = self.pending_uia_operations.remove(&flow_id) else {
            self.emit_failure(
                request_id,
                CoreFailure::AccountOperationFailed {
                    kind: AuthFailureKind::Sdk,
                },
            );
            return;
        };
        let operation = pending.operation;
        let session = match &self.session {
            Some(session) => session.clone(),
            None => {
                self.project_account_management_failure(
                    RequestId {
                        connection_id: request_id.connection_id,
                        sequence: flow_id,
                    },
                    operation,
                    AuthFailureKind::Sdk,
                    CoreFailure::SessionRequired,
                )
                .await;
                return;
            }
        };

        let result = match operation {
            AccountManagementOperation::ChangePassword => {
                let Some(new_password) = pending.new_password.as_ref() else {
                    self.project_account_management_failure(
                        RequestId {
                            connection_id: request_id.connection_id,
                            sequence: flow_id,
                        },
                        operation,
                        AuthFailureKind::Sdk,
                        CoreFailure::AccountOperationFailed {
                            kind: AuthFailureKind::Sdk,
                        },
                    )
                    .await;
                    return;
                };
                koushi_sdk::change_password(
                    &session,
                    new_password,
                    Some(&auth),
                    pending.uiaa_session.as_deref(),
                )
                .await
            }
            AccountManagementOperation::DeactivateAccount => {
                koushi_sdk::deactivate_account(
                    &session,
                    pending.erase_data,
                    Some(&auth),
                    pending.uiaa_session.as_deref(),
                )
                .await
            }
        };
        drop(auth);
        match result {
            Ok(()) => {
                let was_deactivation = operation == AccountManagementOperation::DeactivateAccount;
                self.send_actions(vec![AppAction::AccountManagementSucceeded {
                    request_id: flow_id,
                    operation,
                }])
                .await;
                if was_deactivation {
                    self.perform_logout(
                        RequestId {
                            connection_id: request_id.connection_id,
                            sequence: flow_id,
                        },
                        false,
                        false,
                    )
                    .await;
                }
            }
            Err(koushi_sdk::AccountManagementError::UiaaChallenge { session }) => {
                pending.uiaa_session = session;
                self.pending_uia_operations.insert(flow_id, pending);
                self.emit_failure(
                    request_id,
                    CoreFailure::AccountOperationFailed {
                        kind: AuthFailureKind::Forbidden,
                    },
                );
            }
            Err(koushi_sdk::AccountManagementError::Sdk(_)) => {
                self.project_account_management_failure(
                    RequestId {
                        connection_id: request_id.connection_id,
                        sequence: flow_id,
                    },
                    operation,
                    AuthFailureKind::Sdk,
                    CoreFailure::AccountOperationFailed {
                        kind: AuthFailureKind::Sdk,
                    },
                )
                .await;
            }
        }
    }

    async fn project_account_management_failure(
        &self,
        request_id: RequestId,
        operation: AccountManagementOperation,
        kind: AuthFailureKind,
        failure: CoreFailure,
    ) {
        self.send_actions(vec![AppAction::AccountManagementFailed {
            request_id: request_id.sequence,
            operation,
            kind,
        }])
        .await;
        self.emit_failure(request_id, failure);
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use koushi_state::{
        AppAction, CurrentDeviceTrustState, SessionAuthenticationMethod, SessionInfo,
    };
    use tokio::sync::{mpsc, oneshot};

    use crate::account::{
        actor::AccountMessage,
        session_lifecycle::SessionInvalidationReason,
        test_support::{
            acknowledge_next_verified_projection, consume_initial_unknown_trust_projection,
            inspect_session_runtime, login_gated_actor, shutdown_and_ack, test_request_id,
        },
    };

    async fn promoted_actor_with_blocked_discovery() -> (
        crate::account::actor::AccountActorHandle,
        mpsc::Receiver<Vec<AppAction>>,
        oneshot::Sender<Option<String>>,
    ) {
        let (handle, mut action_rx) = login_gated_actor().await;
        consume_initial_unknown_trust_projection(&mut action_rx).await;
        let (release, result) = oneshot::channel();
        assert!(
            handle
                .send(AccountMessage::ConfigureAccountManagementDiscovery { result })
                .await
        );
        assert!(
            handle
                .send(AccountMessage::CurrentDeviceTrustChanged {
                    generation: 2,
                    trust: CurrentDeviceTrustState::Verified,
                })
                .await
        );
        acknowledge_next_verified_projection(&handle, &mut action_rx).await;
        (handle, action_rx, release)
    }

    #[tokio::test]
    async fn promoted_restored_session_starts_active_account_management_discovery() {
        let (handle, mut action_rx) = login_gated_actor().await;
        consume_initial_unknown_trust_projection(&mut action_rx).await;
        assert!(
            handle
                .send(AccountMessage::CurrentDeviceTrustChanged {
                    generation: 2,
                    trust: CurrentDeviceTrustState::Verified,
                })
                .await
        );
        acknowledge_next_verified_projection(&handle, &mut action_rx).await;

        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                if matches!(
                    action_rx.recv().await.as_deref(),
                    Some([AppAction::ActiveSessionAccountManagementUrlResolved { .. }])
                ) {
                    break;
                }
            }
        })
        .await
        .expect("promotion must discover an active-session destination without login discovery");
        let _ = handle.send(AccountMessage::Shutdown).await;
    }

    #[tokio::test]
    async fn trust_quarantine_aborts_active_account_management_discovery() {
        let (handle, mut action_rx, release) = promoted_actor_with_blocked_discovery().await;
        assert!(
            handle
                .send(AccountMessage::CurrentDeviceTrustChanged {
                    generation: 2,
                    trust: CurrentDeviceTrustState::Unverified,
                })
                .await
        );
        let (generation, transition_id) = loop {
            if let Some(
                [
                    AppAction::AuthoritativeDeviceTrustChanged {
                        generation,
                        transition_id,
                        trust: CurrentDeviceTrustState::Unverified,
                    },
                ],
            ) = action_rx.recv().await.as_deref()
            {
                break (*generation, *transition_id);
            }
        };
        assert!(
            handle
                .send(AccountMessage::TrustProjectionApplied {
                    generation,
                    transition_id,
                    ready: false,
                    locked: false,
                })
                .await
        );
        while inspect_session_runtime(&handle).await != (true, false, false, true) {
            crate::executor::sleep(Duration::from_millis(5)).await;
        }
        assert!(
            release
                .send(Some("https://stale.example/devices".to_owned()))
                .is_err()
        );
        let _ = handle.send(AccountMessage::Shutdown).await;
    }

    #[tokio::test]
    async fn authentication_lock_aborts_active_account_management_discovery() {
        let (handle, _action_rx, release) = promoted_actor_with_blocked_discovery().await;
        assert!(
            handle
                .send(AccountMessage::SessionInvalidated {
                    reason: SessionInvalidationReason::UnknownToken { soft_logout: true },
                })
                .await
        );
        while inspect_session_runtime(&handle).await.1 {
            crate::executor::sleep(Duration::from_millis(5)).await;
        }
        assert!(
            release
                .send(Some("https://stale.example/devices".to_owned()))
                .is_err()
        );
        let _ = handle.send(AccountMessage::Shutdown).await;
    }

    #[tokio::test]
    async fn logout_aborts_active_account_management_discovery() {
        let (handle, _action_rx, release) = promoted_actor_with_blocked_discovery().await;
        assert!(
            handle
                .send(AccountMessage::Command(
                    crate::command::AccountCommand::ChangeHomeserver {
                        request_id: test_request_id(),
                    },
                ))
                .await
        );
        tokio::time::timeout(Duration::from_secs(2), async {
            while !release.is_closed() {
                crate::executor::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("logout must abort discovery");
        let _ = handle.send(AccountMessage::Shutdown).await;
    }

    #[test]
    fn session_replacement_uses_the_teardown_that_aborts_discovery() {
        let install = crate::account::test_source::item_body(
            include_str!("session_lifecycle.rs"),
            "async fn install_provisional_session",
        );
        let teardown = crate::account::test_source::item_body(
            include_str!("runtime_children.rs"),
            "async fn stop_current_session_runtime",
        );
        assert!(install.contains("stop_current_session_runtime().await"));
        assert!(teardown.contains("stop_active_session_account_management_discovery"));
    }

    #[tokio::test]
    async fn shutdown_aborts_active_account_management_discovery() {
        let (handle, _action_rx, release) = promoted_actor_with_blocked_discovery().await;
        shutdown_and_ack(&handle).await;
        assert!(
            release
                .send(Some("https://stale.example/devices".to_owned()))
                .is_err()
        );
    }

    #[tokio::test]
    async fn wrong_session_destination_completion_is_ignored() {
        let (handle, mut action_rx, release) = promoted_actor_with_blocked_discovery().await;
        assert!(
            handle
                .send(AccountMessage::ActiveSessionAccountManagementUrlResolved {
                    generation: 2,
                    info: SessionInfo {
                        homeserver: "https://other.example".to_owned(),
                        user_id: "@other:example".to_owned(),
                        device_id: "OTHER".to_owned(),
                        authentication_method: SessionAuthenticationMethod::Password,
                    },
                    url: Some("https://stale.example/devices".to_owned()),
                })
                .await
        );
        assert!(
            tokio::time::timeout(Duration::from_millis(100), async {
                loop {
                    if matches!(
                        action_rx.recv().await.as_deref(),
                        Some([AppAction::ActiveSessionAccountManagementUrlResolved { .. }])
                    ) {
                        break;
                    }
                }
            })
            .await
            .is_err()
        );
        drop(release);
        let _ = handle.send(AccountMessage::Shutdown).await;
    }
}
