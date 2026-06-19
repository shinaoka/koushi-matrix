use crate::{
    effect::{AppEffect, UiEvent},
    state::{AppError, AppState, RoomListFilter, SettingsPersistenceState, compute_room_list_projection},
};

use super::is_session_ready;

const SETTINGS_LOAD_FAILED_MESSAGE: &str = "Settings could not be loaded";
const SETTINGS_PERSIST_FAILED_MESSAGE: &str = "Settings could not be saved";

pub(crate) fn handle_settings_loaded(
    state: &mut AppState,
    values: crate::state::SettingsValues,
) -> Vec<AppEffect> {
    state.settings.values = values;
    state.settings.persistence = SettingsPersistenceState::Idle;
    vec![AppEffect::EmitUiEvent(UiEvent::SettingsChanged)]
}

pub(crate) fn handle_settings_load_failed(state: &mut AppState, _message: String) -> Vec<AppEffect> {
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

    state.settings.values.apply_patch(patch);
    state.settings.persistence = SettingsPersistenceState::Saving { request_id };

    let new_crawler = &state.settings.values.search_crawler;
    let mut effects = vec![
        AppEffect::PersistSettings {
            request_id,
            values: state.settings.values.clone(),
        },
        AppEffect::EmitUiEvent(UiEvent::SettingsChanged),
    ];

    // Guard: if content-indexing settings changed, invalidate the
    // reducer state AND the actor's completed-room cache so rooms are
    // re-crawled with the new settings.  Stale captions/filenames must
    // not stay searchable after the user opts out (privacy rule).
    let content_changed = prev_crawler.include_media_captions
        != new_crawler.include_media_captions
        || prev_crawler.include_filenames != new_crawler.include_filenames;
    if content_changed {
        for room_state in state.search_crawler.rooms.values_mut() {
            if matches!(room_state, crate::state::SearchCrawlerRoomState::Completed { .. }) {
                *room_state = crate::state::SearchCrawlerRoomState::Idle;
            }
        }
        effects.push(AppEffect::EmitUiEvent(UiEvent::SearchCrawlerChanged));
        // Tell the actor to drop its completed-room cache so the
        // following re-enqueue actually starts new crawls.
        effects.push(AppEffect::InvalidateSearchCrawlerCache);
        // Re-enqueue all currently-known joined rooms so the actor
        // starts fresh crawls with the new content settings.
        let room_ids: Vec<String> =
            state.rooms.iter().map(|r| r.room_id.clone()).collect();
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

    effects
}

pub(crate) fn handle_settings_persisted(
    state: &mut AppState,
    request_id: u64,
) -> Vec<AppEffect> {
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
    _request_id: u64,
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
    } else {
        state
            .link_preview_settings
            .room_overrides
            .insert(room_id, enabled);
    }
    vec![AppEffect::EmitUiEvent(UiEvent::LinkPreviewSettingsChanged)]
}

pub(crate) fn handle_room_notification_mode_set(
    state: &mut AppState,
    request_id: u64,
    room_id: String,
    mode: crate::state::RoomNotificationMode,
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
    entry.operation = crate::state::RoomNotificationModeOperation::Pending { request_id };
    vec![AppEffect::EmitUiEvent(
        UiEvent::RoomNotificationSettingsChanged,
    )]
}

pub(crate) fn handle_room_notification_mode_completed(
    state: &mut AppState,
    request_id: u64,
    room_id: String,
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
            entry.operation = crate::state::RoomNotificationModeOperation::Idle;
        }
    }
    vec![AppEffect::EmitUiEvent(
        UiEvent::RoomNotificationSettingsChanged,
    )]
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
