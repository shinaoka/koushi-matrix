//! #2165: a failed send never puts a Rust `Debug` rendering into the reply.

use koushi_core::{CoreFailure, TimelineFailureKind};

use super::*;

#[test]
fn the_admission_lock_is_the_documented_refusal() {
    let error = send_failure(CoreFailure::TimelineOperationFailed {
        kind: TimelineFailureKind::SecureBackupRequired,
    });
    assert_eq!(error.code, ErrorCode::NotPermitted);
    assert_eq!(error.reason.as_deref(), Some("send_not_admitted"));
    assert_eq!(
        error.message,
        "sending is unavailable until Secure Backup is ready"
    );
}

#[test]
fn other_failures_read_as_a_sentence_without_type_names() {
    for kind in [
        TimelineFailureKind::Network,
        TimelineFailureKind::Sdk,
        TimelineFailureKind::Timeout,
        TimelineFailureKind::QueueOverflow,
        TimelineFailureKind::Forbidden,
    ] {
        let error = send_failure(CoreFailure::TimelineOperationFailed { kind });
        assert_eq!(error.code, ErrorCode::Internal);
        assert_eq!(error.message, "the message could not be sent");
    }
    let error = send_failure(CoreFailure::SessionRequired);
    assert_eq!(error.message, "the message could not be sent");
    for error in [
        send_failure(CoreFailure::SessionRequired),
        send_failure(CoreFailure::TimelineOperationFailed {
            kind: TimelineFailureKind::Sdk,
        }),
    ] {
        assert!(!error.message.contains('{') && !error.message.contains("Failed"));
    }
}
