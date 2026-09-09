use koushi_state::suggest_room_alias_localpart;

#[test]
fn room_name_suggestions_are_editable_local_parts_not_full_aliases() {
    assert_eq!(suggest_room_alias_localpart("Example Room"), "example-room");
    assert_eq!(
        suggest_room_alias_localpart("  Example   Room  "),
        "example-room"
    );
    assert_eq!(suggest_room_alias_localpart("設計の相談"), "設計の相談");
    assert_eq!(
        suggest_room_alias_localpart("Example / 設計"),
        "example-設計"
    );
    assert_eq!(suggest_room_alias_localpart(""), "");
    assert_eq!(suggest_room_alias_localpart(" # : / 😀 "), "");
}
