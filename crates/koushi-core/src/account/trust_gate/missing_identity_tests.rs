use std::sync::{Arc, atomic::Ordering};
use std::time::Duration;

use koushi_state::{
    AppAction, CurrentDeviceTrustState, VerificationAccountKind, VerificationMethodCapability,
};

use crate::account::actor::AccountMessage;
use crate::account::test_support::{
    KeyQueryControl, consume_initial_unknown_trust_projection, inspect_session_runtime,
    login_gated_actor_at, spawn_named_quarantine_password_server_with_controls,
};

#[tokio::test]
async fn existing_identity_unknown_errors_and_stale_absence_cannot_offer_bootstrap() {
    tokio::time::timeout(Duration::from_secs(30), async {
        let control = Arc::new(KeyQueryControl {
            existing_identity: true,
            ..KeyQueryControl::default()
        });
        let homeserver = spawn_named_quarantine_password_server_with_controls(
            "@fixture-user:example.invalid",
            "FIXTUREDEVICE",
            None,
            Some(control.clone()),
            Arc::new(std::sync::atomic::AtomicBool::new(true)),
        );
        let (handle, mut actions) = login_gated_actor_at(homeserver).await;
        consume_initial_unknown_trust_projection(&mut actions).await;
        assert!(
            handle
                .send(AccountMessage::FirstProvisionalEncryptionSyncFinished {
                    generation: 2,
                    succeeded: true
                })
                .await
        );
        loop {
            let batch = actions.recv().await.expect("post-fence query settlement");
            if batch
                .iter()
                .any(|action| matches!(action, AppAction::AuthoritativeDeviceTrustChanged { .. }))
            {
                break;
            }
        }
        for result in [
            Ok((
                CurrentDeviceTrustState::Unknown,
                VerificationAccountKind::ExistingIdentity,
            )),
            Err(koushi_sdk::CurrentDeviceTrustRecheckError::Sdk),
        ] {
            assert!(
                handle
                    .send(AccountMessage::CurrentDeviceTrustRecheckFinished {
                        generation: 2,
                        result
                    })
                    .await
            );
            loop {
                let batch = actions.recv().await.expect("non-admitting settlement");
                assert!(!batch.iter().any(|action| matches!(
                    action,
                    AppAction::VerificationMethodsDiscovered(_)
                        | AppAction::AuthoritativeDeviceTrustChanged {
                            trust: CurrentDeviceTrustState::Verified,
                            ..
                        }
                )));
                if batch.iter().any(|action| {
                    matches!(
                        action,
                        AppAction::AuthoritativeDeviceTrustChanged {
                            trust: CurrentDeviceTrustState::Unknown,
                            ..
                        }
                    )
                }) {
                    break;
                }
            }
        }
        assert!(
            handle
                .send(AccountMessage::CurrentDeviceTrustRecheckFinished {
                    generation: 1,
                    result: Ok((
                        CurrentDeviceTrustState::Unknown,
                        VerificationAccountKind::NewIdentity
                    ))
                })
                .await
        );
        assert_eq!(
            inspect_session_runtime(&handle).await,
            (true, false, false, true)
        );
        while let Ok(batch) = actions.try_recv() {
            assert!(
                !batch
                    .iter()
                    .any(|action| matches!(action, AppAction::VerificationMethodsDiscovered(_)))
            );
        }
        assert_eq!(control.count.load(Ordering::SeqCst), 1);
        assert!(handle.send(AccountMessage::Shutdown).await);
    })
    .await
    .expect("non-admitting identity result deadline");
}

#[tokio::test]
async fn genuine_missing_identity_bootstraps_after_restricted_sync_without_reclassifying_unknown() {
    tokio::time::timeout(Duration::from_secs(30), async {
        let control = Arc::new(KeyQueryControl::default());
        let homeserver = spawn_named_quarantine_password_server_with_controls(
            "@fixture-user:example.invalid",
            "FIXTUREDEVICE",
            None,
            Some(control.clone()),
            Arc::new(std::sync::atomic::AtomicBool::new(true)),
        );
        let (handle, mut actions) = login_gated_actor_at(homeserver).await;
        consume_initial_unknown_trust_projection(&mut actions).await;
        assert!(handle.send(AccountMessage::CheckCurrentDeviceTrust).await);
        loop {
            let batch = actions.recv().await.expect("pre-fence trust result");
            assert!(
                !batch
                    .iter()
                    .any(|action| matches!(action, AppAction::VerificationMethodsDiscovered(_)))
            );
            if batch.iter().any(|action| {
                matches!(
                    action,
                    AppAction::AuthoritativeDeviceTrustChanged {
                        trust: CurrentDeviceTrustState::Unknown,
                        ..
                    }
                )
            }) {
                break;
            }
        }
        assert_eq!(
            inspect_session_runtime(&handle).await,
            (true, false, false, true)
        );
        let before_fence = control.count.load(Ordering::SeqCst);
        assert_eq!(
            before_fence, 1,
            "one authoritative query before the restricted-sync fence"
        );
        assert!(
            handle
                .send(AccountMessage::FirstProvisionalEncryptionSyncFinished {
                    generation: 1,
                    succeeded: true
                })
                .await
        );
        assert_eq!(
            inspect_session_runtime(&handle).await,
            (true, false, false, true)
        );
        assert_eq!(
            control.count.load(Ordering::SeqCst),
            before_fence,
            "stale first-sync callback cannot query or bootstrap"
        );
        assert!(
            handle
                .send(AccountMessage::FirstProvisionalEncryptionSyncFinished {
                    generation: 2,
                    succeeded: true
                })
                .await
        );
        let gate = loop {
            let batch = actions.recv().await.expect("mandatory bootstrap result");
            assert!(!batch.iter().any(|action| matches!(
                action,
                AppAction::AuthoritativeDeviceTrustChanged {
                    trust: CurrentDeviceTrustState::Verified | CurrentDeviceTrustState::Unverified,
                    ..
                }
            )));
            if let Some(gate) = batch.into_iter().find_map(|action| match action {
                AppAction::VerificationMethodsDiscovered(gate) => Some(gate),
                _ => None,
            }) {
                break gate;
            }
        };
        assert_eq!(gate.account_kind, VerificationAccountKind::NewIdentity);
        assert_eq!(gate.methods, vec![VerificationMethodCapability::Bootstrap]);
        assert_eq!(gate.failure, None);
        assert_eq!(
            control.count.load(Ordering::SeqCst),
            before_fence + 1,
            "the existing post-fence recheck must not start another discovery query"
        );
        let (response, trust) = tokio::sync::oneshot::channel();
        assert!(
            handle
                .send(AccountMessage::InspectSdkDeviceTrust { response })
                .await
        );
        assert_eq!(
            trust.await.expect("SDK verdict"),
            Some(CurrentDeviceTrustState::Unknown)
        );
        assert_eq!(
            inspect_session_runtime(&handle).await,
            (true, false, false, true)
        );
        assert!(handle.send(AccountMessage::Shutdown).await);
    })
    .await
    .expect("confirmed-missing bootstrap admission deadline");
}
