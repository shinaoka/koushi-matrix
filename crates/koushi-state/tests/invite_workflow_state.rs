use koushi_state::{
    AppAction, AppState, INVITE_ALREADY_IN_SPACE_MESSAGE, InviteDestination, InviteDestinationKind,
    InviteDestinationResult, InviteDestinationResultKind, InviteHistoryReadiness,
    InviteOperationState, InviteScopeSelection, InviteSelectedTarget, InviteTargetCandidateStatus,
    InviteWorkflowState, OperationFailureKind, ProvisionalPhase, RoomHistoryVisibility,
    RoomJoinRule, RoomManagementOperationState, RoomMemberRole, RoomMemberSummary,
    RoomPermissionFacts, RoomSettingsSnapshot, RoomSummary, RoomTags, SessionInfo, SessionState,
    SlidingSyncCapabilityFailureKind, SpaceSummary, UserProfile, VerificationAccountKind,
    VerificationGateState, VerificationMethod, reduce,
};

const ROOM_A: &str = "!room-a:example.org";
const ROOM_B: &str = "!room-b:example.org";
const SPACE_A: &str = "!space-a:example.org";
const SPACE_B: &str = "!space-b:example.org";
const ALICE: &str = "@alice:example.org";
const BOB: &str = "@bob:example.org";
const CAROL: &str = "@carol:example.org";

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
        recency_stamp: None,
        conversation_activity: None,
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

fn session_info() -> SessionInfo {
    SessionInfo {
        homeserver: "https://example.org".to_owned(),
        user_id: "@owner:example.org".to_owned(),
        device_id: "DEVICE".to_owned(),
        authentication_method: Default::default(),
    }
}

fn ready_state() -> AppState {
    let mut state = AppState::default();
    state.session = SessionState::Ready(session_info());
    state
}

fn ready_with_room(room_id: &str) -> AppState {
    let mut state = ready_state();
    state.rooms.push(room(room_id, "Room", Vec::new()));
    state
}

fn ready_room_with_parent_space() -> AppState {
    let mut state = ready_state();
    state.navigation.active_space_id = Some(SPACE_A.to_owned());
    state.spaces.push(SpaceSummary {
        space_id: SPACE_A.to_owned(),
        display_name: "Project Space".to_owned(),
        avatar: None,
        child_room_ids: vec![ROOM_A.to_owned()],
    });
    state
        .rooms
        .push(room(ROOM_A, "General", vec![SPACE_A.to_owned()]));
    state
}

fn state_with_policy_session(session: SessionState) -> AppState {
    let mut state = ready_room_with_parent_space();
    state.session = session;
    state.rooms[0].is_encrypted = true;
    let mut settings = room_settings(ROOM_A, Vec::new());
    settings.history_visibility = RoomHistoryVisibility::Invited;
    settings.permissions.can_edit_settings = true;
    state.room_management.selected_room_id = Some(ROOM_A.to_owned());
    state.room_management.settings = Some(settings);
    state
}

fn recovery_gate() -> VerificationGateState {
    VerificationGateState {
        methods: Vec::new(),
        account_kind: VerificationAccountKind::Unknown,
        failure: None,
    }
}

fn recovery_sessions() -> Vec<SessionState> {
    let info = session_info();
    vec![
        SessionState::AwaitingVerification {
            info: info.clone(),
            gate: recovery_gate(),
        },
        SessionState::Verifying {
            info: info.clone(),
            gate: recovery_gate(),
            method: VerificationMethod::RecoveryKey,
            flow_id: 1,
            sas_emojis: Vec::new(),
        },
        SessionState::AwaitingBootstrapConfirmation {
            info: info.clone(),
            gate: recovery_gate(),
            flow_id: 1,
            destination_written: false,
        },
        SessionState::Locked(info),
    ]
}

fn assert_inert(state: &mut AppState, action: AppAction) {
    let before = state.clone();
    let effects = reduce(state, action);
    assert!(effects.is_empty(), "invalid invite action emitted effects");
    assert_eq!(&*state, &before, "invalid invite action mutated state");
}

fn selected_target(user_id: &str, display_label: &str) -> InviteSelectedTarget {
    InviteSelectedTarget {
        user_id: user_id.to_owned(),
        display_label: display_label.to_owned(),
        avatar: None,
    }
}

fn room_result(user_id: &str) -> InviteDestinationResult {
    InviteDestinationResult {
        user_id: user_id.to_owned(),
        destination: InviteDestination::Room {
            room_id: ROOM_A.to_owned(),
        },
        kind: InviteDestinationResultKind::Invited,
        message: None,
    }
}

fn state_with_two_selected_targets() -> AppState {
    let mut state = ready_room_with_parent_space();
    state.profile.users.insert(
        ALICE.to_owned(),
        user_profile(ALICE, "Alice A.", &["alice"]),
    );
    state
        .profile
        .users
        .insert(BOB.to_owned(), user_profile(BOB, "Bob B.", &["bob"]));

    reduce(
        &mut state,
        AppAction::InviteWorkflowOpened {
            room_id: ROOM_A.to_owned(),
        },
    );
    reduce(
        &mut state,
        AppAction::InviteTargetQueryChanged {
            room_id: ROOM_A.to_owned(),
            query: "alice".to_owned(),
        },
    );
    reduce(
        &mut state,
        AppAction::InviteTargetSelected {
            room_id: ROOM_A.to_owned(),
            user_id: ALICE.to_owned(),
        },
    );
    reduce(
        &mut state,
        AppAction::InviteTargetQueryChanged {
            room_id: ROOM_A.to_owned(),
            query: "bob".to_owned(),
        },
    );
    reduce(
        &mut state,
        AppAction::InviteTargetSelected {
            room_id: ROOM_A.to_owned(),
            user_id: BOB.to_owned(),
        },
    );
    state.profile.users.insert(
        CAROL.to_owned(),
        user_profile(CAROL, "Carol C.", &["carol"]),
    );
    reduce(
        &mut state,
        AppAction::InviteTargetQueryChanged {
            room_id: ROOM_A.to_owned(),
            query: "carol".to_owned(),
        },
    );
    state
}

fn state_with_pending() -> AppState {
    let mut state = state_with_two_selected_targets();
    reduce(
        &mut state,
        AppAction::InviteBatchRequested {
            request_id: 41,
            room_id: ROOM_A.to_owned(),
            user_ids: vec![ALICE.to_owned(), BOB.to_owned()],
            scope: InviteScopeSelection::ParentSpaceAndRoom {
                space_id: SPACE_A.to_owned(),
            },
        },
    );
    state
}

#[test]
fn invite_target_query_matches_profiles_aliases_members_and_explicit_user_ids() {
    let mut state = ready_state();
    state.rooms.push(room(ROOM_A, "General", Vec::new()));
    state.profile.users.insert(
        "@alice:example.org".to_owned(),
        user_profile("@alice:example.org", "Alice A.", &["alice", "project"]),
    );
    state
        .profile
        .local_aliases
        .insert("@bob:example.org".to_owned(), "Bobby".to_owned());
    state.room_management.selected_room_id = Some(ROOM_A.to_owned());
    state.room_management.settings = Some(room_settings(
        ROOM_A,
        vec![room_member("@carol:example.org", "Carol C.")],
    ));
    state.room_management.operation = RoomManagementOperationState::Idle;

    reduce(
        &mut state,
        AppAction::InviteTargetQueryChanged {
            room_id: ROOM_A.to_owned(),
            query: "bob".to_owned(),
        },
    );

    assert_eq!(state.invite_workflow.query.room_id.as_deref(), Some(ROOM_A));
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
            room_id: ROOM_A.to_owned(),
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
            room_id: ROOM_A.to_owned(),
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
            room_id: ROOM_A.to_owned(),
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
    let mut state = ready_room_with_parent_space();

    reduce(
        &mut state,
        AppAction::InviteWorkflowOpened {
            room_id: ROOM_A.to_owned(),
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
            space_id: SPACE_A.to_owned()
        }
    );
    assert!(plan.options.iter().any(|option| {
        option.scope
            == InviteScopeSelection::ParentSpaceAndRoom {
                space_id: SPACE_A.to_owned(),
            }
    }));
    assert!(
        plan.options
            .iter()
            .any(|option| option.scope == InviteScopeSelection::RoomOnly)
    );
}

#[test]
fn invite_workflow_projects_history_policy_and_preserves_scope_and_draft() {
    let mut locked = state_with_policy_session(SessionState::Locked(session_info()));

    reduce(
        &mut locked,
        AppAction::InviteWorkflowOpened {
            room_id: ROOM_A.to_owned(),
        },
    );

    let policy = locked
        .invite_workflow
        .history_policy
        .as_ref()
        .expect("opening invite should project history policy");
    assert_eq!(policy.current_visibility, RoomHistoryVisibility::Invited);
    assert!(policy.encrypted);
    assert!(policy.can_edit);
    assert_eq!(policy.readiness, InviteHistoryReadiness::RecoveryRequired);
    assert_inert(
        &mut locked,
        AppAction::InviteScopeSelected {
            room_id: ROOM_A.to_owned(),
            scope: InviteScopeSelection::RoomOnly,
        },
    );

    let mut ready = state_with_policy_session(SessionState::Ready(session_info()));
    reduce(
        &mut ready,
        AppAction::InviteWorkflowOpened {
            room_id: ROOM_A.to_owned(),
        },
    );
    reduce(
        &mut ready,
        AppAction::InviteScopeSelected {
            room_id: ROOM_A.to_owned(),
            scope: InviteScopeSelection::RoomOnly,
        },
    );
    reduce(
        &mut ready,
        AppAction::InviteTargetQueryChanged {
            room_id: ROOM_A.to_owned(),
            query: "alice".to_owned(),
        },
    );

    reduce(
        &mut ready,
        AppAction::InviteWorkflowOpened {
            room_id: ROOM_A.to_owned(),
        },
    );

    assert_eq!(
        ready.invite_workflow.selected_scope,
        Some(InviteScopeSelection::RoomOnly)
    );
    assert_eq!(ready.invite_workflow.query.query, "alice");
    assert_eq!(
        ready
            .invite_workflow
            .history_policy
            .as_ref()
            .expect("ready opening should project history policy")
            .readiness,
        InviteHistoryReadiness::Ready
    );
}

#[test]
fn invite_workflow_rejects_scope_not_in_current_plan() {
    let mut state = ready_with_room(ROOM_A);

    reduce(
        &mut state,
        AppAction::InviteWorkflowOpened {
            room_id: ROOM_A.to_owned(),
        },
    );
    let default_scope = state.invite_workflow.selected_scope.clone();

    reduce(
        &mut state,
        AppAction::InviteScopeSelected {
            room_id: ROOM_A.to_owned(),
            scope: InviteScopeSelection::ParentSpaceAndRoom {
                space_id: "!not-a-parent:example.org".to_owned(),
            },
        },
    );

    assert_eq!(state.invite_workflow.selected_scope, default_scope);
}

#[test]
fn invite_batch_completion_records_already_in_space_as_notice_and_keeps_room_result() {
    let mut state = state_with_two_selected_targets();

    reduce(
        &mut state,
        AppAction::InviteBatchRequested {
            request_id: 7,
            room_id: ROOM_A.to_owned(),
            user_ids: vec![ALICE.to_owned(), BOB.to_owned()],
            scope: InviteScopeSelection::ParentSpaceAndRoom {
                space_id: SPACE_A.to_owned(),
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
            room_id: ROOM_A.to_owned(),
            results: vec![
                InviteDestinationResult {
                    user_id: ALICE.to_owned(),
                    destination: InviteDestination::Space {
                        space_id: SPACE_A.to_owned(),
                    },
                    kind: InviteDestinationResultKind::AlreadyInSpace,
                    message: Some(INVITE_ALREADY_IN_SPACE_MESSAGE.to_owned()),
                },
                room_result(BOB),
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
    assert!(state.invite_workflow.selected_targets.is_empty());
}

#[test]
fn invite_workflow_clears_on_logout() {
    let mut state = ready_with_room(ROOM_A);

    reduce(
        &mut state,
        AppAction::InviteTargetQueryChanged {
            room_id: ROOM_A.to_owned(),
            query: "@alice:example.org".to_owned(),
        },
    );
    assert!(!state.invite_workflow.query.query.is_empty());

    reduce(&mut state, AppAction::LogoutFinished);

    assert_eq!(state.invite_workflow, InviteWorkflowState::default());
}

#[test]
fn invite_workflow_commands_are_inert_outside_ready_except_recovery_open() {
    for session in [
        SessionState::SignedOut,
        SessionState::SwitchingAccount {
            info: session_info(),
        },
    ] {
        let mut state = state_with_policy_session(session);
        assert_inert(
            &mut state,
            AppAction::InviteWorkflowOpened {
                room_id: ROOM_A.to_owned(),
            },
        );
        assert_inert(
            &mut state,
            AppAction::InviteTargetQueryChanged {
                room_id: ROOM_A.to_owned(),
                query: "alice".to_owned(),
            },
        );
        assert_inert(
            &mut state,
            AppAction::InviteScopeSelected {
                room_id: ROOM_A.to_owned(),
                scope: InviteScopeSelection::RoomOnly,
            },
        );
        assert_inert(
            &mut state,
            AppAction::InviteTargetSelected {
                room_id: ROOM_A.to_owned(),
                user_id: ALICE.to_owned(),
            },
        );
        assert_inert(
            &mut state,
            AppAction::InviteTargetRemoved {
                user_id: ALICE.to_owned(),
            },
        );
        assert_inert(
            &mut state,
            AppAction::InviteBatchRequested {
                request_id: 1,
                room_id: ROOM_A.to_owned(),
                user_ids: vec![ALICE.to_owned()],
                scope: InviteScopeSelection::RoomOnly,
            },
        );
    }

    let mut locked = state_with_policy_session(SessionState::Locked(session_info()));
    reduce(
        &mut locked,
        AppAction::InviteWorkflowOpened {
            room_id: ROOM_A.to_owned(),
        },
    );
    assert_eq!(
        locked
            .invite_workflow
            .history_policy
            .as_ref()
            .expect("locked disclosure")
            .readiness,
        InviteHistoryReadiness::RecoveryRequired
    );
    for action in [
        AppAction::InviteTargetQueryChanged {
            room_id: ROOM_A.to_owned(),
            query: "alice".to_owned(),
        },
        AppAction::InviteScopeSelected {
            room_id: ROOM_A.to_owned(),
            scope: InviteScopeSelection::RoomOnly,
        },
        AppAction::InviteTargetSelected {
            room_id: ROOM_A.to_owned(),
            user_id: ALICE.to_owned(),
        },
        AppAction::InviteTargetRemoved {
            user_id: ALICE.to_owned(),
        },
        AppAction::InviteBatchRequested {
            request_id: 1,
            room_id: ROOM_A.to_owned(),
            user_ids: vec![ALICE.to_owned()],
            scope: InviteScopeSelection::RoomOnly,
        },
    ] {
        assert_inert(&mut locked, action);
    }
}

#[test]
fn invite_workflow_open_preserves_recovery_disclosure_but_not_edit_admission() {
    for session in recovery_sessions() {
        let mut state = state_with_policy_session(session);
        reduce(
            &mut state,
            AppAction::InviteWorkflowOpened {
                room_id: ROOM_A.to_owned(),
            },
        );
        assert_eq!(
            state
                .invite_workflow
                .history_policy
                .as_ref()
                .expect("recovery disclosure")
                .readiness,
            InviteHistoryReadiness::RecoveryRequired
        );
        assert_eq!(state.invite_workflow.query.room_id.as_deref(), Some(ROOM_A));
        assert_inert(
            &mut state,
            AppAction::InviteTargetQueryChanged {
                room_id: ROOM_A.to_owned(),
                query: "alice".to_owned(),
            },
        );
    }
}

#[test]
fn invite_workflow_rejects_unknown_rooms_and_spaces() {
    let mut state = ready_state();
    for action in [
        AppAction::InviteWorkflowOpened {
            room_id: ROOM_A.to_owned(),
        },
        AppAction::InviteTargetQueryChanged {
            room_id: ROOM_A.to_owned(),
            query: "alice".to_owned(),
        },
    ] {
        assert_inert(&mut state, action);
    }

    let mut space_state = ready_state();
    for action in [
        AppAction::InviteWorkflowOpened {
            room_id: SPACE_A.to_owned(),
        },
        AppAction::InviteTargetQueryChanged {
            room_id: SPACE_A.to_owned(),
            query: "alice".to_owned(),
        },
    ] {
        assert_inert(&mut space_state, action);
    }
}

#[test]
fn invite_first_query_establishes_none_destination_but_some_a_to_b_is_stale() {
    let mut state = ready_state();
    state.rooms.push(room(ROOM_A, "A", Vec::new()));
    state.rooms.push(room(ROOM_B, "B", Vec::new()));
    state.profile.users.insert(
        ALICE.to_owned(),
        user_profile(ALICE, "Alice A.", &["alice"]),
    );

    reduce(
        &mut state,
        AppAction::InviteTargetQueryChanged {
            room_id: ROOM_A.to_owned(),
            query: "alice".to_owned(),
        },
    );
    assert_eq!(state.invite_workflow.query.room_id.as_deref(), Some(ROOM_A));

    assert_inert(
        &mut state,
        AppAction::InviteTargetQueryChanged {
            room_id: ROOM_B.to_owned(),
            query: "alice".to_owned(),
        },
    );
    assert_inert(
        &mut state,
        AppAction::InviteTargetSelected {
            room_id: ROOM_B.to_owned(),
            user_id: ALICE.to_owned(),
        },
    );
}

#[test]
fn invite_pending_fences_open_first_query_query_scope_select_remove_and_batch() {
    let mut pending = state_with_pending();
    assert!(matches!(
        pending.invite_workflow.operation,
        InviteOperationState::Pending { request_id: 41, .. }
    ));

    let mut open_pending = state_with_pending();
    open_pending.rooms.push(room(ROOM_B, "B", Vec::new()));
    assert_inert(
        &mut open_pending,
        AppAction::InviteWorkflowOpened {
            room_id: ROOM_B.to_owned(),
        },
    );
    assert_inert(
        &mut pending,
        AppAction::InviteTargetQueryChanged {
            room_id: ROOM_A.to_owned(),
            query: "alice".to_owned(),
        },
    );

    let mut first_query_pending = state_with_pending();
    first_query_pending.invite_workflow.query.room_id = None;
    assert_inert(
        &mut first_query_pending,
        AppAction::InviteTargetQueryChanged {
            room_id: ROOM_A.to_owned(),
            query: "alice".to_owned(),
        },
    );

    assert_inert(
        &mut pending,
        AppAction::InviteScopeSelected {
            room_id: ROOM_A.to_owned(),
            scope: InviteScopeSelection::RoomOnly,
        },
    );
    assert_inert(
        &mut pending,
        AppAction::InviteTargetSelected {
            room_id: ROOM_A.to_owned(),
            user_id: CAROL.to_owned(),
        },
    );
    assert_inert(
        &mut pending,
        AppAction::InviteTargetRemoved {
            user_id: ALICE.to_owned(),
        },
    );
    assert_inert(
        &mut pending,
        AppAction::InviteBatchRequested {
            request_id: 42,
            room_id: ROOM_A.to_owned(),
            user_ids: vec![ALICE.to_owned(), BOB.to_owned()],
            scope: InviteScopeSelection::ParentSpaceAndRoom {
                space_id: SPACE_A.to_owned(),
            },
        },
    );
}

#[test]
fn invite_scope_requires_ready_known_active_destination_and_matching_plan() {
    let mut state = ready_room_with_parent_space();
    state.rooms.push(room(ROOM_B, "B", Vec::new()));
    reduce(
        &mut state,
        AppAction::InviteWorkflowOpened {
            room_id: ROOM_A.to_owned(),
        },
    );

    assert_inert(
        &mut state,
        AppAction::InviteScopeSelected {
            room_id: ROOM_B.to_owned(),
            scope: InviteScopeSelection::RoomOnly,
        },
    );

    let mut wrong_plan_destination = state.clone();
    wrong_plan_destination
        .invite_workflow
        .scope_plan
        .as_mut()
        .expect("scope plan")
        .room_id = ROOM_B.to_owned();
    assert_inert(
        &mut wrong_plan_destination,
        AppAction::InviteScopeSelected {
            room_id: ROOM_A.to_owned(),
            scope: InviteScopeSelection::RoomOnly,
        },
    );

    let mut absent_option = state.clone();
    absent_option
        .invite_workflow
        .scope_plan
        .as_mut()
        .expect("scope plan")
        .options
        .clear();
    assert_inert(
        &mut absent_option,
        AppAction::InviteScopeSelected {
            room_id: ROOM_A.to_owned(),
            scope: InviteScopeSelection::RoomOnly,
        },
    );

    let mut unknown_active = state;
    unknown_active.invite_workflow.query.room_id = Some("!gone:example.org".to_owned());
    assert_inert(
        &mut unknown_active,
        AppAction::InviteScopeSelected {
            room_id: "!gone:example.org".to_owned(),
            scope: InviteScopeSelection::RoomOnly,
        },
    );
}

#[test]
fn invite_target_selection_and_removal_require_current_selectable_state() {
    let mut state = ready_with_room(ROOM_A);
    state.profile.users.insert(
        ALICE.to_owned(),
        user_profile(ALICE, "Alice A.", &["alice"]),
    );
    state.room_management.settings =
        Some(room_settings(ROOM_A, vec![room_member(ALICE, "Alice A.")]));
    reduce(
        &mut state,
        AppAction::InviteWorkflowOpened {
            room_id: ROOM_A.to_owned(),
        },
    );
    reduce(
        &mut state,
        AppAction::InviteTargetQueryChanged {
            room_id: ROOM_A.to_owned(),
            query: "alice".to_owned(),
        },
    );
    assert_eq!(
        state.invite_workflow.query.candidates[0].status,
        InviteTargetCandidateStatus::AlreadyInDestination
    );
    assert_inert(
        &mut state,
        AppAction::InviteTargetSelected {
            room_id: ROOM_A.to_owned(),
            user_id: ALICE.to_owned(),
        },
    );

    let mut selected = ready_with_room(ROOM_A);
    reduce(
        &mut selected,
        AppAction::InviteWorkflowOpened {
            room_id: ROOM_A.to_owned(),
        },
    );
    selected
        .invite_workflow
        .selected_targets
        .push(selected_target(ALICE, "Alice A."));
    selected.rooms.clear();
    assert_inert(
        &mut selected,
        AppAction::InviteTargetRemoved {
            user_id: ALICE.to_owned(),
        },
    );

    let mut valid_removal = state_with_two_selected_targets();
    reduce(
        &mut valid_removal,
        AppAction::InviteTargetQueryChanged {
            room_id: ROOM_A.to_owned(),
            query: "alice".to_owned(),
        },
    );
    reduce(
        &mut valid_removal,
        AppAction::InviteTargetRemoved {
            user_id: ALICE.to_owned(),
        },
    );
    assert_eq!(
        valid_removal
            .invite_workflow
            .selected_targets
            .iter()
            .map(|target| target.user_id.as_str())
            .collect::<Vec<_>>(),
        vec![BOB]
    );
    assert_eq!(
        valid_removal.invite_workflow.query.candidates[0].user_id,
        ALICE
    );
    assert_eq!(
        valid_removal.invite_workflow.query.candidates[0].status,
        InviteTargetCandidateStatus::Selectable
    );
}

#[test]
fn invite_batch_requires_effective_scope_and_exact_ordered_targets() {
    let valid_ids = vec![ALICE.to_owned(), BOB.to_owned()];
    let valid_scope = InviteScopeSelection::ParentSpaceAndRoom {
        space_id: SPACE_A.to_owned(),
    };

    for action in [
        AppAction::InviteBatchRequested {
            request_id: 1,
            room_id: "!unknown:example.org".to_owned(),
            user_ids: valid_ids.clone(),
            scope: valid_scope.clone(),
        },
        AppAction::InviteBatchRequested {
            request_id: 2,
            room_id: SPACE_A.to_owned(),
            user_ids: valid_ids.clone(),
            scope: valid_scope.clone(),
        },
        AppAction::InviteBatchRequested {
            request_id: 3,
            room_id: ROOM_A.to_owned(),
            user_ids: vec![BOB.to_owned(), ALICE.to_owned()],
            scope: valid_scope.clone(),
        },
        AppAction::InviteBatchRequested {
            request_id: 4,
            room_id: ROOM_A.to_owned(),
            user_ids: vec![ALICE.to_owned()],
            scope: valid_scope.clone(),
        },
        AppAction::InviteBatchRequested {
            request_id: 5,
            room_id: ROOM_A.to_owned(),
            user_ids: vec![
                ALICE.to_owned(),
                BOB.to_owned(),
                "@extra:example.org".to_owned(),
            ],
            scope: valid_scope.clone(),
        },
        AppAction::InviteBatchRequested {
            request_id: 6,
            room_id: ROOM_A.to_owned(),
            user_ids: valid_ids.clone(),
            scope: InviteScopeSelection::ParentSpaceAndRoom {
                space_id: SPACE_B.to_owned(),
            },
        },
        AppAction::InviteBatchRequested {
            request_id: 7,
            room_id: ROOM_A.to_owned(),
            user_ids: valid_ids.clone(),
            scope: InviteScopeSelection::RoomOnly,
        },
    ] {
        let mut state = state_with_two_selected_targets();
        assert_inert(&mut state, action);
    }

    let mut empty = state_with_two_selected_targets();
    assert_inert(
        &mut empty,
        AppAction::InviteBatchRequested {
            request_id: 8,
            room_id: ROOM_A.to_owned(),
            user_ids: Vec::new(),
            scope: valid_scope.clone(),
        },
    );

    let mut no_plan = state_with_two_selected_targets();
    no_plan.invite_workflow.scope_plan = None;
    assert_inert(
        &mut no_plan,
        AppAction::InviteBatchRequested {
            request_id: 9,
            room_id: ROOM_A.to_owned(),
            user_ids: valid_ids.clone(),
            scope: valid_scope.clone(),
        },
    );

    let mut valid = state_with_two_selected_targets();
    reduce(
        &mut valid,
        AppAction::InviteBatchRequested {
            request_id: 10,
            room_id: ROOM_A.to_owned(),
            user_ids: valid_ids.clone(),
            scope: valid_scope.clone(),
        },
    );
    assert_eq!(
        valid.invite_workflow.operation,
        InviteOperationState::Pending {
            request_id: 10,
            room_id: ROOM_A.to_owned(),
            user_ids: valid_ids,
            scope: valid_scope,
        }
    );
}

#[test]
fn invite_batch_uses_plan_default_when_selected_scope_is_none() {
    let mut state = state_with_two_selected_targets();
    let default_scope = state
        .invite_workflow
        .scope_plan
        .as_ref()
        .expect("scope plan")
        .default_scope
        .clone();
    state.invite_workflow.selected_scope = None;

    reduce(
        &mut state,
        AppAction::InviteBatchRequested {
            request_id: 10,
            room_id: ROOM_A.to_owned(),
            user_ids: vec![ALICE.to_owned(), BOB.to_owned()],
            scope: default_scope.clone(),
        },
    );

    assert_eq!(
        state.invite_workflow.operation,
        InviteOperationState::Pending {
            request_id: 10,
            room_id: ROOM_A.to_owned(),
            user_ids: vec![ALICE.to_owned(), BOB.to_owned()],
            scope: default_scope,
        }
    );
}

#[test]
fn invite_space_open_first_query_select_and_batch_flow_is_admitted() {
    let mut opened = ready_state();
    opened.spaces.push(SpaceSummary {
        space_id: SPACE_A.to_owned(),
        display_name: "Project Space".to_owned(),
        avatar: None,
        child_room_ids: Vec::new(),
    });
    reduce(
        &mut opened,
        AppAction::InviteWorkflowOpened {
            room_id: SPACE_A.to_owned(),
        },
    );
    assert_eq!(
        opened
            .invite_workflow
            .scope_plan
            .as_ref()
            .expect("space plan")
            .destination_kind,
        InviteDestinationKind::Space
    );

    let mut first_query = ready_state();
    first_query.spaces.push(SpaceSummary {
        space_id: SPACE_A.to_owned(),
        display_name: "Project Space".to_owned(),
        avatar: None,
        child_room_ids: Vec::new(),
    });
    first_query.profile.users.insert(
        ALICE.to_owned(),
        user_profile(ALICE, "Alice A.", &["alice"]),
    );
    reduce(
        &mut first_query,
        AppAction::InviteTargetQueryChanged {
            room_id: SPACE_A.to_owned(),
            query: "alice".to_owned(),
        },
    );
    assert_eq!(
        first_query.invite_workflow.query.room_id.as_deref(),
        Some(SPACE_A)
    );
    assert_eq!(
        first_query
            .invite_workflow
            .scope_plan
            .as_ref()
            .expect("first space query plan")
            .destination_kind,
        InviteDestinationKind::Space
    );
    reduce(
        &mut first_query,
        AppAction::InviteTargetSelected {
            room_id: SPACE_A.to_owned(),
            user_id: ALICE.to_owned(),
        },
    );
    reduce(
        &mut first_query,
        AppAction::InviteBatchRequested {
            request_id: 11,
            room_id: SPACE_A.to_owned(),
            user_ids: vec![ALICE.to_owned()],
            scope: InviteScopeSelection::RoomOnly,
        },
    );
    assert!(matches!(
        first_query.invite_workflow.operation,
        InviteOperationState::Pending { request_id: 11, .. }
    ));
}

#[test]
fn invite_settlements_require_exact_request_and_destination_correlation() {
    let mut wrong_destination = state_with_pending();
    assert_inert(
        &mut wrong_destination,
        AppAction::InviteBatchCompleted {
            request_id: 41,
            room_id: ROOM_B.to_owned(),
            results: vec![room_result(ALICE)],
        },
    );
    assert_inert(
        &mut wrong_destination,
        AppAction::InviteBatchFailed {
            request_id: 41,
            room_id: ROOM_B.to_owned(),
            kind: OperationFailureKind::Network,
        },
    );

    let mut wrong_request = state_with_pending();
    assert_inert(
        &mut wrong_request,
        AppAction::InviteBatchCompleted {
            request_id: 40,
            room_id: ROOM_A.to_owned(),
            results: vec![room_result(ALICE)],
        },
    );

    let mut failed = state_with_pending();
    let selected_before = failed.invite_workflow.selected_targets.clone();
    reduce(
        &mut failed,
        AppAction::InviteBatchFailed {
            request_id: 41,
            room_id: ROOM_A.to_owned(),
            kind: OperationFailureKind::Network,
        },
    );
    assert!(matches!(
        failed.invite_workflow.operation,
        InviteOperationState::Failed {
            request_id: 41,
            ref room_id,
            kind: OperationFailureKind::Network,
        } if room_id == ROOM_A
    ));
    assert_eq!(failed.invite_workflow.selected_targets, selected_before);

    let mut completed = state_with_pending();
    reduce(
        &mut completed,
        AppAction::InviteBatchCompleted {
            request_id: 41,
            room_id: ROOM_A.to_owned(),
            results: vec![room_result(ALICE)],
        },
    );
    assert!(matches!(
        completed.invite_workflow.operation,
        InviteOperationState::Completed {
            request_id: 41,
            ref room_id,
            ..
        } if room_id == ROOM_A
    ));
    assert!(completed.invite_workflow.selected_targets.is_empty());
}

#[test]
fn invite_failed_and_completed_operations_can_retry_or_resubmit() {
    let mut failed_retry = state_with_pending();
    reduce(
        &mut failed_retry,
        AppAction::InviteBatchFailed {
            request_id: 41,
            room_id: ROOM_A.to_owned(),
            kind: OperationFailureKind::Network,
        },
    );
    reduce(
        &mut failed_retry,
        AppAction::InviteBatchRequested {
            request_id: 42,
            room_id: ROOM_A.to_owned(),
            user_ids: vec![ALICE.to_owned(), BOB.to_owned()],
            scope: InviteScopeSelection::ParentSpaceAndRoom {
                space_id: SPACE_A.to_owned(),
            },
        },
    );
    assert!(matches!(
        failed_retry.invite_workflow.operation,
        InviteOperationState::Pending { request_id: 42, .. }
    ));

    let mut completed_resubmit = state_with_pending();
    reduce(
        &mut completed_resubmit,
        AppAction::InviteBatchCompleted {
            request_id: 41,
            room_id: ROOM_A.to_owned(),
            results: vec![room_result(ALICE)],
        },
    );
    reduce(
        &mut completed_resubmit,
        AppAction::InviteTargetQueryChanged {
            room_id: ROOM_A.to_owned(),
            query: "alice".to_owned(),
        },
    );
    reduce(
        &mut completed_resubmit,
        AppAction::InviteTargetSelected {
            room_id: ROOM_A.to_owned(),
            user_id: ALICE.to_owned(),
        },
    );
    reduce(
        &mut completed_resubmit,
        AppAction::InviteBatchRequested {
            request_id: 43,
            room_id: ROOM_A.to_owned(),
            user_ids: vec![ALICE.to_owned()],
            scope: InviteScopeSelection::ParentSpaceAndRoom {
                space_id: SPACE_A.to_owned(),
            },
        },
    );
    assert!(matches!(
        completed_resubmit.invite_workflow.operation,
        InviteOperationState::Pending { request_id: 43, .. }
    ));
}

#[test]
fn invite_capability_blocked_pending_owner_can_settle_by_correlation() {
    let mut state = state_with_pending();
    state.rooms.clear();
    state.spaces.clear();
    state.session = SessionState::CapabilityBlocked {
        info: session_info(),
        failure: SlidingSyncCapabilityFailureKind::Unsupported,
    };

    assert_inert(
        &mut state,
        AppAction::InviteBatchCompleted {
            request_id: 41,
            room_id: ROOM_B.to_owned(),
            results: vec![room_result(ALICE)],
        },
    );
    reduce(
        &mut state,
        AppAction::InviteBatchCompleted {
            request_id: 41,
            room_id: ROOM_A.to_owned(),
            results: vec![room_result(ALICE)],
        },
    );
    assert!(matches!(
        state.invite_workflow.operation,
        InviteOperationState::Completed { request_id: 41, .. }
    ));
}

#[test]
fn invite_pending_cleanup_serializes_gate_logout_and_switch_before_late_settlement() {
    let mut gated = state_with_pending();
    reduce(&mut gated, AppAction::SessionLocked);
    assert_eq!(
        gated.session,
        SessionState::Provisional {
            info: session_info(),
            phase: ProvisionalPhase::DiscoveringMethods,
        }
    );
    assert_eq!(gated.invite_workflow, InviteWorkflowState::default());
    let gated_after_cleanup = gated.clone();
    assert_inert(
        &mut gated,
        AppAction::InviteBatchCompleted {
            request_id: 41,
            room_id: ROOM_A.to_owned(),
            results: vec![room_result(ALICE)],
        },
    );
    assert_eq!(gated, gated_after_cleanup);

    let mut logging_out = state_with_pending();
    reduce(&mut logging_out, AppAction::LogoutRequested);
    assert_eq!(logging_out.session, SessionState::LoggingOut);
    assert_eq!(logging_out.invite_workflow, InviteWorkflowState::default());
    assert_inert(
        &mut logging_out,
        AppAction::InviteBatchFailed {
            request_id: 41,
            room_id: ROOM_A.to_owned(),
            kind: OperationFailureKind::Network,
        },
    );
    reduce(&mut logging_out, AppAction::LogoutFinished);
    assert_eq!(logging_out.session, SessionState::SignedOut);
    assert_eq!(logging_out.invite_workflow, InviteWorkflowState::default());

    let mut switching = state_with_pending();
    reduce(
        &mut switching,
        AppAction::SwitchAccountRequested {
            info: SessionInfo {
                homeserver: "https://other.example.org".to_owned(),
                user_id: "@other:example.org".to_owned(),
                device_id: "OTHER".to_owned(),
                authentication_method: Default::default(),
            },
        },
    );
    assert!(matches!(
        switching.session,
        SessionState::SwitchingAccount { .. }
    ));
    assert_eq!(switching.invite_workflow, InviteWorkflowState::default());
    assert_inert(
        &mut switching,
        AppAction::InviteBatchCompleted {
            request_id: 41,
            room_id: ROOM_A.to_owned(),
            results: vec![room_result(ALICE)],
        },
    );
}

#[test]
fn invite_workflow_close_is_unconditional_cleanup() {
    let mut ready = state_with_pending();
    assert!(reduce(&mut ready, AppAction::InviteWorkflowClosed).is_empty());
    assert_eq!(ready.invite_workflow, InviteWorkflowState::default());

    let mut recovery = state_with_policy_session(SessionState::AwaitingVerification {
        info: session_info(),
        gate: recovery_gate(),
    });
    reduce(
        &mut recovery,
        AppAction::InviteWorkflowOpened {
            room_id: ROOM_A.to_owned(),
        },
    );
    reduce(&mut recovery, AppAction::InviteWorkflowClosed);
    assert_eq!(recovery.invite_workflow, InviteWorkflowState::default());

    let mut signed_out = AppState::default();
    signed_out.invite_workflow.query.query = "draft".to_owned();
    reduce(&mut signed_out, AppAction::InviteWorkflowClosed);
    assert_eq!(signed_out.invite_workflow, InviteWorkflowState::default());
}
