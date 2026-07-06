use crate::{
    effect::{AppEffect, UiEvent},
    state::{
        AppError, AppState, RoomNotificationMode, RoomNotificationModeOperation,
        RoomNotificationSettings, RoomPreference, RoomPreferencesState, RoomUrlPreviews,
        SettingsPersistenceState, sort_threads_list_items,
    },
};

use super::{is_session_ready, recompute_room_list_projection};

const SETTINGS_LOAD_FAILED_MESSAGE: &str = "Settings could not be loaded";
const SETTINGS_PERSIST_FAILED_MESSAGE: &str = "Settings could not be saved";

pub(crate) fn handle_settings_loaded(
    state: &mut AppState,
    values: crate::state::SettingsValues,
) -> Vec<AppEffect> {
    state.settings.values = values;
    state.settings.persistence = SettingsPersistenceState::Idle;
    recompute_room_list_projection(state);
    let mut effects = vec![
        AppEffect::EmitUiEvent(UiEvent::SettingsChanged),
        AppEffect::EmitUiEvent(UiEvent::RoomListChanged),
    ];
    if let crate::state::ThreadsListState::Open { items, .. } = &mut state.threads_list {
        sort_threads_list_items(items, state.settings.values.thread_list_order);
        effects.push(AppEffect::EmitUiEvent(UiEvent::ThreadsListChanged));
    }
    effects
}

pub(crate) fn handle_settings_load_failed(
    state: &mut AppState,
    _message: String,
) -> Vec<AppEffect> {
    state.settings.persistence = SettingsPersistenceState::Idle;
    state.errors.push(AppError {
        code: "settings_load_failed".to_owned(),
        message: SETTINGS_LOAD_FAILED_MESSAGE.to_owned(),
        recoverable: true,
    });
    vec![
        AppEffect::EmitUiEvent(UiEvent::SettingsChanged),
        AppEffect::EmitUiEvent(UiEvent::ErrorChanged),
    ]
}

pub(crate) fn handle_settings_update_requested(
    state: &mut AppState,
    request_id: u64,
    patch: crate::state::SettingsPatch,
) -> Vec<AppEffect> {
    // Capture the previous search-crawler settings so we can compute
    // invalidation/enqueue transitions after apply_patch.
    let prev_crawler = state.settings.values.search_crawler.clone();
    let prev_room_list_sort = state.settings.values.room_list_sort;
    let prev_thread_list_order = state.settings.values.thread_list_order;

    state.settings.values.apply_patch(patch);
    state.settings.persistence = SettingsPersistenceState::Saving { request_id };

    let new_crawler = state.settings.values.search_crawler.clone();
    let mut effects = vec![
        AppEffect::PersistSettings {
            request_id,
            values: state.settings.values.clone(),
        },
        AppEffect::EmitUiEvent(UiEvent::SettingsChanged),
    ];

    if state.settings.values.room_list_sort != prev_room_list_sort {
        recompute_room_list_projection(state);
        effects.push(AppEffect::EmitUiEvent(UiEvent::RoomListChanged));
    }
    if state.settings.values.thread_list_order != prev_thread_list_order {
        if let crate::state::ThreadsListState::Open { items, .. } = &mut state.threads_list {
            sort_threads_list_items(items, state.settings.values.thread_list_order);
            effects.push(AppEffect::EmitUiEvent(UiEvent::ThreadsListChanged));
        }
    }

    let mut emit_search_crawler_changed = false;

    // Guard: if content-indexing settings changed, invalidate the
    // reducer state AND the actor's completed-room cache so rooms are
    // re-crawled with the new settings.  Stale captions/filenames must
    // not stay searchable after the user opts out (privacy rule).
    let content_changed = prev_crawler.include_media_captions != new_crawler.include_media_captions
        || prev_crawler.include_filenames != new_crawler.include_filenames;
    if content_changed {
        for room_state in state.search_crawler.rooms.values_mut() {
            if matches!(
                room_state,
                crate::state::SearchCrawlerRoomState::Completed { .. }
            ) {
                *room_state = crate::state::SearchCrawlerRoomState::Idle;
            }
        }
        emit_search_crawler_changed = true;
        // Tell the actor to drop its completed-room cache so the
        // following re-enqueue actually starts new crawls.
        effects.push(AppEffect::InvalidateSearchCrawlerCache);
        // Re-enqueue all currently-known joined rooms so the actor
        // starts fresh crawls with the new content settings.
        let room_ids: Vec<String> = state.rooms.iter().map(|r| r.room_id.clone()).collect();
        if !room_ids.is_empty() {
            effects.push(AppEffect::NotifySearchCrawlerRoomsAvailable {
                room_ids,
                settings: new_crawler.clone(),
            });
        }
    }

    // Guard: if speed changed from Paused to active, enqueue all
    // known joined rooms via the SearchActor.
    use crate::state::SearchCrawlerSpeed;
    if prev_crawler.speed == SearchCrawlerSpeed::Paused
        && new_crawler.speed != SearchCrawlerSpeed::Paused
    {
        let room_ids: Vec<String> = state.rooms.iter().map(|r| r.room_id.clone()).collect();
        if !room_ids.is_empty() {
            effects.push(AppEffect::NotifySearchCrawlerRoomsAvailable {
                room_ids,
                settings: new_crawler.clone(),
            });
        }
    }
    if prev_crawler.speed != SearchCrawlerSpeed::Paused
        && new_crawler.speed == SearchCrawlerSpeed::Paused
    {
        let room_ids: Vec<String> = state.rooms.iter().map(|r| r.room_id.clone()).collect();
        effects.push(AppEffect::NotifySearchCrawlerRoomsAvailable {
            room_ids,
            settings: new_crawler.clone(),
        });
    }

    if emit_search_crawler_changed {
        effects.push(AppEffect::EmitUiEvent(UiEvent::SearchCrawlerChanged));
    }

    effects
}

pub(crate) fn handle_settings_persisted(state: &mut AppState, request_id: u64) -> Vec<AppEffect> {
    if state.settings.persistence != (SettingsPersistenceState::Saving { request_id }) {
        return Vec::new();
    }

    state.settings.persistence = SettingsPersistenceState::Idle;
    vec![AppEffect::EmitUiEvent(UiEvent::SettingsChanged)]
}

pub(crate) fn handle_settings_persist_failed(
    state: &mut AppState,
    request_id: u64,
    _message: String,
) -> Vec<AppEffect> {
    if state.settings.persistence != (SettingsPersistenceState::Saving { request_id }) {
        return Vec::new();
    }

    state.settings.persistence = SettingsPersistenceState::Idle;
    state.errors.push(AppError {
        code: "settings_persist_failed".to_owned(),
        message: SETTINGS_PERSIST_FAILED_MESSAGE.to_owned(),
        recoverable: true,
    });
    vec![
        AppEffect::EmitUiEvent(UiEvent::SettingsChanged),
        AppEffect::EmitUiEvent(UiEvent::ErrorChanged),
    ]
}

pub(crate) fn handle_room_url_preview_override_set(
    state: &mut AppState,
    request_id: u64,
    room_id: String,
    enabled: bool,
) -> Vec<AppEffect> {
    let Some(room) = state.rooms.iter().find(|room| room.room_id == room_id) else {
        return Vec::new();
    };
    let default_enabled = if room.is_encrypted {
        state.settings.values.display.encrypted_url_previews_enabled
    } else {
        state.settings.values.display.url_previews_enabled
    };
    if enabled == default_enabled {
        state.link_preview_settings.room_overrides.remove(&room_id);
        clear_room_preference_field(state, &room_id, |preference| {
            preference.url_previews_enabled_override = None;
        });
    } else {
        state
            .link_preview_settings
            .room_overrides
            .insert(room_id.clone(), enabled);
        let preference = state.room_preferences.rooms.entry(room_id).or_default();
        preference.url_previews_enabled_override = Some(enabled);
    }
    vec![
        AppEffect::PersistRoomPreferences {
            request_id,
            preferences: state.room_preferences.clone(),
        },
        AppEffect::EmitUiEvent(UiEvent::LinkPreviewSettingsChanged),
    ]
}

pub(crate) fn handle_room_preferences_loaded(
    state: &mut AppState,
    preferences: RoomPreferencesState,
) -> Vec<AppEffect> {
    let previous_link_preview_settings = state.link_preview_settings.room_overrides.clone();
    let previous_room_notification_settings = state.room_notification_settings.clone();

    state.room_preferences = preferences;
    state.link_preview_settings.room_overrides =
        room_url_preview_overrides_from_preferences(&state.room_preferences);
    state.room_notification_settings =
        room_notification_settings_from_preferences(&state.room_preferences);

    let mut effects = Vec::new();
    if state.link_preview_settings.room_overrides != previous_link_preview_settings {
        effects.push(AppEffect::EmitUiEvent(UiEvent::LinkPreviewSettingsChanged));
    }
    if state.room_notification_settings != previous_room_notification_settings {
        effects.push(AppEffect::EmitUiEvent(
            UiEvent::RoomNotificationSettingsChanged,
        ));
    }
    effects
}

pub(crate) fn handle_room_notification_mode_set(
    state: &mut AppState,
    request_id: u64,
    room_id: String,
    mode: RoomNotificationMode,
) -> Vec<AppEffect> {
    if !is_session_ready(state) {
        return Vec::new();
    }
    let known = state.rooms.iter().any(|r| r.room_id == room_id)
        || state.invites.iter().any(|i| i.room_id == room_id);
    if !known {
        return Vec::new();
    }
    let entry = state.room_notification_settings.entry(room_id).or_default();
    entry.mode = mode;
    entry.operation = RoomNotificationModeOperation::Pending { request_id };
    vec![AppEffect::EmitUiEvent(
        UiEvent::RoomNotificationSettingsChanged,
    )]
}

fn update_room_notification_preference(
    state: &mut AppState,
    room_id: String,
    mode: RoomNotificationMode,
) {
    if mode == RoomNotificationMode::All {
        clear_room_preference_field(state, &room_id, |preference| {
            preference.notification_mode = None;
        });
    } else {
        let preference = state.room_preferences.rooms.entry(room_id).or_default();
        preference.notification_mode = Some(mode);
    }
}

pub(crate) fn handle_room_notification_mode_completed(
    state: &mut AppState,
    request_id: u64,
    room_id: String,
) -> Vec<AppEffect> {
    if !is_session_ready(state) {
        return Vec::new();
    }
    let mut completed_mode = None;
    if let Some(entry) = state.room_notification_settings.get_mut(&room_id) {
        if matches!(
            entry.operation,
            RoomNotificationModeOperation::Pending {
                request_id: pending_id,
            } if pending_id == request_id
        ) {
            entry.operation = RoomNotificationModeOperation::Idle;
            completed_mode = Some(entry.mode);
        }
    }
    let mut effects = Vec::new();
    if let Some(mode) = completed_mode {
        update_room_notification_preference(state, room_id, mode);
        effects.push(AppEffect::PersistRoomPreferences {
            request_id,
            preferences: state.room_preferences.clone(),
        });
    }
    effects.push(AppEffect::EmitUiEvent(
        UiEvent::RoomNotificationSettingsChanged,
    ));
    effects
}

fn clear_room_preference_field(
    state: &mut AppState,
    room_id: &str,
    clear: impl FnOnce(&mut RoomPreference),
) {
    let Some(preference) = state.room_preferences.rooms.get_mut(room_id) else {
        return;
    };
    clear(preference);
    if preference.is_empty() {
        state.room_preferences.rooms.remove(room_id);
    }
}

fn room_url_preview_overrides_from_preferences(
    preferences: &RoomPreferencesState,
) -> RoomUrlPreviews {
    preferences
        .rooms
        .iter()
        .filter_map(|(room_id, preference)| {
            preference
                .url_previews_enabled_override
                .map(|enabled| (room_id.clone(), enabled))
        })
        .collect()
}

fn room_notification_settings_from_preferences(
    preferences: &RoomPreferencesState,
) -> std::collections::HashMap<String, RoomNotificationSettings> {
    preferences
        .rooms
        .iter()
        .filter_map(|(room_id, preference)| {
            preference.notification_mode.map(|mode| {
                (
                    room_id.clone(),
                    RoomNotificationSettings {
                        mode,
                        operation: RoomNotificationModeOperation::Idle,
                    },
                )
            })
        })
        .collect()
}

pub(crate) fn handle_room_notification_mode_failed(
    state: &mut AppState,
    request_id: u64,
    room_id: String,
    kind: crate::state::OperationFailureKind,
) -> Vec<AppEffect> {
    if !is_session_ready(state) {
        return Vec::new();
    }
    if let Some(entry) = state.room_notification_settings.get_mut(&room_id) {
        if matches!(
            entry.operation,
            crate::state::RoomNotificationModeOperation::Pending {
                request_id: pending_id,
            } if pending_id == request_id
        ) {
            entry.operation = crate::state::RoomNotificationModeOperation::Failed {
                request_id,
                failure_kind: kind,
            };
        }
    }
    vec![AppEffect::EmitUiEvent(
        UiEvent::RoomNotificationSettingsChanged,
    )]
}
