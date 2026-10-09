//! #1177: live coherence of the shared access projection — the access
//! observation reconciles ordinary rooms too, compares the full canonical
//! policy, and an accepted local access change reaches the projection without
//! waiting for the SDK echo while the other property keeps its local value.

use std::collections::BTreeMap;

use koushi_state::{
    AppAction, AppEffect, AppState, RestrictedConditions, RoomAccessCondition, RoomAllowTarget,
    RoomAllowTargetKind, RoomHistoryVisibility, RoomJoinRule, RoomListReadiness, RoomListSource,
    RoomPermissionFacts, RoomSettingChange, RoomSettingsSnapshot, RoomSummary, RoomTags,
    SessionInfo, SessionState, UiEvent, reduce,
};

const ROOM: &str = "!room:example.invalid";

fn ready_state() -> AppState {
    AppState {
        session: SessionState::Ready(SessionInfo {
            homeserver: "https://matrix.example.org".to_owned(),
            user_id: "@user-a:example.invalid".to_owned(),
            device_id: "DEVICE".to_owned(),
            authentication_method: koushi_state::SessionAuthenticationMethod::Unknown,
        }),
        ..AppState::default()
    }
}

fn room_summary(room_id: &str) -> RoomSummary {
    RoomSummary {
        display_name_placeholder: None,
        display_label_placeholder: None,
        room_id: room_id.to_owned(),
        display_name: "Room".to_owned(),
        display_label: "Room".to_owned(),
        original_display_label: "Room".to_owned(),
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
        parent_space_ids: Vec::new(),
        dm_space_ids: Vec::new(),
        is_encrypted: false,
        joined_members: 1,
    }
}

fn ready_room_list(state: &mut AppState) {
    state.rooms = vec![room_summary(ROOM)];
    state.room_list.readiness = RoomListReadiness::Ready {
        source: RoomListSource::Live,
        generation: 1,
    };
}

fn editable_settings(room_id: &str) -> RoomSettingsSnapshot {
    RoomSettingsSnapshot {
        room_id: room_id.to_owned(),
        name: Some("Synthetic Room".to_owned()),
        topic: None,
        avatar_url: None,
        canonical_alias: None,
        alternate_aliases: Vec::new(),
        share_link: None,
        join_rule: RoomJoinRule::Invite,
        history_visibility: RoomHistoryVisibility::Shared,
        access: RoomAccessCondition {
            join_rule: Some(RoomJoinRule::Invite),
            restricted: None,
            allow_targets: Vec::new(),
        },
        permissions: RoomPermissionFacts {
            can_edit_settings: true,
            can_change_join_rule: true,
            can_edit_roles: true,
            can_invite: true,
            can_kick: true,
            can_ban: true,
            can_unban: false,
        },
        members: Vec::new(),
    }
}

fn observation(
    rule: RoomJoinRule,
    restricted: Option<RestrictedConditions>,
    targets: &[&str],
) -> RoomAccessCondition {
    RoomAccessCondition {
        join_rule: Some(rule),
        restricted,
        allow_targets: targets
            .iter()
            .map(|room_id| RoomAllowTarget {
                kind: RoomAllowTargetKind::Space,
                room_id: (*room_id).to_owned(),
            })
            .collect(),
    }
}

fn observe(
    state: &mut AppState,
    authoritative: bool,
    access: BTreeMap<String, RoomAccessCondition>,
) -> Vec<AppEffect> {
    reduce(
        state,
        AppAction::RoomAccessUpdated {
            generation: 1,
            source: RoomListSource::Live,
            authoritative,
            access,
        },
    )
}

fn open_settings(state: &mut AppState) {
    reduce(
        state,
        AppAction::RoomSettingsSnapshotLoaded {
            room_id: ROOM.to_owned(),
            settings: editable_settings(ROOM),
        },
    );
}

#[test]
fn an_ordinary_room_access_observation_reconciles_the_open_settings() {
    let mut state = ready_state();
    ready_room_list(&mut state);
    open_settings(&mut state);

    let effects = observe(
        &mut state,
        true,
        BTreeMap::from([(
            ROOM.to_owned(),
            observation(
                RoomJoinRule::Restricted,
                Some(RestrictedConditions::MembershipOnly),
                &["!space:example.invalid"],
            ),
        )]),
    );

    let settings = state.room_management.settings.as_ref().expect("settings");
    assert_eq!(settings.join_rule, RoomJoinRule::Restricted);
    assert_eq!(settings.access.join_rule, Some(RoomJoinRule::Restricted));
    assert_eq!(settings.access.allow_targets.len(), 1);
    assert_eq!(
        settings.access.allow_targets[0].room_id,
        "!space:example.invalid"
    );
    assert!(effects.contains(&AppEffect::EmitUiEvent(UiEvent::RoomListChanged)));
    assert!(effects.contains(&AppEffect::EmitUiEvent(UiEvent::RoomManagementChanged)));
}

#[test]
fn a_reordered_or_duplicated_allow_list_is_not_a_change() {
    let mut state = ready_state();
    ready_room_list(&mut state);
    open_settings(&mut state);

    observe(
        &mut state,
        true,
        BTreeMap::from([(
            ROOM.to_owned(),
            observation(
                RoomJoinRule::Restricted,
                Some(RestrictedConditions::MembershipOnly),
                &[
                    "!b:example.invalid",
                    "!a:example.invalid",
                    "!a:example.invalid",
                ],
            ),
        )]),
    );
    let accepted = state
        .room_management
        .settings
        .as_ref()
        .expect("settings")
        .access
        .clone();

    let effects = observe(
        &mut state,
        true,
        BTreeMap::from([(
            ROOM.to_owned(),
            observation(
                RoomJoinRule::Restricted,
                Some(RestrictedConditions::MembershipOnly),
                &["!a:example.invalid", "!b:example.invalid"],
            ),
        )]),
    );

    assert_eq!(
        state
            .room_management
            .settings
            .as_ref()
            .expect("settings")
            .access,
        accepted,
        "the locally held condition is not replaced by a reordered server list"
    );
    assert!(effects.contains(&AppEffect::EmitUiEvent(UiEvent::RoomListChanged)));
    assert!(
        !effects.contains(&AppEffect::EmitUiEvent(UiEvent::RoomManagementChanged)),
        "a reordered allow list is not a settings change"
    );
}

#[test]
fn an_access_save_then_a_history_save_never_carries_the_stale_access_read() {
    let mut state = ready_state();
    ready_room_list(&mut state);
    open_settings(&mut state);

    let accepted = observation(
        RoomJoinRule::Restricted,
        Some(RestrictedConditions::MembershipOnly),
        &["!space:example.invalid"],
    );
    reduce(
        &mut state,
        AppAction::RoomSettingUpdateRequested {
            request_id: 1,
            room_id: ROOM.to_owned(),
            change: RoomSettingChange::AccessPolicy(koushi_state::RoomAccessPolicy::new(
                RoomJoinRule::Restricted,
                vec!["!space:example.invalid".to_owned()],
            )),
        },
    );
    reduce(
        &mut state,
        AppAction::RoomSettingUpdateSucceeded {
            request_id: 1,
            room_id: ROOM.to_owned(),
            change: RoomSettingChange::AccessPolicy(koushi_state::RoomAccessPolicy::new(
                RoomJoinRule::Restricted,
                vec!["!space:example.invalid".to_owned()],
            )),
            settings: RoomSettingsSnapshot {
                join_rule: RoomJoinRule::Restricted,
                access: accepted.clone(),
                ..editable_settings(ROOM)
            },
        },
    );

    assert_eq!(
        state.room_access.get(ROOM),
        Some(&accepted),
        "the accepted access change reaches the shared projection immediately"
    );

    // The history save's pre-send read still carries the old access content
    // because the join-rules echo has not round-tripped.
    reduce(
        &mut state,
        AppAction::RoomSettingUpdateRequested {
            request_id: 2,
            room_id: ROOM.to_owned(),
            change: RoomSettingChange::HistoryVisibility(RoomHistoryVisibility::Joined),
        },
    );
    reduce(
        &mut state,
        AppAction::RoomSettingUpdateSucceeded {
            request_id: 2,
            room_id: ROOM.to_owned(),
            change: RoomSettingChange::HistoryVisibility(RoomHistoryVisibility::Joined),
            settings: RoomSettingsSnapshot {
                history_visibility: RoomHistoryVisibility::Joined,
                ..editable_settings(ROOM)
            },
        },
    );

    let settings = state.room_management.settings.as_ref().expect("settings");
    assert_eq!(
        settings.access, accepted,
        "the accepted local access value survives the lagging history read"
    );
    assert_eq!(settings.history_visibility, RoomHistoryVisibility::Joined);
}

#[test]
fn a_history_save_then_an_access_save_never_carries_the_stale_history_read() {
    let mut state = ready_state();
    ready_room_list(&mut state);
    open_settings(&mut state);

    reduce(
        &mut state,
        AppAction::RoomSettingUpdateRequested {
            request_id: 1,
            room_id: ROOM.to_owned(),
            change: RoomSettingChange::HistoryVisibility(RoomHistoryVisibility::Joined),
        },
    );
    reduce(
        &mut state,
        AppAction::RoomSettingUpdateSucceeded {
            request_id: 1,
            room_id: ROOM.to_owned(),
            change: RoomSettingChange::HistoryVisibility(RoomHistoryVisibility::Joined),
            settings: RoomSettingsSnapshot {
                history_visibility: RoomHistoryVisibility::Joined,
                ..editable_settings(ROOM)
            },
        },
    );

    let accepted = observation(
        RoomJoinRule::Restricted,
        Some(RestrictedConditions::MembershipOnly),
        &["!space:example.invalid"],
    );
    // The access save's pre-send read still carries `shared` history.
    reduce(
        &mut state,
        AppAction::RoomSettingUpdateRequested {
            request_id: 2,
            room_id: ROOM.to_owned(),
            change: RoomSettingChange::AccessPolicy(koushi_state::RoomAccessPolicy::new(
                RoomJoinRule::Restricted,
                vec!["!space:example.invalid".to_owned()],
            )),
        },
    );
    reduce(
        &mut state,
        AppAction::RoomSettingUpdateSucceeded {
            request_id: 2,
            room_id: ROOM.to_owned(),
            change: RoomSettingChange::AccessPolicy(koushi_state::RoomAccessPolicy::new(
                RoomJoinRule::Restricted,
                vec!["!space:example.invalid".to_owned()],
            )),
            settings: RoomSettingsSnapshot {
                join_rule: RoomJoinRule::Restricted,
                access: accepted.clone(),
                ..editable_settings(ROOM)
            },
        },
    );

    let settings = state.room_management.settings.as_ref().expect("settings");
    assert_eq!(
        settings.history_visibility,
        RoomHistoryVisibility::Joined,
        "the accepted local history value survives the lagging access read"
    );
    assert_eq!(settings.access, accepted);
    assert_eq!(state.room_access.get(ROOM), Some(&accepted));
}
