//! Real homeserver QA binary (Phase 8 — Milestone G).
//!
//! Exercises the full real-homeserver QA scenario against a live Matrix
//! homeserver (matrix.org) using approved test-account credentials stored in
//! `.local-secrets/real-account-qa/credentials.json` (git-ignored, mode 600).
//!
//! ## Secrets protocol (engineering-rules Secrets section)
//!
//! - The credentials file is read by THIS binary only, behind a debug/test
//!   compile-time gate. The file path is passed via env; the path itself is
//!   not a secret.
//! - Passwords and recovery keys are NEVER logged, echoed, printed, or included
//!   in error messages. This binary self-checks its own transcript for those values
//!   before exit and fails if they are found (redaction check).
//! - An unexpected keychain prompt = automation failure. The file credential
//!   store override (KOUSHI_QA_FILE_CREDENTIAL_STORE_DIR) is mandatory.
//! - ABSOLUTE_PROHIBITION: no GUI launch in any form.
//! - Logout cleanup runs even on earlier failures (finally-ish path) so no
//!   stale devices accumulate on the homeserver.
//!
//! ## QA coverage (canon QA Model layer 3)
//!
//! 1. HTTPS login to the homeserver -> pre-sync Ready snapshot (store bootstrap
//!    invariant and reducer gate).
//! 2. Sync lifecycle: Start -> Started -> Running.
//! 3. Recovery is completed during login admission, before starting room sync.
//!    Require the correlated completion events and Ready in either order.
//! 4. Room list: wait non-empty or timeout; print COUNTS ONLY (rooms=N spaces=N dms=N).
//! 5. Create synthetic QA room, subscribe timeline, send edit/redact fixture
//!    messages plus a dedicated search probe, wait SendCompleted + diffs, edit
//!    one, redact the other, paginate backward to EndReached. Only operations
//!    on the QA-created room.
//! 6. Search smoke: query a unique token from the unedited probe message;
//!    assert the QA room/event.
//! 7. Encrypted store restore: stop sync, drop runtime, start fresh runtime over
//!    same data dir, RestoreLastSession -> SessionRestored -> start sync -> Running ->
//!    resubscribe QA room timeline and assert the edited message body arrives.
//! 8. Leave/forget the QA room if a leave primitive is available (checked below).
//! 9. Logout -> SignedOut + post-logout RestoreLastSession = SessionNotFound.
//! 10. Self-check transcript for password/recovery-key leakage.
//!
//! ## Rate limits (matrix.org)
//!
//! - Single login per run. No login/logout cycles.
//! - Bounded retries with backoff on 429.
//! - Logout cleanup MUST run even on failure (no --keep-session).
//!
//! ## Required env
//!
//! - KOUSHI_REAL_QA_CREDENTIALS_PATH - path to the credentials JSON file
//! - KOUSHI_QA_FILE_CREDENTIAL_STORE_DIR - mandatory; see keychain guard
//! - KOUSHI_QA_DATA_DIR (optional) - overrides per-run data dir root

#![allow(dead_code)]

use std::process::ExitCode;

#[cfg(any(debug_assertions, test))]
use koushi_core::runtime::EventStreamLag;
use koushi_core::runtime::{CoreConnection, CoreRuntime};
use koushi_protocol::command::{
    AccountCommand, CoreCommand, CreateRoomOptions, CreateRoomVisibility, RoomCommand,
    SearchCommand, SearchScope, SyncCommand, TimelineCommand,
};
use koushi_protocol::event::{
    AccountEvent, CoreEvent, PaginationDirection, PaginationState, RoomEvent, SearchEvent,
    SyncEvent, TimelineEvent,
};
use koushi_protocol::failure::{CoreFailure, RecoveryFailureKind, TimelineFailureKind};
use koushi_protocol::ids::{AccountKey, RequestId, TimelineKey};
use koushi_state::{
    AppState, AuthSecret, ComposerDocument, LoginRequest, RecoveryRequest, SessionState,
};

fn main() -> ExitCode {
    #[cfg(not(any(debug_assertions, test)))]
    {
        eprintln!("real-homeserver-qa: this binary is only available in debug/test builds");
        return ExitCode::FAILURE;
    }

    #[cfg(any(debug_assertions, test))]
    match run() {
        Ok(summary) => {
            println!("{summary}");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("Real homeserver QA failed: {error}");
            ExitCode::FAILURE
        }
    }
}

// ---------------------------------------------------------------------------
// Sync entry point - loads creds, runs tokio, self-checks transcript
// ---------------------------------------------------------------------------

#[cfg(any(debug_assertions, test))]
fn run() -> Result<String, String> {
    // Hard guard BEFORE credentials are loaded: unattended QA must never
    // touch the OS keychain (a keychain prompt = automation failure).
    assert_file_credential_store_active()?;

    let creds = RealCredentials::load()?;
    let scenario = RealQaScenario::from_env()?;

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|e| format!("Tokio runtime creation failed: {e}"))?;

    let mut transcript: Vec<String> = Vec::new();
    let result = rt.block_on(run_async(&creds, scenario, &mut transcript));

    // Self-check: scan every line of the transcript for the secret values
    // before we emit the summary to stdout.
    let password_str = creds.password.expose_secret();
    let recovery_key_str = creds.recovery_key.expose_secret();
    let combined = transcript.join("\n");

    if combined.contains(password_str) {
        return Err("REDACTION FAILURE: password appears in QA transcript".to_owned());
    }
    if combined.contains(recovery_key_str) {
        return Err("REDACTION FAILURE: recovery_key appears in QA transcript".to_owned());
    }

    result
}

/// Catch-all wrapper around the QA flow. Computes the per-run `data_dir` once,
/// runs the inner flow, and — on ANY failure (including `?`-propagated ones)
/// that did not already reach the final logout — runs a best-effort cleanup
/// pass that leaves/forgets every created room/space and logs out. This is the
/// finally-ish path required by the Secrets/QA canon: no stale device, room, or
/// space may survive a failed run.
#[cfg(any(debug_assertions, test))]
async fn run_async(
    creds: &RealCredentials,
    scenario: RealQaScenario,
    transcript: &mut Vec<String>,
) -> Result<String, String> {
    let data_dir = real_qa_data_dir();
    let mut cleanup = RealQaCleanupState::default();
    let result = if matches!(scenario, RealQaScenario::StartupLatency) {
        startup_latency::run_startup_latency_scenario(creds, &data_dir, transcript, &mut cleanup)
            .await
    } else {
        compat_flow::run_async_inner(creds, scenario, &data_dir, transcript, &mut cleanup).await
    };
    if result.is_err() && !cleanup.logged_out {
        cleanup_real_qa_resources(creds, &data_dir, transcript, &mut cleanup).await;
    }
    result
}

#[cfg(any(debug_assertions, test))]
#[path = "real_homeserver_qa/cleanup.rs"]
mod cleanup;
#[cfg(any(debug_assertions, test))]
#[path = "real_homeserver_qa/compat_flow.rs"]
mod compat_flow;
#[cfg(any(debug_assertions, test))]
#[path = "real_homeserver_qa/config.rs"]
mod config;
#[cfg(any(debug_assertions, test))]
#[path = "real_homeserver_qa/credentials.rs"]
mod credentials;
#[cfg(any(debug_assertions, test))]
#[path = "real_homeserver_qa/event_source.rs"]
mod event_source;
#[cfg(any(debug_assertions, test))]
#[path = "real_homeserver_qa/startup_latency.rs"]
mod startup_latency;
#[cfg(all(test, feature = "qa-bin"))]
#[path = "real_homeserver_qa/tests.rs"]
mod tests;
#[cfg(any(debug_assertions, test))]
#[path = "real_homeserver_qa/waiters.rs"]
mod waiters;

#[cfg(any(debug_assertions, test))]
use cleanup::{RealQaCleanupState, cleanup_real_qa_resources};
#[cfg(any(debug_assertions, test))]
use config::{RealQaScenario, real_qa_data_dir};
#[cfg(any(debug_assertions, test))]
use credentials::{RealCredentials, assert_file_credential_store_active};

#[cfg(any(debug_assertions, test))]
#[path = "real_homeserver_qa/admission.rs"]
mod admission;
