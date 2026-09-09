//! Standard-only receive-side room-key recovery state machine (issue #478).
//!
//! Rust owns a typed recovery operation keyed internally by
//! (room alias, Megolm session alias). The machine exhausts safe standard
//! sources in order — local crypto store, trusted Secure Backup, the user's
//! own verified devices via `m.room_key_request`, then a bounded wait for
//! standard sender re-sharing (#477) — and settles to `Recovered` or an
//! actionable terminal state. No peer requests, no peer-forwarded keys, no
//! custom wire events, no policy weakening.

use std::time::Duration;

use koushi_diagnostics::{DiagnosticEvent, DiagnosticField, DiagnosticLevel, record};

/// Maximum attempts of the automatic recovery sequence before the operation is
/// considered exhausted for now (a manual retry starts a fresh bounded run).
pub const MAX_RECOVERY_ATTEMPTS: u32 = 3;
/// Minimum backoff between automatic attempts.
pub const RECOVERY_BACKOFF: Duration = Duration::from_secs(30);

/// Closed recovery stage (issue #478): temporary and terminal states distinct.
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum RecoveryStage {
    /// A missing-session UTD was observed.
    Detected,
    /// Checking the local crypto store.
    CheckingLocal,
    /// Checking/importing from trusted Secure Backup.
    CheckingBackup,
    /// Requesting the key from the user's own verified devices.
    RequestingOwnDevices,
    /// Flushing Olm unwedge work required to deliver requests.
    RepairingOlm,
    /// Waiting for the key (sender re-sharing, including #477).
    WaitingForKey,
    /// The key was received/imported.
    KeyReceived,
    /// Retrying event-cache/visible-timeline decryption.
    RetryingDecryption,
    /// The visible timeline was updated with decrypted events.
    Recovered,
    /// A retryable failure; a bounded retry may run.
    TemporarilyFailed,
    /// All automatic paths were tried without success.
    AutomaticPathsExhausted,
    /// No known holder of the key can remain (e.g., rotated historical
    /// session with no device or backup).
    UnrecoverableNoKnownHolder,
    /// The operation was cancelled.
    Cancelled,
}

/// Closed outcome of each recovery step, mapped into privacy-safe diagnostics.
///
/// The current actor emits the outcomes needed by its active automatic paths;
/// the remaining outcomes are retained for the same state-machine contract's
/// terminal, retry, and integration paths.
#[allow(dead_code)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RecoveryStepOutcome {
    LocalFound,
    LocalAbsent,
    BackupImported,
    BackupAbsent,
    BackupUnavailable,
    BackupUntrusted,
    BackupTransportFailed,
    OwnDeviceRequestQueued,
    OwnDeviceRequestFailed,
    OwnDeviceNoVerifiedDevices,
    /// The standard Olm unwedge work (one-time-key claim / m.dummy flush) was
    /// dispatched.
    OlmRepairFlushed,
    KeyArrived,
    RedecryptionRequested,
    RedecryptionVerified,
    RedecryptionStillUtd,
    RedecryptionFailed,
    TerminalExhausted,
    TerminalUnrecoverable,
    Cancelled,
}

/// Terminal guidance presented to the user (rendered by React, decided by Rust).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RecoveryGuidance {
    /// Another own verified device may hold the key.
    AnotherOwnDevice,
    /// The original key cannot be recovered; ask the sender to repost.
    AskSenderToRepost,
}

/// An active recovery operation for one (room, session) pair.
#[derive(Debug)]
pub struct RecoveryOperation {
    /// In-memory ordinal alias for this operation's session (never exported).
    session_alias: u64,
    stage: RecoveryStage,
    attempts: u32,
}

/// Minimal safe record persisted across restarts (issue #478): only the
/// attempt count and closed stage token per session, so a bounded retry
/// resumes without duplicate requests. No identifiers or key material.
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct RecoveryResumeRecord {
    /// Attempts already consumed by the automatic sequence.
    pub attempts: u32,
    /// Closed stage token.
    pub stage: RecoveryStage,
}

impl RecoveryOperation {
    pub fn new(session_alias: u64) -> Self {
        Self {
            session_alias,
            stage: RecoveryStage::Detected,
            attempts: 0,
        }
    }

    pub fn stage(&self) -> RecoveryStage {
        self.stage
    }

    pub fn attempts(&self) -> u32 {
        self.attempts
    }

    /// Resume from a persisted record (issue #478): pre-seed the attempt count
    /// and stage so a restart continues the bounded sequence instead of
    /// starting over.
    pub fn resume(&mut self, record: RecoveryResumeRecord) {
        self.attempts = record.attempts.min(MAX_RECOVERY_ATTEMPTS);
        self.stage = record.stage;
    }

    /// The minimal safe resume record for this operation.
    pub fn resume_record(&self) -> RecoveryResumeRecord {
        RecoveryResumeRecord {
            attempts: self.attempts,
            stage: self.stage,
        }
    }

    /// Advance to the next automatic step. Returns false when a manual retry
    /// is required (attempts exhausted) or the operation is terminal.
    pub fn begin_attempt(&mut self) -> bool {
        if self.stage.is_terminal() {
            return false;
        }
        if self.attempts >= MAX_RECOVERY_ATTEMPTS {
            self.stage = RecoveryStage::AutomaticPathsExhausted;
            return false;
        }
        self.attempts += 1;
        self.stage = RecoveryStage::CheckingLocal;
        record_recovery_stage(self.session_alias, self.stage, self.attempts);
        true
    }

    /// Record a step outcome and compute the next stage.
    pub fn observe(&mut self, outcome: RecoveryStepOutcome) -> RecoveryStage {
        self.stage = match (self.stage, outcome) {
            (RecoveryStage::CheckingLocal, RecoveryStepOutcome::LocalFound) => {
                RecoveryStage::KeyReceived
            }
            (RecoveryStage::CheckingLocal, RecoveryStepOutcome::LocalAbsent) => {
                RecoveryStage::CheckingBackup
            }
            (RecoveryStage::CheckingBackup, RecoveryStepOutcome::BackupImported) => {
                RecoveryStage::KeyReceived
            }
            (RecoveryStage::CheckingBackup, RecoveryStepOutcome::BackupAbsent) => {
                RecoveryStage::RequestingOwnDevices
            }
            (RecoveryStage::CheckingBackup, RecoveryStepOutcome::BackupUnavailable)
            | (RecoveryStage::CheckingBackup, RecoveryStepOutcome::BackupUntrusted)
            | (RecoveryStage::CheckingBackup, RecoveryStepOutcome::BackupTransportFailed) => {
                RecoveryStage::RequestingOwnDevices
            }
            (RecoveryStage::RequestingOwnDevices, RecoveryStepOutcome::OwnDeviceRequestQueued) => {
                RecoveryStage::RepairingOlm
            }
            (
                RecoveryStage::RequestingOwnDevices,
                RecoveryStepOutcome::OwnDeviceNoVerifiedDevices,
            ) => RecoveryStage::AutomaticPathsExhausted,
            (RecoveryStage::RequestingOwnDevices, RecoveryStepOutcome::OwnDeviceRequestFailed) => {
                RecoveryStage::TemporarilyFailed
            }
            (RecoveryStage::WaitingForKey, RecoveryStepOutcome::KeyArrived) => {
                RecoveryStage::RetryingDecryption
            }
            (RecoveryStage::RepairingOlm, RecoveryStepOutcome::OlmRepairFlushed) => {
                RecoveryStage::WaitingForKey
            }
            // A waiting tick without the key is retryable and bounded by the
            // attempt limit.
            (RecoveryStage::WaitingForKey, RecoveryStepOutcome::OwnDeviceRequestFailed) => {
                RecoveryStage::TemporarilyFailed
            }
            (RecoveryStage::KeyReceived, RecoveryStepOutcome::RedecryptionVerified)
            | (RecoveryStage::RetryingDecryption, RecoveryStepOutcome::RedecryptionVerified) => {
                RecoveryStage::Recovered
            }
            (RecoveryStage::KeyReceived, RecoveryStepOutcome::RedecryptionStillUtd)
            | (RecoveryStage::KeyReceived, RecoveryStepOutcome::RedecryptionFailed)
            | (RecoveryStage::RetryingDecryption, RecoveryStepOutcome::RedecryptionStillUtd)
            | (RecoveryStage::RetryingDecryption, RecoveryStepOutcome::RedecryptionFailed) => {
                // The key is stored but the timeline is stale: bounded local
                // retry only, never another network request.
                RecoveryStage::TemporarilyFailed
            }
            (RecoveryStage::KeyReceived, _) | (RecoveryStage::RetryingDecryption, _) => {
                RecoveryStage::RetryingDecryption
            }
            (_, RecoveryStepOutcome::TerminalExhausted) => RecoveryStage::AutomaticPathsExhausted,
            (_, RecoveryStepOutcome::TerminalUnrecoverable) => {
                RecoveryStage::UnrecoverableNoKnownHolder
            }
            (_, RecoveryStepOutcome::Cancelled) => RecoveryStage::Cancelled,
            (stage, _) => stage,
        };
        record_recovery_stage(self.session_alias, self.stage, self.attempts);
        self.stage
    }

    /// Guidance for terminal stages, or `None` while recovery can still
    /// progress.
    pub fn guidance(&self) -> Option<RecoveryGuidance> {
        match self.stage {
            RecoveryStage::AutomaticPathsExhausted => Some(RecoveryGuidance::AnotherOwnDevice),
            RecoveryStage::UnrecoverableNoKnownHolder => Some(RecoveryGuidance::AskSenderToRepost),
            _ => None,
        }
    }

    pub fn is_terminal(&self) -> bool {
        matches!(
            self.stage,
            RecoveryStage::Recovered
                | RecoveryStage::AutomaticPathsExhausted
                | RecoveryStage::UnrecoverableNoKnownHolder
                | RecoveryStage::Cancelled
        )
    }
}

impl RecoveryStage {
    fn is_terminal(self) -> bool {
        matches!(
            self,
            RecoveryStage::Recovered
                | RecoveryStage::AutomaticPathsExhausted
                | RecoveryStage::UnrecoverableNoKnownHolder
                | RecoveryStage::Cancelled
        )
    }
}

pub fn stage_token(stage: RecoveryStage) -> &'static str {
    match stage {
        RecoveryStage::Detected => "detected",
        RecoveryStage::CheckingLocal => "checking_local",
        RecoveryStage::CheckingBackup => "checking_backup",
        RecoveryStage::RequestingOwnDevices => "requesting_own_devices",
        RecoveryStage::RepairingOlm => "repairing_olm",
        RecoveryStage::WaitingForKey => "waiting_for_key",
        RecoveryStage::KeyReceived => "key_received",
        RecoveryStage::RetryingDecryption => "retrying_decryption",
        RecoveryStage::Recovered => "recovered",
        RecoveryStage::TemporarilyFailed => "temporarily_failed",
        RecoveryStage::AutomaticPathsExhausted => "automatic_paths_exhausted",
        RecoveryStage::UnrecoverableNoKnownHolder => "unrecoverable_no_known_holder",
        RecoveryStage::Cancelled => "cancelled",
    }
}

fn record_recovery_stage(session_alias: u64, stage: RecoveryStage, attempts: u32) {
    record(
        DiagnosticEvent::new(DiagnosticLevel::Info, "core.room_key_recovery", "stage")
            .field(DiagnosticField::ordinal_alias(
                "session_alias",
                "session",
                session_alias,
            ))
            .field(DiagnosticField::token("stage", stage_token(stage)))
            .field(DiagnosticField::count("attempts", attempts as u64)),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_found_skips_network_work() {
        let mut op = RecoveryOperation::new(1);
        assert!(op.begin_attempt());
        assert_eq!(
            op.observe(RecoveryStepOutcome::LocalFound),
            RecoveryStage::KeyReceived
        );
        assert_eq!(
            op.observe(RecoveryStepOutcome::RedecryptionVerified),
            RecoveryStage::Recovered
        );
        assert!(op.is_terminal());
    }

    #[test]
    fn backup_import_late_decrypts() {
        let mut op = RecoveryOperation::new(2);
        op.begin_attempt();
        assert_eq!(
            op.observe(RecoveryStepOutcome::LocalAbsent),
            RecoveryStage::CheckingBackup
        );
        assert_eq!(
            op.observe(RecoveryStepOutcome::BackupImported),
            RecoveryStage::KeyReceived
        );
        assert_eq!(
            op.observe(RecoveryStepOutcome::RedecryptionVerified),
            RecoveryStage::Recovered
        );
    }

    #[test]
    fn backup_absence_falls_through_to_one_own_device_request() {
        let mut op = RecoveryOperation::new(3);
        op.begin_attempt();
        op.observe(RecoveryStepOutcome::LocalAbsent);
        assert_eq!(
            op.observe(RecoveryStepOutcome::BackupAbsent),
            RecoveryStage::RequestingOwnDevices
        );
        assert_eq!(
            op.observe(RecoveryStepOutcome::OwnDeviceRequestQueued),
            RecoveryStage::RepairingOlm
        );
        assert_eq!(
            op.observe(RecoveryStepOutcome::OlmRepairFlushed),
            RecoveryStage::WaitingForKey
        );
        assert_eq!(
            op.observe(RecoveryStepOutcome::KeyArrived),
            RecoveryStage::RetryingDecryption
        );
        assert_eq!(
            op.observe(RecoveryStepOutcome::RedecryptionVerified),
            RecoveryStage::Recovered
        );
    }

    #[test]
    fn no_verified_device_produces_actionable_exhausted_state() {
        let mut op = RecoveryOperation::new(4);
        op.begin_attempt();
        op.observe(RecoveryStepOutcome::LocalAbsent);
        op.observe(RecoveryStepOutcome::BackupAbsent);
        assert_eq!(
            op.observe(RecoveryStepOutcome::OwnDeviceNoVerifiedDevices),
            RecoveryStage::AutomaticPathsExhausted
        );
        assert_eq!(op.guidance(), Some(RecoveryGuidance::AnotherOwnDevice));
        assert!(op.is_terminal());
    }

    #[test]
    fn attempts_are_bounded_then_exhausted() {
        let mut op = RecoveryOperation::new(5);
        for _ in 0..MAX_RECOVERY_ATTEMPTS {
            assert!(op.begin_attempt());
            op.observe(RecoveryStepOutcome::LocalAbsent);
            op.observe(RecoveryStepOutcome::BackupAbsent);
            assert_eq!(
                op.observe(RecoveryStepOutcome::OwnDeviceRequestFailed),
                RecoveryStage::TemporarilyFailed
            );
        }
        assert!(!op.begin_attempt(), "attempts must be bounded");
        assert_eq!(op.stage(), RecoveryStage::AutomaticPathsExhausted);
    }

    #[test]
    fn stored_key_with_stale_timeline_retries_locally_only() {
        let mut op = RecoveryOperation::new(6);
        op.begin_attempt();
        op.observe(RecoveryStepOutcome::LocalAbsent);
        op.observe(RecoveryStepOutcome::BackupImported);
        op.observe(RecoveryStepOutcome::RedecryptionStillUtd);
        // Key received + stale timeline: retryable, never a new request.
        assert_eq!(op.stage(), RecoveryStage::TemporarilyFailed);
        assert!(!op.is_terminal());
    }

    #[test]
    fn unrecoverable_asks_sender_to_repost() {
        let mut op = RecoveryOperation::new(7);
        op.begin_attempt();
        assert_eq!(
            op.observe(RecoveryStepOutcome::TerminalUnrecoverable),
            RecoveryStage::UnrecoverableNoKnownHolder
        );
        assert_eq!(op.guidance(), Some(RecoveryGuidance::AskSenderToRepost));
    }

    #[test]
    fn resume_pre_seeds_attempts_and_keeps_the_bound() {
        // Simulate a restart: an operation had already consumed all attempts
        // before the process stopped. Resuming must NOT start a fresh sequence
        // that would issue duplicate requests.
        let mut op = RecoveryOperation::new(1);
        let record = RecoveryResumeRecord {
            attempts: MAX_RECOVERY_ATTEMPTS,
            stage: RecoveryStage::TemporarilyFailed,
        };
        op.resume(record);
        assert_eq!(op.attempts(), MAX_RECOVERY_ATTEMPTS);
        assert!(
            !op.begin_attempt(),
            "resumed operation must stay within the bound"
        );
        assert_eq!(op.stage(), RecoveryStage::AutomaticPathsExhausted);
    }

    #[test]
    fn resume_record_round_trips_serde_without_identifiers() {
        let record = RecoveryResumeRecord {
            attempts: 2,
            stage: RecoveryStage::WaitingForKey,
        };
        let encoded = serde_json::to_vec(&record).unwrap();
        let decoded: RecoveryResumeRecord = serde_json::from_slice(&encoded).unwrap();
        assert_eq!(decoded, record);
        let text = String::from_utf8(encoded).unwrap();
        for private in ["@", "!", "http", "session_id", "room_id", "device_id"] {
            assert!(
                !text.contains(private),
                "{private} leaked into resume record: {text}"
            );
        }
    }

    #[test]
    fn own_device_request_queued_flows_through_olm_repair() {
        let mut op = RecoveryOperation::new(2);
        op.begin_attempt();
        op.observe(RecoveryStepOutcome::LocalAbsent);
        op.observe(RecoveryStepOutcome::BackupAbsent);
        assert_eq!(
            op.observe(RecoveryStepOutcome::OwnDeviceRequestQueued),
            RecoveryStage::RepairingOlm
        );
        assert_eq!(
            op.observe(RecoveryStepOutcome::OlmRepairFlushed),
            RecoveryStage::WaitingForKey
        );
    }

    #[test]
    fn stage_records_are_privacy_safe() {
        let _diagnostic_lock = koushi_diagnostics::test_support::lock();
        let mut op = RecoveryOperation::new(99);
        op.begin_attempt();
        op.observe(RecoveryStepOutcome::BackupImported);
        let snapshot = koushi_diagnostics::snapshot();
        let text = format!(
            "{:?}",
            snapshot
                .records
                .iter()
                .filter(|r| r.event.source == "core.room_key_recovery")
                .collect::<Vec<_>>()
        );
        for private in ["@", "!", "example", "http", "PRIVATE", "s3cr3t"] {
            assert!(!text.contains(private), "{private} leaked: {text}");
        }
    }
}

/// Record a settled (terminal) recovery outcome for the diagnostic export.
pub fn record_recovery_settled(stage: RecoveryStage) {
    record(
        DiagnosticEvent::new(DiagnosticLevel::Info, "core.room_key_recovery", "settled")
            .field(DiagnosticField::token("stage", stage_token(stage))),
    );
}

/// Closed token for terminal guidance (issue #478), rendered by React.
pub fn guidance_token(guidance: RecoveryGuidance) -> &'static str {
    match guidance {
        RecoveryGuidance::AnotherOwnDevice => "another_own_device",
        RecoveryGuidance::AskSenderToRepost => "ask_sender_to_repost",
    }
}
