use matrix_desktop_state::{RoomAttentionKind, room_attention_summary};
use serde_json::json;

#[test]
fn room_attention_summary_serializes_only_allowed_fields() {
    let summary = room_attention_summary("Room A".to_owned(), false, 7, 2, 7).unwrap();

    assert_eq!(
        serde_json::to_value(summary).unwrap(),
        json!({
            "room_display_name": "Room A",
            "kind": "mention",
            "notification_count": 7,
            "highlight_count": 2,
            "unread_count": 7,
        })
    );
}

#[test]
fn room_attention_summary_omits_payload_when_unread_is_absent() {
    assert_eq!(
        room_attention_summary("Room A".to_owned(), false, 0, 0, 0),
        None
    );
}

#[test]
fn room_attention_kind_prefers_mentions_over_dm_and_message() {
    assert_eq!(
        matrix_desktop_state::room_attention_kind(true, 4, 2, 4),
        Some(RoomAttentionKind::Mention)
    );
    assert_eq!(
        matrix_desktop_state::room_attention_kind(true, 4, 0, 4),
        Some(RoomAttentionKind::Dm)
    );
    assert_eq!(
        matrix_desktop_state::room_attention_kind(false, 4, 0, 4),
        Some(RoomAttentionKind::Message)
    );
}
