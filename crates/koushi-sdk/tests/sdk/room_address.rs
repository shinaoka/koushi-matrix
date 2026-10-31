use koushi_sdk::{preview_room_address, random_room_address_suffix};
use koushi_state::RoomAddressError;

#[test]
fn previews_the_exact_localpart_on_the_accounts_matrix_server() {
    let preview = preview_room_address("research-papers", Some("@member:example.invalid:8448"));
    assert_eq!(preview.localpart, "research-papers");
    assert_eq!(
        preview.full_alias.as_deref(),
        Some("#research-papers:example.invalid:8448")
    );
    assert_eq!(preview.error, None);
    assert_eq!(preview.server_name.as_deref(), Some("example.invalid:8448"));
    assert_eq!(
        preview_room_address("設計の相談", Some("@member:example.invalid")).error,
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
        let preview = preview_room_address(alias, Some("@member:example.invalid"));
        assert_eq!(preview.error, Some(RoomAddressError::Invalid));
        assert!(preview.full_alias.is_none());
    }
    for empty in ["", "   "] {
        assert_eq!(
            preview_room_address(empty, Some("@member:example.invalid")).error,
            Some(RoomAddressError::Empty)
        );
    }
    assert_eq!(
        preview_room_address("room", None).error,
        Some(RoomAddressError::NotReady)
    );
}

#[test]
fn preview_debug_never_formats_entered_content_or_identity() {
    let preview = preview_room_address("synthetic-secret", Some("@member:example.invalid"));
    let debug = format!("{preview:?}");
    assert!(!debug.contains("synthetic-secret"));
    assert!(!debug.contains("example.invalid"));
}

/// #1416: each create session gets a fresh, valid suffix.
#[test]
fn random_suffixes_are_six_lowercase_hex_characters_and_differ() {
    let first = random_room_address_suffix();
    let second = random_room_address_suffix();
    for suffix in [&first, &second] {
        assert_eq!(suffix.len(), 6, "{suffix}");
        assert!(
            suffix
                .chars()
                .all(|character| character.is_ascii_digit() || ('a'..='f').contains(&character)),
            "{suffix}"
        );
    }
    assert_ne!(first, second);
}
