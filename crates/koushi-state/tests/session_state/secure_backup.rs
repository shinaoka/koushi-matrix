use super::support::session_info;
use koushi_state::{
    AppAction, AppEffect, AppState, PendingKeyCountBucket, SecureBackupFailureDetail,
    SecureBackupFailureStage, SecureBackupFailureTransport, SecureBackupGateFailureKind,
    SecureBackupGateState, SecureBackupMatrixErrorKind, SessionState, UiEvent,
    encrypted_messaging_is_admitted, reduce,
};

#[test]
fn secure_backup_gate_is_closed_until_authoritative_ready_and_can_degrade() {
    let mut state = AppState {
        session: SessionState::Ready(session_info()),
        secure_backup_gate: SecureBackupGateState::Checking,
        ..AppState::default()
    };

    let effects = reduce(
        &mut state,
        AppAction::SecureBackupGateChanged(SecureBackupGateState::ExistingBackupNeedsRecovery {
            failure: None,
        }),
    );
    assert!(effects.contains(&AppEffect::EmitUiEvent(UiEvent::SessionChanged)));
    assert!(matches!(
        state.secure_backup_gate,
        SecureBackupGateState::ExistingBackupNeedsRecovery { failure: None }
    ));

    reduce(
        &mut state,
        AppAction::SecureBackupGateChanged(SecureBackupGateState::Ready),
    );
    assert_eq!(state.secure_backup_gate, SecureBackupGateState::Ready);

    reduce(
        &mut state,
        AppAction::SecureBackupGateChanged(SecureBackupGateState::DegradedRetrying {
            failure: SecureBackupGateFailureKind::Network,
            detail: None,
        }),
    );
    assert!(matches!(
        state.secure_backup_gate,
        SecureBackupGateState::DegradedRetrying {
            failure: SecureBackupGateFailureKind::Network,
            ..
        }
    ));
    assert_eq!(state.session, SessionState::Ready(session_info()));
}

#[test]
fn signed_out_state_ignores_secure_backup_updates() {
    let mut state = AppState::default();
    let before = state.clone();

    let effects = reduce(
        &mut state,
        AppAction::SecureBackupGateChanged(SecureBackupGateState::Ready),
    );

    assert!(effects.is_empty());
    assert_eq!(state, before);
}

#[test]
fn configured_backup_upload_health_does_not_close_encrypted_admission() {
    let blocking = vec![
        SecureBackupGateState::Inactive,
        SecureBackupGateState::Checking,
        SecureBackupGateState::ExistingBackupNeedsRecovery { failure: None },
        SecureBackupGateState::SecureStorageIncomplete,
        SecureBackupGateState::SetupRequired,
        SecureBackupGateState::ExplicitlyDisabledRequiresSetup,
        SecureBackupGateState::CreatingBackup,
        SecureBackupGateState::RecoveryKeyDeliveryRequired,
        SecureBackupGateState::BlockedFailed {
            failure: SecureBackupGateFailureKind::Sdk,
            detail: None,
        },
    ];
    for gate in blocking {
        let state = AppState {
            session: SessionState::Ready(session_info()),
            secure_backup_gate: gate.clone(),
            ..AppState::default()
        };
        assert!(
            !encrypted_messaging_is_admitted(&state),
            "non-ready backup state admitted encrypted sending: {gate:?}"
        );
    }

    for gate in [
        SecureBackupGateState::Ready,
        SecureBackupGateState::UploadingExistingKeys {
            pending: PendingKeyCountBucket::TwoToTen,
        },
        SecureBackupGateState::DegradedRetrying {
            failure: SecureBackupGateFailureKind::Network,
            detail: None,
        },
    ] {
        let state = AppState {
            session: SessionState::Ready(session_info()),
            secure_backup_gate: gate.clone(),
            ..AppState::default()
        };
        assert!(
            encrypted_messaging_is_admitted(&state),
            "operational backup health state blocked encrypted sending: {gate:?}"
        );
    }

    let unverified = AppState {
        session: SessionState::Locked(session_info()),
        secure_backup_gate: SecureBackupGateState::Ready,
        ..AppState::default()
    };
    assert!(!encrypted_messaging_is_admitted(&unverified));
}

#[test]
fn duplicate_ready_is_quiet_and_degradation_preserves_a_nonempty_draft() {
    let mut state = AppState {
        session: SessionState::Ready(session_info()),
        secure_backup_gate: SecureBackupGateState::Ready,
        ..AppState::default()
    };
    state
        .composer_drafts
        .set_room_draft("!synthetic:example.invalid".to_owned(), "unsent draft");
    let draft_before = state.composer_drafts.clone();

    assert!(
        reduce(
            &mut state,
            AppAction::SecureBackupGateChanged(SecureBackupGateState::Ready),
        )
        .is_empty()
    );
    reduce(
        &mut state,
        AppAction::SecureBackupGateChanged(SecureBackupGateState::DegradedRetrying {
            failure: SecureBackupGateFailureKind::RateLimited,
            detail: None,
        }),
    );
    assert_eq!(state.composer_drafts, draft_before);
}

#[test]
fn secure_backup_gate_wire_is_closed_privacy_safe_and_legacy_defaults_inactive() {
    let cases = vec![
        (SecureBackupGateState::Checking, "checking"),
        (
            SecureBackupGateState::ExistingBackupNeedsRecovery { failure: None },
            "existingBackupNeedsRecovery",
        ),
        (
            SecureBackupGateState::ExistingBackupNeedsRecovery {
                failure: Some(SecureBackupGateFailureKind::InvalidRecoveryKey),
            },
            "existingBackupNeedsRecovery",
        ),
        (
            SecureBackupGateState::SecureStorageIncomplete,
            "secureStorageIncomplete",
        ),
        (SecureBackupGateState::SetupRequired, "setupRequired"),
        (
            SecureBackupGateState::ExplicitlyDisabledRequiresSetup,
            "explicitlyDisabledRequiresSetup",
        ),
        (SecureBackupGateState::CreatingBackup, "creatingBackup"),
        (
            SecureBackupGateState::RecoveryKeyDeliveryRequired,
            "recoveryKeyDeliveryRequired",
        ),
        (
            SecureBackupGateState::UploadingExistingKeys {
                pending: PendingKeyCountBucket::TwoToTen,
            },
            "uploadingExistingKeys",
        ),
        (
            SecureBackupGateState::DegradedRetrying {
                failure: SecureBackupGateFailureKind::Network,
                detail: None,
            },
            "degradedRetrying",
        ),
        (
            SecureBackupGateState::BlockedFailed {
                failure: SecureBackupGateFailureKind::Forbidden,
                detail: None,
            },
            "blockedFailed",
        ),
        (
            SecureBackupGateState::BlockedFailed {
                failure: SecureBackupGateFailureKind::ServerResponse,
                detail: Some(SecureBackupFailureDetail {
                    stage: SecureBackupFailureStage::InspectServerTrust,
                    transport: SecureBackupFailureTransport::HttpResponse,
                    http_status: Some(503),
                    matrix_error_kind: Some(SecureBackupMatrixErrorKind::Unknown),
                    retryable: true,
                }),
            },
            "blockedFailed",
        ),
        (SecureBackupGateState::Ready, "ready"),
    ];
    for (gate, kind) in cases {
        let value = serde_json::to_value(&gate).expect("gate serializes");
        assert_eq!(
            value.get("kind").and_then(serde_json::Value::as_str),
            Some(kind)
        );
        let restored: SecureBackupGateState =
            serde_json::from_value(value).expect("gate round trips");
        assert_eq!(restored, gate);
    }

    // The exact wire shape the TypeScript mirror consumes (#1265).
    let detailed = serde_json::to_value(SecureBackupGateState::BlockedFailed {
        failure: SecureBackupGateFailureKind::ServerResponse,
        detail: Some(SecureBackupFailureDetail {
            stage: SecureBackupFailureStage::InspectServerTrust,
            transport: SecureBackupFailureTransport::HttpResponse,
            http_status: Some(503),
            matrix_error_kind: Some(SecureBackupMatrixErrorKind::Unknown),
            retryable: true,
        }),
    })
    .expect("gate serializes");
    assert_eq!(
        detailed,
        serde_json::json!({
            "kind": "blockedFailed",
            "failure": "serverResponse",
            "detail": {
                "stage": "inspectServerTrust",
                "transport": "httpResponse",
                "httpStatus": 503,
                "matrixErrorKind": "unknown",
                "retryable": true
            }
        })
    );
    // A detail that carries no server response omits the status and kind.
    let transport_only = serde_json::to_value(SecureBackupGateState::DegradedRetrying {
        failure: SecureBackupGateFailureKind::Network,
        detail: Some(SecureBackupFailureDetail {
            stage: SecureBackupFailureStage::InspectServerTrust,
            transport: SecureBackupFailureTransport::NoResponse,
            http_status: None,
            matrix_error_kind: None,
            retryable: true,
        }),
    })
    .expect("gate serializes");
    assert_eq!(
        transport_only,
        serde_json::json!({
            "kind": "degradedRetrying",
            "failure": "network",
            "detail": {
                "stage": "inspectServerTrust",
                "transport": "noResponse",
                "httpStatus": null,
                "matrixErrorKind": null,
                "retryable": true
            }
        })
    );

    let state = AppState {
        session: SessionState::Ready(session_info()),
        secure_backup_gate: SecureBackupGateState::ExistingBackupNeedsRecovery {
            failure: Some(SecureBackupGateFailureKind::InvalidRecoveryKey),
        },
        ..AppState::default()
    };
    let mut legacy = serde_json::to_value(&state).expect("state serializes");
    legacy
        .as_object_mut()
        .expect("state object")
        .remove("secure_backup_gate");
    let restored: AppState = serde_json::from_value(legacy).expect("legacy state restores");
    assert_eq!(restored.secure_backup_gate, SecureBackupGateState::Inactive);

    let serialized = serde_json::to_string(&state.secure_backup_gate).unwrap();
    let debug = format!("{:?}", state.secure_backup_gate);
    for private in [
        "EsT1 RcVy KeyM ater",
        "backup-version-1",
        "!room:example.invalid",
        "raw sdk error",
    ] {
        assert!(!serialized.contains(private));
        assert!(!debug.contains(private));
    }
}
