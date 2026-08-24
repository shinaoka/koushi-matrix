use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use tauri::Emitter;

use crate::dto::FrontendDesktopSnapshotDelta;
use koushi_core::{
    CoreCommand, CoreCommandHandle, CoreConnection, CoreEvent, EventStreamLag, SearchEvent,
    TimelineCommand, TimelineEvent, event::AppStateSnapshot,
};
use koushi_diagnostics::{DiagnosticEvent, DiagnosticField, DiagnosticLevel, record};

/// Tauri event for serialized CoreEvent payloads (discrete events + diff batches).
pub(crate) const CORE_EVENT_NAME: &str = "koushi-desktop://event";
/// Tauri event for serialized AppStateSnapshot payloads (latest-wins).
const STATE_EVENT_NAME: &str = "koushi-desktop://state";
const CORE_FORWARDER_TIMELINE_REPLAY_TIMEOUT: Duration = Duration::from_secs(2);
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ForwarderLagDisposition {
    ResyncAndReplay,
    ResyncAndStop,
}
pub(super) struct CoreEventForwarderTask(tauri::async_runtime::JoinHandle<()>);
impl Drop for CoreEventForwarderTask {
    fn drop(&mut self) {
        self.0.abort();
    }
}
#[derive(Clone, Debug, Eq, PartialEq)]
struct ForwardedWebviewEvent {
    event_name: &'static str,
    payload: serde_json::Value,
}
/// Spawn the CoreEvent forwarding task. This task owns a dedicated connection
/// (second `attach()`) so it can loop on `recv_event` without blocking command
/// dispatch.
///
/// On `CoreEvent::StateChanged`: emit `koushi-desktop://state` with the
/// serialized snapshot + update QA window title.
/// On any `CoreEvent`: emit `koushi-desktop://event` with a serialized DTO.
/// On `EventStreamLag`: emit the latest snapshot (resync) + a
/// `ResyncMarker` event so the frontend resets its timeline stores.
fn forwarder_lag_disposition(lag: EventStreamLag) -> ForwarderLagDisposition {
    if lag.skipped == 0 {
        ForwarderLagDisposition::ResyncAndStop
    } else {
        ForwarderLagDisposition::ResyncAndReplay
    }
}
pub(super) fn spawn_core_event_forwarder(
    app: tauri::AppHandle,
    mut event_conn: CoreConnection,
    timeline_items_count: Arc<AtomicUsize>,
) -> CoreEventForwarderTask {
    CoreEventForwarderTask(tauri::async_runtime::spawn(async move {
        loop {
            match event_conn.recv_event().await {
                Ok(event) => {
                    emit_forwarded_webview_events(
                        &app,
                        forwarded_webview_events_for_core_event(&event, &timeline_items_count),
                    );
                }
                Err(lag) => {
                    // Consumer fell behind or the stream closed. Emit the
                    // latest snapshot and marker once before replay or exit.
                    let snapshot = event_conn.snapshot();
                    emit_forwarded_webview_events(
                        &app,
                        forwarded_webview_events_for_lag_resync(&snapshot),
                    );
                    match forwarder_lag_disposition(lag) {
                        ForwarderLagDisposition::ResyncAndReplay => {
                            let command_handle = event_conn.command_handle();
                            let request_id = event_conn.next_request_id();
                            submit_timeline_replay_after_forwarder_lag(command_handle, request_id)
                                .await;
                        }
                        ForwarderLagDisposition::ResyncAndStop => break,
                    }
                }
            }
        }
    }))
}
async fn submit_timeline_replay_after_forwarder_lag(
    command_handle: CoreCommandHandle,
    request_id: koushi_core::RequestId,
) {
    let command = CoreCommand::Timeline(TimelineCommand::ReplaySubscribed { request_id });
    let _ = tokio::time::timeout(
        CORE_FORWARDER_TIMELINE_REPLAY_TIMEOUT,
        command_handle.command(command),
    )
    .await;
}
fn forwarded_webview_events_for_core_event(
    event: &CoreEvent,
    timeline_items_count: &AtomicUsize,
) -> Vec<ForwardedWebviewEvent> {
    let mut forwarded = Vec::new();

    // Track timeline item count for QA window title.
    match event {
        CoreEvent::Timeline(TimelineEvent::InitialItems { items, .. }) => {
            timeline_items_count.store(items.len(), Ordering::Relaxed);
        }
        CoreEvent::Timeline(TimelineEvent::ItemsUpdated { diffs, .. }) => {
            // Apply diff count delta (approximate; exact count tracked by React store)
            let current = timeline_items_count.load(Ordering::Relaxed);
            let delta = diffs_net_count_change(diffs);
            let new_count = (current as i64 + delta).max(0) as usize;
            timeline_items_count.store(new_count, Ordering::Relaxed);
        }
        _ => {}
    }

    if let CoreEvent::StateDelta(delta) = event {
        let requires_snapshot_refresh = delta.changed.session.is_some();
        forwarded.push(ForwardedWebviewEvent {
            event_name: CORE_EVENT_NAME,
            payload: serde_json::json!({
                "kind": "StateDelta",
                "generation": delta.generation,
                "changed": FrontendDesktopSnapshotDelta::from(delta.clone()).changed,
            }),
        });
        if requires_snapshot_refresh {
            forwarded.push(ForwardedWebviewEvent {
                event_name: STATE_EVENT_NAME,
                payload: serde_json::Value::String("stateChanged".to_owned()),
            });
        }
    }

    if let Some(payload) = serialize_core_event(event) {
        forwarded.push(ForwardedWebviewEvent {
            event_name: CORE_EVENT_NAME,
            payload,
        });
    }

    forwarded
}
fn diffs_net_count_change(diffs: &[koushi_core::TimelineDiff]) -> i64 {
    diffs
        .iter()
        .map(|diff| match diff {
            koushi_core::TimelineDiff::PushFront { .. }
            | koushi_core::TimelineDiff::PushBack { .. }
            | koushi_core::TimelineDiff::Insert { .. } => 1_i64,
            koushi_core::TimelineDiff::Remove { .. } => -1_i64,
            koushi_core::TimelineDiff::Truncate { .. }
            | koushi_core::TimelineDiff::Clear
            | koushi_core::TimelineDiff::Reset { .. }
            | koushi_core::TimelineDiff::Set { .. } => 0_i64,
        })
        .sum()
}
fn forwarded_webview_events_for_state_changed(
    _snapshot: &AppStateSnapshot,
) -> Vec<ForwardedWebviewEvent> {
    vec![ForwardedWebviewEvent {
        event_name: STATE_EVENT_NAME,
        payload: serde_json::Value::String("stateChanged".to_owned()),
    }]
}
fn forwarded_webview_events_for_lag_resync(
    snapshot: &AppStateSnapshot,
) -> Vec<ForwardedWebviewEvent> {
    let mut forwarded = forwarded_webview_events_for_state_changed(snapshot);
    forwarded.push(ForwardedWebviewEvent {
        event_name: CORE_EVENT_NAME,
        payload: serde_json::json!({ "kind": "ResyncMarker" }),
    });
    forwarded
}
fn emit_forwarded_webview_events(
    app: &tauri::AppHandle,
    forwarded_events: Vec<ForwardedWebviewEvent>,
) {
    let mut failed = 0_u64;
    for forwarded_event in forwarded_events {
        if app
            .emit(forwarded_event.event_name, forwarded_event.payload)
            .is_err()
        {
            failed = failed.saturating_add(1);
        }
    }
    if failed > 0 {
        record(
            DiagnosticEvent::new(
                DiagnosticLevel::Warn,
                "tauri.transport",
                "webview_emit_failed",
            )
            .field(DiagnosticField::count("events", failed)),
        );
    }
}
/// Serialize a `CoreEvent` to a JSON value for IPC.
///
/// Security: message bodies flow in `Timeline` events. These are visible
/// content (not secret), but we never trace IPC payloads in release.
/// The serialization produces structured JSON only — no raw SDK errors.
fn serialize_core_event(event: &CoreEvent) -> Option<serde_json::Value> {
    Some(match event {
        CoreEvent::StateDelta(_) => {
            return None;
        }
        CoreEvent::StateChanged(_) => {
            // StateChanged snapshots are sent via `koushi-desktop://state`;
            // don't duplicate as a generic event.
            return None;
        }
        CoreEvent::Account(e) => serde_json::json!({ "kind": "Account", "event": e }),
        CoreEvent::Sync(e) => serde_json::json!({ "kind": "Sync", "event": e }),
        CoreEvent::Room(e) => serde_json::json!({ "kind": "Room", "event": e }),
        CoreEvent::Timeline(e) => serde_json::json!({ "kind": "Timeline", "event": e }),
        CoreEvent::LiveSignals(e) => serde_json::json!({ "kind": "LiveSignals", "event": e }),
        CoreEvent::Search(SearchEvent::IndexUpdated { .. }) => {
            // Internal indexer wake-up signal. Forwarding one WebView IPC event
            // per indexed message competes with input and scroll rendering.
            return None;
        }
        CoreEvent::Search(e) => serde_json::json!({ "kind": "Search", "event": e }),
        CoreEvent::E2eeTrust(e) => serde_json::json!({ "kind": "E2eeTrust", "event": e }),
        CoreEvent::Activity(e) => serde_json::json!({ "kind": "Activity", "event": e }),
        CoreEvent::LocalEncryption(e) => {
            serde_json::json!({ "kind": "LocalEncryption", "event": e })
        }
        CoreEvent::NativeAttention(e) => {
            serde_json::json!({ "kind": "NativeAttention", "event": e })
        }
        CoreEvent::CjkTextPolicy(e) => serde_json::json!({ "kind": "CjkTextPolicy", "event": e }),
        CoreEvent::ThreadsList(e) => serde_json::json!({ "kind": "ThreadsList", "event": e }),
        CoreEvent::OperationFailed {
            request_id,
            failure,
        } => {
            serde_json::json!({
                "kind": "OperationFailed",
                "request_id": request_id,
                "failure": failure,
            })
        }
        // Telemetry-lane event: emitted after reduce, never mixed with
        // StateDelta/StateChanged, never drives product state in React.
        CoreEvent::IntentLifecycle {
            request_id,
            outcome,
        } => {
            serde_json::json!({
                "kind": "IntentLifecycle",
                "request_id": request_id,
                "outcome": outcome,
            })
        }
    })
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::{
        CORE_EVENT_NAME, ForwarderLagDisposition, STATE_EVENT_NAME,
        forwarded_webview_events_for_core_event, forwarded_webview_events_for_lag_resync,
        forwarder_lag_disposition, serialize_core_event,
    };

    #[test]
    fn timeline_items_updated_forwarding_emits_core_event_name_and_all_diffs() {
        use koushi_core::{
            AccountKey, CoreEvent, TimelineDiff, TimelineEvent, TimelineKey,
            ids::{TimelineBatchId, TimelineGeneration},
        };
        use serde_json::json;

        let timeline_items_count = AtomicUsize::new(500);
        let diffs = (0..1000)
            .map(|index| TimelineDiff::Remove { index })
            .collect::<Vec<_>>();
        let event = CoreEvent::Timeline(TimelineEvent::ItemsUpdated {
            key: TimelineKey::room(
                AccountKey("@u:example.test".to_owned()),
                "!room:example.test",
            ),
            generation: TimelineGeneration(7),
            batch_id: TimelineBatchId(13),
            diffs,
        });

        let forwarded = forwarded_webview_events_for_core_event(&event, &timeline_items_count);

        assert_eq!(timeline_items_count.load(Ordering::Relaxed), 0);
        assert_eq!(forwarded.len(), 1);
        assert_eq!(forwarded[0].event_name, CORE_EVENT_NAME);
        assert_eq!(forwarded[0].payload["kind"], json!("Timeline"));
        let diffs = forwarded[0].payload["event"]["ItemsUpdated"]["diffs"]
            .as_array()
            .expect("timeline diffs should serialize as an array");
        assert_eq!(diffs.len(), 1000);
        assert_eq!(diffs[0], json!({ "Remove": { "index": 0 } }));
        assert_eq!(diffs[999], json!({ "Remove": { "index": 999 } }));
    }
    #[test]
    fn legacy_state_changed_forwarding_is_not_the_webview_state_path() {
        use koushi_core::CoreEvent;
        use koushi_state::AppState;

        let timeline_items_count = AtomicUsize::new(17);
        let event = CoreEvent::StateChanged(AppState::default());

        let forwarded = forwarded_webview_events_for_core_event(&event, &timeline_items_count);

        assert_eq!(timeline_items_count.load(Ordering::Relaxed), 17);
        assert!(
            forwarded.is_empty(),
            "legacy full StateChanged events must not drive the normal webview state path"
        );
    }
    #[test]
    fn state_delta_forwarding_emits_core_event_changed_slices() {
        use koushi_core::{CoreEvent, build_state_delta};
        use koushi_state::{AppState, SearchCrawlerRoomState};
        use serde_json::json;

        let timeline_items_count = AtomicUsize::new(17);
        let previous = AppState::default();
        let mut next = previous.clone();
        next.navigation.active_room_id = Some("!selected:example.invalid".to_owned());
        next.navigation.active_space_id = Some("!space:example.invalid".to_owned());
        next.search_crawler.rooms.insert(
            "!crawler:example.invalid".to_owned(),
            SearchCrawlerRoomState::Queued,
        );
        let delta = build_state_delta(1, &previous, &next).expect("delta");
        let forwarded = forwarded_webview_events_for_core_event(
            &CoreEvent::StateDelta(delta),
            &timeline_items_count,
        );

        assert_eq!(timeline_items_count.load(Ordering::Relaxed), 17);
        assert_eq!(forwarded.len(), 1);
        assert_eq!(forwarded[0].event_name, CORE_EVENT_NAME);
        assert_eq!(forwarded[0].payload["kind"], json!("StateDelta"));
        assert_eq!(forwarded[0].payload["generation"], json!(1));
        assert_eq!(
            forwarded[0].payload["changed"]["state"]["domain"]["search_crawler"]["rooms"]["!crawler:example.invalid"]
                ["kind"],
            json!("queued")
        );
        assert_eq!(
            forwarded[0].payload["changed"]["state"]["ui"]["navigation"]["active_room_id"],
            json!("!selected:example.invalid")
        );
        assert_eq!(
            forwarded[0].payload["changed"]["state"]["ui"]["navigation"]["active_space_id"],
            json!("!space:example.invalid")
        );
    }
    #[test]
    fn session_state_delta_forwarding_also_requests_snapshot_refresh() {
        use koushi_core::{CoreEvent, build_state_delta};
        use koushi_state::{AppState, ProvisionalPhase, SessionInfo, SessionState};
        use serde_json::json;

        let timeline_items_count = AtomicUsize::new(17);
        let mut previous = AppState::default();
        previous.session = SessionState::Provisional {
            info: SessionInfo {
                homeserver: "https://example.test".to_owned(),
                user_id: "@u:example.test".to_owned(),
                device_id: "DEV".to_owned(),
                authentication_method: koushi_state::SessionAuthenticationMethod::Unknown,
            },
            phase: ProvisionalPhase::CheckingTrust,
        };
        let mut next = previous.clone();
        next.session = SessionState::Provisional {
            info: SessionInfo {
                homeserver: "https://example.test".to_owned(),
                user_id: "@u:example.test".to_owned(),
                device_id: "DEV".to_owned(),
                authentication_method: koushi_state::SessionAuthenticationMethod::Unknown,
            },
            phase: ProvisionalPhase::DiscoveringMethods,
        };
        let delta = build_state_delta(1, &previous, &next).expect("session delta");

        let forwarded = forwarded_webview_events_for_core_event(
            &CoreEvent::StateDelta(delta),
            &timeline_items_count,
        );

        assert_eq!(forwarded.len(), 2);
        assert_eq!(forwarded[0].event_name, CORE_EVENT_NAME);
        assert_eq!(forwarded[0].payload["kind"], json!("StateDelta"));
        assert_eq!(
            forwarded[0].payload["changed"]["state"]["domain"]["session"]["phase"]["kind"],
            json!("discoveringMethods")
        );
        assert_eq!(forwarded[1].event_name, STATE_EVENT_NAME);
        assert_eq!(forwarded[1].payload, json!("stateChanged"));
    }
    #[test]
    fn lag_resync_forwarding_emits_state_then_resync_marker() {
        use koushi_state::AppState;
        use serde_json::json;

        let forwarded = forwarded_webview_events_for_lag_resync(&AppState::default());

        assert_eq!(forwarded.len(), 2);
        assert_eq!(forwarded[0].event_name, STATE_EVENT_NAME);
        assert_eq!(forwarded[0].payload, json!("stateChanged"));
        assert_eq!(forwarded[1].event_name, CORE_EVENT_NAME);
        assert_eq!(forwarded[1].payload, json!({ "kind": "ResyncMarker" }));
    }
    #[test]
    fn forwarder_lag_disposition_replays_positive_lag_and_stops_on_zero_sentinel() {
        assert_eq!(
            forwarder_lag_disposition(koushi_core::EventStreamLag { skipped: 1 }),
            ForwarderLagDisposition::ResyncAndReplay
        );
        assert_eq!(
            forwarder_lag_disposition(koushi_core::EventStreamLag { skipped: 0 }),
            ForwarderLagDisposition::ResyncAndStop
        );
    }
    #[test]
    fn lag_resync_forwarder_requests_core_timeline_replay_after_marker() {
        let forwarder_source = include_str!("core_event_forwarder.rs");
        let root_source = include_str!("lib.rs");
        let production_source = forwarder_source
            .split("#[cfg(test)]\nmod tests")
            .next()
            .expect("production source should precede tests");
        let forwarder_source = production_source
            .split("fn spawn_core_event_forwarder")
            .nth(1)
            .expect("core event forwarder should exist")
            .split("fn forwarded_webview_events_for_core_event")
            .next()
            .expect("forwarded event helper should follow forwarder");
        let lag_branch = forwarder_source
            .split("Err(lag)")
            .nth(1)
            .expect("forwarder should handle EventStreamLag");

        assert!(
            production_source.contains("TimelineCommand::ReplaySubscribed"),
            "forwarder lag recovery must ask core to replay InitialItems for subscribed timelines"
        );
        assert!(
            production_source.contains("struct CoreEventForwarderTask")
                && root_source.contains("forwarder_task: Some"),
            "the managed state must own the forwarder task"
        );
        assert!(
            !production_source.contains("Box::leak"),
            "the forwarder counter must not be leaked"
        );
        assert!(
            !lag_branch.contains("async_runtime::spawn"),
            "positive-lag replay must be awaited inline rather than detached"
        );
        assert!(
            lag_branch.contains("event_conn.command_handle()")
                && lag_branch.contains("event_conn.next_request_id()"),
            "lag recovery should clone a command handle and allocate a request id from the event connection"
        );
        let marker_offset = lag_branch
            .find("emit_forwarded_webview_events")
            .expect("lag branch should emit state + ResyncMarker");
        let replay_offset = lag_branch
            .find("submit_timeline_replay_after_forwarder_lag")
            .expect("lag branch should submit the replay command");
        assert!(
            marker_offset < replay_offset,
            "ResyncMarker must be emitted before replay is requested so fresh InitialItems are not cleared"
        );
        let stop_offset = lag_branch
            .find("ForwarderLagDisposition::ResyncAndStop")
            .expect("zero lag must select the defensive stop disposition");
        assert!(
            marker_offset < stop_offset,
            "zero-lag state + marker emission must precede the defensive loop exit"
        );
    }
    /// Wire-format contract test: pins the serialized JSON shapes the React
    /// layer types against (apps/desktop/src/domain/coreEvents.ts). Serde
    /// enums are externally tagged: struct variants serialize as
    /// {"Variant":{..}}, unit variants as "Variant". If this test changes,
    /// coreEvents.ts and coreEvents.generated.json must change with it.
    #[test]
    fn core_event_wire_format_matches_checked_in_contract_artifact() {
        use koushi_core::{
            AccountKey, CoreEvent, TimelineDiff, TimelineKey, build_state_delta,
            event::{
                AccountEvent, ActivityEvent, CjkTextPolicyEvent, E2eeTrustEvent,
                EncryptionDebugOperationOutcome, EventCacheFailureReasonClass,
                EventCacheSubscribeStatus, IntentNoOpReason, IntentOutcome, LinkPreview,
                LinkPreviewImage, LinkPreviewState, LiveSignalsEvent, LocalEncryptionEvent,
                NativeAttentionEvent, PaginationDirection, PaginationState, ReactionGroup,
                RoomEvent, RoomKeyRequestStage, RoomKeyRequestStateDto, RoomKeyRequestWithheldCode,
                RoomKeyReshareOutcome, SearchEvent, SyncEvent, ThreadRootProjectionDto,
                ThreadRootProjectionStateDto, ThreadSummaryDto, ThreadsListEvent,
                TimelineAnchorRestoreStatus, TimelineCodeBlock, TimelineDisplayLabelUpdate,
                TimelineEvent, TimelineFormattedBody, TimelineGapId, TimelineGapPosition,
                TimelineItem, TimelineItemId, TimelineMedia, TimelineMediaKind,
                TimelineMediaSource, TimelineMediaThumbnail, TimelineMegolmSessionReason,
                TimelineMessageActions, TimelineMessageKind, TimelineMessageSource,
                TimelineNavigationSnapshot, TimelineResyncReason, TimelineSendFailureReason,
                TimelineSendState, TimelineSpoilerSpan, TimelineUnreadPosition,
            },
            failure::{CoreFailure, TimelineFailureKind},
            ids::{RequestId, RuntimeConnectionId, TimelineBatchId, TimelineGeneration},
        };
        use koushi_state::{
            ActivityRow, ActivityStream, ActivityTab, AppState, AttachmentKind, AttachmentResult,
            AvatarThumbnailState, ComposerDocument, ComposerInline, CurrentSessionBackupState,
            CurrentSessionStatusDetails, CurrentSessionStatusState, CurrentSessionSyncState,
            DeviceCleanupOfferReason, DeviceCleanupState, DirectoryPreviewJoinability,
            DirectoryPreviewMembership, DirectoryQuery, DirectoryRoomPreview, DirectoryRoomSummary,
            IdentityResetAuthType, IdentityResetState, JapaneseCatalogProfile,
            LocalEncryptionHealth, MediaTransferProgress, MentionTarget,
            NativeAttentionCapabilities, NativeAttentionCapability, NativeAttentionSummary,
            OwnIdentityVerification, PresenceKind, ReplyQuote, ReplyQuoteCodeBlock,
            ReplyQuoteFormattedBody, ReplyQuoteState, RoomHistoryVisibility, RoomJoinRule,
            RoomMemberRole, RoomModerationAction, RoomPermissionFacts, RoomSettingsSnapshot,
            RoomTagKind, SasEmoji, SearchCrawlerFailureKind, SearchCrawlerRoomState,
            SessionAuthenticationMethod, SubmissionId, UserTrustState, VerificationFlowState,
            VerificationTarget,
        };
        use serde_json::json;

        let request_id = RequestId {
            connection_id: RuntimeConnectionId(3),
            sequence: 7,
        };
        let key = TimelineKey::room(AccountKey("@u:example.test".to_owned()), "!r:example.test");
        let item = TimelineItem {
            request_state: Some(RoomKeyRequestStateDto {
                stage: RoomKeyRequestStage::Withheld,
                withheld_code: Some(RoomKeyRequestWithheldCode::Unavailable),
            }),
            id: TimelineItemId::Event {
                event_id: "$e1".to_owned(),
            },
            sender: Some("@u:example.test".to_owned()),
            sender_label: None,
            sender_avatar: None,
            body: Some("hello".to_owned()),
            notice_i18n: None,
            message_kind: TimelineMessageKind::Emote,
            spoiler_spans: vec![TimelineSpoilerSpan {
                start_utf16: 0,
                end_utf16: 5,
                reason: Some("fixture".to_owned()),
            }],
            timestamp_ms: Some(123),
            in_reply_to_event_id: None,
            formatted: Some(TimelineFormattedBody {
                html: "<strong>hello</strong><pre><code class=\"language-rust\">fn main() {}</code></pre>".to_owned(),
                plain_text: "hellofn main() {}".to_owned(),
                code_blocks: vec![TimelineCodeBlock {
                    language: Some("rust".to_owned()),
                    body: "fn main() {}".to_owned(),
                }],
            }),
            reply_quote: None,
            thread_root: None,
            thread_summary: Some(ThreadSummaryDto {
                reply_count: 2,
                latest_event_id: Some("$thread-reply:example.test".to_owned()),
                latest_sender: Some("@thread:example.test".to_owned()),
                latest_sender_label: None,
                latest_body_preview: Some("thread reply".to_owned()),
                latest_timestamp_ms: Some(124),
            }),
            media: None,
            link_previews: None,
            link_ranges: Vec::new(),
            reactions: vec![ReactionGroup {
                key: "👍".to_owned(),
                count: 2,
                reacted_by_me: true,
                my_reaction_event_id: Some("$reaction:test".to_owned()),
                sender_preview: vec![koushi_core::ReactionSender {
                    user_id: "@u:example.test".to_owned(),
                    display_label: Some("Test User".to_owned()),
                }],
            }],
            can_react: true,
            is_redacted: false,
            is_hidden: false,
            can_redact: true,
            is_edited: true,
            can_edit: true,
            actions: TimelineMessageActions {
                can_copy: true,
                can_forward: true,
                can_reply: true,
                can_permalink: true,
                can_view_source: true,
                permalink: Some("https://matrix.to/#/!r%3Aexample.test/%24e1".to_owned()),
                editable_document: Some(ComposerDocument::new(vec![
                    ComposerInline::Text { text: "Hello ".into() },
                    ComposerInline::Mention {
                        target: MentionTarget::User {
                            user_id: "@mention:example.test".into(),
                            display_label: "Mention User".into(),
                        },
                        display_label: "Mention User".into(),
                    },
                ])),
            },
            send_state: None,
            unable_to_decrypt: None,
        };
        let media_item = TimelineItem {
            request_state: None,
            id: TimelineItemId::Event {
                event_id: "$media1".to_owned(),
            },
            sender: Some("@u:example.test".to_owned()),
            sender_label: None,
            sender_avatar: None,
            body: Some("caption".to_owned()),
            notice_i18n: None,
            message_kind: Default::default(),
            spoiler_spans: Vec::new(),
            timestamp_ms: Some(456),
            in_reply_to_event_id: None,
            formatted: None,
            reply_quote: None,
            thread_root: None,
            thread_summary: None,
            media: Some(TimelineMedia {
                kind: TimelineMediaKind::Image,
                filename: "fixture.png".to_owned(),
                source: TimelineMediaSource {
                    mxc_uri: "mxc://example.test/media".to_owned(),
                    encrypted: true,
                    encryption_version: Some("v2".to_owned()),
                },
                mimetype: Some("image/png".to_owned()),
                size: Some(68),
                width: Some(2),
                height: Some(2),
                thumbnail: Some(TimelineMediaThumbnail {
                    source: TimelineMediaSource {
                        mxc_uri: "mxc://example.test/thumb".to_owned(),
                        encrypted: false,
                        encryption_version: None,
                    },
                    mimetype: Some("image/png".to_owned()),
                    size: Some(32),
                    width: Some(1),
                    height: Some(1),
                }),
            }),
            link_previews: None,
            link_ranges: Vec::new(),
            reactions: Vec::new(),
            can_react: true,
            is_redacted: false,
            is_hidden: false,
            can_redact: true,
            is_edited: false,
            can_edit: false,
            actions: TimelineMessageActions {
                can_copy: true,
                can_forward: true,
                can_reply: true,
                can_permalink: true,
                can_view_source: true,
                permalink: Some("https://matrix.to/#/!r%3Aexample.test/%24media1".to_owned()),
                editable_document: None,
            },
            send_state: None,
            unable_to_decrypt: None,
        };
        let send_state_item = TimelineItem {
            request_state: None,
            id: TimelineItemId::Transaction {
                transaction_id: "txn-not-sent".to_owned(),
            },
            sender: Some("@u:example.test".to_owned()),
            sender_label: None,
            sender_avatar: None,
            body: Some("queued".to_owned()),
            notice_i18n: None,
            message_kind: Default::default(),
            spoiler_spans: Vec::new(),
            timestamp_ms: Some(789),
            in_reply_to_event_id: None,
            formatted: None,
            reply_quote: None,
            thread_root: None,
            thread_summary: None,
            media: None,
            link_previews: None,
            link_ranges: Vec::new(),
            reactions: Vec::new(),
            can_react: false,
            is_redacted: false,
            is_hidden: false,
            can_redact: false,
            is_edited: false,
            can_edit: false,
            actions: TimelineMessageActions::default(),
            send_state: Some(TimelineSendState::NotSent {
                reason: TimelineSendFailureReason::Recoverable,
            }),
            unable_to_decrypt: None,
        };
        let reply_quote_item = TimelineItem {
            request_state: None,
            id: TimelineItemId::Event {
                event_id: "$reply1".to_owned(),
            },
            sender: Some("@u:example.test".to_owned()),
            sender_label: None,
            sender_avatar: None,
            body: Some("reply body".to_owned()),
            notice_i18n: None,
            message_kind: Default::default(),
            spoiler_spans: Vec::new(),
            timestamp_ms: Some(987),
            in_reply_to_event_id: Some("$root1".to_owned()),
            formatted: None,
            reply_quote: Some(ReplyQuote {
                event_id: "$root1".to_owned(),
                sender: Some("@other:example.test".to_owned()),
                sender_label: None,
                body_preview: Some("quoted preview".to_owned()),
                formatted: Some(ReplyQuoteFormattedBody {
                    html: "<p>quoted <strong>preview</strong></p><pre><code class=\"language-rust\">fn main() {}</code></pre>".to_owned(),
                    plain_text: "quoted previewfn main() {}".to_owned(),
                    code_blocks: vec![ReplyQuoteCodeBlock {
                        language: Some("rust".to_owned()),
                        body: "fn main() {}".to_owned(),
                    }],
                }),
                state: ReplyQuoteState::Ready,
            }),
            thread_root: None,
            thread_summary: None,
            media: None,
            link_previews: None,
            link_ranges: Vec::new(),
            reactions: Vec::new(),
            can_react: true,
            is_redacted: false,
            is_hidden: false,
            can_redact: true,
            is_edited: false,
            can_edit: false,
            actions: TimelineMessageActions {
                can_copy: true,
                can_forward: true,
                can_reply: true,
                can_permalink: true,
                can_view_source: true,
                permalink: Some("https://matrix.to/#/!r%3Aexample.test/%24reply1".to_owned()),
                editable_document: None,
            },
            send_state: None,
            unable_to_decrypt: None,
        };
        let link_preview_item = TimelineItem {
            request_state: None,
            id: TimelineItemId::Event {
                event_id: "$linkpreview1".to_owned(),
            },
            sender: Some("@u:example.test".to_owned()),
            sender_label: None,
            sender_avatar: None,
            body: Some("Check out https://example.invalid/page".to_owned()),
            notice_i18n: None,
            message_kind: Default::default(),
            spoiler_spans: Vec::new(),
            timestamp_ms: Some(1111),
            in_reply_to_event_id: None,
            formatted: None,
            reply_quote: None,
            thread_root: None,
            thread_summary: None,
            media: None,
            link_previews: Some(vec![LinkPreview {
                url: "https://example.invalid/page".to_owned(),
                title: Some("Example Page".to_owned()),
                description: Some("A synthetic fixture page.".to_owned()),
                image: Some(LinkPreviewImage {
                    source: TimelineMediaSource {
                        mxc_uri: "mxc://example.invalid/preview-image".to_owned(),
                        encrypted: false,
                        encryption_version: None,
                    },
                    width: Some(1200),
                    height: Some(630),
                    thumbnail: AvatarThumbnailState::Ready {
                        source_url: "koushi-thumbnail://localhost/link-preview/fixture.bin"
                            .to_owned(),
                        width: Some(600),
                        height: Some(315),
                        mime_type: Some("image/png".to_owned()),
                    },
                }),
                state: LinkPreviewState::Ready,
            }]),
            link_ranges: Vec::new(),
            reactions: Vec::new(),
            can_react: true,
            is_redacted: false,
            is_hidden: false,
            can_redact: true,
            is_edited: false,
            can_edit: false,
            actions: TimelineMessageActions {
                can_copy: true,
                can_forward: true,
                can_reply: true,
                can_permalink: true,
                can_view_source: true,
                permalink: Some("https://matrix.to/#/!r%3Aexample.test/%24linkpreview1".to_owned()),
                editable_document: None,
            },
            send_state: None,
            unable_to_decrypt: None,
        };

        // InitialItems envelope + payload
        let initial = serialize_core_event(&CoreEvent::Timeline(TimelineEvent::InitialItems {
            request_id: Some(request_id),
            cause_request_id: Some(request_id),
            key: key.clone(),
            actor_generation: 1,
            generation: TimelineGeneration(1),
            items: vec![item.clone()],
        }))
        .expect("timeline events serialize");
        assert_eq!(initial["kind"], json!("Timeline"));
        let payload = &initial["event"]["InitialItems"];
        assert_eq!(
            payload["request_id"],
            json!({ "connection_id": 3, "sequence": 7 })
        );
        assert_eq!(
            payload["cause_request_id"],
            json!({ "connection_id": 3, "sequence": 7 })
        );
        assert_eq!(
            payload["key"],
            json!({
                "account_key": "@u:example.test",
                "kind": { "Room": { "room_id": "!r:example.test" } }
            })
        );
        assert_eq!(payload["generation"], json!(1));
        assert_eq!(
            payload["items"][0],
            json!({
                "id": { "Event": { "event_id": "$e1" } },
                "sender": "@u:example.test",
                "request_state": { "stage": "withheld", "withheldCode": "unavailable" },
                "sender_label": null,
                "body": "hello",
                "message_kind": "emote",
                "spoiler_spans": [
                    {
                        "start_utf16": 0,
                        "end_utf16": 5,
                        "reason": "fixture"
                    }
                ],
                "timestamp_ms": 123,
                "in_reply_to_event_id": null,
                "formatted": {
                    "html": "<strong>hello</strong><pre><code class=\"language-rust\">fn main() {}</code></pre>",
                    "plain_text": "hellofn main() {}",
                    "code_blocks": [
                        {
                            "language": "rust",
                            "body": "fn main() {}"
                        }
                    ]
                },
                "thread_root": null,
                "thread_summary": {
                    "reply_count": 2,
                    "latest_event_id": "$thread-reply:example.test",
                    "latest_sender": "@thread:example.test",
                    "latest_sender_label": null,
                    "latest_body_preview": "thread reply",
                    "latest_timestamp_ms": 124
                },
                "can_react": true,
                "is_redacted": false,
                "is_hidden": false,
                "can_redact": true,
                "is_edited": true,
                "can_edit": true,
                "actions": {
                    "can_copy": true,
                    "can_forward": true,
                    "can_reply": true,
                    "can_permalink": true,
                    "can_view_source": true,
                    "permalink": "https://matrix.to/#/!r%3Aexample.test/%24e1",
                    "editable_document": {
                        "version": 2,
                        "inlines": [
                            { "kind": "text", "text": "Hello " },
                            {
                                "kind": "mention",
                                "target": {
                                    "kind": "user",
                                    "user_id": "@mention:example.test",
                                    "display_label": "Mention User"
                                },
                                "display_label": "Mention User"
                            }
                        ]
                    }
                },
                "reactions": [
                    {
                        "key": "👍",
                        "count": 2,
                        "reacted_by_me": true,
                        "my_reaction_event_id": "$reaction:test",
                        "sender_preview": [
                            {
                                "user_id": "@u:example.test",
                                "display_label": "Test User"
                            }
                        ]
                    }
                ]
            })
        );

        // ItemsUpdated: diffs are externally tagged; unit variants are strings
        let updated = serialize_core_event(&CoreEvent::Timeline(TimelineEvent::ItemsUpdated {
            key: key.clone(),
            generation: TimelineGeneration(1),
            batch_id: TimelineBatchId(9),
            diffs: vec![
                TimelineDiff::PushFront { item: item.clone() },
                TimelineDiff::Remove { index: 2 },
                TimelineDiff::Clear,
            ],
        }))
        .expect("serialize");
        let diffs = &updated["event"]["ItemsUpdated"]["diffs"];
        assert!(diffs[0]["PushFront"]["item"]["id"]["Event"]["event_id"] == json!("$e1"));
        assert_eq!(diffs[1], json!({ "Remove": { "index": 2 } }));
        assert_eq!(diffs[2], json!("Clear"));
        assert_eq!(updated["event"]["ItemsUpdated"]["batch_id"], json!(9));

        let thread_root_projection =
            serialize_core_event(&CoreEvent::Timeline(TimelineEvent::ThreadRootProjection {
                key: key.clone(),
                projection: ThreadRootProjectionDto {
                    root_event_id: "$e1".to_owned(),
                    activity_event_id: "$thread-reply:example.test".to_owned(),
                    activity_timestamp_ms: Some(124),
                    retain_without_reply: false,
                    source: Default::default(),
                    state: ThreadRootProjectionStateDto::Pending,
                },
            }))
            .expect("serialize thread-root projection");
        assert_eq!(
            thread_root_projection["event"]["ThreadRootProjection"]["projection"]["state"]["kind"],
            json!("pending")
        );

        let media_initial =
            serialize_core_event(&CoreEvent::Timeline(TimelineEvent::InitialItems {
                request_id: Some(request_id),
                cause_request_id: None,
                key: key.clone(),
                actor_generation: 2,
                generation: TimelineGeneration(2),
                items: vec![media_item],
            }))
            .expect("serialize media initial items");
        assert_eq!(
            media_initial["event"]["InitialItems"]["items"][0]["media"],
            json!({
                "kind": "Image",
                "filename": "fixture.png",
                "source": {
                    "mxc_uri": "mxc://example.test/media",
                    "encrypted": true,
                    "encryption_version": "v2"
                },
                "mimetype": "image/png",
                "size": 68,
                "width": 2,
                "height": 2,
                "thumbnail": {
                    "source": {
                        "mxc_uri": "mxc://example.test/thumb",
                        "encrypted": false,
                        "encryption_version": null
                    },
                    "mimetype": "image/png",
                    "size": 32,
                    "width": 1,
                    "height": 1
                }
            })
        );

        let send_state_initial =
            serialize_core_event(&CoreEvent::Timeline(TimelineEvent::InitialItems {
                request_id: Some(request_id),
                cause_request_id: None,
                key: key.clone(),
                actor_generation: 3,
                generation: TimelineGeneration(3),
                items: vec![send_state_item],
            }))
            .expect("serialize send-state initial items");
        assert_eq!(
            send_state_initial["event"]["InitialItems"]["items"][0]["send_state"],
            json!({
                "kind": "notSent",
                "reason": "recoverable"
            })
        );

        let reply_quote_initial =
            serialize_core_event(&CoreEvent::Timeline(TimelineEvent::InitialItems {
                request_id: Some(request_id),
                cause_request_id: None,
                key: key.clone(),
                actor_generation: 4,
                generation: TimelineGeneration(4),
                items: vec![reply_quote_item],
            }))
            .expect("serialize reply quote initial items");
        assert_eq!(
            reply_quote_initial["event"]["InitialItems"]["items"][0]["reply_quote"],
            json!({
                "event_id": "$root1",
                "sender": "@other:example.test",
                "sender_label": null,
                "body_preview": "quoted preview",
                "formatted": {
                    "html": "<p>quoted <strong>preview</strong></p><pre><code class=\"language-rust\">fn main() {}</code></pre>",
                    "plain_text": "quoted previewfn main() {}",
                    "code_blocks": [
                        {
                            "language": "rust",
                            "body": "fn main() {}"
                        }
                    ]
                },
                "state": "ready"
            })
        );

        let link_preview_initial =
            serialize_core_event(&CoreEvent::Timeline(TimelineEvent::InitialItems {
                request_id: Some(request_id),
                cause_request_id: None,
                key: key.clone(),
                actor_generation: 5,
                generation: TimelineGeneration(5),
                items: vec![link_preview_item],
            }))
            .expect("serialize link preview initial items");
        assert_eq!(
            link_preview_initial["event"]["InitialItems"]["items"][0]["link_previews"],
            json!([
                {
                    "url": "https://example.invalid/page",
                    "title": "Example Page",
                    "description": "A synthetic fixture page.",
                    "image": {
                        "source": {
                            "mxc_uri": "mxc://example.invalid/preview-image",
                            "encrypted": false,
                            "encryption_version": null
                        },
                        "width": 1200,
                        "height": 630,
                        "thumbnail": {
                            "kind": "ready",
                            "source_url": "koushi-thumbnail://localhost/link-preview/fixture.bin",
                            "width": 600,
                            "height": 315,
                            "mime_type": "image/png"
                        }
                    },
                    "state": "ready"
                }
            ])
        );

        let media_upload_progress =
            serialize_core_event(&CoreEvent::Timeline(TimelineEvent::MediaUploadProgress {
                request_id: Some(request_id),
                key: key.clone(),
                transaction_id: "txn-media".to_owned(),
                index: 0,
                progress: MediaTransferProgress {
                    current: 1,
                    total: 2,
                },
                source: Some(TimelineMediaSource {
                    mxc_uri: "mxc://example.test/media".to_owned(),
                    encrypted: false,
                    encryption_version: None,
                }),
            }))
            .expect("serialize media upload progress");

        let media_send_queued =
            serialize_core_event(&CoreEvent::Timeline(TimelineEvent::MediaSendQueued {
                request_id,
                key: key.clone(),
                transaction_id: "txn-media".to_owned(),
            }))
            .expect("serialize media queue admission");

        let media_download_progress =
            serialize_core_event(&CoreEvent::Timeline(TimelineEvent::MediaDownloadProgress {
                request_id,
                key: key.clone(),
                event_id: "$media1".to_owned(),
                progress: MediaTransferProgress {
                    current: 0,
                    total: 68,
                },
            }))
            .expect("serialize media download progress");

        let media_download_completed = serialize_core_event(&CoreEvent::Timeline(
            TimelineEvent::MediaDownloadCompleted {
                request_id,
                key: key.clone(),
                event_id: "$media1".to_owned(),
                source_url: "/data/media_downloads/!r:example.test/$media1.bin".to_owned(),
                byte_count: 68,
                mimetype: Some("image/png".to_owned()),
                width: Some(2),
                height: Some(2),
            },
        ))
        .expect("serialize media download completion");

        let media_download_failed =
            serialize_core_event(&CoreEvent::Timeline(TimelineEvent::MediaDownloadFailed {
                request_id,
                key: key.clone(),
                event_id: "$media1".to_owned(),
                kind: TimelineFailureKind::Sdk,
            }))
            .expect("serialize media download failure");

        let message_source_loaded =
            serialize_core_event(&CoreEvent::Timeline(TimelineEvent::MessageSourceLoaded {
                request_id,
                key: key.clone(),
                source: TimelineMessageSource {
                    event_id: "$e1".to_owned(),
                    sender: Some("@u:example.test".to_owned()),
                    timestamp_ms: Some(123),
                    body: Some("hello".to_owned()),
                    in_reply_to_event_id: None,
                    thread_root: None,
                    is_redacted: false,
                    is_edited: true,
                    has_media: false,
                    megolm_session_fingerprint: Some("AbCdEfGhIjKl".to_owned()),
                    megolm_session_rotation_reason: Some(TimelineMegolmSessionReason::ExpiredTime),
                    original_json: None,
                },
            }))
            .expect("serialize message source loaded");
        let message_forwarded =
            serialize_core_event(&CoreEvent::Timeline(TimelineEvent::MessageForwarded {
                request_id,
                key: key.clone(),
                destination_room_id: "!destination:example.test".to_owned(),
                transaction_id: "txn-forward".to_owned(),
                event_id: "$forwarded1".to_owned(),
            }))
            .expect("serialize message forwarded");

        // PaginationStateChanged: unit states are strings, Failed is tagged
        let pagination = serialize_core_event(&CoreEvent::Timeline(
            TimelineEvent::PaginationStateChanged {
                request_id: None,
                key: key.clone(),
                direction: PaginationDirection::Backward,
                state: PaginationState::EndReached,
                prepend_expected: Some(false),
            },
        ))
        .expect("serialize");
        let pagination = &pagination["event"]["PaginationStateChanged"];
        assert_eq!(pagination["request_id"], json!(null));
        assert_eq!(pagination["direction"], json!("Backward"));
        assert_eq!(pagination["state"], json!("EndReached"));
        assert_eq!(pagination["prepend_expected"], json!(false));

        let anchor_restore_finished =
            serialize_core_event(&CoreEvent::Timeline(TimelineEvent::AnchorRestoreFinished {
                request_id,
                key: key.clone(),
                status: TimelineAnchorRestoreStatus::BudgetExhausted,
            }))
            .expect("serialize anchor restore finished");
        assert_eq!(
            anchor_restore_finished["event"]["AnchorRestoreFinished"]["status"],
            json!("BudgetExhausted")
        );

        // ResyncRequired reason is a string
        let resync = serialize_core_event(&CoreEvent::Timeline(TimelineEvent::ResyncRequired {
            key: key.clone(),
            reason: TimelineResyncReason::QueueOverflow,
        }))
        .expect("serialize");
        assert_eq!(
            resync["event"]["ResyncRequired"]["reason"],
            json!("QueueOverflow")
        );

        let navigation_updated =
            serialize_core_event(&CoreEvent::Timeline(TimelineEvent::NavigationUpdated {
                key: key.clone(),
                snapshot: TimelineNavigationSnapshot {
                    read_marker_event_id: Some("$read:example.test".to_owned()),
                    read_marker_display_event_id: Some("$read:example.test".to_owned()),
                    first_unread_event_id: Some("$unread:example.test".to_owned()),
                    local_viewed_event_id: Some("$read:example.test".to_owned()),
                    server_confirmed_read_event_id: Some("$read:example.test".to_owned()),
                    read_state_sync: koushi_core::TimelineReadStateSync::Synced,
                    unread_event_count: 2,
                    unread_position: TimelineUnreadPosition::BelowViewport,
                    newer_event_count: 3,
                    can_jump_to_bottom: true,
                },
            }))
            .expect("serialize navigation update event");
        assert_eq!(
            navigation_updated["event"]["NavigationUpdated"]["snapshot"]["unread_position"],
            json!("belowViewport")
        );

        let gap_positions_updated =
            serialize_core_event(&CoreEvent::Timeline(TimelineEvent::GapPositionsUpdated {
                key: key.clone(),
                actor_generation: 3,
                generation: 4,
                positions: vec![TimelineGapPosition {
                    id: TimelineGapId {
                        topology_revision: 14_695_981_039_346_656_037,
                        ordinal: 0,
                    },
                    before_item_index: 2,
                }],
            }))
            .expect("serialize gap positions update event");
        let gap_repair_released =
            serialize_core_event(&CoreEvent::Timeline(TimelineEvent::GapRepairReleased {
                key: key.clone(),
                actor_generation: 3,
                generation: 5,
            }))
            .expect("serialize gap repair release event");

        let display_labels_updated =
            serialize_core_event(&CoreEvent::Timeline(TimelineEvent::DisplayLabelsUpdated {
                labels: vec![TimelineDisplayLabelUpdate {
                    user_id: "@u:example.test".to_owned(),
                    display_label: "User Alias".to_owned(),
                }],
            }))
            .expect("serialize display label update event");
        assert_eq!(
            display_labels_updated["event"]["DisplayLabelsUpdated"]["labels"][0],
            json!({
                "user_id": "@u:example.test",
                "display_label": "User Alias"
            })
        );
        let display_policy_updated =
            serialize_core_event(&CoreEvent::Timeline(TimelineEvent::DisplayPolicyUpdated {
                hide_redacted: true,
            }))
            .expect("serialize display policy update event");
        assert_eq!(
            display_policy_updated["event"]["DisplayPolicyUpdated"]["hide_redacted"],
            json!(true)
        );

        // Account events are externally tagged under the Account envelope
        let listed = serialize_core_event(&CoreEvent::Account(AccountEvent::SavedSessionsListed {
            request_id,
            sessions: vec![koushi_state::SessionInfo {
                homeserver: "https://example.test".to_owned(),
                user_id: "@u:example.test".to_owned(),
                device_id: "DEV".to_owned(),
                authentication_method: koushi_state::SessionAuthenticationMethod::Unknown,
            }],
        }))
        .expect("serialize");
        assert_eq!(listed["kind"], json!("Account"));
        assert_eq!(
            listed["event"]["SavedSessionsListed"]["sessions"][0]["device_id"],
            json!("DEV")
        );
        let profile_updated =
            serialize_core_event(&CoreEvent::Account(AccountEvent::ProfileUpdated {
                request_id,
                account_key: AccountKey("@u:example.test".to_owned()),
            }))
            .expect("serialize profile update event");

        let account_report_completed =
            serialize_core_event(&CoreEvent::Account(AccountEvent::ReportCompleted {
                request_id,
                kind: koushi_core::event::ReportKind::User,
            }))
            .expect("serialize account report completed event");
        let account_oidc_authorization_created = serialize_core_event(&CoreEvent::Account(
            AccountEvent::OidcAuthorizationCreated {
                request_id,
                authorization_url: "https://auth.example.test/authorize".to_owned(),
                state: "synthetic-state".to_owned(),
            },
        ))
        .expect("serialize OIDC authorization event");

        // OperationFailed: unit failures are strings
        let failed = serialize_core_event(&CoreEvent::OperationFailed {
            request_id,
            failure: CoreFailure::SessionNotFound,
        })
        .expect("serialize");
        assert_eq!(failed["kind"], json!("OperationFailed"));
        assert_eq!(failed["failure"], json!("SessionNotFound"));

        let room_left = serialize_core_event(&CoreEvent::Room(RoomEvent::RoomLeft {
            request_id,
            room_id: "!r:example.test".to_owned(),
        }))
        .expect("serialize");
        assert_eq!(room_left["kind"], json!("Room"));
        assert_eq!(
            room_left["event"]["RoomLeft"]["room_id"],
            json!("!r:example.test")
        );
        let room_key_reshared =
            serialize_core_event(&CoreEvent::Room(RoomEvent::RoomKeyReshared {
                request_id,
                room_id: "!r:example.test".to_owned(),
                outcome: RoomKeyReshareOutcome::Sent {
                    request_count: 2,
                    recipient_count: 3,
                    failed_recipient_count: 0,
                },
            }))
            .expect("serialize room key reshare outcome");
        let index0_room_key_resent =
            serialize_core_event(&CoreEvent::Room(RoomEvent::Index0RoomKeyResent {
                request_id,
                room_id: "!r:example.test".to_owned(),
                outcome: EncryptionDebugOperationOutcome::Completed,
            }))
            .expect("serialize index-0 room key resent event");
        let room_key_request_state_changed =
            serialize_core_event(&CoreEvent::Room(RoomEvent::RoomKeyRequestStateChanged {
                key: key.clone(),
                event_id: "$e1".to_owned(),
                request_id: Some(request_id),
                stage: RoomKeyRequestStage::Withheld,
                withheld_code: Some(RoomKeyRequestWithheldCode::Unavailable),
            }))
            .expect("serialize room key request state changed");
        let composer_slash_command_rejected =
            serialize_core_event(&CoreEvent::Room(RoomEvent::ComposerSlashCommandRejected {
                key: key.clone(),
                request_id,
            }))
            .expect("serialize composer slash command rejection");

        let room_invite_accepted =
            serialize_core_event(&CoreEvent::Room(RoomEvent::InviteAccepted {
                request_id,
                room_id: "!r:example.test".to_owned(),
            }))
            .expect("serialize");
        let room_invite_declined =
            serialize_core_event(&CoreEvent::Room(RoomEvent::InviteDeclined {
                request_id,
                room_id: "!r:example.test".to_owned(),
            }))
            .expect("serialize");
        let room_direct_message_started =
            serialize_core_event(&CoreEvent::Room(RoomEvent::DirectMessageStarted {
                request_id,
                room_id: "!dm:example.test".to_owned(),
            }))
            .expect("serialize");
        let room_tag_set = serialize_core_event(&CoreEvent::Room(RoomEvent::RoomTagSet {
            request_id,
            room_id: "!r:example.test".to_owned(),
            tag: RoomTagKind::Favourite,
        }))
        .expect("serialize room tag set");
        let room_tag_removed = serialize_core_event(&CoreEvent::Room(RoomEvent::RoomTagRemoved {
            request_id,
            room_id: "!r:example.test".to_owned(),
            tag: RoomTagKind::LowPriority,
        }))
        .expect("serialize room tag removed");
        let room_marked_as_read = serialize_core_event(&CoreEvent::Room(RoomEvent::MarkedAsRead {
            request_id,
            room_id: "!r:example.test".to_owned(),
        }))
        .expect("serialize room marked as read");
        let room_marked_as_unread =
            serialize_core_event(&CoreEvent::Room(RoomEvent::MarkedAsUnread {
                request_id,
                room_id: "!r:example.test".to_owned(),
                unread: true,
            }))
            .expect("serialize room marked as unread");
        let room_report_completed =
            serialize_core_event(&CoreEvent::Room(RoomEvent::ReportCompleted {
                request_id,
                kind: koushi_core::event::ReportKind::Event,
            }))
            .expect("serialize room report completed event");
        let sync_started = serialize_core_event(&CoreEvent::Sync(SyncEvent::Started {
            request_id: Some(request_id),
        }))
        .expect("serialize sync started");
        let directory_query_completed =
            serialize_core_event(&CoreEvent::Room(RoomEvent::DirectoryQueryCompleted {
                request_id,
                query: DirectoryQuery {
                    term: Some("public".to_owned()),
                    server_name: Some("example.test".to_owned()),
                    limit: Some(20),
                    since: Some("page-2".to_owned()),
                },
                rooms: vec![DirectoryRoomSummary {
                    room_id: "!public:example.test".to_owned(),
                    canonical_alias: Some("#public:example.test".to_owned()),
                    room_type: None,
                    name: "Public Room".to_owned(),
                    topic: Some("Directory sample".to_owned()),
                    avatar_url: None,
                    joined_members: 5,
                    world_readable: true,
                    guest_can_join: false,
                }],
                next_batch: Some("page-3".to_owned()),
            }))
            .expect("serialize directory query completion");
        let directory_preview_loaded =
            serialize_core_event(&CoreEvent::Room(RoomEvent::DirectoryPreviewLoaded {
                request_id,
                room: DirectoryRoomPreview {
                    room_id: "!previewed:example.test".to_owned(),
                    canonical_alias: Some("#previewed:example.test".to_owned()),
                    room_type: Some("m.space".to_owned()),
                    name: "Previewed Space".to_owned(),
                    topic: Some("Directory preview sample".to_owned()),
                    joined_members: 12,
                    joinability: DirectoryPreviewJoinability::Restricted,
                    membership: DirectoryPreviewMembership::Invited,
                },
            }))
            .expect("serialize room directory preview loaded event");
        let room_settings_snapshot = RoomSettingsSnapshot {
            room_id: "!r:example.test".to_owned(),
            name: Some("Room Settings Sample".to_owned()),
            topic: Some("Private topic sample".to_owned()),
            avatar_url: Some("mxc://example.test/avatar".to_owned()),
            canonical_alias: Some("#private:example.test".to_owned()),
            alternate_aliases: vec!["#private-alt:example.test".to_owned()],
            share_link: Some("https://matrix.to/#/%23private%3Aexample.test".to_owned()),
            join_rule: RoomJoinRule::Invite,
            history_visibility: RoomHistoryVisibility::Shared,
            permissions: RoomPermissionFacts {
                can_edit_settings: true,
                can_edit_roles: true,
                can_invite: true,
                can_kick: true,
                can_ban: true,
                can_unban: true,
            },
            members: vec![koushi_state::RoomMemberSummary {
                user_id: "@member:example.test".to_owned(),
                display_name: Some("Synthetic Member".to_owned()),
                display_label: "Synthetic Member".to_owned(),
                original_display_label: "Synthetic Member".to_owned(),
                avatar_url: Some("mxc://example.test/member-avatar".to_owned()),
                power_level: Some(50),
                role: RoomMemberRole::Moderator,
                user_trust: Some(UserTrustState::Verified),
            }],
        };
        let room_settings_loaded =
            serialize_core_event(&CoreEvent::Room(RoomEvent::RoomSettingsLoaded {
                request_id,
                settings: room_settings_snapshot.clone(),
            }))
            .expect("serialize room settings loaded");
        let room_setting_updated =
            serialize_core_event(&CoreEvent::Room(RoomEvent::RoomSettingUpdated {
                request_id,
                settings: room_settings_snapshot,
            }))
            .expect("serialize room setting updated");
        let room_member_moderated =
            serialize_core_event(&CoreEvent::Room(RoomEvent::RoomMemberModerated {
                request_id,
                room_id: "!r:example.test".to_owned(),
                target_user_id: "@target:example.test".to_owned(),
                action: RoomModerationAction::Kick,
            }))
            .expect("serialize room member moderated");
        let room_member_role_updated =
            serialize_core_event(&CoreEvent::Room(RoomEvent::RoomMemberRoleUpdated {
                request_id,
                room_id: "!r:example.test".to_owned(),
                target_user_id: "@target:example.test".to_owned(),
                power_level: 50,
            }))
            .expect("serialize room member role updated");
        let space_member_role_update_settled =
            serialize_core_event(&CoreEvent::Room(RoomEvent::SpaceMemberRoleUpdateSettled {
                request_id,
                generation: 4,
                outcome: koushi_state::SpaceMemberRoleUpdateOutcome::Succeeded,
            }))
            .expect("serialize Space member role update settled");
        assert_eq!(
            room_settings_loaded["event"]["RoomSettingsLoaded"]["settings"]["permissions"]["can_edit_settings"],
            json!(true)
        );
        assert_eq!(
            room_settings_loaded["event"]["RoomSettingsLoaded"]["settings"]["permissions"]["can_edit_roles"],
            json!(true)
        );
        assert_eq!(
            room_settings_loaded["event"]["RoomSettingsLoaded"]["settings"]["members"][0]["role"],
            json!("moderator")
        );
        assert_eq!(
            room_member_moderated["event"]["RoomMemberModerated"]["action"],
            json!("kick")
        );
        assert_eq!(
            room_member_role_updated["event"]["RoomMemberRoleUpdated"]["power_level"],
            json!(50)
        );
        assert_eq!(
            space_member_role_update_settled["event"]["SpaceMemberRoleUpdateSettled"]["outcome"],
            json!("succeeded")
        );

        let e2ee_trust = serialize_core_event(&CoreEvent::E2eeTrust(
            E2eeTrustEvent::VerificationProgress {
                account_key: AccountKey("@u:example.test".to_owned()),
                state: VerificationFlowState::SasPresented {
                    request_id: request_id.sequence,
                    target: VerificationTarget {
                        user_id: "@other:example.test".to_owned(),
                        device_id: "OTHERDEVICE".to_owned(),
                    },
                    emojis: vec![SasEmoji {
                        symbol: "🐶".to_owned(),
                        description: "Dog".to_owned(),
                    }],
                },
            },
        ))
        .expect("serialize");
        assert_eq!(e2ee_trust["kind"], json!("E2eeTrust"));
        assert_eq!(e2ee_trust["event"]["kind"], json!("verificationProgress"));
        assert_eq!(e2ee_trust["event"]["state"]["kind"], json!("sasPresented"));

        let e2ee_identity_reset = serialize_core_event(&CoreEvent::E2eeTrust(
            E2eeTrustEvent::IdentityResetChanged {
                account_key: AccountKey("@u:example.test".to_owned()),
                state: IdentityResetState::AwaitingAuth {
                    request_id: request_id.sequence,
                    auth_type: IdentityResetAuthType::Uiaa,
                },
            },
        ))
        .expect("serialize identity reset event");
        assert_eq!(
            e2ee_identity_reset["event"]["kind"],
            json!("identityResetChanged")
        );
        assert_eq!(
            e2ee_identity_reset["event"]["state"]["kind"],
            json!("awaitingAuth")
        );

        let live_presence =
            serialize_core_event(&CoreEvent::LiveSignals(LiveSignalsEvent::PresenceSet {
                request_id,
                presence: PresenceKind::Away,
            }))
            .expect("serialize live presence event");
        assert_eq!(live_presence["event"]["kind"], json!("presenceSet"));

        let activity_opened =
            serialize_core_event(&CoreEvent::Activity(ActivityEvent::Opened { request_id }))
                .expect("serialize activity event");
        assert_eq!(activity_opened["kind"], json!("Activity"));
        assert_eq!(
            activity_opened["event"]["Opened"]["request_id"],
            json!({ "connection_id": 3, "sequence": 7 })
        );
        let activity_snapshot_loaded =
            serialize_core_event(&CoreEvent::Activity(ActivityEvent::SnapshotLoaded {
                request_id,
                active_tab: ActivityTab::Unread,
                recent: ActivityStream {
                    rows: vec![ActivityRow {
                        kind: koushi_state::ActivityRowKind::Event,
                        room_id: "!activity-recent:example.test".to_owned(),
                        event_id: Some("$activity-recent:example.test".to_owned()),
                        room_label: "Recent room".to_owned(),
                        sender_label: Some("Recent sender".to_owned()),
                        preview: Some("Recent preview".to_owned()),
                        timestamp_ms: 20,
                        unread: false,
                        highlight: false,
                        ..Default::default()
                    }],
                    next_batch: Some("recent-next".to_owned()),
                    resolution: Default::default(),
                },
                unread: ActivityStream {
                    rows: vec![
                        ActivityRow {
                            kind: koushi_state::ActivityRowKind::Event,
                            room_id: "!activity-unread:example.test".to_owned(),
                            event_id: Some("$activity-unread:example.test".to_owned()),
                            room_label: "Unread room".to_owned(),
                            sender_label: Some("Unread sender".to_owned()),
                            preview: Some("Unread preview".to_owned()),
                            timestamp_ms: 10,
                            unread: true,
                            highlight: true,
                            ..Default::default()
                        },
                        ActivityRow::room_unread_placeholder(
                            "!activity-placeholder:example.test".to_owned(),
                            "Placeholder room".to_owned(),
                            9,
                            false,
                        ),
                    ],
                    next_batch: Some("unread-next".to_owned()),
                    resolution: Default::default(),
                },
            }))
            .expect("serialize activity snapshot event");
        assert_eq!(
            activity_snapshot_loaded["event"]["SnapshotLoaded"]["active_tab"],
            json!("unread")
        );
        assert_eq!(
            activity_snapshot_loaded["event"]["SnapshotLoaded"]["unread"]["rows"][0]["highlight"],
            json!(true)
        );
        assert_eq!(
            activity_snapshot_loaded["event"]["SnapshotLoaded"]["unread"]["rows"][1]["kind"],
            json!("roomUnread")
        );
        assert_eq!(
            activity_snapshot_loaded["event"]["SnapshotLoaded"]["unread"]["rows"][1]["event_id"],
            serde_json::Value::Null
        );
        let activity_marked_read =
            serialize_core_event(&CoreEvent::Activity(ActivityEvent::MarkedRead {
                request_id,
                cleared_event_ids: vec!["$activity-unread:example.test".to_owned()],
            }))
            .expect("serialize activity marked-read event");
        let activity_resolution_retried =
            serialize_core_event(&CoreEvent::Activity(ActivityEvent::ResolutionRetried {
                request_id,
                generation: 4,
            }))
            .expect("serialize activity resolution retry event");
        assert_eq!(
            activity_marked_read["event"]["MarkedRead"]["cleared_event_ids"],
            json!(["$activity-unread:example.test"])
        );

        let local_encryption = serialize_core_event(&CoreEvent::LocalEncryption(
            LocalEncryptionEvent::HealthChanged {
                health: LocalEncryptionHealth::Healthy,
            },
        ))
        .expect("serialize local encryption event");
        assert_eq!(local_encryption["event"]["kind"], json!("healthChanged"));
        assert_eq!(local_encryption["event"]["health"], json!("healthy"));

        let local_encryption_event_cache_enabled = serialize_core_event(
            &CoreEvent::LocalEncryption(LocalEncryptionEvent::EventCacheStatus {
                encrypted_store: true,
                subscribed: true,
                subscribe_status: EventCacheSubscribeStatus::AlreadyEnabled,
                reason_class: None,
            }),
        )
        .expect("serialize enabled local encryption event cache status");
        assert_eq!(
            local_encryption_event_cache_enabled["event"]["kind"],
            json!("eventCacheStatus")
        );
        assert_eq!(
            local_encryption_event_cache_enabled["event"]["encrypted_store"],
            json!(true)
        );
        assert_eq!(
            local_encryption_event_cache_enabled["event"]["subscribed"],
            json!(true)
        );
        assert_eq!(
            local_encryption_event_cache_enabled["event"]["subscribe_status"],
            json!("already_enabled")
        );
        assert!(
            local_encryption_event_cache_enabled["event"]
                .get("reason_class")
                .is_none(),
            "success diagnostics should omit the optional failure reason"
        );

        let local_encryption_event_cache_failed = serialize_core_event(
            &CoreEvent::LocalEncryption(LocalEncryptionEvent::EventCacheStatus {
                encrypted_store: true,
                subscribed: false,
                subscribe_status: EventCacheSubscribeStatus::SubscribeFailed,
                reason_class: Some(EventCacheFailureReasonClass::SubscribeFailed),
            }),
        )
        .expect("serialize failed local encryption event cache status");
        assert_eq!(
            local_encryption_event_cache_failed["event"]["subscribe_status"],
            json!("subscribe_failed")
        );
        assert_eq!(
            local_encryption_event_cache_failed["event"]["reason_class"],
            json!("subscribe_failed")
        );

        let native_attention = serialize_core_event(&CoreEvent::NativeAttention(
            NativeAttentionEvent::SummaryUpdated {
                summary: NativeAttentionSummary {
                    unread_count: 3,
                    highlight_count: 1,
                    badge_count: 3,
                    candidate: None,
                    capabilities: NativeAttentionCapabilities {
                        notifications: NativeAttentionCapability::Available,
                        badge: NativeAttentionCapability::Available,
                        overlay_icon: NativeAttentionCapability::Unknown,
                        sound: NativeAttentionCapability::Unavailable,
                        tray: NativeAttentionCapability::Unknown,
                        activation: NativeAttentionCapability::Available,
                    },
                },
            },
        ))
        .expect("serialize native attention event");
        assert_eq!(native_attention["event"]["kind"], json!("summaryUpdated"));
        assert_eq!(
            native_attention["event"]["summary"]["badge_count"],
            json!(3)
        );

        let cjk_text_policy = serialize_core_event(&CoreEvent::CjkTextPolicy(
            CjkTextPolicyEvent::JapaneseCatalogProfileChanged {
                profile: JapaneseCatalogProfile {
                    catalog_locale: "ja".to_owned(),
                    complete: false,
                    missing_message_ids: vec!["settings.title".to_owned()],
                },
            },
        ))
        .expect("serialize cjk text policy event");
        assert_eq!(
            cjk_text_policy["event"]["kind"],
            json!("japaneseCatalogProfileChanged")
        );

        let search_attachments_results =
            serialize_core_event(&CoreEvent::Search(SearchEvent::AttachmentsResults {
                request_id,
                results: vec![AttachmentResult {
                    room_id: "!r:example.test".to_owned(),
                    event_id: "$f1".to_owned(),
                    sender: "@u:example.test".to_owned(),
                    sender_label: Some("Test User".to_owned()),
                    timestamp_ms: 1,
                    kind: AttachmentKind::Image,
                    filename: "photo.png".to_owned(),
                    mimetype: Some("image/png".to_owned()),
                    size: Some(1234),
                    source_mxc: "mxc://example.invalid/abc".to_owned(),
                    thumbnail_mxc: Some("mxc://example.invalid/abc-thumb".to_owned()),
                    thread_root: None,
                    encrypted: false,
                    encryption_version: None,
                    width: None,
                    height: None,
                    is_edited: false,
                }],
            }))
            .expect("serialize search attachments results event");
        assert_eq!(
            search_attachments_results["event"]["AttachmentsResults"]["results"][0]["kind"],
            json!("image")
        );

        let search_attachments_failed =
            serialize_core_event(&CoreEvent::Search(SearchEvent::AttachmentsFailed {
                request_id,
                message: "index unavailable".to_owned(),
            }))
            .expect("serialize search attachments failed event");
        assert_eq!(
            search_attachments_failed["event"]["AttachmentsFailed"]["message"],
            json!("index unavailable")
        );

        assert!(
            serialize_core_event(&CoreEvent::Search(SearchEvent::IndexUpdated {
                room_id: "!r:example.test".to_owned(),
                event_id: "$indexed:example.test".to_owned(),
            }))
            .is_none(),
            "per-message index updates are internal and must not cross WebView IPC"
        );

        // Search history crawler contract events (#77).
        let search_crawl_progress =
            serialize_core_event(&CoreEvent::Search(SearchEvent::HistoryCrawlProgress {
                room_id: "!r:example.test".to_owned(),
                processed: 100,
                indexed: 42,
            }))
            .expect("serialize history crawl progress event");
        assert_eq!(
            search_crawl_progress["event"]["HistoryCrawlProgress"]["processed"],
            json!(100u64)
        );

        let search_crawl_completed =
            serialize_core_event(&CoreEvent::Search(SearchEvent::HistoryCrawlCompleted {
                room_id: "!r:example.test".to_owned(),
                indexed: 42,
            }))
            .expect("serialize history crawl completed event");
        assert_eq!(
            search_crawl_completed["event"]["HistoryCrawlCompleted"]["indexed"],
            json!(42u64)
        );

        let search_crawl_failed =
            serialize_core_event(&CoreEvent::Search(SearchEvent::HistoryCrawlFailed {
                room_id: "!r:example.test".to_owned(),
                kind: SearchCrawlerFailureKind::Sdk,
            }))
            .expect("serialize history crawl failed event");
        assert_eq!(
            search_crawl_failed["event"]["HistoryCrawlFailed"]["failureKind"],
            json!("sdk")
        );
        // Privacy assertion: no raw error text in the failed event.
        assert!(
            !serde_json::to_string(&search_crawl_failed)
                .unwrap()
                .contains("message"),
            "crawl failure must not carry a raw message field"
        );

        let state_delta_previous = AppState::default();
        let mut state_delta_next = state_delta_previous.clone();
        state_delta_next.search_crawler.rooms.insert(
            "!crawler:example.test".to_owned(),
            SearchCrawlerRoomState::Queued,
        );
        state_delta_next.device_cleanup = DeviceCleanupState::Offered {
            reason: DeviceCleanupOfferReason::NoProofMethod,
        };
        state_delta_next.current_session_status = CurrentSessionStatusState::Ready {
            request_id: 369,
            details: CurrentSessionStatusDetails::new(
                Some("Contract Device".to_owned()),
                "CONTRACTDEVICE".to_owned(),
                SessionAuthenticationMethod::OAuth,
                CurrentSessionSyncState::Running,
                true,
                OwnIdentityVerification::Verified,
                CurrentSessionBackupState::Ready,
                1_722_000_000_000,
            ),
        };
        let state_delta_event = CoreEvent::StateDelta(
            build_state_delta(1, &state_delta_previous, &state_delta_next).expect("fixture delta"),
        );
        let state_delta =
            forwarded_webview_events_for_core_event(&state_delta_event, &AtomicUsize::new(0))
                .into_iter()
                .next()
                .expect("state delta should be forwarded")
                .payload;

        let actual_contract = json!({
            "activityOpened": activity_opened,
            "activityMarkedRead": activity_marked_read,
            "activityResolutionRetried": activity_resolution_retried,
            "activitySnapshotLoaded": activity_snapshot_loaded,
            "cjkTextPolicyJapaneseCatalogProfileChanged": cjk_text_policy,
            "e2eeTrustIdentityResetChanged": e2ee_identity_reset,
            "accountProfileUpdated": profile_updated,
            "accountOidcAuthorizationCreated": account_oidc_authorization_created,
            "accountReportCompleted": account_report_completed,
            "accountSavedSessionsListed": listed,
            "e2eeTrustVerificationProgress": e2ee_trust,
            "localEncryptionHealthChanged": local_encryption,
            "localEncryptionEventCacheStatus": local_encryption_event_cache_failed,
            "liveSignalsPresenceSet": live_presence,
            "nativeAttentionSummaryUpdated": native_attention,
            "operationFailedSessionNotFound": failed,
            "searchAttachmentsFailed": search_attachments_failed,
            "searchAttachmentsResults": search_attachments_results,
            "searchCrawlProgress": search_crawl_progress,
            "searchCrawlCompleted": search_crawl_completed,
            "searchCrawlFailed": search_crawl_failed,
            "stateDeltaSearchCrawlerQueued": state_delta,
            "roomDirectoryQueryCompleted": directory_query_completed,
            "roomDirectoryPreviewLoaded": directory_preview_loaded,
            "roomDirectMessageStarted": room_direct_message_started,
            "roomInviteAccepted": room_invite_accepted,
            "roomInviteDeclined": room_invite_declined,
            "roomLeft": room_left,
            "roomKeyReshared": room_key_reshared,
            "index0RoomKeyResent": index0_room_key_resent,
            "roomKeyRequestStateChanged": room_key_request_state_changed,
            "composerSlashCommandRejected": composer_slash_command_rejected,
            "roomMarkedAsRead": room_marked_as_read,
            "roomMarkedAsUnread": room_marked_as_unread,
            "roomReportCompleted": room_report_completed,
            "roomMemberModerated": room_member_moderated,
            "roomMemberRoleUpdated": room_member_role_updated,
            "spaceMemberRoleUpdateSettled": space_member_role_update_settled,
            "roomSettingUpdated": room_setting_updated,
            "roomSettingsLoaded": room_settings_loaded,
            "roomTagRemoved": room_tag_removed,
            "roomTagSet": room_tag_set,
            "syncStarted": sync_started,
            "timelineDisplayLabelsUpdated": display_labels_updated,
            "timelineDisplayPolicyUpdated": display_policy_updated,
            "timelineInitialItems": initial,
            "timelineItemsUpdated": updated,
            "timelineThreadRootProjection": thread_root_projection,
            "timelineLinkPreviewInitialItems": link_preview_initial,
            "timelineMediaDownloadCompleted": media_download_completed,
            "timelineMediaDownloadFailed": media_download_failed,
            "timelineMediaDownloadProgress": media_download_progress,
            "timelineMediaInitialItems": media_initial,
            "timelineMediaUploadProgress": media_upload_progress,
            "timelineMediaSendQueued": media_send_queued,
            "timelineMessageForwarded": message_forwarded,
            "timelineMessageSourceLoaded": message_source_loaded,
            "timelineNavigationUpdated": navigation_updated,
            "timelineGapPositionsUpdated": gap_positions_updated,
            "timelineGapRepairReleased": gap_repair_released,
            "timelineAnchorRestoreFinished": anchor_restore_finished,
            "timelinePaginationEndReached": serialize_core_event(&CoreEvent::Timeline(
                TimelineEvent::PaginationStateChanged {
                    request_id: None,
                    key: key.clone(),
                    direction: PaginationDirection::Backward,
                    state: PaginationState::EndReached,
                    prepend_expected: Some(false),
                },
            ))
            .expect("serialize"),
            "timelineReplyQuoteInitialItems": reply_quote_initial,
            "timelineResyncRequired": resync,
            "timelineSendStateInitialItems": send_state_initial,
            "timelineSubmissionAccepted": serialize_core_event(&CoreEvent::Timeline(
                TimelineEvent::SubmissionAccepted {
                    request_id,
                    key: TimelineKey {
                        account_key: AccountKey("@user:example.test".to_owned()),
                        kind: koushi_core::TimelineKind::Room {
                            room_id: "!room:example.test".to_owned(),
                        },
                    },
                    submission_id: SubmissionId::new("submission-contract"),
                    transaction_id: "transaction-contract".to_owned(),
                }
            )).expect("serialize submission accepted"),
            "timelineSubmissionRejected": serialize_core_event(&CoreEvent::Timeline(
                TimelineEvent::SubmissionRejected {
                    request_id,
                    key: TimelineKey {
                        account_key: AccountKey("@user:example.test".to_owned()),
                        kind: koushi_core::TimelineKind::Room {
                            room_id: "!room:example.test".to_owned(),
                        },
                    },
                    submission_id: SubmissionId::new("submission-contract"),
                    kind: TimelineFailureKind::NotSubscribed,
                }
            )).expect("serialize submission rejected"),
            "threadsListOpened": serialize_core_event(&CoreEvent::ThreadsList(
                ThreadsListEvent::Opened {
                    request_id,
                    room_id: "!room:example.test".to_owned(),
                    items: vec![],
                    end_reached: false,
                },
            ))
            .expect("serialize threads list opened"),
            "intentLifecycleCommitted": serialize_core_event(&CoreEvent::IntentLifecycle {
                request_id,
                outcome: IntentOutcome::Committed,
            })
            .expect("serialize intent lifecycle committed"),
            "intentLifecycleFailedNoOpRoomNotInState": serialize_core_event(
                &CoreEvent::IntentLifecycle {
                    request_id,
                    outcome: IntentOutcome::FailedNoOp(IntentNoOpReason::RoomNotInState),
                },
            )
            .expect("serialize intent lifecycle failed noop room not in state"),
        });
        let checked_in_contract: serde_json::Value =
            serde_json::from_str(include_str!("../../src/domain/coreEvents.generated.json"))
                .expect("checked-in core event contract artifact must be valid JSON");
        assert_eq!(actual_contract, checked_in_contract);
    }
    /// CoreEvent IPC-contract key-completeness guard.
    ///
    /// The `core_event_wire_format_matches_checked_in_contract_artifact` test
    /// proves the Rust-serialized shapes equal the checked-in JSON. This
    /// companion test locks in the EXACT SET of keys so a later refactor
    /// cannot accidentally remove a variant from the artifact without being
    /// caught — even if the remaining keys still match.
    ///
    /// If a new `CoreEvent` variant is added, extend `core_event_wire_format_...`
    /// first (to produce the serialized form), update the artifact, then this
    /// expected set gains the new key automatically (it reads the artifact). The
    /// test therefore functions as a "no-shrink" guard: the key count must not
    /// decrease, and every key must remain in the known-valid set derived from
    /// the Rust contract test above.
    #[test]
    fn core_event_contract_artifact_key_set_does_not_shrink() {
        // This set is the canonical key list produced by the Rust contract
        // test. It is spelled out here so that deleting a key from the artifact
        // (or from the contract test's `actual_contract` object) fails this test
        // immediately, requiring a deliberate update in both places.
        let expected_keys: std::collections::BTreeSet<&str> = [
            "accountProfileUpdated",
            "accountOidcAuthorizationCreated",
            "accountReportCompleted",
            "accountSavedSessionsListed",
            "activityMarkedRead",
            "activityOpened",
            "activityResolutionRetried",
            "activitySnapshotLoaded",
            "cjkTextPolicyJapaneseCatalogProfileChanged",
            "e2eeTrustIdentityResetChanged",
            "e2eeTrustVerificationProgress",
            "intentLifecycleCommitted",
            "intentLifecycleFailedNoOpRoomNotInState",
            "liveSignalsPresenceSet",
            "localEncryptionHealthChanged",
            "localEncryptionEventCacheStatus",
            "nativeAttentionSummaryUpdated",
            "operationFailedSessionNotFound",
            "roomDirectMessageStarted",
            "roomDirectoryPreviewLoaded",
            "roomDirectoryQueryCompleted",
            "roomInviteAccepted",
            "roomInviteDeclined",
            "roomLeft",
            "roomKeyReshared",
            "index0RoomKeyResent",
            "roomKeyRequestStateChanged",
            "composerSlashCommandRejected",
            "roomMarkedAsRead",
            "roomMarkedAsUnread",
            "roomMemberModerated",
            "roomMemberRoleUpdated",
            "spaceMemberRoleUpdateSettled",
            "roomReportCompleted",
            "roomSettingUpdated",
            "roomSettingsLoaded",
            "roomTagRemoved",
            "roomTagSet",
            "searchAttachmentsFailed",
            "searchAttachmentsResults",
            "searchCrawlCompleted",
            "searchCrawlFailed",
            "searchCrawlProgress",
            "stateDeltaSearchCrawlerQueued",
            "syncStarted",
            "threadsListOpened",
            "timelineDisplayLabelsUpdated",
            "timelineDisplayPolicyUpdated",
            "timelineInitialItems",
            "timelineItemsUpdated",
            "timelineThreadRootProjection",
            "timelineLinkPreviewInitialItems",
            "timelineMediaDownloadCompleted",
            "timelineMediaDownloadFailed",
            "timelineMediaDownloadProgress",
            "timelineMediaInitialItems",
            "timelineMediaUploadProgress",
            "timelineMediaSendQueued",
            "timelineMessageForwarded",
            "timelineMessageSourceLoaded",
            "timelineAnchorRestoreFinished",
            "timelineNavigationUpdated",
            "timelineGapPositionsUpdated",
            "timelineGapRepairReleased",
            "timelinePaginationEndReached",
            "timelineReplyQuoteInitialItems",
            "timelineResyncRequired",
            "timelineSendStateInitialItems",
            "timelineSubmissionAccepted",
            "timelineSubmissionRejected",
        ]
        .iter()
        .copied()
        .collect();

        let artifact: serde_json::Value =
            serde_json::from_str(include_str!("../../src/domain/coreEvents.generated.json"))
                .expect("contract artifact must be valid JSON");

        let artifact_keys: std::collections::BTreeSet<&str> = artifact
            .as_object()
            .expect("contract artifact must be a JSON object")
            .keys()
            .map(String::as_str)
            .collect();

        let missing_from_artifact: Vec<&&str> = expected_keys
            .iter()
            .filter(|k| !artifact_keys.contains(*k))
            .collect();
        assert!(
            missing_from_artifact.is_empty(),
            "CoreEvent contract artifact is missing keys that were previously present: {missing_from_artifact:?}. \
            If a variant was intentionally removed, update the expected set in this test \
            AND the coreEvents.ts TypeScript types in the same PR."
        );

        let unexpected_in_artifact: Vec<&&str> = artifact_keys
            .iter()
            .filter(|k| !expected_keys.contains(*k))
            .collect();
        assert!(
            unexpected_in_artifact.is_empty(),
            "CoreEvent contract artifact contains keys not present in the expected set: {unexpected_in_artifact:?}. \
            Add the new key to the expected set in this test after adding the corresponding \
            Rust serialization entry in core_event_wire_format_matches_checked_in_contract_artifact."
        );
    }
}
