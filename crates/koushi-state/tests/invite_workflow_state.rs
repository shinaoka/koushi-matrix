use koushi_state::{
    AppAction, AppState, INVITE_ALREADY_IN_SPACE_MESSAGE, InviteDestination,
    InviteDestinationResult, InviteDestinationResultKind, InviteOperationState,
    InviteScopeSelection, InviteTargetCandidateStatus, RoomHistoryVisibility, RoomJoinRule,
    RoomManagementOperationState, RoomMemberRole, RoomMemberSummary, RoomPermissionFacts,
    RoomSettingsSnapshot, RoomSummary, RoomTags, SpaceSummary, UserProfile, reduce,
};

fn room(room_id: &str, display_name: &str, parent_space_ids: Vec<String>) -> RoomSummary {
    RoomSummary {
        room_id: room_id.to_owned(),
        display_name: display_name.to_owned(),
        display_label: display_name.to_owned(),
        original_display_label: display_name.to_owned(),
        avatar: None,
        is_dm: false,
        dm_user_ids: Vec::new(),
        tags: RoomTags::default(),
        unread_count: 0,
        notification_count: 0,
        highlight_count: 0,
        marked_unread: false,
        last_activity_ms: 0,
        latest_event: None,
        parent_space_ids,
        dm_space_ids: Vec::new(),
        is_encrypted: false,
        joined_members: 0,
    }
}

fn user_profile(user_id: &str, display_label: &str, terms: &[&str]) -> UserProfile {
    UserProfile {
        user_id: user_id.to_owned(),
        display_name: Some(display_label.to_owned()),
        display_label: display_label.to_owned(),
        original_display_label: display_label.to_owned(),
        mention_search_terms: terms.iter().map(|term| (*term).to_owned()).collect(),
        avatar: None,
    }
}

fn room_member(user_id: &str, display_label: &str) -> RoomMemberSummary {
    RoomMemberSummary {
        user_id: user_id.to_owned(),
        display_name: Some(display_label.to_owned()),
        display_label: display_label.to_owned(),
        original_display_label: display_label.to_owned(),
        avatar_url: None,
        power_level: Some(0),
        role: RoomMemberRole::User,
        user_trust: None,
    }
}

fn room_settings(room_id: &str, members: Vec<RoomMemberSummary>) -> RoomSettingsSnapshot {
    RoomSettingsSnapshot {
        room_id: room_id.to_owned(),
        name: Some("General".to_owned()),
        topic: None,
        avatar_url: None,
        canonical_alias: None,
        alternate_aliases: Vec::new(),
        share_link: None,
        join_rule: RoomJoinRule::Invite,
        history_visibility: RoomHistoryVisibility::Shared,
        permissions: RoomPermissionFacts::default(),
        members,
    }
}

#[test]
fn invite_target_query_matches_profiles_aliases_members_and_explicit_user_ids() {
    let mut state = AppState::default();
    state
        .rooms
        .push(room("!room:example.org", "General", Vec::new()));
    state.profile.users.insert(
        "@alice:example.org".to_owned(),
        user_profile("@alice:example.org", "Alice A.", &["alice", "project"]),
    );
    state
        .profile
        .local_aliases
        .insert("@bob:example.org".to_owned(), "Bobby".to_owned());
    state.room_management.selected_room_id = Some("!room:example.org".to_owned());
    state.room_management.settings = Some(room_settings(
        "!room:example.org",
        vec![room_member("@carol:example.org", "Carol C.")],
    ));
    state.room_management.operation = RoomManagementOperationState::Idle;

    reduce(
        &mut state,
        AppAction::InviteTargetQueryChanged {
            room_id: "!room:example.org".to_owned(),
            query: "bob".to_owned(),
        },
    );

    assert_eq!(
        state.invite_workflow.query.room_id.as_deref(),
        Some("!room:example.org")
    );
    assert_eq!(state.invite_workflow.query.candidates.len(), 1);
    assert_eq!(
        state.invite_workflow.query.candidates[0].user_id,
        "@bob:example.org"
    );
    assert_eq!(
        state.invite_workflow.query.candidates[0].display_label,
        "Bobby"
    );
    assert_eq!(
        state.invite_workflow.query.candidates[0].status,
        InviteTargetCandidateStatus::Selectable
    );

    reduce(
        &mut state,
        AppAction::InviteTargetQueryChanged {
            room_id: "!room:example.org".to_owned(),
            query: "carol".to_owned(),
        },
    );
    assert_eq!(
        state.invite_workflow.query.candidates[0].user_id,
        "@carol:example.org"
    );

    reduce(
        &mut state,
        AppAction::InviteTargetQueryChanged {
            room_id: "!room:example.org".to_owned(),
            query: "@new:example.org".to_owned(),
        },
    );
    let explicit = state
        .invite_workflow
        .query
        .explicit_user_id
        .as_ref()
        .expect("valid explicit Matrix ID should be selectable");
    assert_eq!(explicit.user_id, "@new:example.org");
    assert_eq!(explicit.status, InviteTargetCandidateStatus::Selectable);

    reduce(
        &mut state,
        AppAction::InviteTargetQueryChanged {
            room_id: "!room:example.org".to_owned(),
            query: "@not-a-valid-id".to_owned(),
        },
    );
    let explicit = state
        .invite_workflow
        .query
        .explicit_user_id
        .as_ref()
        .expect("invalid explicit Matrix ID should still be represented");
    assert_eq!(
        explicit.status,
        InviteTargetCandidateStatus::InvalidMatrixId
    );
}

#[test]
fn invite_scope_plan_prefers_active_parent_space_for_room_invites() {
    let mut state = AppState::default();
    state.navigation.active_space_id = Some("!space:example.org".to_owned());
    state.spaces.push(SpaceSummary {
        space_id: "!space:example.org".to_owned(),
        display_name: "Project Space".to_owned(),
        avatar: None,
        child_room_ids: vec!["!room:example.org".to_owned()],
    });
    state.rooms.push(room(
        "!room:example.org",
        "General",
        vec!["!space:example.org".to_owned()],
    ));

    reduce(
        &mut state,
        AppAction::InviteWorkflowOpened {
            room_id: "!room:example.org".to_owned(),
        },
    );

    let plan = state
        .invite_workflow
        .scope_plan
        .as_ref()
        .expect("room in a space should have a scope plan");
    assert_eq!(
        plan.default_scope,
        InviteScopeSelection::ParentSpaceAndRoom {
            space_id: "!space:example.org".to_owned()
        }
    );
    assert!(plan.options.iter().any(|option| {
        option.scope
            == InviteScopeSelection::ParentSpaceAndRoom {
                space_id: "!space:example.org".to_owned(),
            }
    }));
    assert!(
        plan.options
            .iter()
            .any(|option| option.scope == InviteScopeSelection::RoomOnly)
    );
}

#[test]
fn invite_batch_completion_records_already_in_space_as_notice_and_keeps_room_result() {
    let mut state = AppState::default();

    reduce(
        &mut state,
        AppAction::InviteBatchRequested {
            request_id: 7,
            room_id: "!room:example.org".to_owned(),
            user_ids: vec!["@alice:example.org".to_owned()],
            scope: InviteScopeSelection::ParentSpaceAndRoom {
                space_id: "!space:example.org".to_owned(),
            },
        },
    );
    assert!(matches!(
        state.invite_workflow.operation,
        InviteOperationState::Pending { request_id: 7, .. }
    ));

    reduce(
        &mut state,
        AppAction::InviteBatchCompleted {
            request_id: 7,
            room_id: "!room:example.org".to_owned(),
            results: vec![
                InviteDestinationResult {
                    user_id: "@alice:example.org".to_owned(),
                    destination: InviteDestination::Space {
                        space_id: "!space:example.org".to_owned(),
                    },
                    kind: InviteDestinationResultKind::AlreadyInSpace,
                    message: Some(INVITE_ALREADY_IN_SPACE_MESSAGE.to_owned()),
                },
                InviteDestinationResult {
                    user_id: "@alice:example.org".to_owned(),
                    destination: InviteDestination::Room {
                        room_id: "!room:example.org".to_owned(),
                    },
                    kind: InviteDestinationResultKind::Invited,
                    message: None,
                },
            ],
        },
    );

    let InviteOperationState::Completed {
        request_id,
        notice,
        results,
        ..
    } = &state.invite_workflow.operation
    else {
        panic!("invite batch should complete");
    };
    assert_eq!(*request_id, 7);
    assert_eq!(notice.as_deref(), Some(INVITE_ALREADY_IN_SPACE_MESSAGE));
    assert_eq!(results.len(), 2);
    assert_eq!(results[0].kind, InviteDestinationResultKind::AlreadyInSpace);
    assert_eq!(results[1].kind, InviteDestinationResultKind::Invited);
}

#[test]
fn invite_workflow_clears_on_logout() {
    let mut state = AppState::default();

    reduce(
        &mut state,
        AppAction::InviteTargetQueryChanged {
            room_id: "!room:example.org".to_owned(),
            query: "@alice:example.org".to_owned(),
        },
    );
    assert!(!state.invite_workflow.query.query.is_empty());

    reduce(&mut state, AppAction::LogoutFinished);

    assert_eq!(state.invite_workflow, Default::default());
}
