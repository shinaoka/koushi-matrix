use koushi_state::{RoomMemberMembership, RoomMemberSummary};

#[test]
fn legacy_member_role_does_not_imply_joined_membership() {
    let mut wire = serde_json::json!({
        "user_id": "@member:example.invalid",
        "display_name": "Sample Member",
        "display_label": "Sample Member",
        "avatar_url": null,
        "power_level": 0,
        "role": "user"
    });
    let legacy: RoomMemberSummary = serde_json::from_value(wire.clone()).unwrap();
    assert_eq!(legacy.membership, RoomMemberMembership::Unknown);
    for (value, expected) in [
        ("joined", RoomMemberMembership::Joined),
        ("invited", RoomMemberMembership::Invited),
    ] {
        wire["membership"] = value.into();
        let member: RoomMemberSummary = serde_json::from_value(wire.clone()).unwrap();
        assert_eq!(member.membership, expected);
        assert_eq!(serde_json::to_value(member).unwrap()["membership"], value);
    }
}
