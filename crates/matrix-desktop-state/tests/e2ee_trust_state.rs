use matrix_desktop_state::{
    AppAction, AppEffect, AppState, CrossSigningStatus, E2eeTrustState, IdentityResetAuthType,
    IdentityResetState, KeyBackupStatus, SasEmoji, SessionInfo, SessionState,
    TrustOperationFailureKind, UiEvent, VerificationCancelReason, VerificationFlowState,
    VerificationTarget, reduce,
};
use serde_json::json;

fn ready_state() -> AppState {
    let mut state = AppState::default();
    state.session = SessionState::Ready(SessionInfo {
        homeserver: "https://server.example.invalid".to_owned(),
        user_id: "@alice:example.invalid".to_owned(),
        device_id: "ALICEDEVICE".to_owned(),
    });
    state
}

fn target() -> VerificationTarget {
    VerificationTarget {
        user_id: "@bob:example.invalid".to_owned(),
        device_id: "BOBDEVICE".to_owned(),
    }
}

fn sas() -> Vec<SasEmoji> {
    vec![
        SasEmoji {
            symbol: "🐶".to_owned(),
            description: "Dog".to_owned(),
        },
        SasEmoji {
            symbol: "🌙".to_owned(),
            description: "Moon".to_owned(),
        },
    ]
}

#[test]
fn e2ee_trust_state_defaults_to_private_data_free_unknowns() {
    let state = AppState::default();

    assert_eq!(state.e2ee_trust, E2eeTrustState::default());
    assert_eq!(state.e2ee_trust.verification, VerificationFlowState::Idle);
    assert_eq!(state.e2ee_trust.cross_signing, CrossSigningStatus::Unknown);
    assert_eq!(state.e2ee_trust.key_backup, KeyBackupStatus::Unknown);
    assert!(format!("{:?}", state.e2ee_trust).contains("Unknown"));
    assert!(!format!("{:?}", state.e2ee_trust).contains("secret"));
}

#[test]
fn verification_flow_is_rust_owned_guarded_and_request_correlated() {
    let mut state = ready_state();
    let target = target();

    assert_eq!(
        reduce(
            &mut state,
            AppAction::VerificationRequested {
                request_id: 7,
                target: target.clone(),
            },
        ),
        vec![
            AppEffect::RequestVerification {
                request_id: 7,
                target: target.clone(),
            },
            AppEffect::EmitUiEvent(UiEvent::E2eeTrustChanged),
        ]
    );
    assert_eq!(
        state.e2ee_trust.verification,
        VerificationFlowState::Requested {
            request_id: 7,
            target: target.clone(),
        }
    );

    assert!(
        reduce(
            &mut state,
            AppAction::VerificationAccepted { request_id: 999 },
        )
        .is_empty()
    );
    assert_eq!(
        state.e2ee_trust.verification,
        VerificationFlowState::Requested {
            request_id: 7,
            target: target.clone(),
        }
    );

    assert_eq!(
        reduce(
            &mut state,
            AppAction::VerificationAccepted { request_id: 7 },
        ),
        vec![
            AppEffect::AcceptVerification { request_id: 7 },
            AppEffect::EmitUiEvent(UiEvent::E2eeTrustChanged),
        ]
    );

    let emojis = sas();
    reduce(
        &mut state,
        AppAction::VerificationSasPresented {
            request_id: 7,
            emojis: emojis.clone(),
        },
    );
    assert_eq!(
        state.e2ee_trust.verification,
        VerificationFlowState::SasPresented {
            request_id: 7,
            target: target.clone(),
            emojis: emojis.clone(),
        }
    );

    assert_eq!(
        reduce(
            &mut state,
            AppAction::VerificationConfirmed { request_id: 7 },
        ),
        vec![
            AppEffect::ConfirmSasVerification { request_id: 7 },
            AppEffect::EmitUiEvent(UiEvent::E2eeTrustChanged),
        ]
    );
    assert_eq!(
        state.e2ee_trust.verification,
        VerificationFlowState::Confirming {
            request_id: 7,
            target: target.clone(),
            emojis,
        }
    );

    reduce(
        &mut state,
        AppAction::VerificationCompleted { request_id: 7 },
    );
    assert_eq!(
        state.e2ee_trust.verification,
        VerificationFlowState::Done {
            request_id: 7,
            target,
        }
    );
}

#[test]
fn verification_cancel_and_failure_settle_only_the_matching_flow() {
    let mut state = ready_state();
    let target = target();
    reduce(
        &mut state,
        AppAction::VerificationRequested {
            request_id: 9,
            target: target.clone(),
        },
    );

    assert!(
        reduce(
            &mut state,
            AppAction::VerificationCancelled {
                request_id: 123,
                reason: VerificationCancelReason::User,
            },
        )
        .is_empty()
    );
    assert!(matches!(
        state.e2ee_trust.verification,
        VerificationFlowState::Requested { request_id: 9, .. }
    ));

    assert_eq!(
        reduce(
            &mut state,
            AppAction::VerificationCancelled {
                request_id: 9,
                reason: VerificationCancelReason::User,
            },
        ),
        vec![
            AppEffect::CancelVerification {
                request_id: 9,
                reason: VerificationCancelReason::User,
            },
            AppEffect::EmitUiEvent(UiEvent::E2eeTrustChanged),
        ]
    );
    assert_eq!(state.e2ee_trust.verification, VerificationFlowState::Idle);

    reduce(
        &mut state,
        AppAction::VerificationRequested {
            request_id: 10,
            target: target.clone(),
        },
    );
    reduce(
        &mut state,
        AppAction::VerificationFailed {
            request_id: 10,
            kind: TrustOperationFailureKind::Sdk,
        },
    );
    assert_eq!(
        state.e2ee_trust.verification,
        VerificationFlowState::Failed {
            request_id: 10,
            target,
            kind: TrustOperationFailureKind::Sdk,
        }
    );
}

#[test]
fn verification_mismatch_cancel_settles_as_mismatch_failure() {
    let mut state = ready_state();
    let target = target();
    let emojis = sas();
    reduce(
        &mut state,
        AppAction::VerificationRequested {
            request_id: 11,
            target: target.clone(),
        },
    );
    reduce(
        &mut state,
        AppAction::VerificationSasPresented {
            request_id: 11,
            emojis,
        },
    );

    assert_eq!(
        reduce(
            &mut state,
            AppAction::VerificationCancelled {
                request_id: 11,
                reason: VerificationCancelReason::Mismatch,
            },
        ),
        vec![
            AppEffect::CancelVerification {
                request_id: 11,
                reason: VerificationCancelReason::Mismatch,
            },
            AppEffect::EmitUiEvent(UiEvent::E2eeTrustChanged),
        ]
    );
    assert_eq!(
        state.e2ee_trust.verification,
        VerificationFlowState::Failed {
            request_id: 11,
            target,
            kind: TrustOperationFailureKind::Mismatch,
        }
    );
}

#[test]
fn verification_mismatch_cancel_is_ignored_before_sas_is_presented() {
    let mut state = ready_state();
    let target = target();
    reduce(
        &mut state,
        AppAction::VerificationRequested {
            request_id: 12,
            target: target.clone(),
        },
    );

    assert!(
        reduce(
            &mut state,
            AppAction::VerificationCancelled {
                request_id: 12,
                reason: VerificationCancelReason::Mismatch,
            },
        )
        .is_empty()
    );
    assert_eq!(
        state.e2ee_trust.verification,
        VerificationFlowState::Requested {
            request_id: 12,
            target,
        }
    );
}

#[test]
fn cross_signing_key_backup_and_reset_identity_are_request_correlated() {
    let mut state = ready_state();

    assert_eq!(
        reduce(
            &mut state,
            AppAction::BootstrapCrossSigningRequested { request_id: 21 },
        ),
        vec![
            AppEffect::BootstrapCrossSigning { request_id: 21 },
            AppEffect::EmitUiEvent(UiEvent::E2eeTrustChanged),
        ]
    );
    assert_eq!(
        state.e2ee_trust.cross_signing,
        CrossSigningStatus::Bootstrapping { request_id: 21 }
    );

    reduce(
        &mut state,
        AppAction::CrossSigningStatusChanged {
            status: CrossSigningStatus::Trusted,
        },
    );
    assert_eq!(state.e2ee_trust.cross_signing, CrossSigningStatus::Trusted);

    reduce(
        &mut state,
        AppAction::EnableKeyBackupRequested { request_id: 22 },
    );
    assert_eq!(
        state.e2ee_trust.key_backup,
        KeyBackupStatus::Enabling { request_id: 22 }
    );
    assert!(
        reduce(
            &mut state,
            AppAction::KeyBackupEnabled {
                request_id: 999,
                version: "v1".to_owned(),
            },
        )
        .is_empty()
    );
    assert_eq!(
        state.e2ee_trust.key_backup,
        KeyBackupStatus::Enabling { request_id: 22 }
    );
    assert!(
        reduce(
            &mut state,
            AppAction::KeyBackupRestored {
                request_id: 22,
                version: Some("v1".to_owned()),
            },
        )
        .is_empty()
    );
    assert_eq!(
        state.e2ee_trust.key_backup,
        KeyBackupStatus::Enabling { request_id: 22 }
    );

    reduce(
        &mut state,
        AppAction::KeyBackupEnabled {
            request_id: 22,
            version: "v1".to_owned(),
        },
    );
    assert_eq!(
        state.e2ee_trust.key_backup,
        KeyBackupStatus::Enabled {
            version: "v1".to_owned(),
        }
    );

    reduce(
        &mut state,
        AppAction::RestoreKeyBackupRequested {
            request_id: 23,
            version: Some("v1".to_owned()),
        },
    );
    assert_eq!(
        state.e2ee_trust.key_backup,
        KeyBackupStatus::Restoring {
            request_id: 23,
            version: Some("v1".to_owned()),
            restored_rooms: 0,
            total_rooms: None,
        }
    );
    reduce(
        &mut state,
        AppAction::KeyBackupRestoreProgress {
            request_id: 23,
            restored_rooms: 4,
            total_rooms: Some(9),
        },
    );
    assert_eq!(
        state.e2ee_trust.key_backup,
        KeyBackupStatus::Restoring {
            request_id: 23,
            version: Some("v1".to_owned()),
            restored_rooms: 4,
            total_rooms: Some(9),
        }
    );
    reduce(
        &mut state,
        AppAction::KeyBackupFailed {
            request_id: 23,
            kind: TrustOperationFailureKind::Network,
        },
    );
    assert_eq!(
        state.e2ee_trust.key_backup,
        KeyBackupStatus::Failed {
            request_id: 23,
            kind: TrustOperationFailureKind::Network,
        }
    );
    assert!(
        reduce(
            &mut state,
            AppAction::KeyBackupRestored {
                request_id: 23,
                version: Some("v1".to_owned()),
            },
        )
        .is_empty()
    );
    assert_eq!(
        state.e2ee_trust.key_backup,
        KeyBackupStatus::Failed {
            request_id: 23,
            kind: TrustOperationFailureKind::Network,
        }
    );

    reduce(
        &mut state,
        AppAction::ResetIdentityRequested { request_id: 24 },
    );
    assert_eq!(
        state.e2ee_trust.identity_reset,
        IdentityResetState::Resetting { request_id: 24 }
    );
    reduce(
        &mut state,
        AppAction::ResetIdentityCompleted { request_id: 24 },
    );
    assert_eq!(state.e2ee_trust.identity_reset, IdentityResetState::Idle);
    assert_eq!(state.e2ee_trust.cross_signing, CrossSigningStatus::Missing);
    assert_eq!(state.e2ee_trust.key_backup, KeyBackupStatus::Disabled);
}

#[test]
fn identity_reset_auth_required_is_rust_owned_and_request_correlated() {
    let mut state = ready_state();

    assert!(
        reduce(
            &mut state,
            AppAction::ResetIdentityAuthRequired {
                request_id: 999,
                auth_type: IdentityResetAuthType::Uiaa,
            },
        )
        .is_empty()
    );
    assert_eq!(state.e2ee_trust.identity_reset, IdentityResetState::Idle);

    reduce(
        &mut state,
        AppAction::ResetIdentityRequested { request_id: 24 },
    );
    assert_eq!(
        state.e2ee_trust.identity_reset,
        IdentityResetState::Resetting { request_id: 24 }
    );

    assert!(
        reduce(
            &mut state,
            AppAction::ResetIdentityAuthRequired {
                request_id: 999,
                auth_type: IdentityResetAuthType::OAuth,
            },
        )
        .is_empty()
    );
    assert_eq!(
        state.e2ee_trust.identity_reset,
        IdentityResetState::Resetting { request_id: 24 }
    );

    assert_eq!(
        reduce(
            &mut state,
            AppAction::ResetIdentityAuthRequired {
                request_id: 24,
                auth_type: IdentityResetAuthType::Uiaa,
            },
        ),
        vec![AppEffect::EmitUiEvent(UiEvent::E2eeTrustChanged)]
    );
    assert_eq!(
        state.e2ee_trust.identity_reset,
        IdentityResetState::AwaitingAuth {
            request_id: 24,
            auth_type: IdentityResetAuthType::Uiaa,
        }
    );

    assert!(
        reduce(
            &mut state,
            AppAction::ResetIdentityFailed {
                request_id: 999,
                kind: TrustOperationFailureKind::Timeout,
            },
        )
        .is_empty()
    );
    assert_eq!(
        state.e2ee_trust.identity_reset,
        IdentityResetState::AwaitingAuth {
            request_id: 24,
            auth_type: IdentityResetAuthType::Uiaa,
        }
    );

    reduce(
        &mut state,
        AppAction::ResetIdentityFailed {
            request_id: 24,
            kind: TrustOperationFailureKind::Forbidden,
        },
    );
    assert_eq!(
        state.e2ee_trust.identity_reset,
        IdentityResetState::Failed {
            request_id: 24,
            kind: TrustOperationFailureKind::Forbidden,
        }
    );
    assert_eq!(
        state.e2ee_trust.cross_signing,
        CrossSigningStatus::Failed {
            request_id: 24,
            kind: TrustOperationFailureKind::Forbidden,
        }
    );
}

#[test]
fn identity_reset_auth_submission_returns_to_resetting_only_for_matching_flow() {
    let mut state = ready_state();

    assert!(
        reduce(
            &mut state,
            AppAction::ResetIdentityAuthSubmitted { request_id: 24 },
        )
        .is_empty()
    );

    reduce(
        &mut state,
        AppAction::ResetIdentityRequested { request_id: 24 },
    );
    reduce(
        &mut state,
        AppAction::ResetIdentityAuthRequired {
            request_id: 24,
            auth_type: IdentityResetAuthType::OAuth,
        },
    );

    assert!(
        reduce(
            &mut state,
            AppAction::ResetIdentityAuthSubmitted { request_id: 99 },
        )
        .is_empty()
    );
    assert_eq!(
        state.e2ee_trust.identity_reset,
        IdentityResetState::AwaitingAuth {
            request_id: 24,
            auth_type: IdentityResetAuthType::OAuth,
        }
    );

    assert_eq!(
        reduce(
            &mut state,
            AppAction::ResetIdentityAuthSubmitted { request_id: 24 },
        ),
        vec![AppEffect::EmitUiEvent(UiEvent::E2eeTrustChanged)]
    );
    assert_eq!(
        state.e2ee_trust.identity_reset,
        IdentityResetState::Resetting { request_id: 24 }
    );
}

#[test]
fn identity_reset_auth_type_wire_values_are_stable() {
    assert_eq!(
        serde_json::to_value(IdentityResetAuthType::Uiaa).unwrap(),
        json!("uiaa")
    );
    assert_eq!(
        serde_json::to_value(IdentityResetAuthType::OAuth).unwrap(),
        json!("oauth")
    );
    assert_eq!(
        serde_json::to_value(IdentityResetAuthType::Unknown).unwrap(),
        json!("unknown")
    );
}

#[test]
fn e2ee_trust_actions_are_ignored_without_ready_session() {
    let mut state = AppState::default();

    assert!(
        reduce(
            &mut state,
            AppAction::VerificationRequested {
                request_id: 1,
                target: target(),
            },
        )
        .is_empty()
    );
    assert_eq!(state.e2ee_trust, E2eeTrustState::default());
}

#[test]
fn e2ee_trust_state_is_cleared_when_session_views_are_cleared() {
    let mut state = ready_state();
    reduce(
        &mut state,
        AppAction::VerificationRequested {
            request_id: 31,
            target: target(),
        },
    );
    assert_ne!(state.e2ee_trust, E2eeTrustState::default());

    let effects = reduce(&mut state, AppAction::LogoutRequested);

    assert_eq!(state.e2ee_trust, E2eeTrustState::default());
    assert!(effects.contains(&AppEffect::EmitUiEvent(UiEvent::E2eeTrustChanged)));
}
