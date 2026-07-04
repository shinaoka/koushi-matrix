use std::{
    collections::{BTreeSet, hash_map::DefaultHasher},
    hash::{Hash, Hasher},
    sync::{Mutex, OnceLock},
};

use koushi_state::RoomSummary;

const ENV_VAR: &str = "KOUSHI_UNREAD_TRACE";

pub(crate) fn enabled() -> bool {
    std::env::var_os(ENV_VAR).is_some()
}

fn id_token(value: &str) -> String {
    let mut hasher = DefaultHasher::new();
    value.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

fn room_token(room_id: &str) -> String {
    id_token(room_id)
}

fn event_token(event_id: Option<&str>) -> String {
    event_id
        .filter(|event_id| !event_id.trim().is_empty())
        .map(id_token)
        .unwrap_or_else(|| "none".to_owned())
}

fn room_metrics(stage: &str, room: &RoomSummary) -> String {
    let latest_event_id = room
        .latest_event
        .as_ref()
        .map(|event| event.event_id.as_str());
    format!(
        "koushi.unread_trace stage={stage} room={} unread_count={} notification_count={} highlight_count={} marked_unread={} latest_event_present={} latest_event={}",
        room_token(&room.room_id),
        room.unread_count,
        room.notification_count,
        room.highlight_count,
        room.marked_unread,
        latest_event_id.is_some(),
        event_token(latest_event_id)
    )
}

fn room_has_unread_metrics(room: &RoomSummary) -> bool {
    room.unread_count > 0
        || room.notification_count > 0
        || room.highlight_count > 0
        || room.marked_unread
}

pub(crate) fn trace_room_list_snapshot(rooms: &[RoomSummary]) {
    if !enabled() {
        return;
    }
    for room in rooms.iter().filter(|room| room_has_unread_metrics(room)) {
        eprintln!("{}", room_metrics("room_list_snapshot", room));
    }
}

fn room_list_applied_lines(
    raw_rooms: &[RoomSummary],
    applied_rooms: &[RoomSummary],
) -> Vec<String> {
    let raw_unread_room_ids = raw_rooms
        .iter()
        .filter(|room| room_has_unread_metrics(room))
        .map(|room| room.room_id.as_str())
        .collect::<BTreeSet<_>>();
    applied_rooms
        .iter()
        .filter(|room| {
            raw_unread_room_ids.contains(room.room_id.as_str()) || room_has_unread_metrics(room)
        })
        .map(|room| room_metrics("room_list_applied", room))
        .collect()
}

pub(crate) fn trace_room_list_applied(raw_rooms: &[RoomSummary], applied_rooms: &[RoomSummary]) {
    if !enabled() {
        return;
    }
    for line in room_list_applied_lines(raw_rooms, applied_rooms) {
        eprintln!("{line}");
    }
}

fn activity_trace_seen_lines() -> &'static Mutex<BTreeSet<String>> {
    static SEEN: OnceLock<Mutex<BTreeSet<String>>> = OnceLock::new();
    SEEN.get_or_init(|| Mutex::new(BTreeSet::new()))
}

fn activity_room_line(
    stage: &str,
    room: &RoomSummary,
    emitted: bool,
    reason: &str,
) -> Option<String> {
    if room_has_unread_metrics(room) {
        Some(format!(
            "{} emitted={} reason={reason}",
            room_metrics(stage, room),
            emitted
        ))
    } else {
        None
    }
}

fn dedupe_activity_trace_line(seen: &mut BTreeSet<String>, line: String) -> Option<String> {
    if seen.insert(line.clone()) {
        Some(line)
    } else {
        None
    }
}

pub(crate) fn trace_activity_room(stage: &str, room: &RoomSummary, emitted: bool, reason: &str) {
    if !enabled() {
        return;
    }
    let Some(line) = activity_room_line(stage, room, emitted, reason) else {
        return;
    };
    let Ok(mut seen) = activity_trace_seen_lines().lock() else {
        eprintln!("{line}");
        return;
    };
    if let Some(line) = dedupe_activity_trace_line(&mut seen, line) {
        eprintln!("{line}");
    }
}

fn mark_read_line(stage: &str, request_id: u64, room_id: &str, event_id: Option<&str>) -> String {
    let event_id_present = event_id.is_some_and(|event_id| !event_id.trim().is_empty());
    format!(
        "koushi.unread_trace stage={stage} request_id={request_id} room={} event_id_present={event_id_present} event={}",
        room_token(room_id),
        event_token(event_id)
    )
}

pub(crate) fn trace_mark_read(stage: &str, request_id: u64, room_id: &str, event_id: Option<&str>) {
    if enabled() {
        eprintln!("{}", mark_read_line(stage, request_id, room_id, event_id));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use koushi_state::{RoomLatestEventSummary, RoomTags};

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
            last_activity_ms: 42,
            latest_event: Some(RoomLatestEventSummary {
                event_id: "$private-event:example.invalid".to_owned(),
                sender_id: Some("@private-sender:example.invalid".to_owned()),
                sender_label: Some("Private Sender".to_owned()),
                sender_avatar: None,
                preview: Some("private body".to_owned()),
                timestamp_ms: 42,
            }),
            parent_space_ids: Vec::new(),
            dm_space_ids: Vec::new(),
            is_encrypted: false,
            joined_members: 2,
        }
    }

    #[test]
    fn room_metrics_line_is_private_data_free() {
        let line = room_metrics("activity_placeholder", &private_room());

        assert!(line.contains("koushi.unread_trace stage=activity_placeholder"));
        assert!(line.contains("unread_count=3"));
        assert!(line.contains("notification_count=2"));
        assert!(line.contains("highlight_count=1"));
        assert!(line.contains("marked_unread=true"));
        assert!(line.contains("latest_event_present=true"));
        assert!(line.contains("latest_event="));
        for private_value in [
            "!private-room:example.invalid",
            "$private-event:example.invalid",
            "@private-sender:example.invalid",
            "Private Room",
            "Private Sender",
            "private body",
        ] {
            assert!(
                !line.contains(private_value),
                "trace leaked {private_value}: {line}"
            );
        }
    }

    #[test]
    fn mark_read_line_is_private_data_free() {
        let room_id = "!private-room:example.invalid";
        let event_id = "$private-event:example.invalid";
        let line = mark_read_line("mark_read_success", 7, room_id, Some(event_id));
        assert!(!line.contains(room_id), "{line}");
        assert!(!line.contains(event_id), "{line}");
        assert!(line.contains("request_id=7"));
        assert!(line.contains("event_id_present=true"));
        assert!(line.contains("event="));
    }

    #[test]
    fn room_list_applied_lines_include_cleared_raw_unread_rooms() {
        let raw = private_room();
        let mut applied = raw.clone();
        applied.unread_count = 0;
        applied.notification_count = 0;
        applied.highlight_count = 0;
        applied.marked_unread = false;

        let lines = room_list_applied_lines(&[raw], &[applied]);

        assert_eq!(lines.len(), 1);
        assert!(lines[0].contains("stage=room_list_applied"));
        assert!(lines[0].contains("unread_count=0"));
        assert!(lines[0].contains("notification_count=0"));
        assert!(lines[0].contains("highlight_count=0"));
        assert!(lines[0].contains("marked_unread=false"));
    }

    #[test]
    fn activity_trace_lines_are_deduped_by_full_line() {
        let room = private_room();
        let first = activity_room_line("activity_recent_event", &room, false, "plain_unread_only")
            .expect("unread room should produce an activity line");
        let repeated =
            activity_room_line("activity_recent_event", &room, false, "plain_unread_only")
                .expect("same unread room should produce same candidate line");
        let different_reason = activity_room_line("activity_recent_event", &room, true, "unread")
            .expect("different outcome should produce a candidate line");
        let mut seen = BTreeSet::new();

        assert_eq!(
            dedupe_activity_trace_line(&mut seen, first.clone()).as_deref(),
            Some(first.as_str())
        );
        assert_eq!(dedupe_activity_trace_line(&mut seen, repeated), None);
        assert_eq!(
            dedupe_activity_trace_line(&mut seen, different_reason.clone()).as_deref(),
            Some(different_reason.as_str())
        );
    }
}
