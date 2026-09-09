use koushi_sdk::preview_room_address;
use koushi_state::RoomAddressError;

#[test]
fn previews_the_exact_localpart_on_the_accounts_matrix_server() {
    let preview = preview_room_address("Example Room", None, Some("@member:example.invalid:8448"));
    assert_eq!(preview.localpart, "example-room");
    assert_eq!(
        preview.full_alias.as_deref(),
        Some("#example-room:example.invalid:8448")
    );
    assert_eq!(preview.error, None);
    let manual = preview_room_address(
        "Another Name",
        Some("custom"),
        Some("@member:example.invalid"),
    );
    assert_eq!(
        manual.full_alias.as_deref(),
        Some("#custom:example.invalid")
    );
    assert_eq!(
        preview_room_address("設計の相談", None, Some("@member:example.invalid")).error,
        None
    );
}

#[test]
fn invalid_and_empty_drafts_do_not_produce_a_shareable_alias() {
    for alias in [
        "#room",
        "room:server",
        "room name",
        "room\nname",
        &"a".repeat(300),
    ] {
        let preview = preview_room_address("Name", Some(alias), Some("@member:example.invalid"));
        assert_eq!(preview.error, Some(RoomAddressError::Invalid));
        assert!(preview.full_alias.is_none());
    }
    assert_eq!(
        preview_room_address("Name", Some(""), Some("@member:example.invalid")).error,
        Some(RoomAddressError::Empty)
    );
    assert_eq!(
        preview_room_address("Name", None, None).error,
        Some(RoomAddressError::NotReady)
    );
}

#[test]
fn preview_debug_never_formats_entered_content_or_identity() {
    let preview = preview_room_address("Synthetic Secret", None, Some("@member:example.invalid"));
    let debug = format!("{preview:?}");
    assert!(!debug.contains("synthetic-secret"));
    assert!(!debug.contains("example.invalid"));
}
