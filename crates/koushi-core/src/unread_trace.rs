use std::collections::{BTreeMap, BTreeSet, HashMap};

use koushi_diagnostics::{DiagnosticEvent, DiagnosticField, DiagnosticLevel};
use koushi_state::{RoomNotificationMode, RoomSummary, room_attention_projection};

fn unread_stage_token(value: &str) -> &'static str {
    match value {
        "room_list_snapshot" => "room_list_snapshot",
        "room_list_applied" => "room_list_applied",
        "activity_recent_event" => "activity_recent_event",
        "activity_placeholder" => "activity_placeholder",
        "mark_read_requested" => "mark_read_requested",
        "mark_read_success" => "mark_read_success",
        "mark_read_failed" => "mark_read_failed",
        "set_fully_read_requested" => "set_fully_read_requested",
        "set_fully_read_failed" => "set_fully_read_failed",
        "set_fully_read_private_receipt_target" => "set_fully_read_private_receipt_target",
        "set_fully_read_success" => "set_fully_read_success",
        _ => "other",
    }
}

fn unread_reason_token(value: &str) -> &'static str {
    match value {
        "unread" => "unread",
        "plain_unread_only" => "plain_unread_only",
        "fully_read_latest" => "fully_read_latest",
        "cleared_latest" => "cleared_latest",
        "cleared_local" => "cleared_local",
        "latest_event_read" => "latest_event_read",
        "room_metrics" => "room_metrics",
        _ => "other",
    }
}

fn record_room_metrics(
    stage: &str,
    room: &RoomSummary,
    mode: Option<RoomNotificationMode>,
    emitted: Option<bool>,
    reason: Option<&str>,
) {
    let projection = room_attention_projection(room, mode);
    let mode = match mode {
        Some(RoomNotificationMode::All) => "all",
        Some(RoomNotificationMode::Mentions) => "mentions",
        Some(RoomNotificationMode::Mute) => "mute",
        None => "unknown",
    };
    let mut event = DiagnosticEvent::new(
        DiagnosticLevel::Debug,
        "core.unread",
        unread_stage_token(stage),
    )
    .field(DiagnosticField::count("unread", room.unread_count))
    .field(DiagnosticField::count(
        "notifications",
        room.notification_count,
    ))
    .field(DiagnosticField::count("highlights", room.highlight_count))
    .field(DiagnosticField::boolean(
        "marked_unread",
        room.marked_unread,
    ))
    .field(DiagnosticField::token("notification_mode", mode))
    .field(DiagnosticField::count(
        "display_count",
        projection.display_count,
    ))
    .field(DiagnosticField::boolean(
        "has_unread_content",
        projection.has_unread_content,
    ))
    .field(DiagnosticField::boolean(
        "is_attention_highlighted",
        projection.is_attention_highlighted,
    ))
    .field(DiagnosticField::boolean(
        "has_unread_mention",
        projection.has_unread_mention,
    ))
    .field(DiagnosticField::boolean("is_muted", projection.is_muted))
    .field(DiagnosticField::boolean(
        "latest_event_present",
        room.latest_event.is_some(),
    ));
    if let Some(emitted) = emitted {
        event = event.field(DiagnosticField::boolean("emitted", emitted));
    }
    if let Some(reason) = reason {
        event = event.field(DiagnosticField::token(
            "reason",
            unread_reason_token(reason),
        ));
    }
    koushi_diagnostics::record(event);
}

fn room_has_unread_metrics(room: &RoomSummary) -> bool {
    room.unread_count > 0
        || room.notification_count > 0
        || room.highlight_count > 0
        || room.marked_unread
}

pub(crate) struct RoomListAppliedTraceInput {
    raw_unread_room_ids: BTreeSet<String>,
    notification_modes: BTreeMap<String, RoomNotificationMode>,
}

pub(crate) fn capture_room_list_applied(
    rooms: &[RoomSummary],
    room_notification_settings: &HashMap<String, koushi_state::RoomNotificationSettings>,
) -> RoomListAppliedTraceInput {
    RoomListAppliedTraceInput {
        raw_unread_room_ids: rooms
            .iter()
            .filter(|room| room_has_unread_metrics(room))
            .map(|room| room.room_id.clone())
            .collect(),
        notification_modes: room_notification_settings
            .iter()
            .map(|(room_id, settings)| (room_id.clone(), settings.mode))
            .collect(),
    }
}

pub(crate) fn trace_room_list_snapshot(rooms: &[RoomSummary]) {
    for room in rooms.iter().filter(|room| room_has_unread_metrics(room)) {
        record_room_metrics("room_list_snapshot", room, None, None, None);
    }
}

pub(crate) fn trace_room_list_applied(
    input: &RoomListAppliedTraceInput,
    applied_rooms: &[RoomSummary],
) {
    for room in applied_rooms.iter().filter(|room| {
        input.raw_unread_room_ids.contains(room.room_id.as_str()) || room_has_unread_metrics(room)
    }) {
        record_room_metrics(
            "room_list_applied",
            room,
            input.notification_modes.get(&room.room_id).copied(),
            None,
            None,
        );
    }
}

pub(crate) fn trace_activity_room(
    stage: &str,
    room: &RoomSummary,
    mode: Option<RoomNotificationMode>,
    emitted: bool,
    reason: &str,
) {
    if !room_has_unread_metrics(room) {
        return;
    }
    record_room_metrics(stage, room, mode, Some(emitted), Some(reason));
}

pub(crate) fn trace_mark_read(stage: &str, request_id: u64, room_id: &str, event_id: Option<&str>) {
    koushi_diagnostics::record(
        DiagnosticEvent::new(
            DiagnosticLevel::Debug,
            "core.unread",
            unread_stage_token(stage),
        )
        .field(DiagnosticField::count("request_id", request_id))
        .field(DiagnosticField::boolean(
            "room_present",
            !room_id.trim().is_empty(),
        ))
        .field(DiagnosticField::boolean(
            "event_present",
            event_id.is_some_and(|event_id| !event_id.trim().is_empty()),
        )),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use koushi_state::{RoomLatestEventSummary, RoomTags};
    use std::collections::HashMap;

    fn private_room() -> RoomSummary {
        RoomSummary {
            room_id: "!private-room:example.invalid".to_owned(),
            display_name: "Private Room".to_owned(),
            display_label: "Private Room".to_owned(),
            original_display_label: "Private Room".to_owned(),
            avatar: None,
            is_dm: false,
            dm_user_ids: Vec::new(),
            tags: RoomTags::default(),
            unread_count: 3,
            notification_count: 2,
            highlight_count: 1,
            marked_unread: true,
            recency_stamp: Some(42),
            conversation_activity: None,
            latest_event: Some(RoomLatestEventSummary {
                event_id: "$private-event:example.invalid".to_owned(),
                relation_type: None,
                relation_event_id: None,
                sender_id: Some("@private-sender:example.invalid".to_owned()),
                sender_label: Some("Private Sender".to_owned()),
                sender_avatar: None,
                preview: Some("private body".to_owned()),
                timestamp_ms: 42,
                is_redacted: false,
            }),
            parent_space_ids: Vec::new(),
            dm_space_ids: Vec::new(),
            is_encrypted: false,
            joined_members: 2,
        }
    }

    #[test]
    fn unread_helpers_collect_typed_records_without_trace_env() {
        let _diagnostic_lock = koushi_diagnostics::test_support::lock();
        let room = private_room();
        trace_room_list_snapshot(std::slice::from_ref(&room));
        let input = capture_room_list_applied(std::slice::from_ref(&room), &HashMap::new());
        trace_room_list_applied(&input, std::slice::from_ref(&room));
        trace_activity_room("activity_recent_event", &room, None, true, "unread");
        trace_mark_read(
            "mark_read_success",
            77,
            "!private-room:example.invalid",
            Some("$private-event:example.invalid"),
        );

        for stage in [
            "mark_read_requested",
            "mark_read_failed",
            "set_fully_read_requested",
            "set_fully_read_failed",
            "set_fully_read_private_receipt_target",
            "set_fully_read_success",
        ] {
            trace_mark_read(stage, 78, "!private-room:example.invalid", None);
        }
        for reason in [
            "plain_unread_only",
            "unread",
            "fully_read_latest",
            "cleared_latest",
            "cleared_local",
            "latest_event_read",
            "room_metrics",
        ] {
            trace_activity_room("activity_recent_event", &room, None, true, reason);
        }

        let records = koushi_diagnostics::snapshot().records;
        for stage in [
            "room_list_snapshot",
            "room_list_applied",
            "activity_recent_event",
            "mark_read_success",
        ] {
            let event = records
                .iter()
                .find(|record| record.event.source == "core.unread" && record.event.stage == stage)
                .expect("typed unread diagnostic");
            assert!(
                event
                    .event
                    .fields
                    .iter()
                    .any(|field| matches!(field.key, "unread" | "request_id"))
            );
            let serialized = serde_json::to_string(&event.event).expect("serialize diagnostic");
            for private_value in [
                "!private-room:example.invalid",
                "$private-event:example.invalid",
                "@private-sender:example.invalid",
                "private body",
            ] {
                assert!(
                    !serialized.contains(private_value),
                    "leaked {private_value}"
                );
            }
        }

        let records = koushi_diagnostics::snapshot().records;
        for record in records
            .iter()
            .filter(|record| record.event.source == "core.unread")
        {
            for field in &record.event.fields {
                if matches!(
                    field.value,
                    koushi_diagnostics::DiagnosticValue::Token("other")
                ) {
                    panic!("live unread diagnostic collapsed to other: {record:?}");
                }
            }
        }
    }

    #[test]
    fn activity_plain_unread_non_candidates_are_traced() {
        let _diagnostic_lock = koushi_diagnostics::test_support::lock();
        let room = private_room();

        trace_activity_room(
            "activity_recent_event",
            &room,
            None,
            false,
            "plain_unread_only",
        );
        trace_activity_room(
            "activity_placeholder",
            &room,
            None,
            false,
            "plain_unread_only",
        );

        let records = koushi_diagnostics::snapshot().records;
        let traced = records
            .iter()
            .filter(|record| {
                record.event.source == "core.unread"
                    && record.event.fields.iter().any(|field| {
                        field.key == "reason"
                            && matches!(
                                field.value,
                                koushi_diagnostics::DiagnosticValue::Token("plain_unread_only")
                            )
                    })
                    && record.event.fields.iter().any(|field| {
                        field.key == "emitted"
                            && matches!(
                                field.value,
                                koushi_diagnostics::DiagnosticValue::Boolean(false)
                            )
                    })
            })
            .count();
        assert!(
            traced >= 2,
            "plain-unread-only activity diagnostics are required"
        );
    }
}
