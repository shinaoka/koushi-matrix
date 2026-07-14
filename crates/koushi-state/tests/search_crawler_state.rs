use koushi_state::{
    AppAction, AppEffect, AppState, RoomSummary, RoomTags, SearchCrawlerFailureKind,
    SearchCrawlerLastActiveStatus, SearchCrawlerRoomState, SearchCrawlerSettings,
    SearchCrawlerSpeed, SearchCrawlerState, SearchScope, SearchState, SessionInfo, SessionState,
    SettingsPatch, UiEvent, reduce,
};

// Bring the Debug format in scope so assert! messages can print effects.
#[allow(unused_imports)]
use std::fmt::Debug;

fn session_info() -> SessionInfo {
    SessionInfo {
        homeserver: "https://matrix.example.org".to_owned(),
        user_id: "@user-a:example.invalid".to_owned(),
        device_id: "DEVICE".to_owned(),
    }
}

fn ready_state_with_rooms(room_ids: &[&str]) -> AppState {
    AppState {
        session: SessionState::Ready(session_info()),
        rooms: room_ids
            .iter()
            .map(|id| RoomSummary {
                room_id: (*id).to_owned(),
                display_name: (*id).to_owned(),
                display_label: (*id).to_owned(),
                original_display_label: (*id).to_owned(),
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
                joined_members: 0,
            })
            .collect(),
        ..AppState::default()
    }
}

fn settings_standard() -> SearchCrawlerSettings {
    SearchCrawlerSettings::default()
}

fn settings_paused() -> SearchCrawlerSettings {
    SearchCrawlerSettings {
        speed: SearchCrawlerSpeed::Paused,
        ..SearchCrawlerSettings::default()
    }
}

fn settings_no_media_captions() -> SearchCrawlerSettings {
    SearchCrawlerSettings {
        include_media_captions: false,
        ..SearchCrawlerSettings::default()
    }
}

fn settings_no_filenames() -> SearchCrawlerSettings {
    SearchCrawlerSettings {
        include_filenames: false,
        ..SearchCrawlerSettings::default()
    }
}

// ---------------------------------------------------------------------------
// HistoryCrawl state transitions
// ---------------------------------------------------------------------------

#[test]
fn crawl_started_sets_queued_state_and_emits_event() {
    let mut state = ready_state_with_rooms(&["room-a"]);

    let effects = reduce(
        &mut state,
        AppAction::HistoryCrawlStarted {
            request_id: 1,
            room_id: "room-a".to_owned(),
            timestamp_ms: 1_000,
        },
    );

    assert_eq!(
        state.search_crawler.rooms.get("room-a"),
        Some(&SearchCrawlerRoomState::Queued)
    );
    assert_eq!(
        effects,
        vec![AppEffect::EmitUiEvent(UiEvent::SearchCrawlerChanged)]
    );
}

#[test]
fn crawl_progress_updates_running_counters_and_emits_event() {
    let mut state = ready_state_with_rooms(&["room-a"]);

    // Seed Running state.
    reduce(
        &mut state,
        AppAction::HistoryCrawlStarted {
            request_id: 1,
            room_id: "room-a".to_owned(),
            timestamp_ms: 1_000,
        },
    );

    let effects = reduce(
        &mut state,
        AppAction::HistoryCrawlProgress {
            room_id: "room-a".to_owned(),
            processed: 50,
            indexed: 42,
            timestamp_ms: 2_000,
        },
    );

    assert_eq!(
        state.search_crawler.rooms.get("room-a"),
        Some(&SearchCrawlerRoomState::Running {
            processed: 50,
            indexed: 42
        })
    );
    assert_eq!(
        effects,
        vec![AppEffect::EmitUiEvent(UiEvent::SearchCrawlerChanged)]
    );
}

#[test]
fn crawl_completed_sets_completed_state_and_emits_event() {
    let mut state = ready_state_with_rooms(&["room-a"]);

    reduce(
        &mut state,
        AppAction::HistoryCrawlStarted {
            request_id: 1,
            room_id: "room-a".to_owned(),
            timestamp_ms: 1_000,
        },
    );

    let effects = reduce(
        &mut state,
        AppAction::HistoryCrawlCompleted {
            room_id: "room-a".to_owned(),
            indexed: 17,
            timestamp_ms: 3_000,
        },
    );

    assert_eq!(
        state.search_crawler.rooms.get("room-a"),
        Some(&SearchCrawlerRoomState::Completed { indexed: 17 })
    );
    assert_eq!(
        effects,
        vec![AppEffect::EmitUiEvent(UiEvent::SearchCrawlerChanged)]
    );
}

#[test]
fn crawl_failed_carries_only_coarse_kind_and_emits_event() {
    let mut state = ready_state_with_rooms(&["room-a"]);

    reduce(
        &mut state,
        AppAction::HistoryCrawlStarted {
            request_id: 1,
            room_id: "room-a".to_owned(),
            timestamp_ms: 1_000,
        },
    );

    for kind in [
        SearchCrawlerFailureKind::RoomNotFound,
        SearchCrawlerFailureKind::Sdk,
        SearchCrawlerFailureKind::Decryption,
        SearchCrawlerFailureKind::IndexUnavailable,
    ] {
        let mut s = state.clone();
        let effects = reduce(
            &mut s,
            AppAction::HistoryCrawlFailed {
                room_id: "room-a".to_owned(),
                kind: kind.clone(),
                timestamp_ms: 4_000,
            },
        );

        assert_eq!(
            s.search_crawler.rooms.get("room-a"),
            Some(&SearchCrawlerRoomState::Failed { kind: kind.clone() })
        );
        // Failed state carries ONLY the coarse kind — no raw SDK errors, room IDs,
        // event IDs, or message bodies are stored.
        let debug_output = format!("{:?}", s.search_crawler.rooms.get("room-a"));
        assert!(
            !debug_output.contains("room-a"),
            "room id must not appear in Failed debug output"
        );
        assert_eq!(
            effects,
            vec![AppEffect::EmitUiEvent(UiEvent::SearchCrawlerChanged)]
        );
    }
}

// ---------------------------------------------------------------------------
// Paused → active: enqueues all known rooms
// ---------------------------------------------------------------------------

#[test]
fn enable_from_paused_enqueues_all_known_rooms() {
    let mut state = ready_state_with_rooms(&["room-a", "room-b"]);
    // Start paused.
    state.settings.values.search_crawler = settings_paused();

    let effects = reduce(
        &mut state,
        AppAction::SettingsUpdateRequested {
            request_id: 1,
            patch: SettingsPatch {
                search_crawler: Some(settings_standard()),
                ..Default::default()
            },
        },
    );

    // Must include NotifySearchCrawlerRoomsAvailable with all known rooms.
    let notify = effects.iter().find(|e| {
        matches!(
            e,
            AppEffect::NotifySearchCrawlerRoomsAvailable {
                room_ids,
                ..
            } if room_ids.len() == 2
        )
    });
    assert!(
        notify.is_some(),
        "expected NotifySearchCrawlerRoomsAvailable with 2 rooms; got {effects:?}"
    );
    if let Some(AppEffect::NotifySearchCrawlerRoomsAvailable { room_ids, settings }) = notify {
        let mut ids = room_ids.clone();
        ids.sort();
        assert_eq!(ids, vec!["room-a", "room-b"]);
        assert_eq!(settings.speed, SearchCrawlerSpeed::Standard);
    }
}

#[test]
fn enable_from_paused_with_no_rooms_does_not_enqueue() {
    let mut state = AppState {
        session: SessionState::Ready(session_info()),
        ..AppState::default()
    };
    state.settings.values.search_crawler = settings_paused();

    let effects = reduce(
        &mut state,
        AppAction::SettingsUpdateRequested {
            request_id: 1,
            patch: SettingsPatch {
                search_crawler: Some(settings_standard()),
                ..Default::default()
            },
        },
    );

    let has_notify = effects
        .iter()
        .any(|e| matches!(e, AppEffect::NotifySearchCrawlerRoomsAvailable { .. }));
    assert!(!has_notify, "no rooms to enqueue, effect must be absent");
}

// ---------------------------------------------------------------------------
// Active speed changes
// ---------------------------------------------------------------------------

#[test]
fn pause_from_active_notifies_actor_without_invalidating_completed_rooms() {
    let mut state = ready_state_with_rooms(&["room-a", "room-b"]);
    state.search_crawler.rooms.insert(
        "room-a".to_owned(),
        SearchCrawlerRoomState::Completed { indexed: 10 },
    );

    let effects = reduce(
        &mut state,
        AppAction::SettingsUpdateRequested {
            request_id: 1,
            patch: SettingsPatch {
                search_crawler: Some(settings_paused()),
                ..Default::default()
            },
        },
    );

    assert_eq!(
        state.search_crawler.rooms.get("room-a"),
        Some(&SearchCrawlerRoomState::Completed { indexed: 10 })
    );

    let notify = effects.iter().find(|effect| {
        matches!(
            effect,
            AppEffect::NotifySearchCrawlerRoomsAvailable {
                settings,
                ..
            } if settings.speed == SearchCrawlerSpeed::Paused
        )
    });
    assert!(
        notify.is_some(),
        "pausing must notify the actor so queued and active crawler pages stop; got {effects:?}"
    );
}

#[test]
fn pause_from_active_notifies_actor_without_predicting_running_room_state() {
    let mut state =
        ready_state_with_rooms(&["room-a", "room-b", "room-c", "room-d", "room-e", "room-f"]);
    state.search_crawler.rooms.insert(
        "room-a".to_owned(),
        SearchCrawlerRoomState::Running {
            processed: 8,
            indexed: 5,
        },
    );
    state.search_crawler.rooms.insert(
        "room-b".to_owned(),
        SearchCrawlerRoomState::Running {
            processed: 2,
            indexed: 1,
        },
    );
    state.search_crawler.rooms.insert(
        "room-c".to_owned(),
        SearchCrawlerRoomState::Completed { indexed: 10 },
    );
    state.search_crawler.rooms.insert(
        "room-d".to_owned(),
        SearchCrawlerRoomState::Failed {
            kind: SearchCrawlerFailureKind::Sdk,
        },
    );
    state
        .search_crawler
        .rooms
        .insert("room-e".to_owned(), SearchCrawlerRoomState::Idle);
    state
        .search_crawler
        .rooms
        .insert("room-f".to_owned(), SearchCrawlerRoomState::Queued);

    let effects = reduce(
        &mut state,
        AppAction::SettingsUpdateRequested {
            request_id: 1,
            patch: SettingsPatch {
                search_crawler: Some(settings_paused()),
                ..Default::default()
            },
        },
    );

    assert_eq!(
        state.search_crawler.rooms.get("room-a"),
        Some(&SearchCrawlerRoomState::Running {
            processed: 8,
            indexed: 5,
        })
    );
    assert_eq!(
        state.search_crawler.rooms.get("room-b"),
        Some(&SearchCrawlerRoomState::Running {
            processed: 2,
            indexed: 1,
        })
    );
    assert_eq!(
        state.search_crawler.rooms.get("room-c"),
        Some(&SearchCrawlerRoomState::Completed { indexed: 10 })
    );
    assert_eq!(
        state.search_crawler.rooms.get("room-d"),
        Some(&SearchCrawlerRoomState::Failed {
            kind: SearchCrawlerFailureKind::Sdk,
        })
    );
    assert_eq!(
        state.search_crawler.rooms.get("room-e"),
        Some(&SearchCrawlerRoomState::Idle)
    );
    assert_eq!(
        state.search_crawler.rooms.get("room-f"),
        Some(&SearchCrawlerRoomState::Queued)
    );

    assert!(
        !effects.iter().any(|effect| {
            matches!(
                effect,
                AppEffect::EmitUiEvent(UiEvent::SearchCrawlerChanged)
            )
        }),
        "pause must not mutate crawler lifecycle locally; actor stop projection owns the visible settle"
    );

    let notify = effects.iter().find(|effect| {
        matches!(
            effect,
            AppEffect::NotifySearchCrawlerRoomsAvailable {
                settings,
                ..
            } if settings.speed == SearchCrawlerSpeed::Paused
        )
    });
    assert!(
        notify.is_some(),
        "pausing must notify the actor with paused settings; got {effects:?}"
    );
}

#[test]
fn history_crawl_progress_is_ignored_while_paused_until_actor_settles_stop() {
    let mut state = ready_state_with_rooms(&["room-a"]);
    state.settings.values.search_crawler = settings_paused();
    state
        .search_crawler
        .rooms
        .insert("room-a".to_owned(), SearchCrawlerRoomState::Queued);

    let effects = reduce(
        &mut state,
        AppAction::HistoryCrawlProgress {
            room_id: "room-a".to_owned(),
            processed: 3,
            indexed: 2,
            timestamp_ms: 42,
        },
    );

    assert!(effects.is_empty());
    assert_eq!(
        state.search_crawler.rooms.get("room-a"),
        Some(&SearchCrawlerRoomState::Queued)
    );
}

#[test]
fn history_crawl_stopped_settles_room_to_idle() {
    let mut state = ready_state_with_rooms(&["room-a"]);
    state.search_crawler.rooms.insert(
        "room-a".to_owned(),
        SearchCrawlerRoomState::Running {
            processed: 8,
            indexed: 5,
        },
    );

    let effects = reduce(
        &mut state,
        AppAction::HistoryCrawlStopped {
            room_id: "room-a".to_owned(),
        },
    );

    assert_eq!(
        state.search_crawler.rooms.get("room-a"),
        Some(&SearchCrawlerRoomState::Idle)
    );
    assert!(effects.iter().any(|effect| {
        matches!(
            effect,
            AppEffect::EmitUiEvent(UiEvent::SearchCrawlerChanged)
        )
    }));
}

#[test]
fn pure_speed_change_does_not_invalidate_completed_rooms() {
    let mut state = ready_state_with_rooms(&["room-a", "room-b"]);
    // Mark both as Completed.
    state.search_crawler.rooms.insert(
        "room-a".to_owned(),
        SearchCrawlerRoomState::Completed { indexed: 10 },
    );
    state.search_crawler.rooms.insert(
        "room-b".to_owned(),
        SearchCrawlerRoomState::Completed { indexed: 5 },
    );

    // Speed-only change: Standard → Slow.
    let effects = reduce(
        &mut state,
        AppAction::SettingsUpdateRequested {
            request_id: 1,
            patch: SettingsPatch {
                search_crawler: Some(SearchCrawlerSettings {
                    speed: SearchCrawlerSpeed::Slow,
                    ..settings_standard()
                }),
                ..Default::default()
            },
        },
    );

    // Completed rooms must stay Completed.
    assert_eq!(
        state.search_crawler.rooms.get("room-a"),
        Some(&SearchCrawlerRoomState::Completed { indexed: 10 })
    );
    assert_eq!(
        state.search_crawler.rooms.get("room-b"),
        Some(&SearchCrawlerRoomState::Completed { indexed: 5 })
    );

    // No SearchCrawlerChanged emitted for pure speed change (no content invalidation).
    let has_crawler_changed = effects
        .iter()
        .any(|e| matches!(e, AppEffect::EmitUiEvent(UiEvent::SearchCrawlerChanged)));
    assert!(!has_crawler_changed);

    // No NotifySearchCrawlerRoomsAvailable because prev was not Paused.
    let has_notify = effects
        .iter()
        .any(|e| matches!(e, AppEffect::NotifySearchCrawlerRoomsAvailable { .. }));
    assert!(!has_notify);
}

// ---------------------------------------------------------------------------
// Content-setting toggle invalidates Completed → Idle
// ---------------------------------------------------------------------------

#[test]
fn toggle_include_media_captions_resets_completed_to_idle() {
    let mut state = ready_state_with_rooms(&["room-a", "room-b"]);
    state.search_crawler.rooms.insert(
        "room-a".to_owned(),
        SearchCrawlerRoomState::Completed { indexed: 10 },
    );
    state.search_crawler.rooms.insert(
        "room-b".to_owned(),
        SearchCrawlerRoomState::Running {
            processed: 3,
            indexed: 1,
        },
    );

    let effects = reduce(
        &mut state,
        AppAction::SettingsUpdateRequested {
            request_id: 1,
            patch: SettingsPatch {
                search_crawler: Some(settings_no_media_captions()),
                ..Default::default()
            },
        },
    );

    // Completed → Idle; Running stays Running.
    assert_eq!(
        state.search_crawler.rooms.get("room-a"),
        Some(&SearchCrawlerRoomState::Idle)
    );
    assert_eq!(
        state.search_crawler.rooms.get("room-b"),
        Some(&SearchCrawlerRoomState::Running {
            processed: 3,
            indexed: 1
        })
    );

    let has_crawler_changed = effects
        .iter()
        .any(|e| matches!(e, AppEffect::EmitUiEvent(UiEvent::SearchCrawlerChanged)));
    assert!(has_crawler_changed);

    // Must also emit InvalidateSearchCrawlerCache so the actor drops its
    // completed-room set before the subsequent re-enqueue (P1 fix).
    let has_invalidate = effects
        .iter()
        .any(|e| matches!(e, AppEffect::InvalidateSearchCrawlerCache));
    assert!(
        has_invalidate,
        "expected InvalidateSearchCrawlerCache on content-setting toggle; got {effects:?}"
    );

    // Must emit NotifySearchCrawlerRoomsAvailable for re-crawl with new settings.
    let has_notify = effects
        .iter()
        .any(|e| matches!(e, AppEffect::NotifySearchCrawlerRoomsAvailable { room_ids, .. } if !room_ids.is_empty()));
    assert!(
        has_notify,
        "expected NotifySearchCrawlerRoomsAvailable on content-setting toggle; got {effects:?}"
    );
}

#[test]
fn toggle_include_filenames_resets_completed_to_idle() {
    let mut state = ready_state_with_rooms(&["room-a"]);
    state.search_crawler.rooms.insert(
        "room-a".to_owned(),
        SearchCrawlerRoomState::Completed { indexed: 7 },
    );

    let effects = reduce(
        &mut state,
        AppAction::SettingsUpdateRequested {
            request_id: 1,
            patch: SettingsPatch {
                search_crawler: Some(settings_no_filenames()),
                ..Default::default()
            },
        },
    );

    assert_eq!(
        state.search_crawler.rooms.get("room-a"),
        Some(&SearchCrawlerRoomState::Idle)
    );

    // Content-setting toggle must also emit cache invalidation + re-enqueue.
    let has_invalidate = effects
        .iter()
        .any(|e| matches!(e, AppEffect::InvalidateSearchCrawlerCache));
    assert!(
        has_invalidate,
        "expected InvalidateSearchCrawlerCache; got {effects:?}"
    );
    let has_notify = effects
        .iter()
        .any(|e| matches!(e, AppEffect::NotifySearchCrawlerRoomsAvailable { .. }));
    assert!(
        has_notify,
        "expected NotifySearchCrawlerRoomsAvailable; got {effects:?}"
    );
}

// ---------------------------------------------------------------------------
// Duplicate/stale completion handling
// ---------------------------------------------------------------------------

#[test]
fn duplicate_completion_for_already_completed_room_is_idempotent() {
    let mut state = ready_state_with_rooms(&["room-a"]);
    state.search_crawler.rooms.insert(
        "room-a".to_owned(),
        SearchCrawlerRoomState::Completed { indexed: 10 },
    );

    // A second CrawlCompleted for the same room (stale).
    let effects = reduce(
        &mut state,
        AppAction::HistoryCrawlCompleted {
            room_id: "room-a".to_owned(),
            indexed: 12,
            timestamp_ms: 5_000,
        },
    );

    // State is updated to the new indexed count (reducer accepts the update).
    assert_eq!(
        state.search_crawler.rooms.get("room-a"),
        Some(&SearchCrawlerRoomState::Completed { indexed: 12 })
    );
    assert_eq!(
        effects,
        vec![AppEffect::EmitUiEvent(UiEvent::SearchCrawlerChanged)]
    );
}

#[test]
fn stale_failed_for_already_completed_room_updates_state() {
    let mut state = ready_state_with_rooms(&["room-a"]);
    state.search_crawler.rooms.insert(
        "room-a".to_owned(),
        SearchCrawlerRoomState::Completed { indexed: 10 },
    );

    reduce(
        &mut state,
        AppAction::HistoryCrawlFailed {
            room_id: "room-a".to_owned(),
            kind: SearchCrawlerFailureKind::Sdk,
            timestamp_ms: 6_000,
        },
    );

    // The reducer accepts the transition (actor handles dedup via completed_rooms).
    assert_eq!(
        state.search_crawler.rooms.get("room-a"),
        Some(&SearchCrawlerRoomState::Failed {
            kind: SearchCrawlerFailureKind::Sdk
        })
    );
}

// ---------------------------------------------------------------------------
// Idempotent skip of already-Running/Completed rooms by the actor is a
// `search.rs` concern; here we verify the reducer accepts sequential
// updates without blowing up.
// ---------------------------------------------------------------------------

#[test]
fn sequential_crawl_lifecycle_idle_queued_running_completed() {
    let mut state = ready_state_with_rooms(&["room-a"]);

    reduce(
        &mut state,
        AppAction::HistoryCrawlStarted {
            request_id: 1,
            room_id: "room-a".to_owned(),
            timestamp_ms: 1_000,
        },
    );
    assert!(matches!(
        state.search_crawler.rooms.get("room-a"),
        Some(SearchCrawlerRoomState::Queued)
    ));

    reduce(
        &mut state,
        AppAction::HistoryCrawlProgress {
            room_id: "room-a".to_owned(),
            processed: 10,
            indexed: 8,
            timestamp_ms: 2_000,
        },
    );
    assert!(matches!(
        state.search_crawler.rooms.get("room-a"),
        Some(SearchCrawlerRoomState::Running {
            processed: 10,
            indexed: 8
        })
    ));

    reduce(
        &mut state,
        AppAction::HistoryCrawlCompleted {
            room_id: "room-a".to_owned(),
            indexed: 8,
            timestamp_ms: 3_000,
        },
    );
    assert_eq!(
        state.search_crawler.rooms.get("room-a"),
        Some(&SearchCrawlerRoomState::Completed { indexed: 8 })
    );
}

#[test]
fn crawl_lifecycle_tracks_last_active_room_without_debug_identifiers() {
    let mut state = ready_state_with_rooms(&["room-a", "room-b"]);

    reduce(
        &mut state,
        AppAction::HistoryCrawlProgress {
            room_id: "room-a".to_owned(),
            processed: 10,
            indexed: 8,
            timestamp_ms: 2_000,
        },
    );
    assert_eq!(
        state.search_crawler.last_active.as_ref().map(|last| (
            last.room_id.as_str(),
            last.updated_at_ms,
            last.status,
            last.processed,
            last.indexed
        )),
        Some((
            "room-a",
            2_000,
            SearchCrawlerLastActiveStatus::Running,
            10,
            8
        ))
    );

    reduce(
        &mut state,
        AppAction::HistoryCrawlCompleted {
            room_id: "room-b".to_owned(),
            indexed: 12,
            timestamp_ms: 3_000,
        },
    );

    let last_active = state
        .search_crawler
        .last_active
        .as_ref()
        .expect("crawler activity should be tracked");
    assert_eq!(last_active.room_id, "room-b");
    assert_eq!(last_active.updated_at_ms, 3_000);
    assert_eq!(last_active.status, SearchCrawlerLastActiveStatus::Completed);
    assert_eq!(last_active.indexed, 12);

    let debug_output = format!("{:?}", state.search_crawler);
    assert!(
        !debug_output.contains("room-b"),
        "last active room id must not appear in SearchCrawlerState Debug"
    );
}

// ---------------------------------------------------------------------------
// Default / initial state
// ---------------------------------------------------------------------------

#[test]
fn fresh_state_has_empty_crawler_rooms() {
    let state = AppState::default();
    assert!(state.search_crawler.rooms.is_empty());
    assert_eq!(
        state.settings.values.search_crawler.speed,
        SearchCrawlerSpeed::Standard
    );
    assert!(state.settings.values.search_crawler.include_media_captions);
    assert!(state.settings.values.search_crawler.include_filenames);
}

// ---------------------------------------------------------------------------
// P1-A: Running room must be re-crawled after content-setting toggle
// ---------------------------------------------------------------------------

/// When `include_media_captions` or `include_filenames` is toggled while a
/// room's crawl is RUNNING, the reducer leaves the room in `Running` state
/// (it does not reset it to Idle — that would interrupt the current crawl)
/// and emits `InvalidateSearchCrawlerCache` + `NotifySearchCrawlerRoomsAvailable`.
///
/// The actor side uses the generation counter to reject `CrawlFinished` from
/// the stale running crawl, so the room is never added to `completed_rooms`
/// at the old settings; the follow-up `RoomsAvailable` then re-crawls it.
#[test]
fn running_room_is_left_running_and_recrawl_effects_are_emitted_on_content_toggle() {
    let mut state = ready_state_with_rooms(&["room-a", "room-b"]);
    // room-a is Running (in-progress crawl).
    state.search_crawler.rooms.insert(
        "room-a".to_owned(),
        SearchCrawlerRoomState::Running {
            processed: 50,
            indexed: 30,
        },
    );
    // room-b is Completed under the old settings.
    state.search_crawler.rooms.insert(
        "room-b".to_owned(),
        SearchCrawlerRoomState::Completed { indexed: 100 },
    );

    let effects = reduce(
        &mut state,
        AppAction::SettingsUpdateRequested {
            request_id: 1,
            patch: SettingsPatch {
                search_crawler: Some(settings_no_media_captions()),
                ..Default::default()
            },
        },
    );

    // The Running room must stay Running (reducer does not interrupt it).
    assert_eq!(
        state.search_crawler.rooms.get("room-a"),
        Some(&SearchCrawlerRoomState::Running {
            processed: 50,
            indexed: 30
        }),
        "Running room must not be reset by the reducer on content-setting toggle"
    );
    // The Completed room must be reset to Idle so the next RoomsAvailable re-crawls it.
    assert_eq!(
        state.search_crawler.rooms.get("room-b"),
        Some(&SearchCrawlerRoomState::Idle),
        "Completed room must be reset to Idle on content-setting toggle"
    );

    // The actor uses the generation bump (InvalidateSearchCrawlerCache) to
    // reject the stale running crawl's CrawlFinished, preventing the Running
    // room from being silently recorded as Completed at the old settings.
    let has_invalidate = effects
        .iter()
        .any(|e| matches!(e, AppEffect::InvalidateSearchCrawlerCache));
    assert!(
        has_invalidate,
        "InvalidateSearchCrawlerCache must be emitted so the actor bumps its generation \
         and rejects the stale running crawl's CrawlFinished; got {effects:?}"
    );

    // RoomsAvailable must be emitted so both rooms can be re-crawled
    // (room-a after its stale crawl finishes, room-b immediately).
    let has_notify = effects.iter().any(|e| {
        matches!(e, AppEffect::NotifySearchCrawlerRoomsAvailable { room_ids, .. }
            if !room_ids.is_empty())
    });
    assert!(
        has_notify,
        "NotifySearchCrawlerRoomsAvailable must be emitted for re-crawl after \
         content-setting toggle; got {effects:?}"
    );
}

// ---------------------------------------------------------------------------
// Explicit search database rebuild
// ---------------------------------------------------------------------------

#[test]
fn rebuild_search_index_resets_crawler_state_closes_search_and_reenqueues_when_active() {
    let mut state = ready_state_with_rooms(&["room-a", "room-b"]);
    state.search = SearchState::Editing {
        query: "stale query".to_owned(),
        scope: SearchScope::AllRooms,
    };
    state.search_crawler.rooms.insert(
        "room-a".to_owned(),
        SearchCrawlerRoomState::Running {
            processed: 50,
            indexed: 30,
        },
    );
    state.search_crawler.rooms.insert(
        "room-b".to_owned(),
        SearchCrawlerRoomState::Completed { indexed: 20 },
    );

    let effects = reduce(
        &mut state,
        AppAction::SearchIndexRebuildRequested { request_id: 77 },
    );

    assert_eq!(state.search, SearchState::Closed);
    assert_eq!(
        state.search_crawler.rooms.get("room-a"),
        Some(&SearchCrawlerRoomState::Idle)
    );
    assert_eq!(
        state.search_crawler.rooms.get("room-b"),
        Some(&SearchCrawlerRoomState::Idle)
    );
    assert!(
        effects
            .iter()
            .any(|effect| matches!(effect, AppEffect::RebuildSearchIndex)),
        "rebuild must clear the SearchActor document store; got {effects:?}"
    );
    assert!(
        effects
            .iter()
            .any(|effect| matches!(effect, AppEffect::EmitUiEvent(UiEvent::SearchChanged))),
        "rebuild must close stale search results; got {effects:?}"
    );
    assert!(
        effects.iter().any(|effect| {
            matches!(effect, AppEffect::NotifySearchCrawlerRoomsAvailable { room_ids, .. }
                if room_ids.len() == 2)
        }),
        "active crawler should be re-enqueued for all known rooms; got {effects:?}"
    );
}

#[test]
fn rebuild_search_index_does_not_restart_crawler_while_paused() {
    let mut state = ready_state_with_rooms(&["room-a", "room-b"]);
    state.settings.values.search_crawler = settings_paused();

    let effects = reduce(
        &mut state,
        AppAction::SearchIndexRebuildRequested { request_id: 78 },
    );

    assert!(
        effects
            .iter()
            .any(|effect| matches!(effect, AppEffect::RebuildSearchIndex)),
        "rebuild must still clear the local search index while paused; got {effects:?}"
    );
    assert!(
        !effects
            .iter()
            .any(|effect| matches!(effect, AppEffect::NotifySearchCrawlerRoomsAvailable { .. })),
        "paused crawler must not be restarted by rebuild; got {effects:?}"
    );
}

// ---------------------------------------------------------------------------
// SearchCrawlerState serializes without private data
// ---------------------------------------------------------------------------

#[test]
fn crawler_state_debug_output_does_not_contain_room_ids_or_sdk_errors() {
    // P11 fix: SearchCrawlerState has a custom Debug that emits only counts
    // and coarse states — room ids (Matrix identifiers) must not appear.
    let mut crawler = SearchCrawlerState::default();
    crawler.rooms.insert(
        "!secret-room:example.invalid".to_owned(),
        SearchCrawlerRoomState::Running {
            processed: 10,
            indexed: 5,
        },
    );
    crawler.rooms.insert(
        "!another-room:example.invalid".to_owned(),
        SearchCrawlerRoomState::Failed {
            kind: SearchCrawlerFailureKind::Sdk,
        },
    );
    crawler.rooms.insert(
        "!third-room:example.invalid".to_owned(),
        SearchCrawlerRoomState::Completed { indexed: 20 },
    );

    let debug = format!("{crawler:?}");
    // Room ids must NOT appear.
    assert!(
        !debug.contains("secret-room"),
        "room id leaked into crawler Debug: {debug}"
    );
    assert!(
        !debug.contains("another-room"),
        "room id leaked into crawler Debug: {debug}"
    );
    assert!(
        !debug.contains("third-room"),
        "room id leaked into crawler Debug: {debug}"
    );
    // Coarse counts MUST appear so the debug output is still useful.
    assert!(
        debug.contains("running"),
        "running count absent from Debug: {debug}"
    );
    assert!(
        debug.contains("failed"),
        "failed count absent from Debug: {debug}"
    );
    assert!(
        debug.contains("completed"),
        "completed count absent from Debug: {debug}"
    );
    // No raw SDK error strings.
    assert!(
        !debug.contains("error:"),
        "raw SDK error in crawler Debug: {debug}"
    );
    assert!(
        !debug.contains("@"),
        "Matrix user id in crawler Debug: {debug}"
    );
    assert!(
        !debug.contains("$"),
        "Matrix event id in crawler Debug: {debug}"
    );
}
