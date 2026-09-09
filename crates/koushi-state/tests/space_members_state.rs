use koushi_state::{
    AppAction, AppState, OperationFailureKind, SessionInfo, SessionState, SpaceMemberEntry,
    SpaceMemberInviteOutcome, SpaceMemberMembership, SpaceMembersOperationState,
    SpaceMembersProjection, UserProfile, admit_space_member_cancellation,
    admit_space_member_invite, admit_space_members_load, reduce,
};

const SPACE_ID: &str = "!space:example.invalid";
const CHILD_ROOM_ID: &str = "!child:example.invalid";
const USER_ID: &str = "@child:example.invalid";

fn ready_state() -> AppState {
    AppState {
        session: SessionState::Ready(SessionInfo {
            homeserver: "https://example.invalid".to_owned(),
            user_id: "@self:example.invalid".to_owned(),
            device_id: "DEVICE".to_owned(),
            authentication_method: koushi_state::SessionAuthenticationMethod::Unknown,
        }),
        ..AppState::default()
    }
}

fn child_only_entry() -> SpaceMemberEntry {
    SpaceMemberEntry {
        user_id: USER_ID.to_owned(),
        display_name: Some("Child Room Name".to_owned()),
        display_label: "Child Room Name".to_owned(),
        original_display_label: "Child Room Name".to_owned(),
        avatar_url: None,
        power_level: Some(0),
        role: koushi_state::RoomMemberRole::User,
        membership: SpaceMemberMembership::ChildRoomOnly,
        child_room_ids: vec![CHILD_ROOM_ID.to_owned()],
        invite_pending: false,
        role_options: Vec::new(),
    }
}

fn projection(generation: u64, child_only: Vec<SpaceMemberEntry>) -> SpaceMembersProjection {
    SpaceMembersProjection {
        space_id: SPACE_ID.to_owned(),
        generation,
        space_joined: Vec::new(),
        space_invited: Vec::new(),
        child_room_only: child_only,
        child_room_count: 1,
        complete_child_room_count: 1,
        incomplete_child_room_count: 0,
        power_levels_revision: None,
        can_edit_roles: false,
    }
}

fn space_entry(membership: SpaceMemberMembership) -> SpaceMemberEntry {
    SpaceMemberEntry {
        membership,
        invite_pending: false,
        ..child_only_entry()
    }
}

fn state_with_settled_invite() -> AppState {
    let mut state = ready_state();
    reduce(
        &mut state,
        AppAction::SpaceMembersLoadRequested {
            request_id: 70,
            space_id: SPACE_ID.to_owned(),
            generation: 2,
        },
    );
    reduce(
        &mut state,
        AppAction::SpaceMembersLoaded {
            request_id: 70,
            projection: projection(2, vec![child_only_entry()]),
        },
    );
    reduce(
        &mut state,
        AppAction::SpaceMemberInviteRequested {
            request_id: 71,
            space_id: SPACE_ID.to_owned(),
            user_id: USER_ID.to_owned(),
            generation: 2,
        },
    );
    reduce(
        &mut state,
        AppAction::SpaceMemberInviteSettled {
            request_id: 71,
            space_id: SPACE_ID.to_owned(),
            user_id: USER_ID.to_owned(),
            generation: 2,
            outcome: SpaceMemberInviteOutcome::Invited,
        },
    );
    state
}

#[test]
fn cancellation_admission_requires_the_current_invited_entry() {
    let mut state = state_with_settled_invite();
    assert!(admit_space_member_cancellation(&state.space_members, SPACE_ID, USER_ID, 2).is_ok());

    state.space_members.space_invited.clear();
    assert!(admit_space_member_cancellation(&state.space_members, SPACE_ID, USER_ID, 2).is_err());
}

#[test]
fn invited_cancellation_is_fenced_and_settles_to_idle_after_removing_the_invite() {
    let mut state = state_with_settled_invite();
    reduce(
        &mut state,
        AppAction::SpaceMemberInviteCancellationRequested {
            request_id: 72,
            space_id: SPACE_ID.to_owned(),
            user_id: USER_ID.to_owned(),
            generation: 2,
        },
    );
    assert!(matches!(
        state.space_members.operation,
        SpaceMembersOperationState::CancellingInvite {
            request_id: 72,
            ref space_id,
            ref user_id,
            generation: 2,
        } if space_id == SPACE_ID && user_id == USER_ID
    ));
    assert_eq!(state.space_members.space_invited.len(), 1);

    let before_stale = state.clone();
    for (request_id, space_id, user_id, generation) in [
        (71, SPACE_ID.to_owned(), USER_ID.to_owned(), 2),
        (
            72,
            "!other-space:example.invalid".to_owned(),
            USER_ID.to_owned(),
            2,
        ),
        (
            72,
            SPACE_ID.to_owned(),
            "@other-user:example.invalid".to_owned(),
            2,
        ),
        (72, SPACE_ID.to_owned(), USER_ID.to_owned(), 1),
    ] {
        reduce(
            &mut state,
            AppAction::SpaceMemberInviteCancellationSettled {
                request_id,
                space_id,
                user_id,
                generation,
                outcome: SpaceMemberInviteOutcome::Cancelled,
            },
        );
        assert_eq!(state, before_stale);
    }

    reduce(
        &mut state,
        AppAction::SpaceMemberInviteCancellationSettled {
            request_id: 72,
            space_id: SPACE_ID.to_owned(),
            user_id: USER_ID.to_owned(),
            generation: 2,
            outcome: SpaceMemberInviteOutcome::Cancelled,
        },
    );
    assert!(state.space_members.space_invited.is_empty());
    assert!(matches!(
        state.space_members.operation,
        SpaceMembersOperationState::Idle
    ));
}

#[test]
fn not_invited_cancellation_reconciles_a_joined_projection_without_removing_it() {
    let mut state = state_with_settled_invite();
    reduce(
        &mut state,
        AppAction::SpaceMemberInviteCancellationRequested {
            request_id: 73,
            space_id: SPACE_ID.to_owned(),
            user_id: USER_ID.to_owned(),
            generation: 2,
        },
    );
    reduce(
        &mut state,
        AppAction::SpaceMembersProjectionReconciled {
            request_id: 73,
            projection: SpaceMembersProjection {
                space_joined: vec![space_entry(SpaceMemberMembership::SpaceJoined)],
                ..projection(2, Vec::new())
            },
            profiles: Vec::new(),
        },
    );
    reduce(
        &mut state,
        AppAction::SpaceMemberInviteCancellationSettled {
            request_id: 73,
            space_id: SPACE_ID.to_owned(),
            user_id: USER_ID.to_owned(),
            generation: 2,
            outcome: SpaceMemberInviteOutcome::NotInvited,
        },
    );

    assert_eq!(state.space_members.space_joined[0].user_id, USER_ID);
    assert!(state.space_members.space_invited.is_empty());
    assert!(matches!(
        state.space_members.operation,
        SpaceMembersOperationState::Idle
    ));
}

#[test]
fn cancellation_transport_failure_retains_the_invited_entry() {
    let mut state = state_with_settled_invite();
    reduce(
        &mut state,
        AppAction::SpaceMemberInviteCancellationRequested {
            request_id: 74,
            space_id: SPACE_ID.to_owned(),
            user_id: USER_ID.to_owned(),
            generation: 2,
        },
    );
    reduce(
        &mut state,
        AppAction::SpaceMemberInviteCancellationSettled {
            request_id: 74,
            space_id: SPACE_ID.to_owned(),
            user_id: USER_ID.to_owned(),
            generation: 2,
            outcome: SpaceMemberInviteOutcome::Failed(OperationFailureKind::Sdk),
        },
    );

    assert_eq!(state.space_members.space_invited[0].user_id, USER_ID);
    assert!(matches!(
        state.space_members.operation,
        SpaceMembersOperationState::Failed {
            request_id: 74,
            ref space_id,
            user_id: Some(ref user_id),
            generation: 2,
            kind: OperationFailureKind::Sdk,
        } if space_id == SPACE_ID && user_id == USER_ID
    ));
}

#[test]
fn failed_cancellation_can_retry_only_for_the_exact_invited_context() {
    let mut state = state_with_settled_invite();
    reduce(
        &mut state,
        AppAction::SpaceMemberInviteCancellationRequested {
            request_id: 74,
            space_id: SPACE_ID.to_owned(),
            user_id: USER_ID.to_owned(),
            generation: 2,
        },
    );
    reduce(
        &mut state,
        AppAction::SpaceMemberInviteCancellationSettled {
            request_id: 74,
            space_id: SPACE_ID.to_owned(),
            user_id: USER_ID.to_owned(),
            generation: 2,
            outcome: SpaceMemberInviteOutcome::Failed(OperationFailureKind::Sdk),
        },
    );

    assert!(admit_space_member_cancellation(&state.space_members, SPACE_ID, USER_ID, 2).is_ok());
    for (space_id, user_id, generation) in [
        ("!other-space:example.invalid", USER_ID, 2),
        (SPACE_ID, "@other-user:example.invalid", 2),
        (SPACE_ID, USER_ID, 1),
    ] {
        assert!(
            admit_space_member_cancellation(&state.space_members, space_id, user_id, generation)
                .is_err()
        );
    }

    reduce(
        &mut state,
        AppAction::SpaceMemberInviteCancellationRequested {
            request_id: 75,
            space_id: SPACE_ID.to_owned(),
            user_id: USER_ID.to_owned(),
            generation: 2,
        },
    );
    assert!(matches!(
        state.space_members.operation,
        SpaceMembersOperationState::CancellingInvite {
            request_id: 75,
            ref space_id,
            ref user_id,
            generation: 2,
        } if space_id == SPACE_ID && user_id == USER_ID
    ));

    reduce(
        &mut state,
        AppAction::SpaceMemberInviteCancellationSettled {
            request_id: 75,
            space_id: SPACE_ID.to_owned(),
            user_id: USER_ID.to_owned(),
            generation: 2,
            outcome: SpaceMemberInviteOutcome::Cancelled,
        },
    );
    assert!(state.space_members.space_invited.is_empty());
    assert!(matches!(
        state.space_members.operation,
        SpaceMembersOperationState::Idle
    ));

    let mut load_failure = state_with_settled_invite();
    load_failure.space_members.operation = SpaceMembersOperationState::Failed {
        request_id: 76,
        space_id: SPACE_ID.to_owned(),
        user_id: None,
        generation: 2,
        kind: OperationFailureKind::Network,
    };
    assert!(
        admit_space_member_cancellation(&load_failure.space_members, SPACE_ID, USER_ID, 2).is_err()
    );

    let mut invite_failure = state_with_settled_invite();
    invite_failure.space_members.space_invited.clear();
    invite_failure
        .space_members
        .child_room_only
        .push(child_only_entry());
    invite_failure.space_members.operation = SpaceMembersOperationState::Failed {
        request_id: 77,
        space_id: SPACE_ID.to_owned(),
        user_id: Some(USER_ID.to_owned()),
        generation: 2,
        kind: OperationFailureKind::Sdk,
    };
    assert!(
        admit_space_member_cancellation(&invite_failure.space_members, SPACE_ID, USER_ID, 2)
            .is_err()
    );
}

#[test]
fn load_admission_requires_the_active_navigation_space_when_selection_is_unset() {
    let mut state = ready_state();
    assert!(state.space_members.selected_space_id.is_none());
    state.navigation.active_space_id = Some(SPACE_ID.to_owned());

    assert!(admit_space_members_load(&state, SPACE_ID, 1).is_ok());
    assert_eq!(
        admit_space_members_load(&state, "!other-space:example.invalid", 1),
        Err(koushi_state::SpaceMembersCommandRejection::WrongSpace)
    );

    state.navigation.active_space_id = None;
    assert_eq!(
        admit_space_members_load(&state, SPACE_ID, 1),
        Err(koushi_state::SpaceMembersCommandRejection::NoSelectedSpace)
    );
}

#[test]
fn background_projection_reconciles_an_idle_state() {
    let mut state = ready_state();
    reduce(
        &mut state,
        AppAction::SpaceMembersLoadRequested {
            request_id: 40,
            space_id: SPACE_ID.to_owned(),
            generation: 2,
        },
    );
    reduce(
        &mut state,
        AppAction::SpaceMembersLoaded {
            request_id: 40,
            projection: projection(2, vec![child_only_entry()]),
        },
    );

    reduce(
        &mut state,
        AppAction::SpaceMembersBackgroundProjectionReconciled {
            request_id: 43,
            space_id: SPACE_ID.to_owned(),
            generation: 2,
            projection: SpaceMembersProjection {
                space_joined: vec![space_entry(SpaceMemberMembership::SpaceJoined)],
                ..projection(2, vec![])
            },
            profiles: Vec::new(),
        },
    );
    assert_eq!(state.space_members.space_joined.len(), 1);
    assert!(state.space_members.child_room_only.is_empty());
}

#[test]
fn background_projection_resolves_observed_profiles_before_publishing_rows() {
    let mut state = ready_state();
    reduce(
        &mut state,
        AppAction::SpaceMembersLoadRequested {
            request_id: 43,
            space_id: SPACE_ID.to_owned(),
            generation: 2,
        },
    );
    reduce(
        &mut state,
        AppAction::SpaceMembersLoaded {
            request_id: 43,
            projection: projection(2, vec![child_only_entry()]),
        },
    );

    let mut entry = space_entry(SpaceMemberMembership::SpaceJoined);
    entry.display_name = None;
    entry.display_label = "Unknown user".to_owned();
    entry.original_display_label = "Unknown user".to_owned();
    reduce(
        &mut state,
        AppAction::SpaceMembersBackgroundProjectionReconciled {
            request_id: 44,
            space_id: SPACE_ID.to_owned(),
            generation: 2,
            projection: SpaceMembersProjection {
                space_joined: vec![entry],
                ..projection(2, vec![])
            },
            profiles: vec![UserProfile {
                user_id: USER_ID.to_owned(),
                display_name: Some("Observed profile".to_owned()),
                display_label: "Observed profile".to_owned(),
                original_display_label: "Observed profile".to_owned(),
                mention_search_terms: Vec::new(),
                avatar: None,
            }],
        },
    );

    assert_eq!(
        state.space_members.space_joined[0].display_label,
        "Observed profile"
    );
    assert_eq!(
        state.profile.users[USER_ID].display_name.as_deref(),
        Some("Observed profile")
    );
}

#[test]
fn background_projection_settles_an_invite_when_authoritative_invite_or_join_appears() {
    for membership in [
        SpaceMemberMembership::SpaceInvited,
        SpaceMemberMembership::SpaceJoined,
    ] {
        let mut state = ready_state();
        reduce(
            &mut state,
            AppAction::SpaceMembersLoadRequested {
                request_id: 44,
                space_id: SPACE_ID.to_owned(),
                generation: 2,
            },
        );
        reduce(
            &mut state,
            AppAction::SpaceMembersLoaded {
                request_id: 44,
                projection: projection(2, vec![child_only_entry()]),
            },
        );
        reduce(
            &mut state,
            AppAction::SpaceMemberInviteRequested {
                request_id: 45,
                space_id: SPACE_ID.to_owned(),
                user_id: USER_ID.to_owned(),
                generation: 2,
            },
        );
        reduce(
            &mut state,
            AppAction::SpaceMembersBackgroundProjectionReconciled {
                request_id: 46,
                space_id: SPACE_ID.to_owned(),
                generation: 2,
                projection: SpaceMembersProjection {
                    space_joined: (membership == SpaceMemberMembership::SpaceJoined)
                        .then(|| space_entry(membership.clone()))
                        .into_iter()
                        .collect(),
                    space_invited: (membership == SpaceMemberMembership::SpaceInvited)
                        .then(|| space_entry(membership.clone()))
                        .into_iter()
                        .collect(),
                    ..projection(2, vec![])
                },
                profiles: Vec::new(),
            },
        );
        assert!(matches!(
            state.space_members.operation,
            SpaceMembersOperationState::Idle
        ));
        assert_eq!(
            state.space_members.space_joined.len(),
            (membership == SpaceMemberMembership::SpaceJoined) as usize
        );
        assert_eq!(
            state.space_members.space_invited.len(),
            (membership == SpaceMemberMembership::SpaceInvited) as usize
        );
    }
}

#[test]
fn failed_invite_background_projection_moves_authoritative_user_and_clears_failure() {
    let mut state = ready_state();
    reduce(
        &mut state,
        AppAction::SpaceMembersLoadRequested {
            request_id: 60,
            space_id: SPACE_ID.to_owned(),
            generation: 2,
        },
    );
    reduce(
        &mut state,
        AppAction::SpaceMembersLoaded {
            request_id: 60,
            projection: projection(2, vec![child_only_entry()]),
        },
    );
    reduce(
        &mut state,
        AppAction::SpaceMemberInviteRequested {
            request_id: 61,
            space_id: SPACE_ID.to_owned(),
            user_id: USER_ID.to_owned(),
            generation: 2,
        },
    );
    reduce(
        &mut state,
        AppAction::SpaceMemberInviteSettled {
            request_id: 61,
            space_id: SPACE_ID.to_owned(),
            user_id: USER_ID.to_owned(),
            generation: 2,
            outcome: SpaceMemberInviteOutcome::Failed(OperationFailureKind::Network),
        },
    );

    reduce(
        &mut state,
        AppAction::SpaceMembersBackgroundProjectionReconciled {
            request_id: 62,
            space_id: SPACE_ID.to_owned(),
            generation: 2,
            projection: SpaceMembersProjection {
                space_joined: vec![space_entry(SpaceMemberMembership::SpaceJoined)],
                ..projection(2, Vec::new())
            },
            profiles: Vec::new(),
        },
    );

    assert!(matches!(
        state.space_members.operation,
        SpaceMembersOperationState::Idle
    ));
    assert_eq!(state.space_members.space_joined.len(), 1);
    assert!(state.space_members.space_invited.is_empty());
    assert!(state.space_members.child_room_only.is_empty());
}

#[test]
fn failed_invite_background_projection_keeps_failed_child_only_retry_state() {
    let mut state = ready_state();
    reduce(
        &mut state,
        AppAction::SpaceMembersLoadRequested {
            request_id: 63,
            space_id: SPACE_ID.to_owned(),
            generation: 2,
        },
    );
    reduce(
        &mut state,
        AppAction::SpaceMembersLoaded {
            request_id: 63,
            projection: projection(2, vec![child_only_entry()]),
        },
    );
    reduce(
        &mut state,
        AppAction::SpaceMemberInviteRequested {
            request_id: 64,
            space_id: SPACE_ID.to_owned(),
            user_id: USER_ID.to_owned(),
            generation: 2,
        },
    );
    reduce(
        &mut state,
        AppAction::SpaceMemberInviteSettled {
            request_id: 64,
            space_id: SPACE_ID.to_owned(),
            user_id: USER_ID.to_owned(),
            generation: 2,
            outcome: SpaceMemberInviteOutcome::Failed(OperationFailureKind::Forbidden),
        },
    );

    let mut refreshed_entry = child_only_entry();
    refreshed_entry.user_id = "@refreshed:example.invalid".to_owned();
    refreshed_entry.display_label = "Refreshed child".to_owned();
    refreshed_entry.original_display_label = "Refreshed child".to_owned();
    reduce(
        &mut state,
        AppAction::SpaceMembersBackgroundProjectionReconciled {
            request_id: 65,
            space_id: SPACE_ID.to_owned(),
            generation: 2,
            projection: projection(2, vec![refreshed_entry]),
            profiles: Vec::new(),
        },
    );

    assert!(matches!(
        state.space_members.operation,
        SpaceMembersOperationState::Failed {
            user_id: Some(ref failed_user),
            ..
        } if failed_user == USER_ID
    ));
    assert_eq!(
        state
            .space_members
            .child_room_only
            .iter()
            .map(|entry| entry.user_id.as_str())
            .collect::<Vec<_>>(),
        vec![USER_ID, "@refreshed:example.invalid"]
    );
}

#[test]
fn failed_member_load_accepts_complete_background_projection_and_clears_failure() {
    let mut state = ready_state();
    reduce(
        &mut state,
        AppAction::SpaceMembersLoadRequested {
            request_id: 66,
            space_id: SPACE_ID.to_owned(),
            generation: 2,
        },
    );
    reduce(
        &mut state,
        AppAction::SpaceMembersLoadFailed {
            request_id: 66,
            space_id: SPACE_ID.to_owned(),
            generation: 2,
            kind: OperationFailureKind::Sdk,
        },
    );

    reduce(
        &mut state,
        AppAction::SpaceMembersBackgroundProjectionReconciled {
            request_id: 67,
            space_id: SPACE_ID.to_owned(),
            generation: 2,
            projection: projection(2, vec![child_only_entry()]),
            profiles: Vec::new(),
        },
    );

    assert!(matches!(
        state.space_members.operation,
        SpaceMembersOperationState::Idle
    ));
    assert_eq!(state.space_members.child_room_only.len(), 1);
}

#[test]
fn failed_member_load_keeps_failure_for_incomplete_background_projection() {
    let mut state = ready_state();
    reduce(
        &mut state,
        AppAction::SpaceMembersLoadRequested {
            request_id: 68,
            space_id: SPACE_ID.to_owned(),
            generation: 2,
        },
    );
    reduce(
        &mut state,
        AppAction::SpaceMembersLoadFailed {
            request_id: 68,
            space_id: SPACE_ID.to_owned(),
            generation: 2,
            kind: OperationFailureKind::Network,
        },
    );

    reduce(
        &mut state,
        AppAction::SpaceMembersBackgroundProjectionReconciled {
            request_id: 69,
            space_id: SPACE_ID.to_owned(),
            generation: 2,
            projection: SpaceMembersProjection {
                incomplete_child_room_count: 1,
                complete_child_room_count: 0,
                ..projection(2, vec![child_only_entry()])
            },
            profiles: Vec::new(),
        },
    );

    assert!(matches!(
        state.space_members.operation,
        SpaceMembersOperationState::Failed { user_id: None, .. }
    ));
    assert_eq!(state.space_members.child_room_only.len(), 1);
}

#[test]
fn background_projection_preserves_last_known_entries_when_child_scope_is_incomplete() {
    let mut state = ready_state();
    reduce(
        &mut state,
        AppAction::SpaceMembersLoadRequested {
            request_id: 42,
            space_id: SPACE_ID.to_owned(),
            generation: 2,
        },
    );
    reduce(
        &mut state,
        AppAction::SpaceMembersLoaded {
            request_id: 42,
            projection: projection(2, vec![child_only_entry()]),
        },
    );

    reduce(
        &mut state,
        AppAction::SpaceMembersBackgroundProjectionReconciled {
            request_id: 48,
            space_id: SPACE_ID.to_owned(),
            generation: 2,
            projection: SpaceMembersProjection {
                incomplete_child_room_count: 1,
                complete_child_room_count: 0,
                ..projection(2, vec![])
            },
            profiles: Vec::new(),
        },
    );

    assert!(state.space_members.space_joined.is_empty());
    assert_eq!(state.space_members.child_room_only.len(), 1);
}

#[test]
fn incomplete_background_projection_keeps_pending_invite_until_authoritative_membership() {
    let mut state = ready_state();
    reduce(
        &mut state,
        AppAction::SpaceMembersLoadRequested {
            request_id: 48,
            space_id: SPACE_ID.to_owned(),
            generation: 2,
        },
    );
    reduce(
        &mut state,
        AppAction::SpaceMembersLoaded {
            request_id: 48,
            projection: projection(2, vec![child_only_entry()]),
        },
    );
    reduce(
        &mut state,
        AppAction::SpaceMemberInviteRequested {
            request_id: 49,
            space_id: SPACE_ID.to_owned(),
            user_id: USER_ID.to_owned(),
            generation: 2,
        },
    );

    reduce(
        &mut state,
        AppAction::SpaceMembersBackgroundProjectionReconciled {
            request_id: 50,
            space_id: SPACE_ID.to_owned(),
            generation: 2,
            projection: SpaceMembersProjection {
                incomplete_child_room_count: 1,
                complete_child_room_count: 0,
                ..projection(2, vec![])
            },
            profiles: Vec::new(),
        },
    );

    assert!(matches!(
        state.space_members.operation,
        SpaceMembersOperationState::Inviting { request_id: 49, .. }
    ));
    assert_eq!(state.space_members.space_invited.len(), 1);
    assert!(state.space_members.space_invited[0].invite_pending);
}

#[test]
fn background_projection_is_ignored_while_the_explicit_load_is_loading() {
    let mut state = ready_state();
    reduce(
        &mut state,
        AppAction::SpaceMembersLoadRequested {
            request_id: 41,
            space_id: SPACE_ID.to_owned(),
            generation: 2,
        },
    );

    reduce(
        &mut state,
        AppAction::SpaceMembersBackgroundProjectionReconciled {
            request_id: 49,
            space_id: SPACE_ID.to_owned(),
            generation: 2,
            projection: SpaceMembersProjection {
                space_joined: vec![space_entry(SpaceMemberMembership::SpaceJoined)],
                ..projection(2, vec![])
            },
            profiles: Vec::new(),
        },
    );

    assert!(matches!(
        state.space_members.operation,
        SpaceMembersOperationState::Loading {
            request_id: Some(41),
            ..
        }
    ));
    assert!(state.space_members.space_joined.is_empty());
}

#[test]
fn stale_background_projection_cannot_cross_space_or_generation_fences() {
    let mut state = ready_state();
    reduce(
        &mut state,
        AppAction::SpaceMembersLoadRequested {
            request_id: 42,
            space_id: SPACE_ID.to_owned(),
            generation: 2,
        },
    );
    reduce(
        &mut state,
        AppAction::SpaceMembersLoaded {
            request_id: 42,
            projection: projection(2, vec![child_only_entry()]),
        },
    );

    for stale_projection in [
        SpaceMembersProjection {
            space_id: "!other-space:example.invalid".to_owned(),
            ..projection(2, vec![space_entry(SpaceMemberMembership::SpaceJoined)])
        },
        SpaceMembersProjection {
            generation: 1,
            ..projection(1, vec![space_entry(SpaceMemberMembership::SpaceJoined)])
        },
    ] {
        reduce(
            &mut state,
            AppAction::SpaceMembersBackgroundProjectionReconciled {
                request_id: 47,
                space_id: stale_projection.space_id.clone(),
                generation: stale_projection.generation,
                projection: stale_projection,
                profiles: Vec::new(),
            },
        );
    }

    assert!(state.space_members.space_joined.is_empty());
    assert_eq!(state.space_members.child_room_only.len(), 1);
}

#[test]
fn loaded_projection_is_generation_fenced_and_keeps_three_sections() {
    let mut state = ready_state();

    reduce(
        &mut state,
        AppAction::SpaceMembersLoadRequested {
            request_id: 1,
            space_id: SPACE_ID.to_owned(),
            generation: 2,
        },
    );
    reduce(
        &mut state,
        AppAction::SpaceMembersLoaded {
            request_id: 1,
            projection: projection(1, vec![]),
        },
    );
    assert_eq!(state.space_members.generation, 2);
    assert!(state.space_members.child_room_only.is_empty());

    reduce(
        &mut state,
        AppAction::SpaceMembersLoaded {
            request_id: 1,
            projection: projection(2, vec![child_only_entry()]),
        },
    );
    assert_eq!(state.space_members.space_joined.len(), 0);
    assert_eq!(state.space_members.space_invited.len(), 0);
    assert_eq!(state.space_members.child_room_only.len(), 1);
    assert_eq!(state.space_members.child_room_only[0].user_id, USER_ID);
}

#[test]
fn invite_moves_child_only_person_to_pending_optimistically_and_deduplicates() {
    let mut state = ready_state();
    reduce(
        &mut state,
        AppAction::SpaceMembersLoadRequested {
            request_id: 2,
            space_id: SPACE_ID.to_owned(),
            generation: 2,
        },
    );
    reduce(
        &mut state,
        AppAction::SpaceMembersLoaded {
            request_id: 2,
            projection: projection(2, vec![child_only_entry()]),
        },
    );

    reduce(
        &mut state,
        AppAction::SpaceMemberInviteRequested {
            request_id: 7,
            space_id: SPACE_ID.to_owned(),
            user_id: USER_ID.to_owned(),
            generation: 2,
        },
    );
    let first_operation = state.space_members.operation.clone();
    assert!(matches!(
        first_operation,
        SpaceMembersOperationState::Inviting { .. }
    ));
    assert!(state.space_members.child_room_only.is_empty());
    assert_eq!(state.space_members.space_invited.len(), 1);
    assert!(state.space_members.space_invited[0].invite_pending);

    reduce(
        &mut state,
        AppAction::SpaceMemberInviteRequested {
            request_id: 8,
            space_id: SPACE_ID.to_owned(),
            user_id: USER_ID.to_owned(),
            generation: 2,
        },
    );
    assert_eq!(state.space_members.operation, first_operation);
    assert_eq!(state.space_members.space_invited.len(), 1);
}

#[test]
fn invite_settlement_reconciles_success_already_joined_and_failure() {
    let mut state = ready_state();
    reduce(
        &mut state,
        AppAction::SpaceMembersLoadRequested {
            request_id: 3,
            space_id: SPACE_ID.to_owned(),
            generation: 2,
        },
    );
    reduce(
        &mut state,
        AppAction::SpaceMembersLoaded {
            request_id: 3,
            projection: projection(2, vec![child_only_entry()]),
        },
    );

    reduce(
        &mut state,
        AppAction::SpaceMemberInviteRequested {
            request_id: 7,
            space_id: SPACE_ID.to_owned(),
            user_id: USER_ID.to_owned(),
            generation: 2,
        },
    );
    reduce(
        &mut state,
        AppAction::SpaceMemberInviteSettled {
            request_id: 7,
            space_id: SPACE_ID.to_owned(),
            user_id: USER_ID.to_owned(),
            generation: 2,
            outcome: SpaceMemberInviteOutcome::Invited,
        },
    );
    assert!(matches!(
        state.space_members.operation,
        SpaceMembersOperationState::Idle
    ));
    assert_eq!(state.space_members.space_invited.len(), 1);

    reduce(
        &mut state,
        AppAction::SpaceMembersLoadRequested {
            request_id: 4,
            space_id: SPACE_ID.to_owned(),
            generation: 2,
        },
    );
    reduce(
        &mut state,
        AppAction::SpaceMembersLoaded {
            request_id: 4,
            projection: SpaceMembersProjection {
                space_joined: vec![SpaceMemberEntry {
                    membership: SpaceMemberMembership::SpaceJoined,
                    invite_pending: false,
                    ..child_only_entry()
                }],
                ..projection(2, vec![])
            },
        },
    );
    assert_eq!(state.space_members.space_joined.len(), 1);
    assert!(state.space_members.space_invited.is_empty());

    reduce(
        &mut state,
        AppAction::SpaceMembersLoadRequested {
            request_id: 5,
            space_id: SPACE_ID.to_owned(),
            generation: 2,
        },
    );
    reduce(
        &mut state,
        AppAction::SpaceMembersLoaded {
            request_id: 5,
            projection: projection(2, vec![child_only_entry()]),
        },
    );
    reduce(
        &mut state,
        AppAction::SpaceMemberInviteRequested {
            request_id: 9,
            space_id: SPACE_ID.to_owned(),
            user_id: USER_ID.to_owned(),
            generation: 2,
        },
    );
    reduce(
        &mut state,
        AppAction::SpaceMemberInviteSettled {
            request_id: 9,
            space_id: SPACE_ID.to_owned(),
            user_id: USER_ID.to_owned(),
            generation: 2,
            outcome: SpaceMemberInviteOutcome::Failed(OperationFailureKind::Forbidden),
        },
    );
    assert!(state.space_members.space_invited.is_empty());
    assert_eq!(state.space_members.child_room_only.len(), 1);
    assert!(matches!(
        state.space_members.operation,
        SpaceMembersOperationState::Failed { .. }
    ));
}

#[test]
fn stale_invite_settlement_cannot_mutate_new_space_generation() {
    let mut state = ready_state();
    reduce(
        &mut state,
        AppAction::SpaceMembersLoadRequested {
            request_id: 4,
            space_id: SPACE_ID.to_owned(),
            generation: 4,
        },
    );
    reduce(
        &mut state,
        AppAction::SpaceMemberInviteSettled {
            request_id: 1,
            space_id: SPACE_ID.to_owned(),
            user_id: USER_ID.to_owned(),
            generation: 3,
            outcome: SpaceMemberInviteOutcome::Failed(OperationFailureKind::Sdk),
        },
    );
    assert!(matches!(
        state.space_members.operation,
        SpaceMembersOperationState::Loading { .. }
    ));
}

#[test]
fn loading_projection_observes_non_empty_child_profiles_for_receipt_fallback() {
    let mut state = ready_state();
    reduce(
        &mut state,
        AppAction::UserProfilesUpdated {
            profiles: vec![koushi_state::UserProfile {
                user_id: USER_ID.to_owned(),
                display_name: Some("Child Room Name".to_owned()),
                display_label: String::new(),
                original_display_label: String::new(),
                mention_search_terms: Vec::new(),
                avatar: None,
            }],
        },
    );
    reduce(
        &mut state,
        AppAction::LiveRoomReceiptsWindowReconciled {
            room_id: CHILD_ROOM_ID.to_owned(),
            scoped_event_ids: Vec::new(),
            receipts_by_event: vec![koushi_state::LiveEventReceipts {
                event_id: "$event:example.invalid".to_owned(),
                receipts: vec![koushi_state::LiveReadReceipt {
                    user_id: USER_ID.to_owned(),
                    display_name: None,
                    original_display_label: String::new(),
                    avatar: None,
                    timestamp_ms: Some(1),
                }],
            }],
        },
    );
    assert_eq!(
        state.live_signals.rooms[CHILD_ROOM_ID].receipts_by_event["$event:example.invalid"].readers
            [0]
        .display_name
        .as_deref(),
        Some("Child Room Name")
    );
}

#[test]
fn same_generation_out_of_order_loads_cannot_overwrite_newer_request() {
    let mut state = ready_state();
    reduce(
        &mut state,
        AppAction::SpaceMembersLoadRequested {
            request_id: 10,
            space_id: SPACE_ID.to_owned(),
            generation: 2,
        },
    );
    reduce(
        &mut state,
        AppAction::SpaceMembersLoadRequested {
            request_id: 11,
            space_id: SPACE_ID.to_owned(),
            generation: 2,
        },
    );

    reduce(
        &mut state,
        AppAction::SpaceMembersLoaded {
            request_id: 11,
            projection: projection(2, vec![child_only_entry()]),
        },
    );
    reduce(
        &mut state,
        AppAction::SpaceMembersLoaded {
            request_id: 10,
            projection: projection(2, vec![]),
        },
    );

    assert_eq!(state.space_members.child_room_only.len(), 1);
    assert!(matches!(
        state.space_members.operation,
        SpaceMembersOperationState::Idle
    ));
}

#[test]
fn load_result_cannot_clobber_an_active_invite() {
    let mut state = ready_state();
    reduce(
        &mut state,
        AppAction::SpaceMembersLoadRequested {
            request_id: 20,
            space_id: SPACE_ID.to_owned(),
            generation: 2,
        },
    );
    reduce(
        &mut state,
        AppAction::SpaceMembersLoaded {
            request_id: 20,
            projection: projection(2, vec![child_only_entry()]),
        },
    );
    reduce(
        &mut state,
        AppAction::SpaceMemberInviteRequested {
            request_id: 21,
            space_id: SPACE_ID.to_owned(),
            user_id: USER_ID.to_owned(),
            generation: 2,
        },
    );

    reduce(
        &mut state,
        AppAction::SpaceMembersLoaded {
            request_id: 20,
            projection: projection(2, vec![]),
        },
    );

    assert_eq!(state.space_members.space_invited.len(), 1);
    assert!(state.space_members.space_invited[0].invite_pending);
    assert!(matches!(
        state.space_members.operation,
        SpaceMembersOperationState::Inviting { request_id: 21, .. }
    ));
}

#[test]
fn invite_reconciliation_applies_authoritative_projection_and_profile_observation() {
    let mut state = ready_state();
    reduce(
        &mut state,
        AppAction::SpaceMembersLoadRequested {
            request_id: 22,
            space_id: SPACE_ID.to_owned(),
            generation: 2,
        },
    );
    reduce(
        &mut state,
        AppAction::SpaceMembersLoaded {
            request_id: 22,
            projection: projection(2, vec![child_only_entry()]),
        },
    );
    reduce(
        &mut state,
        AppAction::SpaceMemberInviteRequested {
            request_id: 23,
            space_id: SPACE_ID.to_owned(),
            user_id: USER_ID.to_owned(),
            generation: 2,
        },
    );

    let authoritative_entry = SpaceMemberEntry {
        membership: SpaceMemberMembership::SpaceJoined,
        display_name: Some("Authoritative space label".to_owned()),
        display_label: "Authoritative space label".to_owned(),
        original_display_label: "Authoritative space label".to_owned(),
        invite_pending: false,
        ..child_only_entry()
    };
    reduce(
        &mut state,
        AppAction::SpaceMembersProjectionReconciled {
            request_id: 23,
            projection: SpaceMembersProjection {
                space_joined: vec![authoritative_entry],
                ..projection(2, Vec::new())
            },
            profiles: vec![UserProfile {
                user_id: USER_ID.to_owned(),
                display_name: Some("Observed profile label".to_owned()),
                display_label: String::new(),
                original_display_label: String::new(),
                mention_search_terms: Vec::new(),
                avatar: None,
            }],
        },
    );

    assert_eq!(
        state.profile.users[USER_ID].display_name.as_deref(),
        Some("Observed profile label")
    );
    assert_eq!(state.space_members.space_joined.len(), 1);
    assert_eq!(
        state.space_members.space_joined[0].display_label,
        "Authoritative space label"
    );
    assert!(state.space_members.space_invited.is_empty());
    assert!(matches!(
        state.space_members.operation,
        SpaceMembersOperationState::Inviting { request_id: 23, .. }
    ));

    reduce(
        &mut state,
        AppAction::SpaceMemberInviteSettled {
            request_id: 23,
            space_id: SPACE_ID.to_owned(),
            user_id: USER_ID.to_owned(),
            generation: 2,
            outcome: SpaceMemberInviteOutcome::AlreadyJoined,
        },
    );
    assert!(matches!(
        state.space_members.operation,
        SpaceMembersOperationState::Idle
    ));
    assert_eq!(state.space_members.space_joined.len(), 1);
    assert!(state.space_members.space_invited.is_empty());
}

#[test]
fn incomplete_child_projection_preserves_last_known_and_pending_entries() {
    let mut state = ready_state();
    reduce(
        &mut state,
        AppAction::SpaceMembersLoadRequested {
            request_id: 30,
            space_id: SPACE_ID.to_owned(),
            generation: 2,
        },
    );
    reduce(
        &mut state,
        AppAction::SpaceMembersLoaded {
            request_id: 30,
            projection: projection(2, vec![child_only_entry()]),
        },
    );
    reduce(
        &mut state,
        AppAction::SpaceMembersLoadRequested {
            request_id: 31,
            space_id: SPACE_ID.to_owned(),
            generation: 2,
        },
    );
    reduce(
        &mut state,
        AppAction::SpaceMembersLoaded {
            request_id: 31,
            projection: SpaceMembersProjection {
                incomplete_child_room_count: 1,
                complete_child_room_count: 0,
                ..projection(2, Vec::new())
            },
        },
    );
    assert_eq!(state.space_members.child_room_only.len(), 1);

    reduce(
        &mut state,
        AppAction::SpaceMemberInviteRequested {
            request_id: 32,
            space_id: SPACE_ID.to_owned(),
            user_id: USER_ID.to_owned(),
            generation: 2,
        },
    );
    reduce(
        &mut state,
        AppAction::SpaceMembersLoaded {
            request_id: 31,
            projection: SpaceMembersProjection {
                incomplete_child_room_count: 1,
                complete_child_room_count: 0,
                ..projection(2, Vec::new())
            },
        },
    );
    assert_eq!(state.space_members.space_invited.len(), 1);
    assert!(state.space_members.space_invited[0].invite_pending);
}

#[test]
fn space_level_load_failure_preserves_last_valid_projection() {
    let mut state = ready_state();
    reduce(
        &mut state,
        AppAction::SpaceMembersLoadRequested {
            request_id: 60,
            space_id: SPACE_ID.to_owned(),
            generation: 2,
        },
    );
    reduce(
        &mut state,
        AppAction::SpaceMembersLoaded {
            request_id: 60,
            projection: SpaceMembersProjection {
                space_joined: vec![SpaceMemberEntry {
                    membership: SpaceMemberMembership::SpaceJoined,
                    ..child_only_entry()
                }],
                space_invited: vec![SpaceMemberEntry {
                    membership: SpaceMemberMembership::SpaceInvited,
                    ..child_only_entry()
                }],
                child_room_only: Vec::new(),
                ..projection(2, Vec::new())
            },
        },
    );
    let previous = state.space_members.clone();

    reduce(
        &mut state,
        AppAction::SpaceMembersLoadRequested {
            request_id: 61,
            space_id: SPACE_ID.to_owned(),
            generation: 2,
        },
    );
    reduce(
        &mut state,
        AppAction::SpaceMembersLoadFailed {
            request_id: 61,
            space_id: SPACE_ID.to_owned(),
            generation: 2,
            kind: OperationFailureKind::Sdk,
        },
    );

    assert_eq!(state.space_members.space_joined, previous.space_joined);
    assert_eq!(state.space_members.space_invited, previous.space_invited);
    assert_eq!(
        state.space_members.child_room_only,
        previous.child_room_only
    );
    assert_eq!(
        state.space_members.child_room_count,
        previous.child_room_count
    );
    assert!(matches!(
        state.space_members.operation,
        SpaceMembersOperationState::Failed { request_id: 61, .. }
    ));
}

#[test]
fn invite_admission_rejects_wrong_space_and_duplicate_without_side_effect_ticket() {
    let mut state = ready_state();
    state.space_members.selected_space_id = Some(SPACE_ID.to_owned());
    state.space_members.generation = 2;
    state.space_members.child_room_only = vec![child_only_entry()];

    assert!(admit_space_member_invite(&state.space_members, SPACE_ID, USER_ID, 2).is_ok());
    assert!(
        admit_space_member_invite(&state.space_members, "!other:example.invalid", USER_ID, 2)
            .is_err()
    );

    reduce(
        &mut state,
        AppAction::SpaceMemberInviteRequested {
            request_id: 40,
            space_id: SPACE_ID.to_owned(),
            user_id: USER_ID.to_owned(),
            generation: 2,
        },
    );
    assert!(admit_space_member_invite(&state.space_members, SPACE_ID, USER_ID, 2).is_err());
}

#[test]
fn cached_avatar_refreshes_space_member_row_when_room_avatar_is_missing() {
    let mut state = ready_state();
    state.profile.users.insert(
        USER_ID.to_owned(),
        koushi_state::UserProfile {
            user_id: USER_ID.to_owned(),
            display_name: None,
            display_label: String::new(),
            original_display_label: String::new(),
            mention_search_terms: Vec::new(),
            avatar: Some(koushi_state::AvatarImage {
                mxc_uri: "mxc://example.invalid/cached-avatar".to_owned(),
                thumbnail: koushi_state::AvatarThumbnailState::NotRequested,
            }),
        },
    );
    reduce(
        &mut state,
        AppAction::SpaceMembersLoadRequested {
            request_id: 50,
            space_id: SPACE_ID.to_owned(),
            generation: 2,
        },
    );
    reduce(
        &mut state,
        AppAction::SpaceMembersLoaded {
            request_id: 50,
            projection: projection(2, vec![child_only_entry()]),
        },
    );
    assert_eq!(
        state.space_members.child_room_only[0].avatar_url.as_deref(),
        Some("mxc://example.invalid/cached-avatar")
    );
}

#[test]
fn app_state_without_space_members_deserializes_with_default_projection() {
    let mut value = serde_json::to_value(AppState::default()).expect("serialize app state");
    value
        .as_object_mut()
        .expect("app state object")
        .remove("space_members");

    let restored: AppState = serde_json::from_value(value).expect("legacy app state");
    assert_eq!(restored.space_members, Default::default());
}
