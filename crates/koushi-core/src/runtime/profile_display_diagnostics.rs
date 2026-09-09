use koushi_diagnostics::{DiagnosticEvent, DiagnosticField, DiagnosticLevel, record};
use koushi_state::{
    AppAction, AppEffect, AppState, ProfileResolutionInput, ProfileResolutionSource, SessionState,
    SpaceMemberEntry, SpaceMemberMembership, UserProfile, resolve_people_label,
};

#[derive(Default)]
struct ProfileResolutionDiagnosticCounts {
    input_count: u64,
    output_count: u64,
    local_alias_count: u64,
    relevant_room_count: u64,
    space_room_count: u64,
    payload_count: u64,
    global_cache_count: u64,
    local_homeserver_count: u64,
    unresolved_count: u64,
    cache_hit_count: u64,
    cache_miss_count: u64,
}

impl ProfileResolutionDiagnosticCounts {
    fn observe(&mut self, input: ProfileResolutionInput<'_>) {
        self.input_count += 1;
        let cache_available = input.cached_label.is_some_and(has_profile_label);
        if cache_available {
            self.cache_hit_count += 1;
        } else {
            self.cache_miss_count += 1;
        }

        let resolution = resolve_people_label(input);
        self.output_count += 1;
        match resolution.source {
            ProfileResolutionSource::LocalAlias => self.local_alias_count += 1,
            ProfileResolutionSource::RelevantRoom => self.relevant_room_count += 1,
            ProfileResolutionSource::SpaceRoom => self.space_room_count += 1,
            ProfileResolutionSource::Payload => self.payload_count += 1,
            ProfileResolutionSource::GlobalCache => self.global_cache_count += 1,
            ProfileResolutionSource::LocalHomeserver => self.local_homeserver_count += 1,
            ProfileResolutionSource::Unresolved => self.unresolved_count += 1,
        }
    }

    fn event(self, trigger: &'static str) -> DiagnosticEvent {
        DiagnosticEvent::new(
            DiagnosticLevel::Debug,
            "core.profile_resolution",
            "resolution",
        )
        .field(DiagnosticField::token("trigger", trigger))
        .field(DiagnosticField::count("input_count", self.input_count))
        .field(DiagnosticField::count("output_count", self.output_count))
        .field(DiagnosticField::count(
            "local_alias_count",
            self.local_alias_count,
        ))
        .field(DiagnosticField::count(
            "relevant_room_count",
            self.relevant_room_count,
        ))
        .field(DiagnosticField::count(
            "space_room_count",
            self.space_room_count,
        ))
        .field(DiagnosticField::count("payload_count", self.payload_count))
        .field(DiagnosticField::count(
            "global_cache_count",
            self.global_cache_count,
        ))
        .field(DiagnosticField::count(
            "local_homeserver_count",
            self.local_homeserver_count,
        ))
        .field(DiagnosticField::count(
            "unresolved_count",
            self.unresolved_count,
        ))
        .field(DiagnosticField::count(
            "cache_hit_count",
            self.cache_hit_count,
        ))
        .field(DiagnosticField::count(
            "cache_miss_count",
            self.cache_miss_count,
        ))
        .field(DiagnosticField::token(
            "cache_stale_hit_status",
            "not_tracked",
        ))
        .field(DiagnosticField::token(
            "cache_freshness_status",
            "not_tracked",
        ))
    }
}

fn has_profile_label(label: &str) -> bool {
    let label = label.trim();
    !label.is_empty() && label != "Unknown user"
}

fn profile_display_label(profile: &UserProfile) -> Option<&str> {
    profile
        .display_name
        .as_deref()
        .filter(|label| has_profile_label(label))
}

fn session_user_id(state: &AppState) -> Option<&str> {
    match &state.session {
        SessionState::Provisional { info, .. }
        | SessionState::AwaitingVerification { info, .. }
        | SessionState::Verifying { info, .. }
        | SessionState::AwaitingBootstrapConfirmation { info, .. }
        | SessionState::Rejecting { info, .. }
        | SessionState::Ready(info)
        | SessionState::Locked(info)
        | SessionState::CapabilityBlocked { info, .. }
        | SessionState::SwitchingAccount { info } => Some(info.user_id.as_str()),
        SessionState::SignedOut
        | SessionState::Restoring
        | SessionState::Authenticating { .. }
        | SessionState::LoggingOut => None,
    }
}

fn relevant_room_profile_label<'a>(
    state: &'a AppState,
    room_id: &str,
    user_id: &str,
) -> Option<&'a str> {
    state
        .profile
        .room_users
        .get(room_id)
        .and_then(|profiles| profiles.get(user_id))
        .and_then(profile_display_label)
}

fn space_room_profile_label<'a>(
    state: &'a AppState,
    room_id: &str,
    user_id: &str,
) -> Option<&'a str> {
    let room = state.rooms.iter().find(|room| room.room_id == room_id)?;
    room.parent_space_ids.iter().find_map(|space_id| {
        state
            .profile
            .room_users
            .get(space_id)
            .and_then(|profiles| profiles.get(user_id))
            .and_then(profile_display_label)
    })
}

fn local_homeserver_profile_label<'a>(state: &'a AppState, user_id: &str) -> Option<&'a str> {
    (session_user_id(state) == Some(user_id))
        .then(|| state.profile.own.display_name.as_deref())
        .flatten()
        .filter(|label| has_profile_label(label))
}

fn observe_receipt_profile_resolution(
    state: &AppState,
    room_id: &str,
    receipt: &koushi_state::LiveReadReceipt,
    counts: &mut ProfileResolutionDiagnosticCounts,
) {
    let cached_label = state
        .profile
        .users
        .get(&receipt.user_id)
        .and_then(profile_display_label);
    let payload_label = receipt
        .display_name
        .as_deref()
        .filter(|label| has_profile_label(label))
        .or_else(|| {
            has_profile_label(&receipt.original_display_label)
                .then_some(receipt.original_display_label.as_str())
        });
    counts.observe(ProfileResolutionInput {
        local_alias: state
            .profile
            .local_aliases
            .get(&receipt.user_id)
            .map(String::as_str)
            .filter(|label| has_profile_label(label)),
        relevant_room_label: relevant_room_profile_label(state, room_id, &receipt.user_id),
        space_room_label: space_room_profile_label(state, room_id, &receipt.user_id),
        payload_label,
        cached_label,
        local_homeserver_label: local_homeserver_profile_label(state, &receipt.user_id),
    });
}

fn observe_space_member_profile_resolution(
    state: &AppState,
    entry: &SpaceMemberEntry,
    observed_profiles: &[UserProfile],
    counts: &mut ProfileResolutionDiagnosticCounts,
) {
    let cached_label = observed_profiles
        .iter()
        .find(|profile| profile.user_id == entry.user_id)
        .and_then(profile_display_label)
        .or_else(|| {
            state
                .profile
                .users
                .get(&entry.user_id)
                .and_then(profile_display_label)
        });
    let (relevant_room_label, space_room_label) = match entry.membership {
        SpaceMemberMembership::ChildRoomOnly => (
            entry
                .display_name
                .as_deref()
                .filter(|label| has_profile_label(label)),
            None,
        ),
        SpaceMemberMembership::SpaceJoined | SpaceMemberMembership::SpaceInvited => (
            None,
            entry
                .display_name
                .as_deref()
                .filter(|label| has_profile_label(label)),
        ),
    };
    counts.observe(ProfileResolutionInput {
        local_alias: state
            .profile
            .local_aliases
            .get(&entry.user_id)
            .map(String::as_str)
            .filter(|label| has_profile_label(label)),
        relevant_room_label,
        space_room_label,
        payload_label: None,
        cached_label,
        local_homeserver_label: local_homeserver_profile_label(state, &entry.user_id),
    });
}

pub(super) fn profile_resolution_diagnostic_event(
    state: &AppState,
    action: &AppAction,
) -> Option<DiagnosticEvent> {
    let mut counts = ProfileResolutionDiagnosticCounts::default();
    let trigger = match action {
        AppAction::LiveRoomReceiptsWindowReconciled {
            room_id,
            receipts_by_event,
            ..
        } => {
            for receipt in receipts_by_event
                .iter()
                .flat_map(|entry| entry.receipts.iter())
            {
                observe_receipt_profile_resolution(state, room_id, receipt, &mut counts);
            }
            "live_receipt"
        }
        AppAction::SpaceMembersLoaded { projection, .. } => {
            for entry in projection
                .space_joined
                .iter()
                .chain(projection.space_invited.iter())
                .chain(projection.child_room_only.iter())
            {
                observe_space_member_profile_resolution(state, entry, &[], &mut counts);
            }
            "space_member_projection"
        }
        AppAction::SpaceMembersProjectionReconciled {
            projection,
            profiles,
            ..
        }
        | AppAction::SpaceMembersBackgroundProjectionReconciled {
            projection,
            profiles,
            ..
        } => {
            for entry in projection
                .space_joined
                .iter()
                .chain(projection.space_invited.iter())
                .chain(projection.child_room_only.iter())
            {
                observe_space_member_profile_resolution(state, entry, profiles, &mut counts);
            }
            "space_member_projection"
        }
        _ => return None,
    };

    (counts.input_count > 0).then(|| counts.event(trigger))
}

pub(super) fn record_native_attention_recomputed(effect: &AppEffect) {
    let AppEffect::RecordNativeAttentionRecomputed {
        observation,
        unread_count,
        notification_count,
        badge_count,
        badge_room_count,
        badge_excluded_room_count,
        candidate,
        suppression,
        window_focused,
        active_room_match,
    } = effect
    else {
        return;
    };

    let observation = match observation {
        koushi_state::NativeAttentionObservationKind::Live => "live",
        koushi_state::NativeAttentionObservationKind::InitialSync => "initial_sync",
        koushi_state::NativeAttentionObservationKind::Backfill => "backfill",
        koushi_state::NativeAttentionObservationKind::SelfEvent => "self_event",
    };
    let candidate = match candidate {
        Some(koushi_state::RoomAttentionKind::Message) => "message",
        Some(koushi_state::RoomAttentionKind::Dm) => "dm",
        Some(koushi_state::RoomAttentionKind::Mention) => "mention",
        None => "none",
    };
    let suppression = match suppression {
        Some(koushi_state::NativeAttentionSuppressionReason::InitialSync) => "initial_sync",
        Some(koushi_state::NativeAttentionSuppressionReason::Backfill) => "backfill",
        Some(koushi_state::NativeAttentionSuppressionReason::SelfMessage) => "self_message",
        Some(koushi_state::NativeAttentionSuppressionReason::WindowFocused) => "window_focused",
        Some(koushi_state::NativeAttentionSuppressionReason::RoomMuted) => "room_muted",
        Some(koushi_state::NativeAttentionSuppressionReason::LowPriority) => "low_priority",
        Some(koushi_state::NativeAttentionSuppressionReason::Duplicate) => "duplicate",
        Some(koushi_state::NativeAttentionSuppressionReason::CapabilityUnavailable) => {
            "capability_unavailable"
        }
        None => "none",
    };

    record(
        DiagnosticEvent::new(DiagnosticLevel::Debug, "native.attention", "recomputed")
            .field(DiagnosticField::token("observation", observation))
            .field(DiagnosticField::count("unread_count", *unread_count))
            .field(DiagnosticField::count(
                "notification_count",
                *notification_count,
            ))
            .field(DiagnosticField::count("badge_count", *badge_count))
            .field(DiagnosticField::token(
                "badge_source",
                "raw_unread_messages",
            ))
            .field(DiagnosticField::count(
                "badge_room_count",
                *badge_room_count,
            ))
            .field(DiagnosticField::count(
                "badge_excluded_room_count",
                *badge_excluded_room_count,
            ))
            .field(DiagnosticField::token("candidate", candidate))
            .field(DiagnosticField::token("suppression", suppression))
            .field(DiagnosticField::boolean("window_focused", *window_focused))
            .field(DiagnosticField::boolean(
                "active_room_match",
                *active_room_match,
            )),
    );
}

pub(super) fn live_receipt_profile_diagnostic_event(
    state: &AppState,
    action: &AppAction,
) -> Option<DiagnosticEvent> {
    let (room_id, receipts_by_event, update_kind) = match action {
        AppAction::LiveRoomReceiptsWindowReconciled {
            room_id,
            receipts_by_event,
            ..
        } => (room_id, receipts_by_event, "window_reconciled"),
        _ => return None,
    };

    let receipt_count = receipts_by_event
        .iter()
        .map(|entry| entry.receipts.len() as u64)
        .sum::<u64>();
    if receipt_count == 0 {
        return None;
    }

    let own_user_id = match &state.session {
        SessionState::Provisional { info, .. }
        | SessionState::AwaitingVerification { info, .. }
        | SessionState::Verifying { info, .. }
        | SessionState::AwaitingBootstrapConfirmation { info, .. }
        | SessionState::Rejecting { info, .. }
        | SessionState::Ready(info)
        | SessionState::Locked(info)
        | SessionState::CapabilityBlocked { info, .. }
        | SessionState::SwitchingAccount { info } => Some(info.user_id.as_str()),
        SessionState::SignedOut
        | SessionState::Restoring
        | SessionState::Authenticating { .. }
        | SessionState::LoggingOut => None,
    };
    let room = state.rooms.iter().find(|room| room.room_id == *room_id);
    let parent_space_count = room.map_or(0, |room| room.parent_space_ids.len() as u64);
    let mut own_receipt_count = 0_u64;
    let mut payload_label_count = 0_u64;
    let mut profile_cache_hit_count = 0_u64;
    let mut profile_cache_miss_count = 0_u64;
    let mut profile_display_name_missing_count = 0_u64;
    let mut friendly_name_unresolved_count = 0_u64;

    for receipt in receipts_by_event
        .iter()
        .flat_map(|entry| entry.receipts.iter())
    {
        if own_user_id.is_some_and(|own_user_id| own_user_id == receipt.user_id) {
            own_receipt_count += 1;
            continue;
        }

        let has_payload_label = receipt
            .display_name
            .as_deref()
            .is_some_and(|name| !name.trim().is_empty())
            || !receipt.original_display_label.trim().is_empty();
        payload_label_count += u64::from(has_payload_label);

        let local_alias_resolves = state
            .profile
            .local_aliases
            .get(&receipt.user_id)
            .is_some_and(|alias| !alias.trim().is_empty());
        let profile = state.profile.users.get(&receipt.user_id);
        let profile_resolves = profile
            .and_then(|profile| profile.display_name.as_deref())
            .is_some_and(|name| !name.trim().is_empty());
        if let Some(profile) = profile {
            profile_cache_hit_count += 1;
            if !local_alias_resolves
                && profile
                    .display_name
                    .as_deref()
                    .is_none_or(|name| name.trim().is_empty())
            {
                profile_display_name_missing_count += 1;
            }
        } else {
            profile_cache_miss_count += 1;
        }
        if !has_payload_label && !local_alias_resolves && !profile_resolves {
            friendly_name_unresolved_count += 1;
        }
    }

    let unresolved_reason = if friendly_name_unresolved_count == 0 {
        "none"
    } else if profile_cache_miss_count > 0 {
        "profile_cache_miss"
    } else if profile_display_name_missing_count > 0 {
        "profile_display_name_missing"
    } else {
        "receipt_label_missing"
    };

    Some(
        DiagnosticEvent::new(
            DiagnosticLevel::Debug,
            "core.read_receipt_profile",
            "resolution",
        )
        .field(DiagnosticField::token("update_kind", update_kind))
        .field(DiagnosticField::count("receipt_count", receipt_count))
        .field(DiagnosticField::count(
            "own_receipt_count",
            own_receipt_count,
        ))
        .field(DiagnosticField::count(
            "payload_label_count",
            payload_label_count,
        ))
        .field(DiagnosticField::count(
            "profile_cache_hit_count",
            profile_cache_hit_count,
        ))
        .field(DiagnosticField::count(
            "profile_cache_miss_count",
            profile_cache_miss_count,
        ))
        .field(DiagnosticField::count(
            "profile_display_name_missing_count",
            profile_display_name_missing_count,
        ))
        .field(DiagnosticField::count(
            "friendly_name_unresolved_count",
            friendly_name_unresolved_count,
        ))
        .field(DiagnosticField::boolean(
            "room_in_space",
            parent_space_count > 0,
        ))
        .field(DiagnosticField::count(
            "parent_space_count",
            parent_space_count,
        ))
        .field(DiagnosticField::token(
            "lookup_scope",
            "global_profile_cache",
        ))
        .field(DiagnosticField::boolean(
            "room_member_lookup_attempted",
            false,
        ))
        .field(DiagnosticField::boolean(
            "space_member_lookup_attempted",
            false,
        ))
        .field(DiagnosticField::token(
            "unresolved_reason",
            unresolved_reason,
        )),
    )
}

#[cfg(test)]
mod tests {
    use super::super::tests::unread_diagnostic_room;
    use super::*;
    use koushi_state::{LiveEventReceipts, LiveReadReceipt, SessionInfo};

    #[test]
    fn read_receipt_profile_diagnostic_reports_child_room_profile_cache_miss() {
        let _diagnostic_lock = koushi_diagnostics::test_support::lock();
        let room_id = "!child:example.invalid";
        let mut state = AppState {
            session: SessionState::Ready(SessionInfo {
                homeserver: "https://example.invalid".to_owned(),
                user_id: "@own:example.invalid".to_owned(),
                device_id: "OWN".to_owned(),
                authentication_method: koushi_state::SessionAuthenticationMethod::Unknown,
            }),
            ..AppState::default()
        };
        let mut room = unread_diagnostic_room(room_id);
        room.parent_space_ids = vec!["!space:example.invalid".to_owned()];
        state.rooms.push(room);

        let action = AppAction::LiveRoomReceiptsWindowReconciled {
            room_id: room_id.to_owned(),
            scoped_event_ids: Vec::new(),
            receipts_by_event: vec![LiveEventReceipts {
                event_id: "$event".to_owned(),
                receipts: vec![LiveReadReceipt {
                    user_id: "@child-only:example.invalid".to_owned(),
                    display_name: None,
                    original_display_label: String::new(),
                    avatar: None,
                    timestamp_ms: Some(42),
                }],
            }],
        };

        let event = live_receipt_profile_diagnostic_event(&state, &action)
            .expect("receipt diagnostics should be emitted");
        assert_eq!(event.source, "core.read_receipt_profile");
        assert_eq!(event.stage, "resolution");
        let field = |key| {
            event
                .fields
                .iter()
                .find(|field| field.key == key)
                .map(|field| &field.value)
        };
        assert_eq!(
            field("profile_cache_miss_count"),
            Some(&koushi_diagnostics::DiagnosticValue::Count(1))
        );
        assert_eq!(
            field("room_in_space"),
            Some(&koushi_diagnostics::DiagnosticValue::Boolean(true))
        );
        assert_eq!(
            field("lookup_scope"),
            Some(&koushi_diagnostics::DiagnosticValue::Token(
                "global_profile_cache"
            ))
        );
        assert_eq!(
            field("unresolved_reason"),
            Some(&koushi_diagnostics::DiagnosticValue::Token(
                "profile_cache_miss"
            ))
        );
    }

    #[test]
    fn profile_resolution_diagnostic_counts_actual_resolution_sources() {
        let _diagnostic_lock = koushi_diagnostics::test_support::lock();
        let room_id = "!resolution-room:example.invalid";
        let mut state = AppState {
            session: SessionState::Ready(SessionInfo {
                homeserver: "https://example.invalid".to_owned(),
                user_id: "@own:example.invalid".to_owned(),
                device_id: "OWN".to_owned(),
                authentication_method: koushi_state::SessionAuthenticationMethod::Unknown,
            }),
            ..AppState::default()
        };
        let profile = |user_id: &str, display_name: &str| UserProfile {
            user_id: user_id.to_owned(),
            display_name: Some(display_name.to_owned()),
            display_label: String::new(),
            original_display_label: String::new(),
            mention_search_terms: Vec::new(),
            avatar: None,
        };
        state.profile.local_aliases.insert(
            "@alias:example.invalid".to_owned(),
            "Private alias".to_owned(),
        );
        state.profile.users.insert(
            "@cached:example.invalid".to_owned(),
            profile("@cached:example.invalid", "Cached label"),
        );
        state
            .profile
            .room_users
            .entry(room_id.to_owned())
            .or_default()
            .insert(
                "@room:example.invalid".to_owned(),
                profile("@room:example.invalid", "Room label"),
            );

        let receipt = |user_id: &str, display_name: Option<&str>| LiveReadReceipt {
            user_id: user_id.to_owned(),
            display_name: display_name.map(ToOwned::to_owned),
            original_display_label: String::new(),
            avatar: None,
            timestamp_ms: Some(42),
        };
        let action = AppAction::LiveRoomReceiptsWindowReconciled {
            room_id: room_id.to_owned(),
            scoped_event_ids: Vec::new(),
            receipts_by_event: vec![
                LiveEventReceipts {
                    event_id: "$alias-event:example.invalid".to_owned(),
                    receipts: vec![receipt("@alias:example.invalid", None)],
                },
                LiveEventReceipts {
                    event_id: "$room-event:example.invalid".to_owned(),
                    receipts: vec![receipt("@room:example.invalid", None)],
                },
                LiveEventReceipts {
                    event_id: "$payload-event:example.invalid".to_owned(),
                    receipts: vec![receipt("@payload:example.invalid", Some("Payload label"))],
                },
                LiveEventReceipts {
                    event_id: "$cache-event:example.invalid".to_owned(),
                    receipts: vec![receipt("@cached:example.invalid", None)],
                },
                LiveEventReceipts {
                    event_id: "$unknown-event:example.invalid".to_owned(),
                    receipts: vec![receipt("@unknown:example.invalid", None)],
                },
            ],
        };

        let event = profile_resolution_diagnostic_event(&state, &action)
            .expect("profile resolution diagnostics should be emitted");
        let field = |key| {
            event
                .fields
                .iter()
                .find(|field| field.key == key)
                .map(|field| &field.value)
        };
        assert_eq!(
            field("input_count"),
            Some(&koushi_diagnostics::DiagnosticValue::Count(5))
        );
        assert_eq!(
            field("output_count"),
            Some(&koushi_diagnostics::DiagnosticValue::Count(5))
        );
        assert_eq!(
            field("local_alias_count"),
            Some(&koushi_diagnostics::DiagnosticValue::Count(1))
        );
        assert_eq!(
            field("relevant_room_count"),
            Some(&koushi_diagnostics::DiagnosticValue::Count(1))
        );
        assert_eq!(
            field("payload_count"),
            Some(&koushi_diagnostics::DiagnosticValue::Count(1))
        );
        assert_eq!(
            field("global_cache_count"),
            Some(&koushi_diagnostics::DiagnosticValue::Count(1))
        );
        assert_eq!(
            field("unresolved_count"),
            Some(&koushi_diagnostics::DiagnosticValue::Count(1))
        );
        assert_eq!(
            field("cache_stale_hit_status"),
            Some(&koushi_diagnostics::DiagnosticValue::Token("not_tracked"))
        );

        let encoded = serde_json::to_string(&event).expect("diagnostic should serialize");
        for forbidden in [
            "@alias:example.invalid",
            "Private alias",
            "mxc://example.invalid/avatar",
        ] {
            assert!(
                !encoded.contains(forbidden),
                "diagnostic leaked {forbidden}"
            );
        }
    }
}
