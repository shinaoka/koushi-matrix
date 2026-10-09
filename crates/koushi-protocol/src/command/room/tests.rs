use super::super::{CoreCommand, test_support::fake_rid};
use super::*;

#[test]
fn pin_event_debug_redacts_room_and_event_ids() {
    let pin = RoomCommand::PinEvent {
        request_id: fake_rid(11),
        room_id: "!room:example.invalid".to_owned(),
        event_id: "$event:example.invalid".to_owned(),
    };
    let unpin = RoomCommand::UnpinEvent {
        request_id: fake_rid(12),
        room_id: "!room:example.invalid".to_owned(),
        event_id: "$event:example.invalid".to_owned(),
    };

    for debug in [format!("{pin:?}"), format!("{unpin:?}")] {
        assert!(debug.contains("RoomId(..)"), "{debug}");
        assert!(debug.contains("EventId(..)"), "{debug}");
        assert!(!debug.contains("!room:example.invalid"), "{debug}");
        assert!(!debug.contains("$event:example.invalid"), "{debug}");
    }
}

#[test]
fn force_rotate_outbound_session_is_correlated_and_redacts_room_id() {
    let request_id = fake_rid(12);
    let command = RoomCommand::ForceRotateOutboundSession {
        request_id,
        room_id: "!private-room:example.invalid".to_owned(),
    };

    assert_eq!(
        CoreCommand::Room(RoomCommand::ForceRotateOutboundSession {
            request_id,
            room_id: "!private-room:example.invalid".to_owned(),
        })
        .request_id(),
        request_id
    );
    let debug = format!("{command:?}");
    assert!(debug.contains("ForceRotateOutboundSession"), "{debug}");
    assert!(debug.contains("RoomId(..)"), "{debug}");
    assert!(!debug.contains("!private-room:example.invalid"), "{debug}");
}

#[test]
fn set_room_notification_mode_debug_redacts_room_id() {
    let command = RoomCommand::SetRoomNotificationMode {
        request_id: fake_rid(13),
        room_id: "!room:example.invalid".to_owned(),
        mode: koushi_state::RoomNotificationMode::Mute,
    };
    let debug = format!("{command:?}");
    assert!(debug.contains("SetRoomNotificationMode"), "{debug}");
    assert!(debug.contains("RoomId(..)"), "{debug}");
    assert!(!debug.contains("!room:example.invalid"), "{debug}");
}

#[test]
fn directory_commands_debug_redacts_query_alias_and_server() {
    let query = RoomCommand::QueryDirectory {
        request_id: fake_rid(15),
        query: DirectoryQuery {
            term: Some("private search".to_owned()),
            server_name: Some("example.invalid".to_owned()),
            limit: Some(10),
            since: Some("opaque-page-token".to_owned()),
        },
    };
    let join_request_id = fake_rid(14);
    let join = RoomCommand::JoinDirectoryRoom {
        request_id: join_request_id,
        room_id_or_alias: "#private-room:example.invalid".to_owned(),
        via_servers: vec!["example.invalid".to_owned()],
    };
    let create_request_id = fake_rid(15);
    let create_public = RoomCommand::CreatePublicDirectoryRoom {
        request_id: create_request_id,
        name: "Private Public Room Name".to_owned(),
        alias_localpart: "private-public-alias".to_owned(),
    };

    assert_eq!(
        CoreCommand::Room(RoomCommand::JoinDirectoryRoom {
            request_id: join_request_id,
            room_id_or_alias: "#private-room:example.invalid".to_owned(),
            via_servers: vec!["example.invalid".to_owned()],
        })
        .request_id(),
        join_request_id
    );
    assert_eq!(
        CoreCommand::Room(RoomCommand::CreatePublicDirectoryRoom {
            request_id: create_request_id,
            name: "Private Public Room Name".to_owned(),
            alias_localpart: "private-public-alias".to_owned(),
        })
        .request_id(),
        create_request_id
    );
    for debug in [
        format!("{query:?}"),
        format!("{join:?}"),
        format!("{create_public:?}"),
    ] {
        assert!(!debug.contains("private search"), "{debug}");
        assert!(!debug.contains("#private-room:example.invalid"), "{debug}");
        assert!(!debug.contains("Private Public Room Name"), "{debug}");
        assert!(!debug.contains("private-public-alias"), "{debug}");
        assert!(!debug.contains("example.invalid"), "{debug}");
        assert!(!debug.contains("opaque-page-token"), "{debug}");
    }
}

#[test]
fn mention_candidate_command_is_correlated_and_redacts_private_values() {
    let request_id = fake_rid(151);
    let command = RoomCommand::QueryMentionCandidates {
        request_id,
        account_key: AccountKey("@private-account:example.invalid".to_owned()),
        room_id: "!private-room:example.invalid".to_owned(),
        surface: koushi_state::MentionSurface::Thread,
        query: "Private Person Alias".to_owned(),
    };

    assert_eq!(
        CoreCommand::Room(RoomCommand::QueryMentionCandidates {
            request_id,
            account_key: AccountKey("@private-account:example.invalid".to_owned()),
            room_id: "!private-room:example.invalid".to_owned(),
            surface: koushi_state::MentionSurface::Thread,
            query: "Private Person Alias".to_owned(),
        })
        .request_id(),
        request_id
    );
    let debug = format!("{command:?}");
    assert!(debug.contains("QueryMentionCandidates"), "{debug}");
    assert!(debug.contains("Thread"), "{debug}");
    for private_value in [
        "@private-account:example.invalid",
        "!private-room:example.invalid",
        "Private Person Alias",
    ] {
        assert!(!debug.contains(private_value), "{debug}");
    }
}

#[test]
fn room_management_commands_debug_redacts_room_user_and_settings_values() {
    use koushi_state::{RoomJoinRule, RoomModerationAction, RoomSettingChange};

    let load_request_id = fake_rid(16);
    let load = RoomCommand::LoadRoomSettings {
        request_id: load_request_id,
        room_id: "!private-room:example.invalid".to_owned(),
    };
    let update_request_id = fake_rid(17);
    let update = RoomCommand::UpdateRoomSetting {
        request_id: update_request_id,
        room_id: "!private-room:example.invalid".to_owned(),
        change: RoomSettingChange::Name(Some("Private Room Name".to_owned())),
    };
    let moderation_request_id = fake_rid(18);
    let moderation = RoomCommand::ModerateRoomMember {
        request_id: moderation_request_id,
        room_id: "!private-room:example.invalid".to_owned(),
        target_user_id: "@private-target:example.invalid".to_owned(),
        action: RoomModerationAction::Ban,
        reason: Some("Private moderation reason".to_owned()),
    };
    let role_request_id = fake_rid(19);
    let role = RoomCommand::UpdateRoomMemberRole {
        request_id: role_request_id,
        room_id: "!private-room:example.invalid".to_owned(),
        target_user_id: "@private-target:example.invalid".to_owned(),
        power_level: 50,
    };

    assert_eq!(CoreCommand::Room(load).request_id(), load_request_id);
    assert_eq!(
        CoreCommand::Room(RoomCommand::UpdateRoomSetting {
            request_id: update_request_id,
            room_id: "!private-room:example.invalid".to_owned(),
            change: RoomSettingChange::JoinRule(RoomJoinRule::Public),
        })
        .request_id(),
        update_request_id
    );
    assert_eq!(
        CoreCommand::Room(RoomCommand::ModerateRoomMember {
            request_id: moderation_request_id,
            room_id: "!private-room:example.invalid".to_owned(),
            target_user_id: "@private-target:example.invalid".to_owned(),
            action: RoomModerationAction::Kick,
            reason: None,
        })
        .request_id(),
        moderation_request_id
    );
    assert_eq!(
        CoreCommand::Room(RoomCommand::UpdateRoomMemberRole {
            request_id: role_request_id,
            room_id: "!private-room:example.invalid".to_owned(),
            target_user_id: "@private-target:example.invalid".to_owned(),
            power_level: 50,
        })
        .request_id(),
        role_request_id
    );

    for debug in [
        format!(
            "{:?}",
            RoomCommand::LoadRoomSettings {
                request_id: fake_rid(20),
                room_id: "!private-room:example.invalid".to_owned(),
            }
        ),
        format!("{update:?}"),
        format!("{moderation:?}"),
        format!("{role:?}"),
    ] {
        assert!(debug.contains("RoomId(..)"), "{debug}");
        assert!(!debug.contains("!private-room:example.invalid"), "{debug}");
        assert!(
            !debug.contains("@private-target:example.invalid"),
            "{debug}"
        );
        assert!(!debug.contains("Private Room Name"), "{debug}");
        assert!(!debug.contains("Private moderation reason"), "{debug}");
    }
}

#[test]
fn create_room_options_serialize_the_explicit_policy_and_history() {
    use koushi_state::{RoomAccessPolicy, RoomHistoryVisibility, RoomJoinRule};

    let options = CreateRoomOptions {
        name: "Synthetic".to_owned(),
        topic: None,
        alias_localpart: None,
        encrypted: true,
        invited_only: false,
        visibility: CreateRoomVisibility::Private,
        parent_space: None,
        access_policy: Some(RoomAccessPolicy::new(
            RoomJoinRule::Restricted,
            vec![
                "!b:example.invalid".to_owned(),
                "!a:example.invalid".to_owned(),
            ],
        )),
        history: Some(RoomHistoryVisibility::Joined),
    };
    let value = serde_json::to_value(&options).expect("serialize create options");
    assert_eq!(
        value["accessPolicy"],
        serde_json::json!({
            "rule": "restricted",
            "allowTargets": ["!a:example.invalid", "!b:example.invalid"],
        })
    );
    assert_eq!(value["history"], serde_json::json!("joined"));

    let debug = format!("{options:?}");
    assert!(debug.contains("RoomAccessPolicy"), "{debug}");
    assert!(!debug.contains("!a:example.invalid"), "{debug}");
    assert!(!debug.contains("!b:example.invalid"), "{debug}");
    assert!(debug.contains("allow_target_count"), "{debug}");
}
