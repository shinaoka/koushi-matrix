use matrix_desktop_state::RoomAttentionKind;

#[test]
fn room_attention_summary_from_counts_preserves_unread_fallback() {
    let summary = matrix_desktop_sdk::room_attention_summary_from_counts(
        Some("Room A".to_owned()),
        false,
        0,
        0,
        3,
        false,
    )
    .unwrap();

    assert_eq!(summary.kind, RoomAttentionKind::Message);
    assert_eq!(summary.notification_count, 0);
    assert_eq!(summary.highlight_count, 0);
    assert_eq!(summary.unread_count, 3);
}

#[test]
fn room_attention_summary_from_counts_uses_marked_unread_fallback() {
    let summary = matrix_desktop_sdk::room_attention_summary_from_counts(
        Some("Room A".to_owned()),
        false,
        0,
        0,
        0,
        true,
    )
    .unwrap();

    assert_eq!(summary.kind, RoomAttentionKind::Message);
    assert_eq!(summary.unread_count, 1);
}

#[test]
fn room_attention_summary_from_counts_prefers_dm_and_mentions() {
    let dm_summary = matrix_desktop_sdk::room_attention_summary_from_counts(
        Some("Direct".to_owned()),
        true,
        4,
        0,
        4,
        false,
    )
    .unwrap();
    assert_eq!(dm_summary.kind, RoomAttentionKind::Dm);

    let mention_summary = matrix_desktop_sdk::room_attention_summary_from_counts(
        Some("Direct".to_owned()),
        true,
        4,
        2,
        4,
        false,
    )
    .unwrap();
    assert_eq!(mention_summary.kind, RoomAttentionKind::Mention);
}

#[test]
fn room_attention_summary_from_counts_uses_private_safe_blank_room_label() {
    let summary = matrix_desktop_sdk::room_attention_summary_from_counts(
        Some("   ".to_owned()),
        false,
        2,
        0,
        2,
        false,
    )
    .unwrap();

    assert_eq!(summary.room_display_name, "Room");
}
