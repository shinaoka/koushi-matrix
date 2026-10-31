use koushi_state::default_room_alias_localpart;

/// #1416: the default address is a random per-session suffix behind an ASCII
/// prefix from the attachment Space, never derived from the room name.
#[test]
fn the_default_address_is_an_ascii_space_prefix_and_the_session_suffix() {
    assert_eq!(
        default_room_alias_localpart(Some("Research Group"), "3fa9c2"),
        "research-group-3fa9c2"
    );
    assert_eq!(
        default_room_alias_localpart(Some("  Design / Team 2026 "), "3fa9c2"),
        "design-team-2026-3fa9c2"
    );
    // At Home, or for a Space name without ASCII letters or digits, the
    // prefix is `room`.
    assert_eq!(default_room_alias_localpart(None, "3fa9c2"), "room-3fa9c2");
    assert_eq!(
        default_room_alias_localpart(Some("春学"), "3fa9c2"),
        "room-3fa9c2"
    );
    assert_eq!(
        default_room_alias_localpart(Some("🎉 !!!"), "3fa9c2"),
        "room-3fa9c2"
    );
    // Mixed names keep only their ASCII segments.
    assert_eq!(
        default_room_alias_localpart(Some("物性 Lab"), "3fa9c2"),
        "lab-3fa9c2"
    );
}

#[test]
fn a_long_space_prefix_is_bounded_without_a_trailing_separator() {
    let localpart =
        default_room_alias_localpart(Some("alpha beta gamma delta epsilon zeta"), "3fa9c2");
    let prefix = localpart.strip_suffix("-3fa9c2").expect("suffix kept");
    assert!(prefix.len() <= 24, "{prefix}");
    assert!(!prefix.ends_with('-'), "{prefix}");
    assert!(prefix.starts_with("alpha-beta"), "{prefix}");
}
