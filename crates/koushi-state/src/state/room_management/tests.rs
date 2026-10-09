use super::*;
use crate::SpaceSummary;
use crate::state::errors::OperationFailureKind;

fn snapshot_with_access(access: RoomAccessCondition) -> RoomSettingsSnapshot {
    RoomSettingsSnapshot {
        room_id: "!room:example.invalid".to_owned(),
        name: None,
        topic: None,
        avatar_url: None,
        canonical_alias: None,
        alternate_aliases: Vec::new(),
        share_link: None,
        join_rule: access.join_rule.unwrap_or(RoomJoinRule::Invite),
        history_visibility: RoomHistoryVisibility::Shared,
        access,
        permissions: RoomPermissionFacts::default(),
        members: Vec::new(),
    }
}

fn restricted(completeness: RestrictedConditions) -> RoomAccessCondition {
    RoomAccessCondition {
        join_rule: Some(RoomJoinRule::Restricted),
        restricted: Some(completeness),
        allow_targets: vec![RoomAllowTarget {
            kind: RoomAllowTargetKind::Space,
            room_id: "!space:example.invalid".to_owned(),
        }],
    }
}

#[test]
fn access_policy_is_canonical_and_submittable() {
    let policy = RoomAccessPolicy::new(
        RoomJoinRule::Restricted,
        vec![
            "!b:example.invalid".to_owned(),
            "!a:example.invalid".to_owned(),
            "!a:example.invalid".to_owned(),
            String::new(),
        ],
    );
    assert_eq!(
        policy.allow_targets,
        vec![
            "!a:example.invalid".to_owned(),
            "!b:example.invalid".to_owned()
        ]
    );
    assert!(policy.is_submittable());
    assert!(!RoomAccessPolicy::new(RoomJoinRule::Restricted, Vec::new()).is_submittable());
    assert!(
        !RoomAccessPolicy::new(RoomJoinRule::Public, vec!["!a:example.invalid".to_owned()])
            .is_submittable()
    );
    assert!(!RoomAccessPolicy::new(RoomJoinRule::Private, Vec::new()).is_submittable());
    assert!(!RoomAccessPolicy::new(RoomJoinRule::KnockRestricted, Vec::new()).is_submittable());
}

#[test]
fn access_policy_rejection_separates_unsupported_from_unverified() {
    let policy = RoomAccessPolicy::new(
        RoomJoinRule::Restricted,
        vec!["!s:example.invalid".to_owned()],
    );

    assert_eq!(
        snapshot_with_access(restricted(RestrictedConditions::MembershipOnly))
            .access_policy_rejection(&policy),
        None
    );
    assert_eq!(
        snapshot_with_access(restricted(RestrictedConditions::ConfirmedEmpty))
            .access_policy_rejection(&policy),
        None
    );
    assert_eq!(
        snapshot_with_access(restricted(RestrictedConditions::MembershipPlusUnsupported))
            .access_policy_rejection(&policy),
        Some(OperationFailureKind::UnsupportedPolicyCondition)
    );
    assert_eq!(
        snapshot_with_access(restricted(RestrictedConditions::UnsupportedOnly))
            .access_policy_rejection(&policy),
        Some(OperationFailureKind::UnsupportedPolicyCondition)
    );
    assert_eq!(
        snapshot_with_access(RoomAccessCondition {
            join_rule: None,
            restricted: Some(RestrictedConditions::NotInspected),
            allow_targets: Vec::new(),
        })
        .access_policy_rejection(&policy),
        Some(OperationFailureKind::PolicyNotVerified)
    );
    assert_eq!(
        snapshot_with_access(RoomAccessCondition {
            join_rule: Some(RoomJoinRule::Public),
            restricted: None,
            allow_targets: Vec::new(),
        })
        .access_policy_rejection(&policy),
        None
    );
    assert_eq!(
        snapshot_with_access(restricted(RestrictedConditions::MembershipOnly))
            .access_policy_rejection(&RoomAccessPolicy::new(RoomJoinRule::Restricted, Vec::new())),
        Some(OperationFailureKind::Invalid)
    );
}

#[test]
fn draft_comparison_is_canonical_and_ignores_a_reordered_server_list() {
    let settings = snapshot_with_access(restricted(RestrictedConditions::MembershipOnly));
    let mut draft = RoomAccessDraft::new(RoomAccessDraftScope::Room {
        room_id: "!room:example.invalid".to_owned(),
    });
    assert!(
        !draft.differs_from(&settings),
        "an empty draft is not a change"
    );
    draft.set_rule(Some(RoomJoinRule::Restricted));
    draft.set_allow_targets(vec!["!space:example.invalid".to_owned()]);
    assert!(
        !draft.differs_from(&settings),
        "the same canonical policy is not a change"
    );
    draft.set_allow_targets(vec![
        "!space:example.invalid".to_owned(),
        "!space:example.invalid".to_owned(),
    ]);
    assert!(
        !draft.differs_from(&settings),
        "a duplicated target is not a change"
    );
    draft.set_allow_targets(vec!["!other:example.invalid".to_owned()]);
    assert!(
        draft.differs_from(&settings),
        "a changed target is a change"
    );
    let revision = draft.revision;
    draft.set_allow_targets(vec!["!other:example.invalid".to_owned()]);
    assert_eq!(
        draft.revision, revision,
        "a no-op mutation does not bump the revision"
    );
}

#[test]
fn access_policy_change_uses_the_join_rule_permission() {
    let change = RoomSettingChange::AccessPolicy(RoomAccessPolicy::new(
        RoomJoinRule::Restricted,
        vec!["!s:example.invalid".to_owned()],
    ));
    let mut permissions = RoomPermissionFacts {
        can_change_join_rule: true,
        ..RoomPermissionFacts::default()
    };
    assert!(permissions.allows_setting_change(&change));
    permissions.can_change_join_rule = false;
    assert!(!permissions.allows_setting_change(&change));
}

#[test]
fn preview_combines_each_panel_with_the_confirmed_other_property() {
    let scope = RoomAccessDraftScope::Room {
        room_id: "!room:example.invalid".to_owned(),
    };
    let mut state = AppState::default();
    state.room_management.settings = Some(snapshot_with_access(restricted(
        RestrictedConditions::MembershipOnly,
    )));
    let mut draft = RoomAccessDraft::new(scope.clone());
    draft.set_rule(Some(RoomJoinRule::Public));
    draft.set_history(Some(RoomHistoryVisibility::Invited));
    state.room_management.draft = Some(draft);

    let access = preview_room_access_draft(&state, &scope, RoomAccessPreviewContext::Access);
    assert_eq!(
        access.outcome.join.message_id,
        "room.accessOutcomeJoinPublic"
    );
    assert_eq!(
        access.outcome.history.message_id,
        "room.accessOutcomeHistoryShared"
    );
    assert!(!access.confirmed, "the access panel shows an unsaved rule");

    let history = preview_room_access_draft(&state, &scope, RoomAccessPreviewContext::History);
    assert_eq!(
        history.outcome.join.message_id,
        "room.accessOutcomeJoinMembershipRoute"
    );
    assert_eq!(
        history.outcome.history.message_id,
        "room.accessOutcomeHistoryInvited"
    );
    assert!(
        !history.confirmed,
        "the history panel shows an unsaved value"
    );
}

#[test]
fn preview_of_a_stale_scope_uses_the_confirmed_value() {
    let mut state = AppState::default();
    state.room_management.settings = Some(snapshot_with_access(restricted(
        RestrictedConditions::MembershipOnly,
    )));
    state.room_management.draft = Some(RoomAccessDraft::new(RoomAccessDraftScope::Room {
        room_id: "!other:example.invalid".to_owned(),
    }));
    let scope = RoomAccessDraftScope::Room {
        room_id: "!room:example.invalid".to_owned(),
    };
    let preview = preview_room_access_draft(&state, &scope, RoomAccessPreviewContext::Access);
    assert!(preview.confirmed);
    assert_eq!(
        preview.outcome.join.message_id,
        "room.accessOutcomeJoinMembershipRoute"
    );
}

#[test]
fn access_panel_uses_confirmed_targets_without_a_draft_rule() {
    let scope = RoomAccessDraftScope::Room {
        room_id: "!room:example.invalid".to_owned(),
    };
    let mut state = AppState::default();
    state.room_management.settings = Some(snapshot_with_access(restricted(
        RestrictedConditions::MembershipOnly,
    )));
    state.spaces = vec![SpaceSummary {
        space_id: "!space:example.invalid".to_owned(),
        raw_name: Some("Design".to_owned()),
        display_name: "Design".to_owned(),
        avatar: None,
        join_rule: None,
        child_room_ids: Vec::new(),
        parent_side_child_room_ids: Vec::new(),
    }];
    // A history-only draft must not blank the access panel's confirmed
    // targets or its verified single-Space route.
    let mut draft = RoomAccessDraft::new(scope.clone());
    draft.set_history(Some(RoomHistoryVisibility::Invited));
    state.room_management.draft = Some(draft);

    let preview = preview_room_access_draft(&state, &scope, RoomAccessPreviewContext::Access);
    assert_eq!(
        preview.outcome.join.message_id,
        "room.accessOutcomeJoinSpaceMembers"
    );
    assert_eq!(
        preview.outcome.join.substitutions,
        vec!["Design".to_owned()]
    );
    assert!(preview.confirmed);
}

#[test]
fn create_preview_reports_the_effective_rule_and_history() {
    let scope = RoomAccessDraftScope::Create { session_id: 1 };
    let state = AppState::default();
    // A private room in a Space keeps the legacy restricted preset and its
    // `invited` history, and it pins room version V9.
    let legacy = preview_create_room_access(
        &state,
        &scope,
        CreateRoomAccessPreviewInput {
            visibility: CreateRoomVisibility::Private,
            invited_only: false,
            encrypted: true,
            parent_space_id: Some("!space:example.invalid".to_owned()),
        },
    );
    assert_eq!(legacy.effective_rule, Some(RoomJoinRule::Restricted));
    assert_eq!(legacy.effective_history, RoomHistoryVisibility::Invited);
    assert_eq!(legacy.rejection, None);
    assert!(legacy.room_version_pinned);

    // A private room at Home is invite-only with the shared-history default.
    let home = preview_create_room_access(&state, &scope, CreateRoomAccessPreviewInput::default());
    assert_eq!(home.effective_rule, Some(RoomJoinRule::Invite));
    assert_eq!(home.effective_history, RoomHistoryVisibility::Shared);
    assert!(!home.room_version_pinned);
}

#[test]
fn create_preview_reports_rejections_and_room_version() {
    let scope = RoomAccessDraftScope::Create { session_id: 1 };
    let mut state = AppState::default();
    let mut draft = RoomAccessDraft::new(scope.clone());
    draft.set_rule(Some(RoomJoinRule::Restricted));
    draft.set_allow_targets(vec!["!space:example.invalid".to_owned()]);
    state.room_management.draft = Some(draft);

    let public = preview_create_room_access(
        &state,
        &scope,
        CreateRoomAccessPreviewInput {
            visibility: CreateRoomVisibility::Public,
            invited_only: false,
            encrypted: true,
            parent_space_id: None,
        },
    );
    assert_eq!(
        public.rejection,
        Some(CreateRoomAccessRejection::PublicWithRestrictedAccess)
    );
    assert!(public.room_version_pinned);
    assert!(!public.confirmed);

    let invited = preview_create_room_access(
        &state,
        &scope,
        CreateRoomAccessPreviewInput {
            visibility: CreateRoomVisibility::Private,
            invited_only: true,
            encrypted: true,
            parent_space_id: None,
        },
    );
    assert_eq!(
        invited.rejection,
        Some(CreateRoomAccessRejection::ExplicitPolicyWithInvitedOnly)
    );
}

#[test]
fn preview_reports_confirmed_and_proposed_directory_publication() {
    let scope = RoomAccessDraftScope::Room {
        room_id: "!room:example.invalid".to_owned(),
    };
    let mut state = AppState::default();
    state.room_management.settings = Some(snapshot_with_access(restricted(
        RestrictedConditions::MembershipOnly,
    )));

    state.room_management.directory = RoomDirectoryVisibility::Public;
    assert_eq!(
        preview_room_access_draft(&state, &scope, RoomAccessPreviewContext::Access)
            .outcome
            .directory
            .message_id,
        "room.accessOutcomeDirectoryPublic"
    );
    state.room_management.directory = RoomDirectoryVisibility::Private;
    assert_eq!(
        preview_room_access_draft(&state, &scope, RoomAccessPreviewContext::Access)
            .outcome
            .directory
            .message_id,
        "room.accessOutcomeDirectoryPrivate"
    );
    for (visibility, expected) in [
        (
            RoomDirectoryVisibility::Unavailable,
            "room.accessOutcomeDirectoryUnavailable",
        ),
        (
            RoomDirectoryVisibility::Loading,
            "room.accessOutcomeDirectoryLoading",
        ),
        (
            RoomDirectoryVisibility::Failed,
            "room.accessOutcomeDirectoryFailed",
        ),
    ] {
        state.room_management.directory = visibility;
        assert_eq!(
            preview_room_access_draft(&state, &scope, RoomAccessPreviewContext::Access)
                .outcome
                .directory
                .message_id,
            expected
        );
    }

    // Creation describes the proposal, never a confirmed listing.
    let create_scope = RoomAccessDraftScope::Create { session_id: 1 };
    let public = preview_create_room_access(
        &state,
        &create_scope,
        CreateRoomAccessPreviewInput {
            visibility: CreateRoomVisibility::Public,
            ..CreateRoomAccessPreviewInput::default()
        },
    );
    assert_eq!(
        public.outcome.directory.message_id,
        "room.accessOutcomeDirectoryWillBePublic"
    );
    let private = preview_create_room_access(
        &state,
        &create_scope,
        CreateRoomAccessPreviewInput::default(),
    );
    assert_eq!(
        private.outcome.directory.message_id,
        "room.accessOutcomeDirectoryWillBePrivate"
    );
}

#[test]
fn an_untouched_confirmed_draft_resolves_from_confirmed_completeness() {
    let scope = RoomAccessDraftScope::Room {
        room_id: "!room:example.invalid".to_owned(),
    };
    let mut state = AppState::default();
    state.room_management.settings = Some(snapshot_with_access(restricted(
        RestrictedConditions::MembershipPlusUnsupported,
    )));
    let mut draft = RoomAccessDraft::new(scope.clone());
    draft.seed_room_selection(state.room_management.settings.as_ref().unwrap());
    state.room_management.draft = Some(draft);

    let preview = preview_room_access_draft(&state, &scope, RoomAccessPreviewContext::Access);
    assert!(
        preview.confirmed,
        "an untouched seeded draft is the confirmed policy"
    );
    assert_eq!(
        preview.outcome.join.message_id, "room.accessOutcomeJoinConditionsUnverifiedContent",
        "the confirmed completeness is not replaced by a membership-only guess"
    );
}

#[test]
fn a_proposed_membership_policy_does_not_erase_a_confirmed_unsupported_condition() {
    let scope = RoomAccessDraftScope::Room {
        room_id: "!room:example.invalid".to_owned(),
    };
    let mut state = AppState::default();
    state.room_management.settings = Some(snapshot_with_access(restricted(
        RestrictedConditions::MembershipPlusUnsupported,
    )));
    let mut draft = RoomAccessDraft::new(scope.clone());
    draft.seed_room_selection(state.room_management.settings.as_ref().unwrap());
    draft.toggle_allow_target("!other:example.invalid", true);
    state.room_management.draft = Some(draft);

    let preview = preview_room_access_draft(&state, &scope, RoomAccessPreviewContext::Access);
    assert!(!preview.confirmed, "the draft proposes a change");
    assert_eq!(
        preview.outcome.join.message_id, "room.accessOutcomeJoinConditionsUnverifiedContent",
        "the proposed membership policy does not erase the uneditable condition"
    );
}
