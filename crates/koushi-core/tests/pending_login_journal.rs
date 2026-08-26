//! RED contract tests for the bounded crash-recoverable pending-login journal.

use koushi_core::login_store_test_support::{self as journal, PendingLoginCase};

#[test]
fn pending_login_cap_is_eight_and_occupied_slots_refuse_new_allocations() {
    let report = journal::run(PendingLoginCase::CapAndOccupiedSlot);

    assert_eq!(report.max_allocations, 8);
    assert_eq!(report.allocations, 8);
    assert_eq!(report.new_allocation, "refused");
}

#[test]
fn startup_rejects_invalid_duplicate_missing_mismatched_and_ambiguous_roots() {
    for case in [
        PendingLoginCase::InvalidId,
        PendingLoginCase::Duplicate,
        PendingLoginCase::MissingRoot,
        PendingLoginCase::MismatchedRoot,
        PendingLoginCase::AmbiguousRoot,
    ] {
        let report = journal::run(case);
        assert_eq!(report.startup, "fail_closed");
        assert_eq!(report.deleted_roots, 0);
    }
}

#[test]
fn abandoning_resumes_before_and_after_exact_root_deletion() {
    for case in [
        PendingLoginCase::AbandonInterruptedBeforeDelete,
        PendingLoginCase::AbandonInterruptedAfterDelete,
    ] {
        let report = journal::run(case);
        assert_eq!(report.final_state, "removed");
        assert_eq!(report.deleted_roots, 1);
        assert_eq!(report.parent_syncs, 1);
    }
}

#[test]
fn immediate_cleanup_requires_closed_no_request_or_server_rejection() {
    assert_eq!(
        journal::cleanup(PendingLoginCase::NoRequestSent),
        "immediate"
    );
    assert_eq!(
        journal::cleanup(PendingLoginCase::ServerRejectedBeforeSession),
        "immediate"
    );
    for case in [
        PendingLoginCase::Timeout,
        PendingLoginCase::TransportFailure,
        PendingLoginCase::BrowserCancellation,
        PendingLoginCase::CallbackLoss,
        PendingLoginCase::TokenExchangeAmbiguous,
    ] {
        assert_eq!(journal::cleanup(case), "retain");
    }
}

#[test]
fn cancellation_retains_one_allocation_and_stale_generation_cannot_bind_or_delete() {
    let report = journal::run(PendingLoginCase::CancelledThenStaleCallback);

    assert_eq!(report.allocations, 1);
    assert_eq!(report.callback_mutations, 0);
    assert_eq!(report.deleted_roots, 0);
    assert_eq!(report.final_state, "resumable");
}
