use koushi_sdk::preview_room_address;
use koushi_state::RoomAddressError;

#[test]
fn previews_the_exact_localpart_on_the_accounts_matrix_server() {
    let preview = preview_room_address(
        "Example Room",
        None,
        None,
        Some("@member:example.invalid:8448"),
    );
    assert_eq!(preview.localpart, "example-room");
    assert_eq!(
        preview.full_alias.as_deref(),
        Some("#example-room:example.invalid:8448")
    );
    assert_eq!(preview.error, None);
    let manual = preview_room_address(
        "Another Name",
        Some("custom"),
        None,
        Some("@member:example.invalid"),
    );
    assert_eq!(
        manual.full_alias.as_deref(),
        Some("#custom:example.invalid")
    );
    assert_eq!(
        preview_room_address("設計の相談", None, None, Some("@member:example.invalid")).error,
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
        let preview =
            preview_room_address("Name", Some(alias), None, Some("@member:example.invalid"));
        assert_eq!(preview.error, Some(RoomAddressError::Invalid));
        assert!(preview.full_alias.is_none());
    }
    assert_eq!(
        preview_room_address("Name", Some(""), None, Some("@member:example.invalid")).error,
        Some(RoomAddressError::Empty)
    );
    assert_eq!(
        preview_room_address("Name", None, None, None).error,
        Some(RoomAddressError::NotReady)
    );
}

#[test]
fn preview_debug_never_formats_entered_content_or_identity() {
    let preview = preview_room_address(
        "Synthetic Secret",
        None,
        Some("Synthetic Workspace"),
        Some("@member:example.invalid"),
    );
    let debug = format!("{preview:?}");
    assert!(!debug.contains("synthetic-secret"));
    assert!(!debug.contains("example.invalid"));
}

/// #1006: an unedited address from a Space is `<space>-<room>`; the server is
/// reported so the dialog can explain the alias scope.
#[test]
fn space_rooms_are_suggested_with_the_space_prefix_on_the_accounts_server() {
    let preview = preview_room_address(
        "papers",
        None,
        Some("research-group"),
        Some("@member:example.invalid"),
    );
    assert_eq!(preview.localpart, "research-group-papers");
    assert_eq!(
        preview.full_alias.as_deref(),
        Some("#research-group-papers:example.invalid")
    );
    assert_eq!(preview.server_name.as_deref(), Some("example.invalid"));

    // Normalization applies to both parts; non-Latin letters are kept.
    assert_eq!(
        preview_room_address(
            "Weekly Papers",
            None,
            Some("研究 グループ"),
            Some("@m:example.invalid")
        )
        .localpart,
        "研究-グループ-weekly-papers"
    );
    // A Space name without usable characters leaves the room-only suggestion.
    assert_eq!(
        preview_room_address("papers", None, Some("!!!"), Some("@m:example.invalid")).localpart,
        "papers"
    );
    // A room name without usable characters still needs a manual address.
    assert_eq!(
        preview_room_address(
            "!!!",
            None,
            Some("research-group"),
            Some("@m:example.invalid")
        )
        .error,
        Some(RoomAddressError::Empty)
    );
    // A manually edited address is never prefixed.
    assert_eq!(
        preview_room_address(
            "papers",
            Some("papers-2026"),
            Some("research-group"),
            Some("@m:example.invalid")
        )
        .localpart,
        "papers-2026"
    );
}

#[test]
fn an_overlong_space_prefix_is_dropped_instead_of_invalidating_the_suggestion() {
    let long_space = "s".repeat(240);
    let preview = preview_room_address(
        "papers",
        None,
        Some(&long_space),
        Some("@member:example.invalid"),
    );
    assert_eq!(preview.error, None);
    assert_eq!(preview.localpart, "papers");
}
