use koushi_state::{
    AppAction, AppState, RoomMemberRole, SessionInfo, SessionState, SpaceMemberEntry,
    SpaceMemberMembership, SpaceMemberRoleFailureKind, SpaceMemberRoleOption,
    SpaceMemberRoleUpdateOutcome, SpaceMembersCommandRejection, SpaceMembersOperationState,
    SpaceMembersProjection, admit_space_member_invite, admit_space_member_role,
    admit_space_members_load, reduce,
};

const SPACE_ID: &str = "!space:example.invalid";
const ADMIN_ID: &str = "@admin:example.invalid";
const TARGET_ID: &str = "@target:example.invalid";

fn ready_state() -> AppState {
    AppState {
        session: SessionState::Ready(SessionInfo {
            homeserver: "https://example.invalid".into(),
            user_id: ADMIN_ID.into(),
            device_id: "DEVICE".into(),
            authentication_method: koushi_state::SessionAuthenticationMethod::Unknown,
        }),
        ..AppState::default()
    }
}

fn entry(
    user_id: &str,
    power_level: Option<i64>,
    membership: SpaceMemberMembership,
) -> SpaceMemberEntry {
    SpaceMemberEntry {
        user_id: user_id.into(),
        display_name: Some(user_id.into()),
        display_label: user_id.into(),
        original_display_label: user_id.into(),
        avatar_url: None,
        power_level,
        role: RoomMemberRole::from_power_level(power_level),
        membership,
        child_room_ids: Vec::new(),
        invite_pending: false,
        role_options: Vec::new(),
    }
}

fn projection(revision: Option<&str>, can_edit_roles: bool) -> SpaceMembersProjection {
    let mut target = entry(TARGET_ID, Some(0), SpaceMemberMembership::SpaceJoined);
    target.role_options = vec![
        SpaceMemberRoleOption {
            power_level: 0,
            role: RoomMemberRole::User,
            requires_confirmation: false,
        },
        SpaceMemberRoleOption {
            power_level: 50,
            role: RoomMemberRole::Moderator,
            requires_confirmation: false,
        },
        SpaceMemberRoleOption {
            power_level: 100,
            role: RoomMemberRole::Administrator,
            requires_confirmation: true,
        },
    ];
    SpaceMembersProjection {
        space_id: SPACE_ID.into(),
        generation: 4,
        space_joined: vec![
            entry(ADMIN_ID, Some(100), SpaceMemberMembership::SpaceJoined),
            target,
        ],
        space_invited: Vec::new(),
        child_room_only: Vec::new(),
        child_room_count: 1,
        complete_child_room_count: 0,
        incomplete_child_room_count: 1,
        power_levels_revision: revision.map(str::to_owned),
        can_edit_roles,
    }
}

fn loaded_state() -> AppState {
    let mut state = ready_state();
    state.navigation.active_space_id = Some(SPACE_ID.into());
    reduce(
        &mut state,
        AppAction::SpaceMembersLoadRequested {
            request_id: 1,
            space_id: SPACE_ID.into(),
            generation: 4,
        },
    );
    reduce(
        &mut state,
        AppAction::SpaceMembersLoaded {
            request_id: 1,
            projection: projection(Some("$pl0:example.invalid"), true),
        },
    );
    state
}

#[test]
fn authorized_direct_space_role_admission_is_strict_and_child_sync_independent() {
    let state = loaded_state();
    assert!(
        admit_space_member_role(
            &state,
            SPACE_ID,
            TARGET_ID,
            4,
            Some("$pl0:example.invalid"),
            0,
            50,
            false,
        )
        .is_ok()
    );
    assert!(
        admit_space_member_role(
            &state,
            SPACE_ID,
            TARGET_ID,
            4,
            Some("$pl0:example.invalid"),
            0,
            50,
            false,
        )
        .is_ok()
    );
}

#[test]
fn role_update_reducer_has_no_optimistic_target_mutation_and_exact_settlement() {
    let mut state = loaded_state();
    reduce(
        &mut state,
        AppAction::SpaceMemberRoleUpdateRequested {
            request_id: 2,
            space_id: SPACE_ID.into(),
            user_id: TARGET_ID.into(),
            generation: 4,
            expected_power_levels_revision: Some("$pl0:example.invalid".into()),
            expected_power_level: 0,
            power_level: 50,
            confirmed: false,
        },
    );
    assert!(matches!(
        state.space_members.operation,
        SpaceMembersOperationState::UpdatingRole { .. }
    ));
    assert_eq!(
        state
            .space_members
            .space_joined
            .iter()
            .find(|e| e.user_id == TARGET_ID)
            .unwrap()
            .power_level,
        Some(0)
    );

    let mut next = projection(Some("$pl1:example.invalid"), true);
    next.space_joined
        .iter_mut()
        .find(|e| e.user_id == TARGET_ID)
        .unwrap()
        .power_level = Some(50);
    next.space_joined
        .iter_mut()
        .find(|e| e.user_id == TARGET_ID)
        .unwrap()
        .role = RoomMemberRole::Moderator;
    reduce(
        &mut state,
        AppAction::SpaceMemberRoleUpdateSettled {
            request_id: 2,
            space_id: SPACE_ID.into(),
            user_id: TARGET_ID.into(),
            generation: 4,
            outcome: SpaceMemberRoleUpdateOutcome::Succeeded,
            sent_revision: Some("$pl1:example.invalid".into()),
            projection: Some(next),
        },
    );
    assert!(matches!(
        state.space_members.operation,
        SpaceMembersOperationState::Idle
    ));
    assert_eq!(
        state.space_members.power_levels_revision.as_deref(),
        Some("$pl1:example.invalid")
    );
    assert_eq!(
        state
            .space_members
            .space_joined
            .iter()
            .find(|e| e.user_id == TARGET_ID)
            .unwrap()
            .power_level,
        Some(50)
    );
}

#[test]
fn stale_role_failure_retries_against_advanced_current_projection() {
    let mut state = loaded_state();
    reduce(
        &mut state,
        AppAction::SpaceMemberRoleUpdateRequested {
            request_id: 3,
            space_id: SPACE_ID.into(),
            user_id: TARGET_ID.into(),
            generation: 4,
            expected_power_levels_revision: Some("$pl0:example.invalid".into()),
            expected_power_level: 0,
            power_level: 50,
            confirmed: false,
        },
    );

    let mut advanced = projection(Some("$pl1:example.invalid"), true);
    let target = advanced
        .space_joined
        .iter_mut()
        .find(|entry| entry.user_id == TARGET_ID)
        .unwrap();
    target.power_level = Some(25);
    target.role = RoomMemberRole::from_power_level(Some(25));
    reduce(
        &mut state,
        AppAction::SpaceMemberRoleUpdateSettled {
            request_id: 3,
            space_id: SPACE_ID.into(),
            user_id: TARGET_ID.into(),
            generation: 4,
            outcome: SpaceMemberRoleUpdateOutcome::Failed(SpaceMemberRoleFailureKind::Stale),
            sent_revision: None,
            projection: Some(advanced),
        },
    );
    assert_eq!(
        state.space_members.power_levels_revision.as_deref(),
        Some("$pl1:example.invalid")
    );
    assert_eq!(
        state
            .space_members
            .space_joined
            .iter()
            .find(|entry| entry.user_id == TARGET_ID)
            .unwrap()
            .power_level,
        Some(25)
    );
    assert!(matches!(
        state.space_members.operation,
        SpaceMembersOperationState::RoleUpdateFailed {
            kind: SpaceMemberRoleFailureKind::Stale,
            expected_power_levels_revision: Some(ref revision),
            expected_power_level: 0,
            power_level: 50,
            ..
        } if revision == "$pl0:example.invalid"
    ));

    assert_eq!(
        admit_space_member_role(
            &state,
            SPACE_ID,
            TARGET_ID,
            4,
            Some("$pl1:example.invalid"),
            0,
            50,
            false,
        ),
        Err(SpaceMembersCommandRejection::RoleCurrentPowerMismatch)
    );
    assert_eq!(
        admit_space_member_role(
            &state,
            SPACE_ID,
            TARGET_ID,
            4,
            Some("$pl1:example.invalid"),
            25,
            0,
            false,
        ),
        Err(SpaceMembersCommandRejection::RoleUpdateAlreadyInFlight)
    );
    assert_eq!(
        admit_space_member_role(
            &state,
            SPACE_ID,
            "@other:example.invalid",
            4,
            Some("$pl1:example.invalid"),
            25,
            50,
            false,
        ),
        Err(SpaceMembersCommandRejection::RoleUpdateAlreadyInFlight)
    );
    assert_eq!(
        admit_space_member_role(
            &state,
            SPACE_ID,
            TARGET_ID,
            5,
            Some("$pl1:example.invalid"),
            25,
            50,
            false,
        ),
        Err(SpaceMembersCommandRejection::StaleGeneration)
    );

    reduce(
        &mut state,
        AppAction::SpaceMemberRoleUpdateRequested {
            request_id: 4,
            space_id: SPACE_ID.into(),
            user_id: TARGET_ID.into(),
            generation: 4,
            expected_power_levels_revision: Some("$pl1:example.invalid".into()),
            expected_power_level: 25,
            power_level: 50,
            confirmed: false,
        },
    );
    assert!(matches!(
        state.space_members.operation,
        SpaceMembersOperationState::UpdatingRole {
            request_id: 4,
            expected_power_levels_revision: Some(ref revision),
            expected_power_level: 25,
            power_level: 50,
            ..
        } if revision == "$pl1:example.invalid"
    ));
}

#[test]
fn role_admission_rejects_every_stale_or_unauthorized_matrix_case() {
    let base = loaded_state();
    let mut cases = Vec::new();

    let mut not_ready = base.clone();
    not_ready.session = SessionState::SignedOut;
    cases.push((not_ready, SpaceMembersCommandRejection::RoleSessionRequired));

    let mut wrong_space = base.clone();
    wrong_space.navigation.active_space_id = Some("!other:example.invalid".into());
    cases.push((wrong_space, SpaceMembersCommandRejection::WrongSpace));

    cases.push((base.clone(), SpaceMembersCommandRejection::StaleGeneration));
    cases.push((
        base.clone(),
        SpaceMembersCommandRejection::RoleRevisionMismatch,
    ));

    let mut forbidden = base.clone();
    forbidden.space_members.can_edit_roles = false;
    cases.push((forbidden, SpaceMembersCommandRejection::RoleNotEditable));

    for operation in [
        SpaceMembersOperationState::Loading {
            request_id: Some(9),
            space_id: SPACE_ID.into(),
            generation: 4,
        },
        SpaceMembersOperationState::Inviting {
            request_id: 9,
            space_id: SPACE_ID.into(),
            user_id: TARGET_ID.into(),
            generation: 4,
        },
        SpaceMembersOperationState::CancellingInvite {
            request_id: 9,
            space_id: SPACE_ID.into(),
            user_id: TARGET_ID.into(),
            generation: 4,
        },
    ] {
        let mut pending = base.clone();
        pending.space_members.operation = operation;
        cases.push((
            pending,
            SpaceMembersCommandRejection::RoleUpdateAlreadyInFlight,
        ));
    }

    let mut invited = base.clone();
    invited.space_members.space_joined.clear();
    invited.space_members.space_invited.push(entry(
        TARGET_ID,
        Some(0),
        SpaceMemberMembership::SpaceInvited,
    ));
    cases.push((invited, SpaceMembersCommandRejection::RoleTargetInvalid));

    for (user_id, membership, power_level) in [
        (
            "@missing:example.invalid",
            SpaceMemberMembership::SpaceJoined,
            None,
        ),
        (ADMIN_ID, SpaceMemberMembership::SpaceJoined, Some(100)),
        (TARGET_ID, SpaceMemberMembership::ChildRoomOnly, Some(0)),
    ] {
        let mut target_state = base.clone();
        target_state.space_members.space_joined = vec![entry(user_id, power_level, membership)];
        cases.push((
            target_state,
            SpaceMembersCommandRejection::RoleTargetInvalid,
        ));
    }

    let mut current_mismatch = base.clone();
    current_mismatch.space_members.space_joined[1].power_level = Some(50);
    cases.push((
        current_mismatch,
        SpaceMembersCommandRejection::RoleCurrentPowerMismatch,
    ));
    cases.push((
        base.clone(),
        SpaceMembersCommandRejection::RoleOptionUnavailable,
    ));
    cases.push((
        base.clone(),
        SpaceMembersCommandRejection::RoleConfirmationRequired,
    ));

    for (state, expected) in cases {
        let (generation, revision, expected_power, new_power, confirmed) =
            if matches!(expected, SpaceMembersCommandRejection::StaleGeneration) {
                (5, Some("$pl0:example.invalid"), 0, 50, false)
            } else if matches!(expected, SpaceMembersCommandRejection::RoleRevisionMismatch) {
                (4, Some("$stale:example.invalid"), 0, 50, false)
            } else if matches!(
                expected,
                SpaceMembersCommandRejection::RoleOptionUnavailable
            ) {
                (4, Some("$pl0:example.invalid"), 0, 25, false)
            } else if matches!(
                expected,
                SpaceMembersCommandRejection::RoleConfirmationRequired
            ) {
                (4, Some("$pl0:example.invalid"), 0, 100, false)
            } else {
                (4, Some("$pl0:example.invalid"), 0, 50, false)
            };
        let actual = admit_space_member_role(
            &state,
            SPACE_ID,
            TARGET_ID,
            generation,
            revision,
            expected_power,
            new_power,
            confirmed,
        )
        .expect_err("matrix rejection must be explicit");
        assert_eq!(actual, expected);
    }
}

#[test]
fn stale_settlements_are_ignored_and_network_forbidden_preserve_authoritative_role() {
    for failure in [
        SpaceMemberRoleFailureKind::Forbidden,
        SpaceMemberRoleFailureKind::Network,
    ] {
        let mut state = loaded_state();
        reduce(
            &mut state,
            AppAction::SpaceMemberRoleUpdateRequested {
                request_id: 10,
                space_id: SPACE_ID.into(),
                user_id: TARGET_ID.into(),
                generation: 4,
                expected_power_levels_revision: Some("$pl0:example.invalid".into()),
                expected_power_level: 0,
                power_level: 50,
                confirmed: false,
            },
        );
        let before = state.space_members.space_joined[1].clone();
        reduce(
            &mut state,
            AppAction::SpaceMemberRoleUpdateSettled {
                request_id: 99,
                space_id: SPACE_ID.into(),
                user_id: TARGET_ID.into(),
                generation: 4,
                outcome: SpaceMemberRoleUpdateOutcome::Failed(failure),
                sent_revision: None,
                projection: None,
            },
        );
        assert!(matches!(
            state.space_members.operation,
            SpaceMembersOperationState::UpdatingRole { .. }
        ));
        assert_eq!(state.space_members.space_joined[1], before);

        reduce(
            &mut state,
            AppAction::SpaceMemberRoleUpdateSettled {
                request_id: 10,
                space_id: SPACE_ID.into(),
                user_id: TARGET_ID.into(),
                generation: 4,
                outcome: SpaceMemberRoleUpdateOutcome::Failed(failure),
                sent_revision: None,
                projection: None,
            },
        );
        assert_eq!(state.space_members.space_joined[1], before);
        assert!(matches!(
            state.space_members.operation,
            SpaceMembersOperationState::RoleUpdateFailed { kind, .. } if kind == failure
        ));
    }
}

#[test]
fn invite_and_load_cannot_replace_an_active_role_update_but_failure_can_reload() {
    let mut state = loaded_state();
    let child_id = "@child:example.invalid";
    state.space_members.child_room_only.push(entry(
        child_id,
        Some(0),
        SpaceMemberMembership::ChildRoomOnly,
    ));
    reduce(
        &mut state,
        AppAction::SpaceMemberRoleUpdateRequested {
            request_id: 70,
            space_id: SPACE_ID.into(),
            user_id: TARGET_ID.into(),
            generation: 4,
            expected_power_levels_revision: Some("$pl0:example.invalid".into()),
            expected_power_level: 0,
            power_level: 50,
            confirmed: false,
        },
    );
    let before = state.clone();
    assert_eq!(
        admit_space_member_invite(&state.space_members, SPACE_ID, child_id, 4),
        Err(SpaceMembersCommandRejection::InviteAlreadyInFlight)
    );
    assert_eq!(
        admit_space_members_load(&state, SPACE_ID, 4),
        Err(SpaceMembersCommandRejection::LoadBlockedByInvite)
    );
    assert!(
        reduce(
            &mut state,
            AppAction::SpaceMemberInviteRequested {
                request_id: 71,
                space_id: SPACE_ID.into(),
                user_id: child_id.into(),
                generation: 4,
            },
        )
        .is_empty()
    );
    assert!(
        reduce(
            &mut state,
            AppAction::SpaceMembersLoadRequested {
                request_id: 72,
                space_id: SPACE_ID.into(),
                generation: 4,
            },
        )
        .is_empty()
    );
    assert_eq!(state, before);

    reduce(
        &mut state,
        AppAction::SpaceMemberRoleUpdateSettled {
            request_id: 70,
            space_id: SPACE_ID.into(),
            user_id: TARGET_ID.into(),
            generation: 4,
            outcome: SpaceMemberRoleUpdateOutcome::Failed(SpaceMemberRoleFailureKind::Stale),
            sent_revision: None,
            projection: None,
        },
    );
    assert!(admit_space_members_load(&state, SPACE_ID, 4).is_ok());
    assert!(
        !reduce(
            &mut state,
            AppAction::SpaceMembersLoadRequested {
                request_id: 73,
                space_id: SPACE_ID.into(),
                generation: 4,
            },
        )
        .is_empty()
    );
    assert!(matches!(
        state.space_members.operation,
        SpaceMembersOperationState::Loading {
            request_id: Some(73),
            ..
        }
    ));
}

#[test]
fn background_role_reconciliation_requires_a_new_authoritative_revision() {
    let mut state = loaded_state();
    reduce(
        &mut state,
        AppAction::SpaceMemberRoleUpdateRequested {
            request_id: 11,
            space_id: SPACE_ID.into(),
            user_id: TARGET_ID.into(),
            generation: 4,
            expected_power_levels_revision: Some("$pl0:example.invalid".into()),
            expected_power_level: 0,
            power_level: 50,
            confirmed: false,
        },
    );
    reduce(
        &mut state,
        AppAction::SpaceMemberRoleUpdateSettled {
            request_id: 11,
            space_id: SPACE_ID.into(),
            user_id: TARGET_ID.into(),
            generation: 4,
            outcome: SpaceMemberRoleUpdateOutcome::Failed(SpaceMemberRoleFailureKind::Timeout),
            sent_revision: Some("$pl1:example.invalid".into()),
            projection: None,
        },
    );

    let mut old_revision = projection(Some("$pl0:example.invalid"), true);
    old_revision.space_joined[1].power_level = Some(50);
    old_revision.space_joined[1].role = RoomMemberRole::Moderator;
    reduce(
        &mut state,
        AppAction::SpaceMembersBackgroundProjectionReconciled {
            request_id: 12,
            space_id: SPACE_ID.into(),
            generation: 4,
            projection: old_revision,
            profiles: Vec::new(),
        },
    );
    assert!(matches!(
        state.space_members.operation,
        SpaceMembersOperationState::RoleUpdateFailed { .. }
    ));

    let mut sent_revision = projection(Some("$pl1:example.invalid"), true);
    sent_revision.space_joined[1].power_level = Some(50);
    sent_revision.space_joined[1].role = RoomMemberRole::Moderator;
    reduce(
        &mut state,
        AppAction::SpaceMembersBackgroundProjectionReconciled {
            request_id: 13,
            space_id: SPACE_ID.into(),
            generation: 4,
            projection: sent_revision,
            profiles: Vec::new(),
        },
    );
    assert_eq!(
        state.space_members.operation,
        SpaceMembersOperationState::Idle
    );
}
