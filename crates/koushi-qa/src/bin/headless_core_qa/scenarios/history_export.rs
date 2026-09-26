//! History export (room and Space archive folders) through `CoreCommand`
//! against a local homeserver.
//!
//! Output is token-only. The exported folders hold synthetic QA messages in a
//! per-run temporary directory and are deleted when the stage ends; no path,
//! identifier, or body is printed.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use koushi_core::runtime::CoreConnection;
use koushi_protocol::command::{
    AccountCommand, CoreCommand, HistoryExportLabels, HistoryExportRequest,
};
use koushi_protocol::ids::{AccountKey, RequestId, TimelineKey};
use koushi_state::{
    AppState, HistoryExportRange, HistoryExportRoom, HistoryExportRoomCounts,
    HistoryExportRoomPhase, HistoryExportScope, HistoryExportState,
};
use serde_json::Value;

use super::event_wait::{
    subscribe_timeline_for_qa, wait_for_encrypted_room_projection_for_qa,
    wait_for_invite_in_snapshot, wait_for_item_with_body, wait_for_media_send_flow_completion,
    wait_for_send_flow_completion, wait_for_space_child_projection, wait_for_space_in_space_list,
    wait_for_withheld_event_projection_from_source,
};
use super::fixtures::{
    accept_invite_for_qa, create_room_for_qa, create_space_for_qa, invite_user_for_qa,
    set_space_child_for_qa, start_direct_message_for_qa,
};
use super::participants::{
    QaOwnedRuntimeParticipant, QaParticipantLoginGate, cleanup_owned_e2ee_participant_best_effort,
    login_synced_participant_for_qa, qa_data_dir,
};
use super::registry::{E2EE_EVENT_TIMEOUT, QaConfig};

const EXPORT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(180);
const HEADER_KEYS: [&str; 6] = [
    "room_name",
    "room_creator",
    "topic",
    "export_date",
    "exported_by",
    "messages",
];
/// A 1×1 PNG, uploaded as an image attachment.
const PNG_1X1: &[u8] = &[
    0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44, 0x52,
    0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1F, 0x15, 0xC4,
    0x89, 0x00, 0x00, 0x00, 0x0A, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9C, 0x63, 0x00, 0x01, 0x00, 0x00,
    0x05, 0x00, 0x01, 0x0D, 0x0A, 0x2D, 0xB4, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4E, 0x44, 0xAE,
    0x42, 0x60, 0x82,
];

/// A terminal export outcome read from the authoritative snapshot.
enum ExportOutcome {
    Completed(Vec<HistoryExportRoom>),
    Stopped,
    Failed(String),
}

fn terminal_outcome(state: &AppState, request_id: RequestId) -> Option<ExportOutcome> {
    match &state.history_export {
        HistoryExportState::Completed {
            request_id: id,
            rooms,
            ..
        } if *id == request_id.sequence => Some(ExportOutcome::Completed(rooms.clone())),
        HistoryExportState::Stopped { request_id: id, .. } if *id == request_id.sequence => {
            Some(ExportOutcome::Stopped)
        }
        HistoryExportState::Failed {
            request_id: id,
            failure_kind,
            ..
        } if *id == request_id.sequence => Some(ExportOutcome::Failed(format!("{failure_kind:?}"))),
        _ => None,
    }
}

async fn wait_for_terminal(
    conn: &mut CoreConnection,
    request_id: RequestId,
    label: &str,
) -> Result<ExportOutcome, String> {
    if let Some(outcome) = terminal_outcome(&conn.snapshot(), request_id) {
        return Ok(outcome);
    }
    let mut failure: Option<String> = None;
    let result = tokio::time::timeout(EXPORT_TIMEOUT, async {
        loop {
            match conn.recv_event().await {
                Ok(koushi_protocol::event::CoreEvent::OperationFailed {
                    request_id: failed,
                    failure: kind,
                }) if failed == request_id => failure = Some(format!("{kind:?}")),
                Ok(_) => {}
                // A lagged stream still leaves the snapshot authoritative.
                Err(_) => tokio::time::sleep(std::time::Duration::from_millis(100)).await,
            }
            if let Some(outcome) = terminal_outcome(&conn.snapshot(), request_id) {
                return Ok(outcome);
            }
        }
    })
    .await;
    result.map_err(|_| {
        format!(
            "{label}: timed out waiting for a terminal export state ({}; operation_failed={})",
            export_state_summary(&conn.snapshot().history_export, request_id),
            failure.as_deref().unwrap_or("none")
        )
    })?
}

/// Private-data-free summary of the export state for a timeout diagnosis.
fn export_state_summary(state: &HistoryExportState, request_id: RequestId) -> String {
    let (kind, rooms) = match state {
        HistoryExportState::Idle => ("idle", None),
        HistoryExportState::Preparing { .. } => ("preparing", None),
        HistoryExportState::Running { rooms, .. } => ("running", Some(rooms)),
        HistoryExportState::Completed { rooms, .. } => ("completed", Some(rooms)),
        HistoryExportState::Stopped { rooms, .. } => ("stopped", Some(rooms)),
        HistoryExportState::Failed { rooms, .. } => ("failed", Some(rooms)),
    };
    let ours = state.request_id() == Some(request_id.sequence);
    let rooms = rooms
        .map(|rooms| {
            rooms
                .iter()
                .map(|room| {
                    format!(
                        "{:?}:f{}:a{}/{}",
                        room.phase,
                        room.counts.fetched_events,
                        room.counts.attachments_done,
                        room.counts.attachments_total
                    )
                })
                .collect::<Vec<_>>()
                .join(",")
        })
        .unwrap_or_default();
    format!("state={kind} ours={ours} rooms=[{rooms}]")
}

async fn start_export(
    conn: &mut CoreConnection,
    scope: HistoryExportScope,
    range: HistoryExportRange,
    chosen_dir: &Path,
    label: &str,
) -> Result<RequestId, String> {
    let request_id = conn.next_request_id();
    conn.register_native_artifact(
        request_id,
        koushi_core::NativeArtifactKind::HistoryExportDirectory,
        chosen_dir.to_path_buf(),
    )
    .map_err(|_| format!("{label}: register export directory"))?;
    conn.command(CoreCommand::Account(AccountCommand::ExportHistory {
        request_id,
        request: HistoryExportRequest {
            scope,
            range,
            display_time_zone: "UTC".to_owned(),
            export_date_utc_offset_minutes: 0,
            folder_name_stem: "QA Export".to_owned(),
            labels: HistoryExportLabels {
                lang: "en".to_owned(),
                edited: "(edited)".to_owned(),
                ..HistoryExportLabels::default()
            },
        },
    }))
    .await
    .map_err(|_| format!("{label}: submit export"))?;
    Ok(request_id)
}

async fn export_completed(
    conn: &mut CoreConnection,
    scope: HistoryExportScope,
    range: HistoryExportRange,
    chosen_dir: &Path,
    label: &str,
) -> Result<Vec<HistoryExportRoom>, String> {
    let request_id = start_export(conn, scope, range, chosen_dir, label).await?;
    match wait_for_terminal(conn, request_id, label).await? {
        ExportOutcome::Completed(rooms) => Ok(rooms),
        ExportOutcome::Stopped => Err(format!("{label}: export stopped")),
        ExportOutcome::Failed(kind) => Err(format!("{label}: export failed kind={kind}")),
    }
}

/// The export folder Core created inside `chosen_dir`, or `chosen_dir` itself
/// when it already held a manifest.
fn export_dir(chosen_dir: &Path) -> Result<PathBuf, String> {
    if chosen_dir.join("koushi-export.json").is_file() {
        return Ok(chosen_dir.to_path_buf());
    }
    let mut folders = std::fs::read_dir(chosen_dir)
        .map_err(|_| "history export: read chosen directory".to_owned())?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.join("koushi-export.json").is_file())
        .collect::<Vec<_>>();
    match (folders.pop(), folders.is_empty()) {
        (Some(folder), true) => Ok(folder),
        _ => Err("history export: expected exactly one export folder".to_owned()),
    }
}

fn room_folders(export_dir: &Path) -> Result<Vec<PathBuf>, String> {
    let mut folders = std::fs::read_dir(export_dir.join("rooms"))
        .map_err(|_| "history export: read rooms folder".to_owned())?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .collect::<Vec<_>>();
    folders.sort();
    Ok(folders)
}

fn no_partial_folders(export_dir: &Path) -> bool {
    std::fs::read_dir(export_dir.join("rooms")).is_ok_and(|entries| {
        entries
            .filter_map(Result::ok)
            .all(|entry| !entry.file_name().to_string_lossy().ends_with(".partial"))
    })
}

fn read_messages(room_folder: &Path, label: &str) -> Result<Value, String> {
    let bytes = std::fs::read(room_folder.join("messages.json"))
        .map_err(|_| format!("{label}: read messages.json"))?;
    let value = serde_json::from_slice::<Value>(&bytes)
        .map_err(|_| format!("{label}: messages.json is not JSON"))?;
    // Element's top-level key order, as `JSON.stringify` writes it.
    let text = String::from_utf8(bytes).map_err(|_| format!("{label}: export is not UTF-8"))?;
    let positions = HEADER_KEYS
        .iter()
        .map(|key| text.find(&format!("\n  \"{key}\": ")))
        .collect::<Option<Vec<_>>>()
        .ok_or_else(|| format!("{label}: export lacks an Element top-level key"))?;
    if !positions.windows(2).all(|pair| pair[0] < pair[1]) {
        return Err(format!("{label}: Element top-level key order differs"));
    }
    for file in [
        "events.jsonl",
        "room.json",
        "attachments.json",
        "index.html",
    ] {
        if !room_folder.join(file).is_file() {
            return Err(format!("{label}: room folder lacks {file}"));
        }
    }
    Ok(value)
}

/// Export one room into a fresh directory and return its `messages.json` and
/// the Rust-reported counts.
async fn export_room_to_value(
    conn: &mut CoreConnection,
    room_id: &str,
    range: HistoryExportRange,
    chosen_dir: &Path,
    label: &str,
) -> Result<(Value, HistoryExportRoomCounts), String> {
    std::fs::create_dir_all(chosen_dir).map_err(|_| format!("{label}: prepare directory"))?;
    let rooms = export_completed(
        conn,
        HistoryExportScope::Room {
            room_id: room_id.to_owned(),
        },
        range,
        chosen_dir,
        label,
    )
    .await?;
    let [room] = rooms.as_slice() else {
        return Err(format!(
            "{label}: a room export listed {} rooms",
            rooms.len()
        ));
    };
    if room.phase != HistoryExportRoomPhase::Completed {
        return Err(format!("{label}: room phase {:?}", room.phase));
    }
    let dir = export_dir(chosen_dir)?;
    let folders = room_folders(&dir)?;
    let [folder] = folders.as_slice() else {
        return Err(format!("{label}: expected one room folder"));
    };
    Ok((read_messages(folder, label)?, room.counts))
}

fn messages(value: &Value) -> &[Value] {
    value["messages"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or(&[])
}

fn event_ids(value: &Value) -> Vec<String> {
    messages(value)
        .iter()
        .filter_map(|event| event["event_id"].as_str().map(str::to_owned))
        .collect()
}

fn is_undecryptable(event: &Value) -> bool {
    event["type"] == "m.room.message" && event["content"]["msgtype"] == "m.bad.encrypted"
}

fn is_text_message(event: &Value) -> bool {
    event["type"] == "m.room.message" && event["content"]["msgtype"] == "m.text"
}

/// Structural checks shared by every completed export.
fn check_export(
    value: &Value,
    progress: &HistoryExportRoomCounts,
    label: &str,
) -> Result<(), String> {
    for key in [
        "room_name",
        "room_creator",
        "topic",
        "export_date",
        "exported_by",
    ] {
        if !value[key].is_string() {
            return Err(format!("{label}: header field {key} is not a string"));
        }
    }
    let events = messages(value);
    if events.len() as u64 != progress.exported_events {
        return Err(format!(
            "{label}: exported count {} differs from the file ({})",
            progress.exported_events,
            events.len()
        ));
    }
    let ids = event_ids(value);
    if ids.len() != events.len() || ids.iter().collect::<BTreeSet<_>>().len() != ids.len() {
        return Err(format!(
            "{label}: exported event ids are missing or repeated"
        ));
    }
    for event in events {
        for field in ["type", "sender", "room_id", "event_id"] {
            if !event[field].is_string() {
                return Err(format!("{label}: exported event lacks {field}"));
            }
        }
        if !event["origin_server_ts"].is_u64() {
            return Err(format!("{label}: exported event lacks origin_server_ts"));
        }
        let kind = event["type"].as_str().unwrap_or_default();
        if matches!(kind, "m.reaction" | "m.room.redaction") {
            return Err(format!(
                "{label}: exported an event Element does not render"
            ));
        }
        if event["content"]["m.relates_to"]["rel_type"] == "m.replace" {
            return Err(format!("{label}: exported an edit event"));
        }
    }
    let undecryptable = events
        .iter()
        .filter(|event| is_undecryptable(event))
        .count() as u64;
    if undecryptable != progress.undecryptable_events {
        return Err(format!(
            "{label}: undecryptable count {} differs from the file ({undecryptable})",
            progress.undecryptable_events
        ));
    }
    Ok(())
}

struct ExportDirectory(PathBuf);

impl Drop for ExportDirectory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

async fn send_text(
    conn: &mut CoreConnection,
    key: &TimelineKey,
    transaction_id: &str,
    body: &str,
    label: &str,
) -> Result<String, String> {
    let request_id = conn.next_request_id();
    conn.command(CoreCommand::Timeline(
        koushi_protocol::command::TimelineCommand::SendText {
            request_id,
            key: key.clone(),
            transaction_id: transaction_id.to_owned(),
            document: koushi_state::ComposerDocument::from_plain_text(body.to_owned()),
        },
    ))
    .await
    .map_err(|_| format!("{label}: submit send"))?;
    let outcome =
        wait_for_send_flow_completion(conn, request_id, key, transaction_id, body, label).await?;
    Ok(outcome.event_id)
}

/// Proofs (token-only):
/// - `history_export_full=ok`: a Core-owned export of an encrypted room walks
///   the server history forward from the first visible event and writes
///   Element's top-level object with every sent message decrypted, no edit,
///   reaction, or redaction events, and counts that match the file.
/// - `history_export_period=ok`: a period export contains exactly the events
///   of the full export whose timestamps fall in `[start, end)`.
/// - `history_export_period_fallback=ok`: a period after the room, where
///   `timestamp_to_event` finds no event, reads the whole visible history and
///   writes an empty file. A period before the room also writes an empty
///   file; `history_export_period_bounded` reports whether the seek and the
///   margin cutoff read fewer events than the full export.
/// - `history_export_utd_counted=ok`: a member whose device was denied a room
///   key exports that message as Element's `m.bad.encrypted` placeholder, and
///   the Rust-owned result counts every placeholder; messages sent after they
///   joined stay decrypted.
/// - `history_export_cancel=ok`: a stopped export settles as stopped and
///   leaves no partial room folder.
/// - `history_export_attachments=ok`: an image uploaded to the encrypted room
///   is downloaded, decrypted byte-for-byte, and given a JPEG thumbnail.
/// - `history_export_space=ok`: a Space export covers its joined rooms, lists
///   them in the table of contents, and leaves out a direct message.
/// - `history_export_resume=ok`: exporting the same folder again after a room
///   joined the Space exports only that room and keeps the earlier rooms.
pub(super) async fn run_room_history_export_stage(
    config: &QaConfig,
    conn_a: &mut CoreConnection,
    account_key_a: &AccountKey,
) -> Result<(), String> {
    // A disposable member whose device can be denied a room key without
    // affecting the stages that later rely on B's devices.
    let user_c = config
        .user_c
        .as_deref()
        .ok_or("history export requires synthetic user C")?;
    let suffix = user_c
        .strip_prefix("qa_c_")
        .ok_or("history export requires the local QA user naming contract")?;
    let password_c = std::env::var("KOUSHI_LOCAL_QA_PASSWORD_C")
        .unwrap_or_else(|_| format!("koushi-desktop-local-c-{suffix}"));
    let outcome_c = login_synced_participant_for_qa(
        &config.homeserver,
        qa_data_dir("history-export-c"),
        user_c,
        &password_c,
        "Koushi Core QA C",
        "history export C",
        "history export bootstrap C",
        QaParticipantLoginGate::BootstrapNewIdentity,
    )
    .await?;
    let account_key_c = outcome_c.account_key.clone();
    let mut participant_c = QaOwnedRuntimeParticipant::from(outcome_c);
    let result = async {
        let conn_c = &mut participant_c.conn;
        let directory = ExportDirectory(qa_data_dir("history-export"));
        std::fs::create_dir_all(&directory.0)
            .map_err(|_| "history export: prepare export directory".to_owned())?;

        let room_id =
            create_room_for_qa(conn_a, "QA History Export", true, "history export room").await?;
        wait_for_encrypted_room_projection_for_qa(conn_a, &room_id, "history export room").await?;
        let key_a = TimelineKey::room(account_key_a.clone(), room_id.clone());
        subscribe_timeline_for_qa(conn_a, &key_a, "history export timeline").await?;

        let mut before_join = Vec::new();
        for index in 1..=3 {
            before_join.push(
                send_text(
                    conn_a,
                    &key_a,
                    &format!("qa-history-export-before-{index}"),
                    &format!("Synthetic history export message {index}"),
                    "history export send before join",
                )
                .await?,
            );
        }
        invite_user_for_qa(conn_a, &room_id, &account_key_c.0, "history export invite").await?;
        wait_for_invite_in_snapshot(conn_c, &room_id, None, "history export invite").await?;
        accept_invite_for_qa(conn_c, &room_id, "history export join").await?;
        wait_for_encrypted_room_projection_for_qa(conn_c, &room_id, "history export join").await?;
        let key_c = TimelineKey::room(account_key_c.clone(), room_id.clone());
        let initial_c =
            subscribe_timeline_for_qa(conn_c, &key_c, "history export late member timeline")
                .await?;
        let mut after_join = Vec::new();
        for index in 4..=5 {
            after_join.push(
                send_text(
                    conn_a,
                    &key_a,
                    &format!("qa-history-export-after-{index}"),
                    &format!("Synthetic history export message {index}"),
                    "history export send after join",
                )
                .await?,
            );
        }
        // The later member holds the keys for messages sent after they joined
        // once the last one decrypts in their timeline.
        wait_for_item_with_body(
            conn_c,
            &key_c,
            "Synthetic history export message 5",
            "history export late member decrypt",
        )
        .await?;

        // A withholds the next room key from B's device, so B cannot decrypt it.
        let device_c = match &conn_c.snapshot().session {
            koushi_state::SessionState::Ready(info) => koushi_state::VerificationTarget {
                user_id: info.user_id.clone(),
                device_id: info.device_id.clone(),
            },
            _ => return Err("history export: late member is not Ready".to_owned()),
        };
        tokio::time::timeout(
            E2EE_EVENT_TIMEOUT,
            conn_a.qa_set_local_device_blacklisted(device_c, room_id.clone()),
        )
        .await
        .map_err(|_| "history export: block device ack timeout".to_owned())?
        .map_err(|_| "history export: block device failed".to_owned())?;
        let withheld_body = "Synthetic history export withheld message";
        let withheld = send_text(
            conn_a,
            &key_a,
            "qa-history-export-withheld",
            withheld_body,
            "history export withheld send",
        )
        .await?;
        wait_for_withheld_event_projection_from_source(
            conn_c,
            &key_c,
            &withheld,
            withheld_body,
            &initial_c,
            "history export withheld receive",
            E2EE_EVENT_TIMEOUT,
        )
        .await?;
        let sent = before_join.iter().chain(&after_join).collect::<Vec<_>>();

        // Full export by the room creator.
        let full_path = directory.0.join("full");
        let (full, full_progress) = export_room_to_value(
            conn_a,
            &room_id,
            HistoryExportRange::AllAvailable,
            &full_path,
            "history export full",
        )
        .await?;
        check_export(&full, &full_progress, "history export full")?;
        let full_events = messages(&full);
        let sent_events = sent
            .iter()
            .map(|event_id| {
                full_events
                    .iter()
                    .find(|event| event["event_id"].as_str() == Some(event_id.as_str()))
                    .ok_or_else(|| "history export full: a sent message is missing".to_owned())
            })
            .collect::<Result<Vec<_>, _>>()?;
        if !sent_events.iter().all(|event| is_text_message(event)) {
            return Err("history export full: a sent message was not decrypted".to_owned());
        }
        if full_progress.undecryptable_events != 0
            || full_progress.fetched_events < sent.len() as u64
        {
            return Err("history export full: unexpected counts for the room creator".to_owned());
        }
        println!("history_export_full=ok");

        // Period export: [message 2, message 4).
        let start_ms = sent_events[1]["origin_server_ts"]
            .as_u64()
            .unwrap_or_default();
        let end_exclusive_ms = sent_events[3]["origin_server_ts"]
            .as_u64()
            .unwrap_or_default();
        if start_ms >= end_exclusive_ms {
            return Err("history export period: sent messages share a timestamp".to_owned());
        }
        let expected_period = full_events
            .iter()
            .filter(|event| {
                event["origin_server_ts"]
                    .as_u64()
                    .is_some_and(|ts| start_ms <= ts && ts < end_exclusive_ms)
            })
            .filter_map(|event| event["event_id"].as_str().map(str::to_owned))
            .collect::<Vec<_>>();
        let period_path = directory.0.join("period");
        let (period, period_progress) = export_room_to_value(
            conn_a,
            &room_id,
            HistoryExportRange::Period {
                start_ms,
                end_exclusive_ms,
                time_zone: "UTC".to_owned(),
            },
            &period_path,
            "history export period",
        )
        .await?;
        check_export(&period, &period_progress, "history export period")?;
        if event_ids(&period) != expected_period
            || !expected_period.contains(sent[1])
            || expected_period.contains(sent[3])
        {
            return Err(
                "history export period: events differ from the full export range".to_owned(),
            );
        }
        println!("history_export_period=ok");

        // A period long before the room: the seek lands on the room's first
        // event, which is already past the period's margin, so the walk stops
        // after one page. A server that cannot seek reads from the first
        // visible event instead; either way the file is empty.
        let before_path = directory.0.join("period-before");
        let (before, before_progress) = export_room_to_value(
            conn_a,
            &room_id,
            HistoryExportRange::Period {
                start_ms: 0,
                end_exclusive_ms: 1_000,
                time_zone: "UTC".to_owned(),
            },
            &before_path,
            "history export period before room",
        )
        .await?;
        check_export(
            &before,
            &before_progress,
            "history export period before room",
        )?;
        if !messages(&before).is_empty() {
            return Err("history export period before room: exported events".to_owned());
        }
        let bounded = before_progress.fetched_events < full_progress.fetched_events;
        println!("history_export_period_bounded={bounded}");

        // A period after the room: `timestamp_to_event` finds no event, so the
        // export falls back to reading the whole visible history.
        let last_ms = full_events
            .iter()
            .filter_map(|event| event["origin_server_ts"].as_u64())
            .max()
            .unwrap_or_default();
        let after_start_ms = last_ms + 365 * 24 * 60 * 60 * 1000;
        let after_path = directory.0.join("period-after");
        let (after, after_progress) = export_room_to_value(
            conn_a,
            &room_id,
            HistoryExportRange::Period {
                start_ms: after_start_ms,
                end_exclusive_ms: after_start_ms + 24 * 60 * 60 * 1000,
                time_zone: "UTC".to_owned(),
            },
            &after_path,
            "history export period after room",
        )
        .await?;
        check_export(&after, &after_progress, "history export period after room")?;
        if !messages(&after).is_empty()
            || after_progress.fetched_events < full_progress.fetched_events
        {
            return Err(
                "history export period after room: the fallback did not read the history"
                    .to_owned(),
            );
        }
        println!("history_export_period_fallback=ok");

        // The later member cannot decrypt the withheld message.
        let late_path = directory.0.join("late-member");
        let (late, late_progress) = export_room_to_value(
            conn_c,
            &room_id,
            HistoryExportRange::AllAvailable,
            &late_path,
            "history export late member",
        )
        .await?;
        check_export(&late, &late_progress, "history export late member")?;
        let late_events = messages(&late);
        let find = |event_id: &String| {
            late_events
                .iter()
                .find(|event| event["event_id"].as_str() == Some(event_id.as_str()))
        };
        // Whether keys for messages sent before the join are shared depends on
        // the SDK's history-sharing state, so those only need to be present.
        let earlier_present = before_join.iter().all(|event_id| find(event_id).is_some());
        let readable = after_join
            .iter()
            .all(|event_id| find(event_id).is_some_and(is_text_message));
        let withheld_undecryptable = find(&withheld).is_some_and(is_undecryptable);
        if !earlier_present
            || !readable
            || !withheld_undecryptable
            || late_progress.undecryptable_events == 0
        {
            return Err(format!(
                "history export late member: unexpected decryption outcome \
                 (earlier_present={earlier_present} readable={readable} \
                 withheld_undecryptable={withheld_undecryptable} undecryptable_total={})",
                late_progress.undecryptable_events
            ));
        }
        println!("history_export_utd_counted=ok");

        // Stop. A tiny room can finish before the stop reaches the task, so
        // retry a bounded number of times.
        let mut stopped = false;
        for attempt in 0..3 {
            let chosen = directory.0.join(format!("stopped-{attempt}"));
            std::fs::create_dir_all(&chosen)
                .map_err(|_| "history export stop: prepare directory".to_owned())?;
            let export_id = start_export(
                conn_a,
                HistoryExportScope::Room {
                    room_id: room_id.clone(),
                },
                HistoryExportRange::AllAvailable,
                &chosen,
                "history export stop",
            )
            .await?;
            let stop_id = conn_a.next_request_id();
            conn_a
                .command(CoreCommand::Account(AccountCommand::StopHistoryExport {
                    request_id: stop_id,
                    target_request_id: export_id,
                }))
                .await
                .map_err(|_| "history export stop: submit stop".to_owned())?;
            match wait_for_terminal(conn_a, export_id, "history export stop").await? {
                ExportOutcome::Stopped => {
                    let dir = export_dir(&chosen)?;
                    if !no_partial_folders(&dir) {
                        return Err(
                            "history export stop: a stopped export left a partial room".to_owned()
                        );
                    }
                    stopped = true;
                    break;
                }
                ExportOutcome::Completed(_) => {
                    println!("history_export_cancel_race_attempt={attempt}");
                }
                ExportOutcome::Failed(kind) => {
                    return Err(format!("history export stop: export failed kind={kind}"));
                }
            }
        }
        if !stopped {
            return Err("history export stop: export completed before every stop".to_owned());
        }
        println!("history_export_cancel=ok");

        run_space_export(
            config,
            conn_a,
            account_key_a,
            &account_key_c.0,
            &room_id,
            &key_a,
            &directory.0,
        )
        .await?;
        println!("room_history_export=ok");
        Ok(())
    }
    .await;
    let cleanup =
        cleanup_owned_e2ee_participant_best_effort(participant_c, "history export C cleanup").await;
    result.and(cleanup)
}

fn modified(path: &Path) -> Option<std::time::SystemTime> {
    std::fs::metadata(path)
        .and_then(|metadata| metadata.modified())
        .ok()
}

/// Space export, attachments, and resume.
#[allow(clippy::too_many_arguments)]
async fn run_space_export(
    config: &QaConfig,
    conn_a: &mut CoreConnection,
    account_key_a: &AccountKey,
    user_c: &str,
    encrypted_room_id: &str,
    key_a: &TimelineKey,
    directory: &Path,
) -> Result<(), String> {
    // An image in the encrypted room: the export must decrypt it.
    let expected_account = match conn_a.snapshot().session {
        koushi_state::SessionState::Ready(info) => {
            koushi_core::store::session_key_id_from_info(&info)
        }
        _ => return Err("history export space: requires a ready session".to_owned()),
    };
    let media_txn = "qa-history-export-image".to_owned();
    let send_media_id = conn_a.next_request_id();
    conn_a
        .command(CoreCommand::Timeline(
            koushi_protocol::command::TimelineCommand::UploadAndSendMedia {
                request_id: send_media_id,
                expected_account,
                key: key_a.clone(),
                transaction_id: media_txn.clone(),
                request: koushi_protocol::command::UploadMediaRequest {
                    filename: "qa-history-export.png".to_owned(),
                    mime_type: "image/png".to_owned(),
                    bytes: PNG_1X1.to_vec(),
                    kind: koushi_protocol::command::UploadMediaKind::Image {
                        width: Some(1),
                        height: Some(1),
                    },
                    compression: None,
                    thumbnail: None,
                    caption: None,
                },
            },
        ))
        .await
        .map_err(|_| "history export space: submit image".to_owned())?;
    wait_for_media_send_flow_completion(
        conn_a,
        send_media_id,
        key_a,
        &media_txn,
        "history export image",
    )
    .await?;

    let space_id =
        create_space_for_qa(conn_a, "QA History Export Space", "history export space").await?;
    set_space_child_for_qa(
        conn_a,
        &space_id,
        encrypted_room_id,
        "history export space child",
    )
    .await?;
    let plain_room = create_room_for_qa(
        conn_a,
        "QA History Export Plain",
        false,
        "history export plain room",
    )
    .await?;
    set_space_child_for_qa(conn_a, &space_id, &plain_room, "history export plain child").await?;
    let dm_room = start_direct_message_for_qa(conn_a, user_c, "history export dm").await?;
    set_space_child_for_qa(conn_a, &space_id, &dm_room, "history export dm child").await?;
    let _ = account_key_a;
    // The reducer admits a Space export only for a Space in `AppState.spaces`.
    wait_for_space_in_space_list(conn_a, &space_id, "history export space projection").await?;
    wait_for_space_child_projection(
        conn_a,
        &space_id,
        &[
            encrypted_room_id.to_owned(),
            plain_room.clone(),
            dm_room.clone(),
        ],
        "history export space children",
    )
    .await?;

    let chosen = directory.join("space");
    std::fs::create_dir_all(&chosen)
        .map_err(|_| "history export space: prepare directory".to_owned())?;
    let rooms = export_completed(
        conn_a,
        HistoryExportScope::Space {
            space_id: space_id.clone(),
        },
        HistoryExportRange::AllAvailable,
        &chosen,
        "history export space",
    )
    .await?;
    let listed = rooms
        .iter()
        .map(|room| room.room_id.as_str())
        .collect::<BTreeSet<_>>();
    if !listed.contains(encrypted_room_id) || !listed.contains(plain_room.as_str()) {
        return Err("history export space: a joined room was not listed".to_owned());
    }
    if listed.contains(dm_room.as_str()) {
        return Err("history export space: the direct message was exported".to_owned());
    }
    if rooms.iter().any(|room| {
        room.phase != HistoryExportRoomPhase::Completed
            && room.phase != HistoryExportRoomPhase::Skipped
    }) {
        return Err("history export space: a room did not complete".to_owned());
    }
    let dir = export_dir(&chosen)?;
    let folders = room_folders(&dir)?;
    if folders.len() != 2 || !no_partial_folders(&dir) {
        return Err(format!(
            "history export space: expected two room folders, found {}",
            folders.len()
        ));
    }
    let index = std::fs::read_to_string(dir.join("index.html"))
        .map_err(|_| "history export space: read table of contents".to_owned())?;
    if !index.contains("QA History Export Plain") {
        return Err("history export space: the table of contents lacks a room".to_owned());
    }
    println!("history_export_space=ok");

    // The encrypted room's image, decrypted, with a thumbnail.
    let mut image_checked = false;
    for folder in &folders {
        let attachments: Value = serde_json::from_slice(
            &std::fs::read(folder.join("attachments.json"))
                .map_err(|_| "history export attachments: read attachments.json".to_owned())?,
        )
        .map_err(|_| "history export attachments: attachments.json is not JSON".to_owned())?;
        let Some(record) = attachments["attachments"].as_array().and_then(|records| {
            records
                .iter()
                .find(|record| record["name"] == "qa-history-export.png")
        }) else {
            continue;
        };
        let file = record["file"]
            .as_str()
            .ok_or("history export attachments: image not retrieved")?;
        let thumb = record["thumb"]
            .as_str()
            .ok_or("history export attachments: no thumbnail")?;
        let bytes = std::fs::read(folder.join(file))
            .map_err(|_| "history export attachments: read image".to_owned())?;
        let thumb_bytes = std::fs::read(folder.join(thumb))
            .map_err(|_| "history export attachments: read thumbnail".to_owned())?;
        if bytes != PNG_1X1 || !thumb_bytes.starts_with(&[0xFF, 0xD8]) {
            return Err("history export attachments: image or thumbnail differs".to_owned());
        }
        image_checked = true;
    }
    if !image_checked {
        return Err("history export attachments: the image was not exported".to_owned());
    }
    println!("history_export_attachments=ok");

    // Resume: a room joins the Space; exporting the same folder adds only it.
    let before = folders
        .iter()
        .map(|folder| modified(&folder.join("messages.json")))
        .collect::<Vec<_>>();
    let added_room = create_room_for_qa(
        conn_a,
        "QA History Export Added",
        false,
        "history export added room",
    )
    .await?;
    set_space_child_for_qa(conn_a, &space_id, &added_room, "history export added child").await?;
    wait_for_space_child_projection(
        conn_a,
        &space_id,
        &[
            encrypted_room_id.to_owned(),
            plain_room.clone(),
            dm_room.clone(),
            added_room.clone(),
        ],
        "history export added child projection",
    )
    .await?;
    let rooms = export_completed(
        conn_a,
        HistoryExportScope::Space { space_id },
        HistoryExportRange::AllAvailable,
        &dir,
        "history export resume",
    )
    .await?;
    if !rooms
        .iter()
        .any(|room| room.room_id == added_room && room.phase == HistoryExportRoomPhase::Completed)
    {
        return Err("history export resume: the added room was not exported".to_owned());
    }
    let after = folders
        .iter()
        .map(|folder| modified(&folder.join("messages.json")))
        .collect::<Vec<_>>();
    if before != after || room_folders(&dir)?.len() != 3 {
        return Err(
            "history export resume: earlier rooms were rewritten or the new room is missing"
                .to_owned(),
        );
    }
    println!("history_export_resume=ok");
    Ok(())
}
