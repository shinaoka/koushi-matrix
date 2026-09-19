use super::super::tests::unread_diagnostic_room;
use super::*;
use koushi_state::{
    ConversationActivity, ConversationActivitySource, RoomNotificationModeOperation,
    RoomNotificationSettings, RoomTags, UserProfile,
};
use std::collections::BTreeSet;

#[test]
fn activity_resolution_cannot_succeed_while_room_placeholders_remain() {
    let generation = 7;
    let mut state = AppState::default();
    state.activity = ActivityState::Open {
        active_tab: ActivityTab::Unread,
        recent: ActivityStream::default(),
        unread: ActivityStream {
            rows: vec![ActivityRow {
                kind: ActivityRowKind::RoomUnread,
                room_id: "!room:example.invalid".to_owned(),
                ..ActivityRow::default()
            }],
            next_batch: None,
            resolution: ActivityResolutionState::Resolving {
                generation,
                unresolved_room_count: 1,
            },
        },
        mark_read: Default::default(),
    };

    assert_eq!(
        guard_activity_resolution_completion(
            &state,
            AppAction::ActivityResolutionSucceeded { generation },
        ),
        AppAction::ActivityResolutionFailed {
            generation,
            unresolved_room_count: 1,
            kind: OperationFailureKind::Timeout,
        }
    );
}

#[test]
fn activity_resolution_rows_are_generation_guarded() {
    let generation = 7;
    let mut state = AppState::default();
    state.activity = ActivityState::Open {
        active_tab: ActivityTab::Unread,
        recent: ActivityStream::default(),
        unread: ActivityStream {
            rows: Vec::new(),
            next_batch: None,
            resolution: ActivityResolutionState::Resolving {
                generation,
                unresolved_room_count: 1,
            },
        },
        mark_read: Default::default(),
    };
    let row = ActivityRow::event(
        "!room:example.invalid".to_owned(),
        "$event:example.invalid".to_owned(),
        None,
        String::new(),
        None,
        None,
        1,
        false,
        false,
    );

    assert!(
        normalize_activity_resolution_action(
            &state,
            AppAction::ActivityResolutionRowsObserved {
                generation: generation - 1,
                rows: vec![row.clone()],
            },
        )
        .is_none()
    );
    assert_eq!(
        normalize_activity_resolution_action(
            &state,
            AppAction::ActivityResolutionRowsObserved {
                generation,
                rows: vec![row.clone()],
            },
        ),
        Some(AppAction::ActivityResolutionRowsObserved {
            generation,
            rows: vec![row],
        })
    );
}

#[test]
fn activity_resolution_request_batch_has_an_account_wide_cap() {
    let requests = (0..(MAX_ACTIVITY_RESOLUTION_ROOMS + 3))
        .map(|index| ActivityResolutionRequest {
            room_id: format!("!room-{index}:example.invalid"),
            fully_read_event_id: None,
            minimum_unread_count: 1,
        })
        .collect::<Vec<_>>();
    let first = cap_activity_resolution_requests(requests.clone(), 1);
    let second = cap_activity_resolution_requests(requests, 2);
    assert_eq!(first.len(), MAX_ACTIVITY_RESOLUTION_ROOMS);
    assert_eq!(second.len(), MAX_ACTIVITY_RESOLUTION_ROOMS);
    let attempted = first
        .into_iter()
        .chain(second)
        .map(|request| request.room_id)
        .collect::<BTreeSet<_>>();
    assert_eq!(attempted.len(), MAX_ACTIVITY_RESOLUTION_ROOMS + 3);
}

#[test]
fn activity_projection_ignores_plain_unread_count_for_activity_unread() {
    let mut state = AppState::default();
    state.rooms = vec![RoomSummary {
        room_id: "!room:example.invalid".to_owned(),
        display_name: "Room".to_owned(),
        display_label: "Room".to_owned(),
        original_display_label: "Room".to_owned(),
        avatar: None,
        is_dm: false,
        dm_user_ids: Vec::new(),
        tags: RoomTags::default(),
        unread_count: 3,
        notification_count: 0,
        highlight_count: 0,
        marked_unread: false,
        recency_stamp: Some(42),
        conversation_activity: Some(ConversationActivity {
            timestamp_ms: 42,
            source: ConversationActivitySource::Message,
        }),
        latest_event: Some(RoomLatestEventSummary {
            event_id: "$latest:example.invalid".to_owned(),
            relation_type: None,
            relation_event_id: None,
            sender_id: Some("@sender:example.invalid".to_owned()),
            sender_label: Some("Sender".to_owned()),
            sender_avatar: None,
            preview: Some("body".to_owned()),
            timestamp_ms: 42,
            is_redacted: false,
        }),
        parent_space_ids: Vec::new(),
        dm_space_ids: Vec::new(),
        is_encrypted: false,
        joined_members: 2,
    }];

    let mut projection = ActivityProjection::default();
    let (recent, unread, _excluded_room_ids) = projection.snapshot(&state);

    assert!(
        unread.rows.is_empty(),
        "Activity Unread should not invent un-navigable rows from plain unread message counts"
    );
    assert_eq!(recent.rows.len(), 1);
    assert!(
        !recent.rows[0].unread,
        "plain unread message counts should not mark Activity recent rows unread"
    );
}

#[test]
fn canonical_activity_authority_hidden_reversal_and_redaction_tombstone_converge() {
    let room_id = "!room:example.invalid";
    let event_id = "$event:example.invalid";
    let row = |preview: &str, timestamp_ms| {
        ActivityRow::event(
            room_id.to_owned(),
            event_id.to_owned(),
            Some("@sender:example.invalid".to_owned()),
            "Room".to_owned(),
            Some("Sender".to_owned()),
            Some(preview.to_owned()),
            timestamp_ms,
            false,
            false,
        )
    };
    let mut projection = ActivityProjection::default();
    projection.ingest_resolution_rows(vec![row("resolver", 1)]);
    projection.reconcile_canonical_window(
        room_id.to_owned(),
        vec![row("canonical", 2)],
        Vec::new(),
        Vec::new(),
    );
    assert_eq!(
        projection
            .effective_rows()
            .values()
            .next()
            .and_then(|row| row.preview.as_deref()),
        Some("canonical")
    );

    projection.reconcile_canonical_window(
        room_id.to_owned(),
        vec![row("canonical", 2)],
        Vec::new(),
        vec![event_id.to_owned()],
    );
    assert!(projection.effective_rows().is_empty());
    projection.reconcile_canonical_window(
        room_id.to_owned(),
        vec![row("restored", 3)],
        Vec::new(),
        Vec::new(),
    );
    assert_eq!(
        projection
            .effective_rows()
            .values()
            .next()
            .and_then(|row| row.preview.as_deref()),
        Some("restored")
    );

    projection.reconcile_canonical_window(
        room_id.to_owned(),
        vec![row("redacted", 4)],
        vec![event_id.to_owned()],
        Vec::new(),
    );
    assert!(projection.effective_rows().is_empty());
    projection.reconcile_canonical_window(
        room_id.to_owned(),
        vec![row("must not resurrect", 5)],
        Vec::new(),
        Vec::new(),
    );
    assert!(projection.effective_rows().is_empty());
}

#[test]
fn canonical_activity_provenance_enforces_every_reviewed_bound() {
    let row = |room_index: usize, event_index: usize| {
        ActivityRow::event(
            format!("!room-{room_index}:example.invalid"),
            format!("$event-{room_index}-{event_index}:example.invalid"),
            None,
            String::new(),
            None,
            Some("body".to_owned()),
            event_index as u64,
            false,
            false,
        )
    };

    let mut room_bound = ActivityProjection::default();
    for room_index in 0..=MAX_CANONICAL_ROOM_SLOTS {
        room_bound.reconcile_canonical_window(
            format!("!room-{room_index}:example.invalid"),
            vec![row(room_index, 0)],
            Vec::new(),
            Vec::new(),
        );
    }
    assert_eq!(
        room_bound.canonical_rows_by_room.len(),
        MAX_CANONICAL_ROOM_SLOTS
    );

    let mut row_bound = ActivityProjection::default();
    row_bound.reconcile_canonical_window(
        "!room-0:example.invalid".to_owned(),
        (0..=MAX_CANONICAL_ROWS_PER_ROOM)
            .map(|event_index| row(0, event_index))
            .collect(),
        Vec::new(),
        (0..=MAX_CANONICAL_ROWS_PER_ROOM)
            .map(|index| format!("$hidden-{index}:example.invalid"))
            .collect(),
    );
    assert_eq!(
        row_bound.canonical_rows_by_room["!room-0:example.invalid"].len(),
        MAX_CANONICAL_ROWS_PER_ROOM
    );
    assert_eq!(
        row_bound.hidden_event_ids_by_room["!room-0:example.invalid"].len(),
        MAX_CANONICAL_ROWS_PER_ROOM
    );

    let mut global_bound = ActivityProjection::default();
    for room_index in 0..MAX_CANONICAL_ROOM_SLOTS {
        global_bound.reconcile_canonical_window(
            format!("!room-{room_index}:example.invalid"),
            (0..5)
                .map(|event_index| row(room_index, event_index))
                .collect(),
            Vec::new(),
            Vec::new(),
        );
    }
    assert_eq!(
        global_bound.canonical_row_ordinals.len(),
        MAX_CANONICAL_ROWS_GLOBAL
    );

    let mut resolver_bound = ActivityProjection::default();
    resolver_bound.ingest_resolution_rows(
        (0..=ACTIVITY_RECENT_MAX_ROWS)
            .map(|event_index| row(0, event_index))
            .collect(),
    );
    assert_eq!(
        resolver_bound.resolution_rows_by_event_id.len(),
        ACTIVITY_RECENT_MAX_ROWS
    );

    let mut tombstone_bound = ActivityProjection::default();
    tombstone_bound.reconcile_canonical_window(
        "!room-0:example.invalid".to_owned(),
        Vec::new(),
        (0..=MAX_ACTIVITY_REDACTION_TOMBSTONES)
            .map(|index| format!("$redacted-{index}:example.invalid"))
            .collect(),
        Vec::new(),
    );
    assert_eq!(
        tombstone_bound.redacted_event_ids.len(),
        MAX_ACTIVITY_REDACTION_TOMBSTONES
    );
}

#[test]
fn activity_projection_bounds_recent_history_to_newest_observed_rows() {
    let mut state = AppState::default();
    let mut room = unread_diagnostic_room("!room:example.invalid");
    room.unread_count = 0;
    room.notification_count = 0;
    room.highlight_count = 0;
    room.marked_unread = false;
    state.rooms = vec![room];

    let mut projection = ActivityProjection::default();
    projection.ingest(
        (0..=ACTIVITY_RECENT_MAX_ROWS)
            .map(|index| {
                ActivityRow::event(
                    "!room:example.invalid".to_owned(),
                    format!("$event-{index}:example.invalid"),
                    Some("@sender:example.invalid".to_owned()),
                    "Room".to_owned(),
                    Some("Sender".to_owned()),
                    Some(format!("body {index}")),
                    index as u64,
                    false,
                    false,
                )
            })
            .collect(),
    );

    let (recent, _unread, _excluded_room_ids) = projection.snapshot(&state);

    assert_eq!(recent.rows.len(), ACTIVITY_RECENT_MAX_ROWS);
    assert_eq!(
        recent.rows.first().and_then(|row| row.event_id.as_deref()),
        Some("$event-200:example.invalid")
    );
    assert_eq!(
        recent.rows.last().and_then(|row| row.event_id.as_deref()),
        Some("$event-1:example.invalid")
    );
    assert_eq!(
        projection
            .canonical_rows_by_room
            .values()
            .map(BTreeMap::len)
            .sum::<usize>(),
        MAX_CANONICAL_ROWS_PER_ROOM
    );
    assert_eq!(projection.resolution_rows_by_event_id.len(), 81);
}

#[test]
fn activity_projection_keeps_old_unread_rows_outside_recent_window() {
    let mut state = AppState::default();
    state.rooms = vec![unread_diagnostic_room("!room:example.invalid")];

    let rows = (0..=ACTIVITY_RECENT_MAX_ROWS)
        .map(|index| {
            ActivityRow::event(
                "!room:example.invalid".to_owned(),
                format!("$event-{index}:example.invalid"),
                Some("@sender:example.invalid".to_owned()),
                "Room".to_owned(),
                Some("Sender".to_owned()),
                Some(format!("body {index}")),
                index as u64,
                false,
                false,
            )
        })
        .collect::<Vec<_>>();
    let mut projection = ActivityProjection::default();
    projection.ingest(rows);

    let (recent, unread, _excluded_room_ids) = projection.snapshot(&state);

    assert_eq!(recent.rows.len(), ACTIVITY_RECENT_MAX_ROWS);
    assert_eq!(unread.rows.len(), ACTIVITY_RECENT_MAX_ROWS + 1);
    assert!(
        unread
            .rows
            .iter()
            .any(|row| { row.event_id.as_deref() == Some("$event-0:example.invalid") })
    );
    assert_eq!(
        projection.effective_rows().len(),
        ACTIVITY_RECENT_MAX_ROWS + 1
    );
}

#[test]
fn activity_projection_ignores_plain_unread_count_for_ingested_event_rows() {
    let mut state = AppState::default();
    state.rooms = vec![RoomSummary {
        room_id: "!room:example.invalid".to_owned(),
        display_name: "Room".to_owned(),
        display_label: "Room".to_owned(),
        original_display_label: "Room".to_owned(),
        avatar: None,
        is_dm: false,
        dm_user_ids: Vec::new(),
        tags: RoomTags::default(),
        unread_count: 3,
        notification_count: 0,
        highlight_count: 0,
        marked_unread: false,
        recency_stamp: Some(42),
        conversation_activity: None,
        latest_event: None,
        parent_space_ids: Vec::new(),
        dm_space_ids: Vec::new(),
        is_encrypted: false,
        joined_members: 2,
    }];

    let mut projection = ActivityProjection::default();
    projection.ingest(vec![ActivityRow::event(
        "!room:example.invalid".to_owned(),
        "$event:example.invalid".to_owned(),
        Some("@sender:example.invalid".to_owned()),
        "Room".to_owned(),
        Some("Sender".to_owned()),
        Some("body".to_owned()),
        42,
        true,
        false,
    )]);
    let (recent, unread, _excluded_room_ids) = projection.snapshot(&state);

    assert!(unread.rows.is_empty());
    assert_eq!(recent.rows.len(), 1);
    assert!(
        !recent.rows[0].unread,
        "ingested event rows must not inherit plain unread-only state"
    );
}

#[test]
fn activity_projection_skips_recent_rows_for_mentions_mode_without_highlight() {
    let mut state = AppState::default();
    state.rooms = vec![RoomSummary {
        room_id: "!room:example.invalid".to_owned(),
        display_name: "Room".to_owned(),
        display_label: "Room".to_owned(),
        original_display_label: "Room".to_owned(),
        avatar: None,
        is_dm: false,
        dm_user_ids: Vec::new(),
        tags: RoomTags::default(),
        unread_count: 1,
        notification_count: 1,
        highlight_count: 0,
        marked_unread: false,
        recency_stamp: Some(42),
        conversation_activity: Some(ConversationActivity {
            timestamp_ms: 42,
            source: ConversationActivitySource::Message,
        }),
        latest_event: Some(RoomLatestEventSummary {
            event_id: "$latest:example.invalid".to_owned(),
            relation_type: None,
            relation_event_id: None,
            sender_id: Some("@sender:example.invalid".to_owned()),
            sender_label: Some("Sender".to_owned()),
            sender_avatar: None,
            preview: Some("body".to_owned()),
            timestamp_ms: 42,
            is_redacted: false,
        }),
        parent_space_ids: Vec::new(),
        dm_space_ids: Vec::new(),
        is_encrypted: false,
        joined_members: 2,
    }];
    state.room_notification_settings.insert(
        "!room:example.invalid".to_owned(),
        RoomNotificationSettings {
            mode: RoomNotificationMode::Mentions,
            operation: RoomNotificationModeOperation::Idle,
        },
    );

    let mut projection = ActivityProjection::default();
    projection.ingest(vec![ActivityRow::event(
        "!room:example.invalid".to_owned(),
        "$event:example.invalid".to_owned(),
        Some("@sender:example.invalid".to_owned()),
        "Room".to_owned(),
        Some("Sender".to_owned()),
        Some("body".to_owned()),
        41,
        true,
        false,
    )]);
    let (recent, unread, _excluded_room_ids) = projection.snapshot(&state);

    assert!(recent.rows.is_empty());
    assert!(unread.rows.is_empty());
}

#[test]
fn activity_projection_context_label_uses_space_and_room_names() {
    let mut state = AppState::default();
    state.spaces = vec![SpaceSummary {
        space_id: "!space:example.invalid".to_owned(),
        display_name: "Science".to_owned(),
        avatar: None,
        child_room_ids: vec!["!room:example.invalid".to_owned()],
    }];
    state.rooms = vec![RoomSummary {
        room_id: "!room:example.invalid".to_owned(),
        display_name: "Room".to_owned(),
        display_label: "Papers".to_owned(),
        original_display_label: "Room".to_owned(),
        avatar: None,
        is_dm: false,
        dm_user_ids: Vec::new(),
        tags: RoomTags::default(),
        unread_count: 0,
        notification_count: 0,
        highlight_count: 0,
        marked_unread: false,
        recency_stamp: Some(42),
        conversation_activity: Some(ConversationActivity {
            timestamp_ms: 42,
            source: ConversationActivitySource::Message,
        }),
        latest_event: Some(RoomLatestEventSummary {
            event_id: "$latest:example.invalid".to_owned(),
            relation_type: None,
            relation_event_id: None,
            sender_id: Some("@sender:example.invalid".to_owned()),
            sender_label: Some("Sender".to_owned()),
            sender_avatar: None,
            preview: Some("body".to_owned()),
            timestamp_ms: 42,
            is_redacted: false,
        }),
        parent_space_ids: vec!["!space:example.invalid".to_owned()],
        dm_space_ids: Vec::new(),
        is_encrypted: false,
        joined_members: 2,
    }];

    let mut projection = ActivityProjection::default();
    let (recent, _unread, _excluded_room_ids) = projection.snapshot(&state);

    assert_eq!(recent.rows[0].context_label, "Science / Papers");
}

#[test]
fn activity_projection_ignores_latest_profile_event_after_message() {
    let mut state = AppState::default();
    let mut room = super::super::tests::unread_diagnostic_room("!room:example.invalid");
    room.unread_count = 0;
    room.notification_count = 0;
    room.highlight_count = 0;
    room.marked_unread = false;
    room.conversation_activity = Some(ConversationActivity {
        timestamp_ms: 42,
        source: ConversationActivitySource::Message,
    });
    room.latest_event = Some(RoomLatestEventSummary {
        event_id: "$profile-update:example.invalid".to_owned(),
        relation_type: None,
        relation_event_id: None,
        sender_id: Some("@sender:example.invalid".to_owned()),
        sender_label: Some("Sender".to_owned()),
        sender_avatar: None,
        preview: None,
        timestamp_ms: 99,
        is_redacted: false,
    });
    state.rooms = vec![room];

    let mut projection = ActivityProjection::default();
    let (recent, unread, _excluded_room_ids) = projection.snapshot(&state);

    assert!(recent.rows.is_empty());
    assert!(unread.rows.is_empty());
    assert!(
        projection
            .fully_read_marker_updates(&state, &ActivityMarkReadTarget::All)
            .is_empty()
    );
}

#[test]
fn activity_projection_reconciles_replacement_latest_with_original_timeline_row() {
    let room_id = "!room:example.invalid";
    let original_event_id = "$original:example.invalid";
    let sender_id = "@sender:example.invalid";
    let mut state = AppState::default();
    state.profile.users.insert(
        sender_id.to_owned(),
        UserProfile {
            user_id: sender_id.to_owned(),
            display_name: Some("Sender".to_owned()),
            display_label: "Sender".to_owned(),
            original_display_label: "Sender".to_owned(),
            mention_search_terms: vec!["Sender".to_owned()],
            avatar: Some(koushi_state::AvatarImage {
                mxc_uri: "mxc://example.invalid/enriched".to_owned(),
                thumbnail: Default::default(),
            }),
        },
    );
    state.rooms = vec![RoomSummary {
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
        recency_stamp: Some(42),
        conversation_activity: None,
        latest_event: Some(RoomLatestEventSummary {
            event_id: "$edit:example.invalid".to_owned(),
            relation_type: Some("m.replace".to_owned()),
            relation_event_id: Some(original_event_id.to_owned()),
            sender_id: Some(sender_id.to_owned()),
            sender_label: Some("Sender".to_owned()),
            sender_avatar: None,
            preview: Some("edited body".to_owned()),
            timestamp_ms: 42,
            is_redacted: false,
        }),
        parent_space_ids: Vec::new(),
        dm_space_ids: Vec::new(),
        is_encrypted: false,
        joined_members: 2,
    }];
    state.rooms[0].unread_count = 1;
    state.rooms[0].notification_count = 1;

    let mut fallback_projection = ActivityProjection::default();
    let (_recent, unread_without_canonical, _excluded) = fallback_projection.snapshot(&state);
    assert_eq!(
        unread_without_canonical.rows[0].kind,
        ActivityRowKind::RoomUnread
    );
    assert_eq!(unread_without_canonical.rows[0].event_id, None);
    assert!(
        fallback_projection
            .fully_read_marker_updates(&state, &ActivityMarkReadTarget::All)
            .is_empty(),
        "a defensive m.replace latest must not invent a fully-read target"
    );

    let mut projection = ActivityProjection::default();
    projection.ingest(vec![ActivityRow::event(
        room_id.to_owned(),
        original_event_id.to_owned(),
        Some(sender_id.to_owned()),
        "Room".to_owned(),
        Some("Sender".to_owned()),
        Some("edited body".to_owned()),
        41,
        false,
        false,
    )]);

    let (recent, _unread, _excluded) = projection.snapshot(&state);

    assert_eq!(recent.rows.len(), 1);
    assert_eq!(recent.rows[0].event_id.as_deref(), Some(original_event_id));
    assert_eq!(recent.rows[0].timestamp_ms, 41);
    assert_eq!(
        recent.rows[0]
            .sender_avatar
            .as_ref()
            .map(|avatar| avatar.mxc_uri.as_str()),
        Some("mxc://example.invalid/enriched")
    );
}

#[test]
fn room_unread_placeholder_guards_latest_identity_and_timestamp() {
    let latest = |relation_type: Option<&str>, is_redacted: bool| RoomLatestEventSummary {
        event_id: "$latest:example.invalid".to_owned(),
        relation_type: relation_type.map(ToOwned::to_owned),
        relation_event_id: Some("$target:example.invalid".to_owned()),
        sender_id: Some("@sender:example.invalid".to_owned()),
        sender_label: Some("Sender".to_owned()),
        sender_avatar: None,
        preview: Some("body".to_owned()),
        timestamp_ms: 99,
        is_redacted,
    };

    for (relation_type, is_redacted) in [(None, true), (Some("m.replace"), false)] {
        let mut state = AppState::default();
        let mut room = super::super::tests::unread_diagnostic_room("!room:example.invalid");
        room.unread_count = 1;
        room.notification_count = 1;
        room.highlight_count = 0;
        room.marked_unread = false;
        room.conversation_activity = Some(ConversationActivity {
            timestamp_ms: 37,
            source: ConversationActivitySource::Message,
        });
        room.latest_event = Some(latest(relation_type, is_redacted));
        state.rooms = vec![room];

        let mut projection = ActivityProjection::default();
        let (_recent, unread, _excluded) = projection.snapshot(&state);
        let placeholder = unread
            .rows
            .first()
            .expect("guarded room unread placeholder");
        assert_eq!(placeholder.kind, ActivityRowKind::RoomUnread);
        assert_eq!(placeholder.timestamp_ms, 37);
        assert!(
            projection
                .fully_read_marker_updates(&state, &ActivityMarkReadTarget::All)
                .is_empty()
        );
    }
}

#[test]
fn activity_projection_does_not_append_annotation_latest_event() {
    let mut state = AppState::default();
    state.rooms = vec![RoomSummary {
        room_id: "!room:example.invalid".to_owned(),
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
        recency_stamp: Some(42),
        conversation_activity: None,
        latest_event: Some(RoomLatestEventSummary {
            event_id: "$reaction:example.invalid".to_owned(),
            relation_type: Some("m.annotation".to_owned()),
            relation_event_id: Some("$target:example.invalid".to_owned()),
            sender_id: Some("@sender:example.invalid".to_owned()),
            sender_label: Some("Sender".to_owned()),
            sender_avatar: None,
            preview: None,
            timestamp_ms: 42,
            is_redacted: false,
        }),
        parent_space_ids: Vec::new(),
        dm_space_ids: Vec::new(),
        is_encrypted: false,
        joined_members: 2,
    }];

    let (recent, unread, _excluded) = ActivityProjection::default().snapshot(&state);

    assert!(recent.rows.is_empty());
    assert!(unread.rows.is_empty());
}
