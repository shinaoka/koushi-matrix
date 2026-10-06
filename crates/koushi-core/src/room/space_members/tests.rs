use super::{
    SpaceMemberDemand, SpaceMemberRefreshFence, record_core_profile_resolution,
    record_core_space_members_load_failure, record_core_space_members_projection,
    should_clear_space_member_demand, space_member_refresh_fence_is_current,
    space_member_role_failure_from_error, space_member_role_failure_kind,
    space_members_refresh_is_current, space_members_update_affects_demand,
    state_space_members_projection, user_profiles_from_space_projection,
};

use crate::room::actor::make_request_id;

use koushi_sdk::{MatrixRoomMemberRole, MatrixSpaceMemberEntry, MatrixSpaceMembersProjection};

use koushi_state::{
    RoomMemberRole, SpaceMemberEntry, SpaceMemberMembership, SpaceMembersProjection,
};

use std::collections::BTreeSet;

#[test]
fn space_member_sync_updates_are_relevant_only_for_the_demanded_scope() {
    let child_room_ids = BTreeSet::from([
        "!child-a:example.invalid".to_owned(),
        "!child-b:example.invalid".to_owned(),
    ]);
    let space_update = BTreeSet::from(["!space:example.invalid".to_owned()]);
    let child_update = BTreeSet::from(["!child-a:example.invalid".to_owned()]);
    let unrelated_update = BTreeSet::from(["!unrelated:example.invalid".to_owned()]);

    assert!(space_members_update_affects_demand(
        "!space:example.invalid",
        &child_room_ids,
        Some(&space_update),
    ));
    assert!(space_members_update_affects_demand(
        "!space:example.invalid",
        &child_room_ids,
        Some(&child_update),
    ));
    assert!(!space_members_update_affects_demand(
        "!space:example.invalid",
        &child_room_ids,
        Some(&unrelated_update),
    ));
    assert!(space_members_update_affects_demand(
        "!space:example.invalid",
        &child_room_ids,
        None,
    ));
    assert!(!space_members_update_affects_demand(
        "!space:example.invalid",
        &child_room_ids,
        Some(&BTreeSet::new()),
    ));
}

#[test]
fn stale_space_member_refreshes_are_rejected_by_space_and_generation() {
    assert!(space_members_refresh_is_current(
        "!space:example.invalid",
        4,
        "!space:example.invalid",
        4,
    ));
    assert!(!space_members_refresh_is_current(
        "!space:example.invalid",
        3,
        "!space:example.invalid",
        4,
    ));
    assert!(!space_members_refresh_is_current(
        "!old-space:example.invalid",
        4,
        "!space:example.invalid",
        4,
    ));
}

#[test]
fn space_member_reload_clears_only_a_different_demand() {
    let demand = SpaceMemberDemand {
        space_id: "!space:example.invalid".to_owned(),
        generation: 4,
        child_room_ids: BTreeSet::new(),
        demand_generation: 1,
    };

    assert!(!should_clear_space_member_demand(
        Some(&demand),
        "!space:example.invalid",
        4,
    ));
    assert!(should_clear_space_member_demand(
        Some(&demand),
        "!other-space:example.invalid",
        4,
    ));
    assert!(should_clear_space_member_demand(
        Some(&demand),
        "!space:example.invalid",
        5,
    ));
    assert!(!should_clear_space_member_demand(
        None,
        "!space:example.invalid",
        4,
    ));
}

#[test]
fn stale_space_member_refreshes_are_rejected_by_session_demand_and_request_fences() {
    let fence = SpaceMemberRefreshFence {
        request_id: make_request_id(1),
        session_generation: 2,
        demand_generation: 3,
        refresh_generation: 4,
    };
    let current = |active_fence, session_generation, demand_generation, request_id| {
        space_member_refresh_fence_is_current(
            active_fence,
            SpaceMemberRefreshFence {
                request_id,
                ..fence
            },
            session_generation,
            demand_generation,
            "!space:example.invalid",
            4,
            "!space:example.invalid",
            4,
        )
    };

    assert!(current(Some(fence), 2, 3, make_request_id(1)));
    assert!(!current(Some(fence), 1, 3, make_request_id(1)));
    assert!(!current(Some(fence), 2, 9, make_request_id(1)));
    assert!(!current(Some(fence), 2, 3, make_request_id(2)));
}

#[test]
fn space_members_projection_load_path_emits_non_empty_child_profile_observations() {
    let raw = MatrixSpaceMembersProjection {
        space_id: "!space:example.invalid".to_owned(),
        child_room_ids: vec!["!child:example.invalid".to_owned()],
        space_joined: Vec::new(),
        space_invited: Vec::new(),
        child_room_only: vec![MatrixSpaceMemberEntry {
            user_id: "@child:example.invalid".to_owned(),
            display_name: Some("Child room profile".to_owned()),
            avatar_url: None,
            power_level: Some(0),
            role: MatrixRoomMemberRole::User,
            child_room_ids: vec!["!child:example.invalid".to_owned()],
            role_options: Vec::new(),
        }],
        child_room_profiles: Vec::new(),
        space_joined_input_count: 0,
        space_invited_input_count: 0,
        child_join_input_count: 1,
        child_join_union_count: 1,
        duplicate_child_membership_count: 0,
        child_room_count: 1,
        complete_child_room_count: 1,
        incomplete_child_room_count: 0,
        power_levels_revision: None,
        can_edit_roles: false,
    };

    let profiles = user_profiles_from_space_projection(&raw);
    assert_eq!(profiles.len(), 1);
    assert_eq!(profiles[0].user_id, "@child:example.invalid");
    assert_eq!(
        profiles[0].display_name.as_deref(),
        Some("Child room profile")
    );

    let projection = state_space_members_projection(raw, 4);
    assert_eq!(
        projection.child_room_only[0].display_name.as_deref(),
        Some("Child room profile")
    );
}

#[test]
fn failed_space_member_diagnostics_do_not_fabricate_member_counts() {
    let _diagnostic_lock = koushi_diagnostics::test_support::lock();
    let before = koushi_diagnostics::test_support::detail_cursor();
    record_core_space_members_load_failure("sync_refresh", 7);
    let record = koushi_diagnostics::test_support::detail_records_since(before)
        .into_iter()
        .find(|record| {
            record.event.source == "core.space_members_projection"
                && record.event.fields.iter().any(|field| {
                    field.key == "outcome"
                        && field.value
                            == koushi_diagnostics::DiagnosticValue::Token("lookup_failed")
                })
        })
        .expect("Space load failure diagnostic");

    assert!(record.event.fields.iter().any(|field| {
        field.key == "outcome"
            && field.value == koushi_diagnostics::DiagnosticValue::Token("lookup_failed")
    }));
    for field in &record.event.fields {
        if matches!(
            field.key,
            "space_joined_count"
                | "space_invited_count"
                | "child_room_count"
                | "child_room_only_count"
                | "input_count"
                | "output_count"
        ) {
            assert_ne!(
                field.value,
                koushi_diagnostics::DiagnosticValue::Count(0),
                "failed Space diagnostics must not report member counts as zero"
            );
        }
    }
}

#[test]
fn core_space_members_diagnostics_are_private_data_free() {
    let _diagnostic_lock = koushi_diagnostics::test_support::lock();
    let projection = SpaceMembersProjection {
        space_id: "!private:example.invalid".to_owned(),
        generation: 4,
        space_joined: vec![SpaceMemberEntry {
            user_id: "@alice:example.invalid".to_owned(),
            display_name: Some("Alice private".to_owned()),
            display_label: "Alice private".to_owned(),
            original_display_label: "Alice private".to_owned(),
            avatar_url: Some("mxc://example.invalid/avatar".to_owned()),
            power_level: Some(100),
            role: RoomMemberRole::Administrator,
            membership: SpaceMemberMembership::SpaceJoined,
            child_room_ids: Vec::new(),
            invite_pending: false,
            role_options: Vec::new(),
        }],
        space_invited: Vec::new(),
        child_room_only: Vec::new(),
        child_room_count: 0,
        complete_child_room_count: 0,
        incomplete_child_room_count: 0,
        power_levels_revision: None,
        can_edit_roles: false,
    };
    record_core_space_members_projection("load", 4, &projection, "success");
    record_core_profile_resolution(&projection);

    let snapshot = koushi_diagnostics::test_support::detail_snapshot();
    let encoded = serde_json::to_string(&snapshot).expect("diagnostics serialize");
    assert!(!encoded.contains("@alice:example.invalid"));
    assert!(!encoded.contains("Alice private"));
    assert!(!encoded.contains("mxc://example.invalid/avatar"));
    assert!(
        snapshot
            .records
            .iter()
            .any(|record| record.event.source == "core.space_members_projection")
    );
    assert!(
        snapshot
            .records
            .iter()
            .any(|record| record.event.source == "core.profile_resolution")
    );
}

#[test]
fn role_failure_mapping_is_closed_and_raw_sdk_values_do_not_escape() {
    use koushi_sdk::{
        MatrixRoomOperationError, MatrixRoomOperationFailureKind, MatrixSpaceMemberRoleFailureKind,
    };

    for (kind, expected) in [
        (
            MatrixSpaceMemberRoleFailureKind::Forbidden,
            koushi_state::SpaceMemberRoleFailureKind::Forbidden,
        ),
        (
            MatrixSpaceMemberRoleFailureKind::Stale,
            koushi_state::SpaceMemberRoleFailureKind::Stale,
        ),
        (
            MatrixSpaceMemberRoleFailureKind::NotFound,
            koushi_state::SpaceMemberRoleFailureKind::NotFound,
        ),
        (
            MatrixSpaceMemberRoleFailureKind::Network,
            koushi_state::SpaceMemberRoleFailureKind::Network,
        ),
        (
            MatrixSpaceMemberRoleFailureKind::Timeout,
            koushi_state::SpaceMemberRoleFailureKind::Timeout,
        ),
        (
            MatrixSpaceMemberRoleFailureKind::Invalid,
            koushi_state::SpaceMemberRoleFailureKind::Invalid,
        ),
        (
            MatrixSpaceMemberRoleFailureKind::Sdk,
            koushi_state::SpaceMemberRoleFailureKind::Sdk,
        ),
    ] {
        assert_eq!(space_member_role_failure_kind(kind), expected);
    }
    assert_eq!(
        space_member_role_failure_from_error(&MatrixRoomOperationError::RoomUnavailable),
        koushi_state::SpaceMemberRoleFailureKind::NotFound
    );
    assert_eq!(
        space_member_role_failure_from_error(&MatrixRoomOperationError::InvalidUserId),
        koushi_state::SpaceMemberRoleFailureKind::Invalid
    );
    assert_eq!(
        space_member_role_failure_from_error(&MatrixRoomOperationError::Sdk(
            MatrixRoomOperationFailureKind::Forbidden,
        )),
        koushi_state::SpaceMemberRoleFailureKind::Forbidden
    );
    assert_eq!(
        space_member_role_failure_from_error(&MatrixRoomOperationError::Sdk(
            MatrixRoomOperationFailureKind::Http,
        )),
        koushi_state::SpaceMemberRoleFailureKind::Network
    );
}

#[test]
fn role_settlement_debug_is_identifier_free() {
    let event = koushi_protocol::event::RoomEvent::SpaceMemberRoleUpdateSettled {
        request_id: make_request_id(42),
        space_id: "!private-space:example.invalid".to_owned(),
        user_id: "@private-target:example.invalid".to_owned(),
        generation: 7,
        outcome: koushi_state::SpaceMemberRoleUpdateOutcome::Failed(
            koushi_state::SpaceMemberRoleFailureKind::Stale,
        ),
    };
    let debug = format!("{event:?}");
    assert!(debug.contains("SpaceMemberRoleUpdateSettled"));
    assert!(debug.contains("generation"));
    assert!(!debug.contains("@"));
    assert!(!debug.contains("!"));
    assert!(!debug.contains("EventId"));
}
