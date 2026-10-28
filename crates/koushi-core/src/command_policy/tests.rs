use super::*;
use koushi_protocol::{RuntimeConnectionId, SyncCommand};

fn request(sequence: u64) -> RequestId {
    RequestId {
        connection_id: RuntimeConnectionId(1),
        sequence,
    }
}

#[test]
fn ready_admission_policy_stays_in_core() {
    let id = request(1);
    for command in [
        CoreCommand::Account(AccountCommand::SoftLogoutReauth {
            request_id: id,
            password: koushi_state::AuthSecret::new("synthetic"),
        }),
        CoreCommand::Account(AccountCommand::RetrySlidingSyncCapability { request_id: id }),
        CoreCommand::Account(AccountCommand::ResetLocalData { request_id: id }),
        CoreCommand::Account(AccountCommand::ChangeHomeserver { request_id: id }),
        CoreCommand::Account(AccountCommand::StartDeviceCleanup { request_id: id }),
        CoreCommand::Account(AccountCommand::EraseDeviceCleanupLocalDataAnyway { request_id: id }),
        // Closing User info only tears down; it must settle quietly after
        // sign-out, lock, or account switch unmounts the panel (#1024).
        CoreCommand::Account(AccountCommand::ContactSecurity {
            request_id: id,
            request: koushi_protocol::command::ContactSecurityRequest::Close,
        }),
    ] {
        assert!(!command.requires_ready_session());
    }

    for command in [
        CoreCommand::Sync(SyncCommand::Start { request_id: id }),
        CoreCommand::Account(AccountCommand::ContactSecurity {
            request_id: id,
            request: koushi_protocol::command::ContactSecurityRequest::Load {
                user_id: "@contact:example.invalid".to_owned(),
            },
        }),
        CoreCommand::App(AppCommand::OpenTimelineAtTimestamp {
            request_id: id,
            room_id: "!room:example.invalid".to_owned(),
            timestamp_ms: 1,
        }),
        CoreCommand::App(AppCommand::ClearUploadStaging {
            request_id: id,
            target: koushi_state::ComposerTarget::Main {
                room_id: "!room:example.invalid".to_owned(),
            },
        }),
        CoreCommand::App(AppCommand::OpenScheduledSendsList {
            request_id: id,
            scope: koushi_state::ScheduledSendsScope::Home,
        }),
        CoreCommand::App(AppCommand::CloseScheduledSendsList { request_id: id }),
    ] {
        assert!(command.requires_ready_session());
    }
}

#[test]
fn composer_scope_and_timeline_account_fence_are_core_policy() {
    let expected_account = koushi_protocol::SessionKeyId {
        homeserver: "https://example.invalid".to_owned(),
        user_id: "@user:example.invalid".to_owned(),
        device_id: "DEVICE".to_owned(),
    };
    let id = request(2);
    let command = CoreCommand::App(AppCommand::SetComposerDraft {
        request_id: id,
        expected_account: expected_account.clone(),
        room_id: "!room:example.invalid".to_owned(),
        document: koushi_state::ComposerDocument::default(),
        revision: 1.into(),
    });
    assert!(command.composer_draft_scope().is_some());

    // #1494: the advisory seed holds no draft lease and needs a Ready session.
    let seed = CoreCommand::App(AppCommand::SeedComposerDraft {
        request_id: id,
        expected_account: expected_account.clone(),
        room_id: "!room:example.invalid".to_owned(),
        document: koushi_state::ComposerDocument::from_plain_text("advisory"),
    });
    assert!(seed.composer_draft_scope().is_none());
    assert!(seed.requires_ready_session());

    let timeline = TimelineCommand::SubmitText {
        request_id: id,
        expected_account: expected_account.clone(),
        submission_id: koushi_state::SubmissionId::new("submission"),
        key: koushi_protocol::TimelineKey::room(
            koushi_protocol::AccountKey("@user:example.invalid".to_owned()),
            "!room:example.invalid",
        ),
        transaction_id: "transaction".to_owned(),
        document: koushi_state::ComposerDocument::default(),
        draft_revision: 1.into(),
    };
    assert_eq!(
        timeline_composer_account_fence(&timeline),
        Some((id, &expected_account))
    );
}

fn synthetic_room(room_id: &str) -> koushi_state::RoomSummary {
    koushi_state::RoomSummary {
        display_name_placeholder: None,
        display_label_placeholder: None,
        room_id: room_id.to_owned(),
        display_name: "Synthetic".to_owned(),
        display_label: "Synthetic".to_owned(),
        original_display_label: "Synthetic".to_owned(),
        avatar: None,
        is_dm: false,
        dm_user_ids: Vec::new(),
        tags: koushi_state::RoomTags::default(),
        unread_count: 0,
        notification_count: 0,
        highlight_count: 0,
        thread_unread_count: 0,
        thread_highlight_count: 0,
        marked_unread: false,
        recency_stamp: None,
        conversation_activity: None,
        latest_event: None,
        parent_space_ids: Vec::new(),
        dm_space_ids: Vec::new(),
        is_encrypted: false,
        joined_members: 1,
    }
}

#[test]
fn leave_space_admission_keeps_only_joined_children_of_that_space() {
    let space_id = "!space:example.invalid";
    let state = AppState {
        spaces: vec![koushi_state::SpaceSummary {
            space_id: space_id.to_owned(),
            raw_name: None,
            display_name: "Synthetic Workspace".to_owned(),
            avatar: None,
            join_rule: None,
            topic: None,
            child_room_ids: vec!["!child:example.invalid".to_owned()],
            parent_side_child_room_ids: vec!["!child:example.invalid".to_owned()],
        }],
        rooms: vec![
            synthetic_room("!child:example.invalid"),
            synthetic_room("!outside:example.invalid"),
        ],
        ..AppState::default()
    };
    let id = request(7);
    let admitted = admit_leave_space_command(
        &state,
        koushi_protocol::command::RoomCommand::LeaveSpace {
            request_id: id,
            space_id: space_id.to_owned(),
            child_room_ids: vec![
                "!outside:example.invalid".to_owned(),
                "!child:example.invalid".to_owned(),
            ],
        },
    );
    match admitted {
        koushi_protocol::command::RoomCommand::LeaveSpace {
            request_id,
            space_id: admitted_space_id,
            child_room_ids,
        } => {
            assert_eq!(request_id, id);
            assert_eq!(admitted_space_id, space_id);
            assert_eq!(child_room_ids, ["!child:example.invalid"]);
        }
        other => panic!("unexpected command: {other:?}"),
    }

    let passthrough = admit_leave_space_command(
        &state,
        koushi_protocol::command::RoomCommand::LeaveRoom {
            request_id: id,
            room_id: "!outside:example.invalid".to_owned(),
        },
    );
    assert!(matches!(
        passthrough,
        koushi_protocol::command::RoomCommand::LeaveRoom { room_id, .. }
            if room_id == "!outside:example.invalid"
    ));
}

/// Issue #1410: a bulk join joins only the admitted offer, so a forged or
/// widened room list never reaches the SDK.
#[test]
fn suggested_rooms_join_admission_uses_the_held_offer() {
    let space_id = "!space:example.invalid";
    let id = request(9);
    let mut state = AppState::default();
    state.space_children.suggested_offer = koushi_state::SuggestedRoomsOffer::Joining {
        request_id: id.sequence,
        space_id: space_id.to_owned(),
        rooms: vec![koushi_state::SuggestedRoomCandidate {
            room_id: "!suggested:example.invalid".to_owned(),
            display_name: "Suggested".to_owned(),
            display_name_placeholder: None,
        }],
    };
    let join = |request_id| koushi_protocol::command::RoomCommand::JoinSuggestedSpaceRooms {
        request_id,
        space_id: space_id.to_owned(),
        room_ids: vec!["!forged:example.invalid".to_owned()],
    };
    let room_ids_of = |command| match command {
        koushi_protocol::command::RoomCommand::JoinSuggestedSpaceRooms { room_ids, .. } => room_ids,
        other => panic!("unexpected command: {other:?}"),
    };

    assert_eq!(
        room_ids_of(admit_leave_space_command(&state, join(id))),
        ["!suggested:example.invalid"]
    );
    assert!(room_ids_of(admit_leave_space_command(&state, join(request(10)))).is_empty());
}
