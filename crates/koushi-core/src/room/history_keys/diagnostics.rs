//! Private-data-free records for the history-key request path.
//!
//! Only counts, closed enum tokens and milliseconds leave this module, so a
//! nightly artifact can say whether a request was issued and how long it took
//! to reach `Status::Requested` without carrying a room, user or key material.
use koushi_diagnostics::{DiagnosticEvent, DiagnosticField, DiagnosticLevel};

const SOURCE: &str = "core.history_key_request";

fn event(stage: &'static str) -> DiagnosticEvent {
    DiagnosticEvent::new(DiagnosticLevel::Info, SOURCE, stage)
        .field(DiagnosticField::token("kind", "history_key_request"))
}

/// The room's unresolved-history window opened (`open`) or closed (`close`).
///
/// A window with no `issued` record after it means the dispatch never
/// completed; see [`record_issued`] for why that is not the same as "nothing
/// was attempted".
pub(super) fn record_window(stage: &'static str) {
    koushi_diagnostics::record_and_stderr(event(stage));
}

/// Every targeted device accepted the request and the episode was durably
/// marked sent. A dispatch that fails or stalls part way through writes no
/// `issued` record, so absence means "not completed", not "not attempted".
pub(super) fn record_issued(trigger: &'static str, recipients: u16) {
    koushi_diagnostics::record_and_stderr(
        event("issued")
            .field(DiagnosticField::token("trigger", trigger))
            .field(DiagnosticField::count("recipients", u64::from(recipients))),
    );
}

/// The episode was published as `Status::Requested` after a dispatch.
///
/// `elapsed_ms` runs from the start of the dispatch to this publish call; the
/// projection commit that a QA wait observes follows one action-queue turn
/// later, so the number is the dispatch cost, not the observed latency.
pub(super) fn record_requested(trigger: &'static str, recipients: u16, elapsed_ms: u128) {
    koushi_diagnostics::record_and_stderr(
        event("requested")
            .field(DiagnosticField::token("trigger", trigger))
            .field(DiagnosticField::count("recipients", u64::from(recipients)))
            .field(DiagnosticField::milliseconds("elapsed_ms", elapsed_ms)),
    );
}

#[cfg(test)]
#[path = "diagnostics_tests.rs"]
mod tests;
