use koushi_sdk::{MatrixInvitePreview, MatrixRoomListSnapshot};

#[test]
fn normalize_invites_preserves_preview_fields_including_structural_name_placeholder() {
    let snapshot = MatrixRoomListSnapshot {
        invites: vec![MatrixInvitePreview {
            room_id: "!invite:example.test".to_owned(),
            display_name: "Empty Room".to_owned(),
            display_name_placeholder: Some(koushi_state::RoomNamePlaceholder::Empty),
            avatar_mxc_uri: None,
            topic: Some("Project topic".to_owned()),
            inviter_display_name: Some("Inviter".to_owned()),
            inviter_user_id: Some("@inviter:example.test".to_owned()),
            is_dm: false,
            is_space: true,
        }],
        ..MatrixRoomListSnapshot::default()
    };
    let invites = super::normalize_invites(&snapshot);

    assert_eq!(invites.len(), 1);
    assert_eq!(invites[0].room_id, "!invite:example.test");
    assert_eq!(invites[0].display_name, "Empty Room");
    assert_eq!(
        invites[0].display_name_placeholder,
        Some(koushi_state::RoomNamePlaceholder::Empty)
    );
    assert_eq!(invites[0].topic.as_deref(), Some("Project topic"));
    assert_eq!(invites[0].inviter_display_name.as_deref(), Some("Inviter"));
    assert!(!invites[0].is_dm);
    assert!(invites[0].is_space);
}
