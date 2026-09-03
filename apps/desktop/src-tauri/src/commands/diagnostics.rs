use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager, State};

use crate::{
    CoreRuntimeState,
    viewport_sync::{
        ViewportSyncObservation, ViewportSyncReceipt, record_diagnostic, synchronize_now,
        validate_observation,
    },
};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FrontendDiagnosticLogEntry {
    timestamp_ms: u64,
    source: &'static str,
    message: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FrontendDiagnosticLogSnapshot {
    entries: Vec<FrontendDiagnosticLogEntry>,
    dropped_entries: u64,
    sliding_sync: koushi_core::SlidingSyncDiagnosticsSnapshot,
}

fn map_snapshot(
    snapshot: koushi_diagnostics::DiagnosticSnapshot,
    sliding_sync: koushi_core::SlidingSyncDiagnosticsSnapshot,
) -> FrontendDiagnosticLogSnapshot {
    FrontendDiagnosticLogSnapshot {
        entries: snapshot
            .records
            .into_iter()
            .map(|record| FrontendDiagnosticLogEntry {
                timestamp_ms: record.timestamp_ms,
                source: record.event.source,
                message: koushi_diagnostics::format_event(&record.event),
            })
            .collect(),
        dropped_entries: snapshot.dropped_records,
        sliding_sync,
    }
}

fn snapshot_with_media_memory_summaries(
    thumbnail_stats: koushi_core::renderable_thumbnail::RenderableThumbnailCacheStats,
    media_stats: koushi_core::media_preparation::MediaPreparationStats,
    sliding_sync: koushi_core::SlidingSyncDiagnosticsSnapshot,
) -> FrontendDiagnosticLogSnapshot {
    koushi_core::renderable_thumbnail::record_renderable_thumbnail_summary(thumbnail_stats);
    koushi_core::media_preparation::record_media_preparation_summary(media_stats);
    map_snapshot(koushi_diagnostics::snapshot(), sliding_sync)
}

#[tauri::command]
pub async fn get_diagnostic_snapshot(
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendDiagnosticLogSnapshot, String> {
    Ok(snapshot_with_media_memory_summaries(
        koushi_core::renderable_thumbnail::renderable_thumbnail_cache_stats(),
        state.runtime.media_preparation().stats().await,
        state.runtime.sliding_sync_diagnostics(),
    ))
}

#[tauri::command]
pub async fn observe_viewport_sync(
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
    observation: ViewportSyncObservation,
) -> Result<ViewportSyncReceipt, String> {
    validate_observation(&observation).map_err(|error| error.to_string())?;
    let Some(window) = app.get_webview_window("main") else {
        return Err("main webview is unavailable".to_owned());
    };
    let receipt =
        synchronize_now(window, &state.viewport_sync_generation, observation.trigger).await?;
    let receipt = receipt.with_dom_observation(&observation);
    record_diagnostic(&receipt, Some(&observation));
    super::update_qa_window_title_from_viewport_receipt(&app, state.inner(), &receipt).await;
    Ok(receipt)
}

#[cfg(any(debug_assertions, test))]
use koushi_state::{AuthSecret, DisplayPlatform, LoginRequest};
#[cfg(any(debug_assertions, test))]
use std::path::PathBuf;
#[cfg(any(debug_assertions, test))]
use tauri::Emitter;

// ---- QA login pipe (debug/test only) ----

#[cfg(any(debug_assertions, test))]
#[derive(Deserialize)]
struct QaLoginPipePayload {
    homeserver: String,
    username: String,
    password: String,
    device_display_name: Option<String>,
    recovery_secret: Option<String>,
}

#[cfg(any(debug_assertions, test))]
#[derive(Debug)]
pub(crate) struct QaLoginPipeRequest {
    pub login: LoginRequest,
    pub recovery_secret: Option<AuthSecret>,
}

#[cfg(any(debug_assertions, test))]
pub(crate) fn parse_qa_login_pipe_payload(payload: &str) -> Result<QaLoginPipeRequest, String> {
    let payload: QaLoginPipePayload =
        serde_json::from_str(payload).map_err(|_| "QA login payload was invalid".to_owned())?;
    if payload.homeserver.trim().is_empty()
        || payload.username.trim().is_empty()
        || payload.password.is_empty()
    {
        return Err("QA login payload was incomplete".to_owned());
    }

    Ok(QaLoginPipeRequest {
        login: LoginRequest {
            homeserver: payload.homeserver,
            username: payload.username,
            password: AuthSecret::new(payload.password),
            device_display_name: payload.device_display_name,
        },
        recovery_secret: payload
            .recovery_secret
            .filter(|secret| !secret.trim().is_empty())
            .map(AuthSecret::new),
    })
}

#[cfg(any(debug_assertions, test))]
pub(crate) fn spawn_qa_login_pipe_reader(app: AppHandle, pipe_path: PathBuf) {
    tauri::async_runtime::spawn(async move {
        let payload = match read_qa_login_pipe(pipe_path).await {
            Ok(payload) => payload,
            Err(message) => {
                record_qa_login_failure(&app, &message).await;
                return;
            }
        };
        let request = match parse_qa_login_pipe_payload(&payload) {
            Ok(request) => request,
            Err(message) => {
                record_qa_login_failure(&app, &message).await;
                return;
            }
        };
        let state = app.state::<CoreRuntimeState>();
        if let Err(message) = super::session::submit_login_request(
            app.clone(),
            state.inner(),
            request.login,
            DisplayPlatform::Linux,
        )
        .await
        {
            record_qa_login_failure(&app, &message).await;
            return;
        }
        if let Some(recovery_secret) = request.recovery_secret {
            let state = app.state::<CoreRuntimeState>();
            if let Err(message) =
                wait_for_qa_recovery_prompt(&app, state.inner(), QA_RECOVERY_PROMPT_TIMEOUT).await
            {
                record_qa_login_failure(&app, &message).await;
                return;
            }
            let state = app.state::<CoreRuntimeState>();
            if let Err(message) =
                super::session::submit_recovery_request(app.clone(), state.inner(), recovery_secret)
                    .await
            {
                record_qa_login_failure(&app, &message).await;
            }
        }
    });
}

#[cfg(any(debug_assertions, test))]
async fn read_qa_login_pipe(pipe_path: PathBuf) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        std::fs::read_to_string(pipe_path).map_err(|_| "QA login pipe could not be read".to_owned())
    })
    .await
    .map_err(|_| "QA login pipe reader failed".to_owned())?
}

#[cfg(any(debug_assertions, test))]
async fn record_qa_login_failure(app: &AppHandle, message: &str) {
    // Emit a QA title update so the harness sees `session=signedOut`.
    let state = app.state::<CoreRuntimeState>();
    super::update_qa_window_title_from_state(app, state.inner()).await;
    // Also emit a discrete error event.
    let _ = app.emit(
        crate::CORE_EVENT_NAME,
        serde_json::json!({
            "kind": "OperationFailed",
            "request_id": null,
            "failure": { "kind": "LoginFailed", "message": message },
        }),
    );
}

#[cfg(any(debug_assertions, test))]
async fn wait_for_qa_recovery_prompt(
    app: &AppHandle,
    state: &CoreRuntimeState,
    timeout: std::time::Duration,
) -> Result<(), String> {
    let started_at = std::time::Instant::now();
    while started_at.elapsed() < timeout {
        let snapshot = state.connection.lock().await.snapshot();
        if qa_recovery_prompt_is_available(&snapshot) {
            super::update_qa_window_title_from_state(app, state).await;
            return Ok(());
        }
        super::update_qa_window_title_from_state(app, state).await;
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }
    Err("QA recovery prompt did not become available".to_owned())
}

#[cfg(any(debug_assertions, test))]
pub(super) fn qa_recovery_prompt_is_available(state: &koushi_state::AppState) -> bool {
    matches!(
        state.session,
        koushi_state::SessionState::AwaitingVerification { .. }
    )
}

// ---- QA control pipe (debug/test only) ----
//
// A newline-delimited JSON control channel that lets unattended GUI smoke drive
// a clean logout after a real login, so no stale device survives the run. This
// mirrors the QA login pipe: it carries no secrets, only control commands, and
// is gated to debug/test builds (release builds never read the env var).

#[cfg(any(debug_assertions, test))]
#[derive(Deserialize)]
struct QaControlPipeCommand {
    command: String,
}

/// Parsed QA control command. Only logout is supported today; unknown commands
/// are ignored by the reader rather than treated as failures.
#[cfg(any(debug_assertions, test))]
#[derive(Debug, Eq, PartialEq)]
pub(super) enum QaControlCommand {
    Logout,
    Unknown(String),
}

#[cfg(any(debug_assertions, test))]
pub(super) fn parse_qa_control_pipe_line(line: &str) -> Result<QaControlCommand, String> {
    let parsed: QaControlPipeCommand =
        serde_json::from_str(line).map_err(|_| "QA control command was invalid".to_owned())?;
    Ok(match parsed.command.as_str() {
        "logout" => QaControlCommand::Logout,
        other => QaControlCommand::Unknown(other.to_owned()),
    })
}

#[cfg(any(debug_assertions, test))]
pub(crate) fn spawn_qa_control_pipe_reader(app: AppHandle, pipe_path: PathBuf) {
    tauri::async_runtime::spawn(async move {
        let contents = match read_qa_control_pipe(pipe_path).await {
            Ok(contents) => contents,
            Err(message) => {
                record_qa_login_failure(&app, &message).await;
                return;
            }
        };
        for line in contents.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            match parse_qa_control_pipe_line(line) {
                Ok(QaControlCommand::Logout) => {
                    let state = app.state::<CoreRuntimeState>();
                    let request_id = super::next_request_id(state.inner()).await;
                    if let Err(message) = super::submit_core_command(
                        state.inner(),
                        super::session::build_logout_command(request_id),
                    )
                    .await
                    {
                        record_qa_login_failure(&app, &message).await;
                        continue;
                    }
                    // Surface the post-logout state in the QA window title so the
                    // smoke harness can wait for `session=signedOut`.
                    super::update_qa_window_title_from_state(&app, state.inner()).await;
                }
                Ok(QaControlCommand::Unknown(_)) => {
                    // Forward-compatible: ignore commands we do not recognise.
                }
                Err(message) => {
                    record_qa_login_failure(&app, &message).await;
                }
            }
        }
    });
}

#[cfg(any(debug_assertions, test))]
async fn read_qa_control_pipe(pipe_path: PathBuf) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        std::fs::read_to_string(pipe_path)
            .map_err(|_| "QA control pipe could not be read".to_owned())
    })
    .await
    .map_err(|_| "QA control pipe reader failed".to_owned())?
}

#[cfg(any(debug_assertions, test))]
const QA_RECOVERY_PROMPT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);

#[cfg(test)]
mod qa_tests {
    use koushi_protocol::{AccountCommand, CoreCommand};
    use koushi_state::{AppState, RoomSummary, RoomTags, SessionInfo, SessionState};

    #[test]
    fn qa_login_pipe_payload_maps_to_login_request_without_debugging_secret() {
        let request = super::parse_qa_login_pipe_payload(
            r#"{"homeserver":"https://matrix.example.org","username":"fixture-user","password":"synthetic-password","device_display_name":"Koushi GUI Smoke","recovery_secret":"synthetic-recovery-secret"}"#,
        )
        .expect("payload should parse");

        assert_eq!(request.login.homeserver, "https://matrix.example.org");
        assert_eq!(request.login.username, "fixture-user");
        assert_eq!(request.login.password.expose_secret(), "synthetic-password");
        assert_eq!(
            request.login.device_display_name.as_deref(),
            Some("Koushi GUI Smoke")
        );
        assert_eq!(
            request
                .recovery_secret
                .as_ref()
                .map(|secret| secret.expose_secret()),
            Some("synthetic-recovery-secret")
        );
        assert!(!format!("{request:?}").contains("synthetic-password"));
        assert!(!format!("{request:?}").contains("synthetic-recovery-secret"));
    }

    #[test]
    fn qa_control_pipe_line_parses_logout_and_ignores_unknown_commands() {
        assert_eq!(
            super::parse_qa_control_pipe_line(r#"{"command":"logout"}"#)
                .expect("logout should parse"),
            super::QaControlCommand::Logout
        );
        assert_eq!(
            super::parse_qa_control_pipe_line(r#"{"command":"focus"}"#)
                .expect("unknown should parse"),
            super::QaControlCommand::Unknown("focus".to_owned())
        );
        assert!(super::parse_qa_control_pipe_line("not json").is_err());
    }

    #[test]
    fn qa_control_logout_builds_account_logout_command() {
        // The control pipe must reuse the same logout core command the manual
        // logout button submits — no bespoke logout path.
        match crate::commands::session::build_logout_command(
            crate::commands::contracts::fake_request_id(99),
        ) {
            CoreCommand::Account(AccountCommand::Logout { request_id }) => {
                assert_eq!(request_id, crate::commands::contracts::fake_request_id(99));
            }
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn qa_recovery_prompt_available_iff_needs_recovery() {
        let mut state = AppState::default();
        assert!(!super::qa_recovery_prompt_is_available(&state));

        state.session = SessionState::AwaitingVerification {
            info: SessionInfo {
                homeserver: "https://matrix.example.org".to_owned(),
                user_id: "@user:example.org".to_owned(),
                device_id: "DEVICE".to_owned(),
                authentication_method: koushi_state::SessionAuthenticationMethod::Unknown,
            },
            gate: koushi_state::VerificationGateState {
                methods: vec![],
                account_kind: koushi_state::VerificationAccountKind::Unknown,
                failure: None,
            },
        };
        assert!(super::qa_recovery_prompt_is_available(&state));
    }

    #[test]
    fn qa_window_title_reflects_session_sync_room_and_timeline_counts() {
        let mut snapshot = AppState::default();
        snapshot.rooms = vec![
            RoomSummary {
                room_id: "!room1:example.org".to_owned(),
                display_name: "Room 1".to_owned(),
                display_label: "Room 1".to_owned(),
                original_display_label: "Room 1".to_owned(),
                avatar: None,
                is_dm: false,
                dm_user_ids: Vec::new(),
                tags: RoomTags::default(),
                unread_count: 0,
                notification_count: 0,
                highlight_count: 0,
                marked_unread: false,
                recency_stamp: None,
                conversation_activity: None,
                latest_event: None,
                parent_space_ids: vec![],
                dm_space_ids: vec![],
                is_encrypted: false,
                joined_members: 0,
            },
            RoomSummary {
                room_id: "!room2:example.org".to_owned(),
                display_name: "Room 2".to_owned(),
                display_label: "Room 2".to_owned(),
                original_display_label: "Room 2".to_owned(),
                avatar: None,
                is_dm: false,
                dm_user_ids: Vec::new(),
                tags: RoomTags::default(),
                unread_count: 0,
                notification_count: 0,
                highlight_count: 0,
                marked_unread: false,
                recency_stamp: None,
                conversation_activity: None,
                latest_event: None,
                parent_space_ids: vec![],
                dm_space_ids: vec![],
                is_encrypted: false,
                joined_members: 0,
            },
        ];

        let title = crate::commands::qa_window_title_string(&snapshot, 42);

        assert!(title.contains("session=signedOut"));
        assert!(title.contains("sync=stopped"));
        assert!(title.contains("rooms=2"));
        assert!(title.contains("timeline_items=42"));
    }

    #[test]
    fn viewport_qa_title_tokens_follow_the_rust_receipt() {
        let snapshot = AppState::default();
        let receipt = crate::viewport_sync::ViewportSyncReceipt {
            generation: 17,
            trigger: crate::viewport_sync::ViewportSyncTrigger::Resized,
            density: Some(crate::viewport_sync::ViewportDensity::Comfortable),
            native_support: crate::viewport_sync::NativeViewportSupport::Supported,
            decision: crate::viewport_sync::ViewportSyncDecision::RepairToParentBounds,
            native_aligned: true,
            native_origin_aligned: true,
            native_size_aligned: true,
            dom_aligned: true,
            dom_js_aligned: true,
            dom_root_aligned: true,
            parent: Some(crate::viewport_sync::ViewportRect {
                top: 0.0,
                left: 0.0,
                width: 1200.0,
                height: 800.0,
            }),
            webview: None,
        };

        let title = crate::commands::qa_window_title_with_viewport_receipt(&snapshot, 0, &receipt);

        assert!(title.contains("viewport=aligned"));
        assert!(title.contains("viewport_generation=17"));
        assert!(title.contains("viewport_parent=true"));
        assert!(title.contains("viewport_webview=true"));
        assert!(title.contains("viewport_js=true"));
        assert!(title.contains("viewport_root=true"));
        assert!(title.contains("viewport_decision=repair_to_parent_bounds"));
    }
}

#[cfg(test)]
mod snapshot_tests {
    use super::*;
    #[test]
    fn diagnostic_snapshot_maps_structured_snapshot_to_camel_case_frontend_contract() {
        let snapshot = koushi_diagnostics::DiagnosticSnapshot {
            records: vec![koushi_diagnostics::DiagnosticRecord {
                timestamp_ms: 42,
                event: koushi_diagnostics::DiagnosticEvent::new(
                    koushi_diagnostics::DiagnosticLevel::Debug,
                    "desktop.timeline",
                    "submit",
                )
                .field(koushi_diagnostics::DiagnosticField::token(
                    "operation",
                    "send_reaction",
                )),
            }],
            dropped_records: 7,
        };
        let json = serde_json::to_value(map_snapshot(
            snapshot,
            koushi_core::SlidingSyncDiagnosticsSnapshot::default(),
        ))
        .unwrap();
        assert_eq!(
            json,
            serde_json::json!({
                "entries": [{
                    "timestampMs": 42,
                    "source": "desktop.timeline",
                    "message": "stage=submit operation=send_reaction"
                }],
                "droppedEntries": 7,
                "slidingSync": {
                    "discoveryState": "not_started",
                    "advertised": false,
                    "discoverySource": "unknown",
                    "lastProbeAgeBucket": "never",
                    "lastHttpStatusClass": "unknown",
                    "requestSchema": "element_x_all_rooms",
                    "engine": "SyncService",
                    "sdkSlidingSyncVersion": "unknown",
                    "roomListSharePos": true,
                    "encryptionSharePos": false,
                    "encryptionConnectionProfile": "sdk_default_encryption",
                    "encryptionExtensionProfile": "e2ee_to_device",
                    "provisionalEncryptionStarted": false,
                    "provisionalFirstResponseSeen": false,
                    "provisionalStoppedBeforeFirstResponse": false,
                    "provisionalToNormalHandoffBucket": "never",
                    "lifecycle": "stopped",
                    "connectivityProven": false,
                    "committedGeneration": 0,
                    "lastSuccessAgeBucket": "never",
                    "consecutiveFailureCount": 0,
                    "lastFailureOrigin": "none",
                    "lastFailureKind": "none",
                    "lastFailureStage": "none",
                    "lastHttpErrorSource": "none",
                    "lastHttpStatus": "none",
                    "lastMatrixErrorKind": "none",
                    "lastFailureRetryability": "none",
                    "roomListTaskRunning": false,
                    "encryptionTaskRunning": false,
                    "posPresent": false,
                    "directAccountDataSource": "unavailable",
                    "directMappedRoomCount": 0,
                    "directTargetCount": 0,
                    "projectedDmCount": 0,
                    "explicitDmCount": 0,
                    "fallbackDmCount": 0,
                    "directNonDmCount": 0,
                    "directInvalidEntryCount": 0,
                    "directEventWakeCount": 0,
                    "directEventAppliedCount": 0,
                    "directEventStreamRunning": false
                }
            })
        );
    }
    #[test]
    fn diagnostic_snapshot_serialization_excludes_synthetic_private_values() {
        let snapshot = koushi_diagnostics::DiagnosticSnapshot {
            records: vec![koushi_diagnostics::DiagnosticRecord {
                timestamp_ms: 42,
                event: koushi_diagnostics::DiagnosticEvent::new(
                    koushi_diagnostics::DiagnosticLevel::Debug,
                    "desktop.search",
                    "submit",
                )
                .field(koushi_diagnostics::DiagnosticField::count(
                    "query_bytes",
                    23,
                ))
                .field(koushi_diagnostics::DiagnosticField::count(
                    "query_chars",
                    17,
                )),
            }],
            dropped_records: 0,
        };
        let serialized = serde_json::to_string(&map_snapshot(
            snapshot,
            koushi_core::SlidingSyncDiagnosticsSnapshot::default(),
        ))
        .unwrap();
        for forbidden in [
            "!room:synthetic.invalid",
            "@user:synthetic.invalid",
            "$event:synthetic.invalid",
            "/Users/alice/private",
            "secret message",
            "synthetic search query",
        ] {
            assert!(
                !serialized.contains(forbidden),
                "serialized diagnostics leaked {forbidden}"
            );
        }
        assert!(serialized.contains("query_bytes"));
        assert!(serialized.contains("query_chars"));
    }
    #[test]
    fn diagnostic_snapshot_exports_current_media_memory_summaries() {
        let _guard = koushi_diagnostics::test_support::lock();
        let exported = snapshot_with_media_memory_summaries(
            koushi_core::renderable_thumbnail::RenderableThumbnailCacheStats {
                entry_count: 3,
                retained_bytes: 300,
                high_water_entry_count: 5,
                high_water_bytes: 500,
                eviction_count: 2,
                clear_count: 1,
                oversize_rejection_count: 4,
            },
            koushi_core::media_preparation::MediaPreparationStats {
                source_count: 2,
                source_bytes: 200,
                variant_count: 3,
                source_backed_variant_count: 2,
                variant_bytes: 80,
                selected_count: 2,
                high_water_source_count: 4,
                high_water_source_bytes: 400,
                high_water_variant_count: 6,
                high_water_variant_bytes: 160,
            },
            koushi_core::SlidingSyncDiagnosticsSnapshot::default(),
        );
        let thumbnail = exported
            .entries
            .iter()
            .rev()
            .find(|entry| entry.source == "core.renderable_thumbnail")
            .expect("renderable-thumbnail summary must be exported");
        assert!(thumbnail.message.contains("stage=summary"));
        assert!(thumbnail.message.contains("entry_count=3"));
        let media = exported
            .entries
            .iter()
            .rev()
            .find(|entry| entry.source == "core.media_preparation")
            .expect("media-preparation summary must be exported");
        assert!(media.message.contains("stage=summary"));
        assert!(media.message.contains("source_backed_variant_count=2"));
        let details = koushi_diagnostics::test_support::detail_snapshot();
        let summaries = ["core.renderable_thumbnail", "core.media_preparation"].map(|source| {
            details
                .records
                .iter()
                .rev()
                .find(|record| record.event.source == source && record.event.stage == "summary")
                .expect("media summary diagnostic must be present")
        });
        for record in summaries {
            assert!(record.event.fields.iter().all(|field| matches!(
                field.value,
                koushi_diagnostics::DiagnosticValue::Count(_)
                    | koushi_diagnostics::DiagnosticValue::Token(_)
            )));
        }
        let serialized = serde_json::to_string(&exported).unwrap();
        for forbidden in [
            "!room:synthetic.invalid",
            "@user:synthetic.invalid",
            "$event:synthetic.invalid",
            "/Users/alice/private",
            "secret message",
        ] {
            assert!(!serialized.contains(forbidden));
        }
    }
}
