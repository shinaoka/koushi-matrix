use koushi_state::{
    PendingKeyCountBucket, SecureBackupFailureStage, SecureBackupFailureTransport,
    SecureBackupGateFailureKind, SecureBackupGateState, SecureBackupInspectionFailure,
    SecureBackupMatrixErrorKind,
};

use super::{
    E2eeTrustError, MatrixSecureBackupInspection, MatrixSecureBackupLocalState,
    MatrixSecureBackupRecoveryState, MatrixSecureBackupServerState, MatrixSecureBackupState,
    MatrixSecureBackupStateObservation, MatrixSecureBackupTrustState,
    MatrixSecureBackupUploadState, SecureBackupStateStream,
    classify_secure_backup_inspection_failure, classify_secure_backup_upload,
};

#[test]
fn secure_backup_upload_snapshot_classifies_without_waiting_for_settlement() {
    use matrix_sdk::encryption::backups::UploadState;
    use matrix_sdk_base::crypto::store::types::RoomKeyCounts;

    assert_eq!(
        classify_secure_backup_upload(
            Ok(RoomKeyCounts {
                total: 125,
                backed_up: 20,
            }),
            UploadState::Uploading(RoomKeyCounts {
                total: 125,
                backed_up: 20,
            }),
        ),
        MatrixSecureBackupUploadState::Pending(PendingKeyCountBucket::OverOneHundred)
    );
    assert_eq!(
        classify_secure_backup_upload(
            Ok(RoomKeyCounts {
                total: 125,
                backed_up: 125,
            }),
            UploadState::Done,
        ),
        MatrixSecureBackupUploadState::Settled
    );
    assert_eq!(
        classify_secure_backup_upload(
            Ok(RoomKeyCounts {
                total: 125,
                backed_up: 20,
            }),
            UploadState::Error,
        ),
        MatrixSecureBackupUploadState::Failed
    );
}

fn inspection(
    server: MatrixSecureBackupServerState,
    local: MatrixSecureBackupLocalState,
    recovery: MatrixSecureBackupRecoveryState,
    upload: MatrixSecureBackupUploadState,
    trust: MatrixSecureBackupTrustState,
) -> MatrixSecureBackupInspection {
    MatrixSecureBackupInspection {
        server,
        local,
        recovery,
        upload,
        trust,
        recovery_key_delivery_pending: false,
        local_cross_signing_complete: true,
    }
}

#[test]
fn secure_backup_inspection_classifies_required_cartesian_cases() {
    assert_eq!(
        inspection(
            MatrixSecureBackupServerState::Present,
            MatrixSecureBackupLocalState::Enabled,
            MatrixSecureBackupRecoveryState::Enabled,
            MatrixSecureBackupUploadState::Settled,
            MatrixSecureBackupTrustState::Trusted,
        )
        .recommended_gate_state(),
        SecureBackupGateState::Ready
    );

    assert_eq!(
        inspection(
            MatrixSecureBackupServerState::Present,
            MatrixSecureBackupLocalState::Disabled,
            MatrixSecureBackupRecoveryState::Enabled,
            MatrixSecureBackupUploadState::Unknown,
            MatrixSecureBackupTrustState::Unknown,
        )
        .recommended_gate_state(),
        SecureBackupGateState::ExistingBackupNeedsRecovery { failure: None }
    );

    assert_eq!(
        inspection(
            MatrixSecureBackupServerState::Absent,
            MatrixSecureBackupLocalState::Disabled,
            MatrixSecureBackupRecoveryState::Unknown,
            MatrixSecureBackupUploadState::Unknown,
            MatrixSecureBackupTrustState::Unknown,
        )
        .recommended_gate_state(),
        SecureBackupGateState::SetupRequired
    );

    assert_eq!(
        inspection(
            MatrixSecureBackupServerState::Absent,
            MatrixSecureBackupLocalState::Disabled,
            MatrixSecureBackupRecoveryState::Disabled,
            MatrixSecureBackupUploadState::Unknown,
            MatrixSecureBackupTrustState::Unknown,
        )
        .recommended_gate_state(),
        SecureBackupGateState::ExplicitlyDisabledRequiresSetup
    );

    assert_eq!(
        inspection(
            MatrixSecureBackupServerState::Unknown,
            MatrixSecureBackupLocalState::Enabled,
            MatrixSecureBackupRecoveryState::Enabled,
            MatrixSecureBackupUploadState::Settled,
            MatrixSecureBackupTrustState::Trusted,
        )
        .recommended_gate_state(),
        SecureBackupGateState::Checking
    );

    assert_eq!(
        inspection(
            MatrixSecureBackupServerState::Present,
            MatrixSecureBackupLocalState::Enabled,
            MatrixSecureBackupRecoveryState::Enabled,
            MatrixSecureBackupUploadState::Settled,
            MatrixSecureBackupTrustState::Mismatch,
        )
        .recommended_gate_state(),
        SecureBackupGateState::ExistingBackupNeedsRecovery {
            failure: Some(SecureBackupGateFailureKind::BackupKeyMismatch),
        }
    );

    assert_eq!(
        inspection(
            MatrixSecureBackupServerState::Present,
            MatrixSecureBackupLocalState::Enabled,
            MatrixSecureBackupRecoveryState::Incomplete,
            MatrixSecureBackupUploadState::Settled,
            MatrixSecureBackupTrustState::Trusted,
        )
        .recommended_gate_state(),
        SecureBackupGateState::SecureStorageIncomplete
    );

    assert_eq!(
        inspection(
            MatrixSecureBackupServerState::Present,
            MatrixSecureBackupLocalState::Enabled,
            MatrixSecureBackupRecoveryState::Enabled,
            MatrixSecureBackupUploadState::Failed,
            MatrixSecureBackupTrustState::Trusted,
        )
        .recommended_gate_state(),
        SecureBackupGateState::DegradedRetrying {
            failure: SecureBackupGateFailureKind::Network,
            detail: None,
        }
    );
}

#[test]
fn pending_recovery_key_delivery_survives_inspection_and_keeps_gate_closed() {
    let mut inspection = inspection(
        MatrixSecureBackupServerState::Present,
        MatrixSecureBackupLocalState::Enabled,
        MatrixSecureBackupRecoveryState::Enabled,
        MatrixSecureBackupUploadState::Settled,
        MatrixSecureBackupTrustState::Trusted,
    );
    inspection.recovery_key_delivery_pending = true;

    assert_eq!(
        inspection.recommended_gate_state(),
        koushi_state::SecureBackupGateState::RecoveryKeyDeliveryRequired
    );
}

#[test]
fn secure_backup_inspection_requires_typed_trust_evidence() {
    assert_eq!(
        inspection(
            MatrixSecureBackupServerState::Present,
            MatrixSecureBackupLocalState::Enabled,
            MatrixSecureBackupRecoveryState::Enabled,
            MatrixSecureBackupUploadState::Settled,
            MatrixSecureBackupTrustState::Unknown,
        )
        .recommended_gate_state(),
        SecureBackupGateState::Checking
    );

    assert_eq!(
        inspection(
            MatrixSecureBackupServerState::Present,
            MatrixSecureBackupLocalState::Enabled,
            MatrixSecureBackupRecoveryState::Enabled,
            MatrixSecureBackupUploadState::Settled,
            MatrixSecureBackupTrustState::Mismatch,
        )
        .recommended_gate_state(),
        SecureBackupGateState::ExistingBackupNeedsRecovery {
            failure: Some(SecureBackupGateFailureKind::BackupKeyMismatch),
        }
    );
}

#[test]
fn secure_backup_state_observation_is_public_and_private_data_free() {
    let state = MatrixSecureBackupState {
        backup: MatrixSecureBackupLocalState::Enabled,
        recovery: MatrixSecureBackupRecoveryState::Enabled,
    };
    let serialized = serde_json::to_string(&state).expect("state is serializable");
    let debug = format!("{state:?}");

    assert!(serialized.contains("backup"));
    assert!(serialized.contains("recovery"));
    for forbidden in [
        "backup-version-42",
        "recovery-key-secret",
        "@alice:example.invalid",
        "!room:example.invalid",
        "/tmp/recovery-key.txt",
        "raw SDK failure",
    ] {
        assert!(
            !serialized.contains(forbidden),
            "serialized state leaked {forbidden}"
        );
        assert!(!debug.contains(forbidden), "debug state leaked {forbidden}");
    }

    let _observation: Option<MatrixSecureBackupStateObservation> = None;
    let _stream: Option<SecureBackupStateStream> = None;
    let _observe: fn(&super::MatrixClientSession) -> MatrixSecureBackupStateObservation =
        super::MatrixClientSession::observe_secure_backup_state;
}

#[test]
fn secure_backup_inspection_has_no_secret_or_identifier_surface() {
    let inspection = inspection(
        MatrixSecureBackupServerState::Present,
        MatrixSecureBackupLocalState::Enabled,
        MatrixSecureBackupRecoveryState::Enabled,
        MatrixSecureBackupUploadState::Pending(PendingKeyCountBucket::One),
        MatrixSecureBackupTrustState::Trusted,
    );
    let serialized = serde_json::to_string(&inspection).expect("inspection is serializable");
    let debug = format!("{inspection:?}");
    for forbidden in [
        "backup-version-42",
        "recovery-key-secret",
        "@alice:example.invalid",
        "!room:example.invalid",
        "/tmp/recovery-key.txt",
        "raw SDK failure",
    ] {
        assert!(
            !serialized.contains(forbidden),
            "serialized inspection leaked {forbidden}"
        );
        assert!(
            !debug.contains(forbidden),
            "debug inspection leaked {forbidden}"
        );
    }
    assert!(!serialized.contains("version"));
    assert!(!debug.contains("version"));

    let error = E2eeTrustError::Sdk("raw SDK failure with a recovery-key-secret".to_owned());
    assert!(!format!("{error:?}").contains("raw SDK failure"));
    assert!(!format!("{error:?}").contains("recovery-key-secret"));
}

/// Right after a restart the persisted delivery marker may be read before the
/// local backup and secret storage settle. The lost-key reset is offered
/// (`RecoveryKeyDeliveryRequired`) only once `reset_key()` can succeed.
#[test]
fn pending_delivery_offers_the_reset_only_when_the_reset_can_succeed() {
    let settled = || {
        let mut inspection = inspection(
            MatrixSecureBackupServerState::Present,
            MatrixSecureBackupLocalState::Enabled,
            MatrixSecureBackupRecoveryState::Enabled,
            MatrixSecureBackupUploadState::Unknown,
            MatrixSecureBackupTrustState::Trusted,
        );
        inspection.recovery_key_delivery_pending = true;
        inspection
    };
    assert!(settled().recovery_key_reset_is_possible());
    assert_eq!(
        settled().recommended_gate_state(),
        SecureBackupGateState::RecoveryKeyDeliveryRequired
    );

    for local in [
        MatrixSecureBackupLocalState::Unknown,
        MatrixSecureBackupLocalState::Enabling,
        MatrixSecureBackupLocalState::Resuming,
        MatrixSecureBackupLocalState::Downloading,
    ] {
        let unsettled = MatrixSecureBackupInspection { local, ..settled() };
        assert!(!unsettled.recovery_key_reset_is_possible());
        assert_eq!(
            unsettled.recommended_gate_state(),
            SecureBackupGateState::Checking,
            "{local:?}"
        );
    }
    let untrusted = MatrixSecureBackupInspection {
        trust: MatrixSecureBackupTrustState::Unknown,
        ..settled()
    };
    assert!(!untrusted.recovery_key_reset_is_possible());
    assert_eq!(
        untrusted.recommended_gate_state(),
        SecureBackupGateState::Checking
    );
    let disabled = MatrixSecureBackupInspection {
        local: MatrixSecureBackupLocalState::Disabled,
        ..settled()
    };
    assert_eq!(
        disabled.recommended_gate_state(),
        SecureBackupGateState::ExistingBackupNeedsRecovery { failure: None }
    );
}

/// NEW-1: `enable()` created and enabled the backup but secret storage was
/// never created (network error or the app was killed). Nothing can be lost:
/// the backup key and complete cross-signing keys are local, so the lost-key
/// reset (`reset_key()`) is offered and admitted to finish the missing step.
#[test]
fn pending_delivery_without_secret_storage_offers_the_reset() {
    let interrupted = |recovery| {
        let mut inspection = inspection(
            MatrixSecureBackupServerState::Present,
            MatrixSecureBackupLocalState::Enabled,
            recovery,
            MatrixSecureBackupUploadState::Unknown,
            MatrixSecureBackupTrustState::Trusted,
        );
        inspection.recovery_key_delivery_pending = true;
        inspection
    };
    for recovery in [
        MatrixSecureBackupRecoveryState::Unknown,
        MatrixSecureBackupRecoveryState::Disabled,
    ] {
        assert!(
            interrupted(recovery).recovery_key_reset_is_possible(),
            "{recovery:?}"
        );
        assert_eq!(
            interrupted(recovery).recommended_gate_state(),
            SecureBackupGateState::RecoveryKeyDeliveryRequired,
            "{recovery:?}"
        );

        // Incomplete local cross-signing keys would be lost by a new store.
        let missing_keys = MatrixSecureBackupInspection {
            local_cross_signing_complete: false,
            ..interrupted(recovery)
        };
        assert!(
            !missing_keys.recovery_key_reset_is_possible(),
            "{recovery:?}"
        );
        assert_ne!(
            missing_keys.recommended_gate_state(),
            SecureBackupGateState::RecoveryKeyDeliveryRequired,
            "{recovery:?}"
        );
    }

    // Secret storage exists but secrets are missing locally: keep refusing.
    let incomplete = interrupted(MatrixSecureBackupRecoveryState::Incomplete);
    assert!(!incomplete.recovery_key_reset_is_possible());
    assert_eq!(
        incomplete.recommended_gate_state(),
        SecureBackupGateState::SecureStorageIncomplete
    );
}

// ---------------------------------------------------------------------------
// #1265: a server response must not be reported as a transport failure.
//
// Synthetic fixtures only: a constructed `matrix_sdk::Error` is exactly what
// `Backups::inspect_server_trust()` surfaces for the matching HTTP response,
// with a synthetic body string so a leak into Debug/UI would be visible.
// ---------------------------------------------------------------------------

const SYNTHETIC_RESPONSE_BODY: &str = "synthetic-response-body-must-not-surface";

fn synthetic_http_response_error(
    status: u16,
    kind: matrix_sdk::ruma::api::error::ErrorKind,
) -> matrix_sdk::Error {
    use matrix_sdk::ruma::api::client::uiaa::UiaaResponse;
    use matrix_sdk::ruma::api::error::{
        Error as RumaError, ErrorBody, FromHttpResponseError, StandardErrorBody,
    };
    use matrix_sdk::ruma::exports::http::StatusCode;

    let ruma_error = RumaError::new(
        StatusCode::from_u16(status).expect("synthetic status is valid"),
        ErrorBody::Standard(StandardErrorBody::new(
            kind,
            SYNTHETIC_RESPONSE_BODY.to_owned(),
        )),
    );
    matrix_sdk::Error::Http(Box::new(matrix_sdk::HttpError::Api(Box::new(
        FromHttpResponseError::Server(UiaaResponse::MatrixError(ruma_error)),
    ))))
}

fn synthetic_transport_error() -> matrix_sdk::Error {
    matrix_sdk::Error::Io(std::io::Error::new(
        std::io::ErrorKind::ConnectionReset,
        SYNTHETIC_RESPONSE_BODY,
    ))
}

fn classification(error: &matrix_sdk::Error) -> SecureBackupInspectionFailure {
    classify_secure_backup_inspection_failure(error)
}

#[test]
fn received_http_error_response_is_not_a_transport_failure() {
    use matrix_sdk::ruma::api::error::ErrorKind;
    let failure = classification(&synthetic_http_response_error(500, ErrorKind::Unknown));
    let detail = failure.detail.expect("a structured detail is preserved");
    assert_eq!(failure.kind, SecureBackupGateFailureKind::ServerResponse);
    assert_eq!(detail.stage, SecureBackupFailureStage::InspectServerTrust);
    assert_eq!(detail.transport, SecureBackupFailureTransport::HttpResponse);
    assert_eq!(detail.http_status, Some(500));
    assert_eq!(
        detail.matrix_error_kind,
        Some(SecureBackupMatrixErrorKind::Unknown)
    );
    assert!(detail.retryable);
}

#[test]
fn classified_http_responses_match_status_kind_and_retryability() {
    use matrix_sdk::ruma::api::error::{ErrorKind, UnknownTokenErrorData};

    let cases: Vec<(
        u16,
        ErrorKind,
        SecureBackupGateFailureKind,
        Option<SecureBackupMatrixErrorKind>,
        bool,
    )> = vec![
        (
            401,
            ErrorKind::UnknownToken(UnknownTokenErrorData::new()),
            SecureBackupGateFailureKind::Unauthorized,
            Some(SecureBackupMatrixErrorKind::UnknownToken),
            false,
        ),
        (
            403,
            ErrorKind::Forbidden,
            SecureBackupGateFailureKind::Forbidden,
            Some(SecureBackupMatrixErrorKind::Forbidden),
            false,
        ),
        (
            404,
            ErrorKind::NotFound,
            SecureBackupGateFailureKind::ServerResponse,
            Some(SecureBackupMatrixErrorKind::NotFound),
            false,
        ),
        (
            429,
            ErrorKind::LimitExceeded(Default::default()),
            SecureBackupGateFailureKind::RateLimited,
            Some(SecureBackupMatrixErrorKind::LimitExceeded),
            true,
        ),
        (
            503,
            ErrorKind::Unknown,
            SecureBackupGateFailureKind::ServerResponse,
            Some(SecureBackupMatrixErrorKind::Unknown),
            true,
        ),
    ];

    for (status, kind, expected_kind, expected_matrix_kind, retryable) in cases {
        let failure = classification(&synthetic_http_response_error(status, kind));
        let detail = failure.detail.expect("a structured detail is preserved");
        assert_eq!(detail.transport, SecureBackupFailureTransport::HttpResponse);
        assert_eq!(detail.http_status, Some(status), "status {status}");
        assert_eq!(failure.kind, expected_kind, "status {status}");
        assert_eq!(
            detail.matrix_error_kind, expected_matrix_kind,
            "status {status}"
        );
        assert_eq!(detail.retryable, retryable, "status {status}");
    }
}

#[test]
fn transport_failure_and_timeout_are_distinguished_from_a_response() {
    let transport = classification(&synthetic_transport_error());
    let detail = transport.detail.expect("a structured detail is preserved");
    assert_eq!(transport.kind, SecureBackupGateFailureKind::Network);
    assert_eq!(detail.transport, SecureBackupFailureTransport::NoResponse);
    assert_eq!(detail.http_status, None);
    assert_eq!(detail.matrix_error_kind, None);
    assert!(detail.retryable);

    let timeout = classification(&matrix_sdk::Error::Timeout);
    let detail = timeout.detail.expect("a structured detail is preserved");
    assert_eq!(timeout.kind, SecureBackupGateFailureKind::Timeout);
    assert_eq!(detail.transport, SecureBackupFailureTransport::Timeout);
    assert!(detail.retryable);
}

#[test]
fn structured_failure_never_carries_the_response_body_or_error_text() {
    use matrix_sdk::ruma::api::error::ErrorKind;

    let failure = classification(&synthetic_http_response_error(500, ErrorKind::Unknown));
    let debug = format!("{failure:?}");
    assert!(!debug.contains(SYNTHETIC_RESPONSE_BODY), "{debug}");

    let serialized = serde_json::to_string(&failure.detail.expect("detail")).expect("serializes");
    assert!(
        !serialized.contains(SYNTHETIC_RESPONSE_BODY),
        "{serialized}"
    );
    assert!(!serialized.contains("example.invalid"), "{serialized}");
    // The wire shape is exactly the bounded field set.
    let value: serde_json::Value = serde_json::from_str(&serialized).expect("json");
    let mut keys: Vec<&str> = value
        .as_object()
        .expect("object")
        .keys()
        .map(String::as_str)
        .collect();
    keys.sort_unstable();
    assert_eq!(
        keys,
        vec![
            "httpStatus",
            "matrixErrorKind",
            "retryable",
            "stage",
            "transport"
        ]
    );
}
