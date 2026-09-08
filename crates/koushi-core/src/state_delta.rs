//! Incremental AppState slice deltas.

use koushi_protocol::state_update::{StateDelta, StateDeltaChangedSlices};
use koushi_state::{ActivityRow, ActivityState, AppState, compose_sidebar_for_state};
use std::collections::{BTreeMap, BTreeSet};

pub fn build_state_delta(
    generation: u64,
    previous: &AppState,
    next: &AppState,
) -> Option<StateDelta> {
    audit_app_state_delta_slices(previous);
    audit_app_state_delta_slices(next);

    let mut changed = StateDeltaChangedSlices::default();

    macro_rules! changed_slice {
        ($field:ident) => {
            if previous.$field != next.$field {
                changed.$field = Some(next.$field.clone());
            }
        };
    }

    changed_slice!(session);
    changed_slice!(session_lock_reason);
    changed_slice!(secure_backup_gate);
    changed_slice!(device_cleanup);
    changed_slice!(current_session_status);
    changed_slice!(auth);
    changed_slice!(account_management_url);
    changed_slice!(account_management);
    changed_slice!(account_management_capabilities);
    changed_slice!(soft_logout_reauth);
    changed_slice!(qr_login);
    changed_slice!(settings);
    changed_slice!(link_preview_settings);
    changed_slice!(room_preferences);
    if previous.profile != next.profile {
        if previous.profile.own != next.profile.own {
            changed.profile_own = Some(next.profile.own.clone());
        }
        if previous.profile.users != next.profile.users {
            let mut user_changes = BTreeMap::new();
            for (user_id, user) in &next.profile.users {
                if previous.profile.users.get(user_id) != Some(user) {
                    user_changes.insert(user_id.clone(), Some(user.clone()));
                }
            }
            for user_id in previous.profile.users.keys() {
                if !next.profile.users.contains_key(user_id) {
                    user_changes.insert(user_id.clone(), None);
                }
            }
            changed.profile_users_by_id = (!user_changes.is_empty()).then_some(user_changes);
        }
        if previous.profile.room_users != next.profile.room_users {
            let mut room_changes = BTreeMap::new();
            for (room_id, users) in &next.profile.room_users {
                let previous_users = previous.profile.room_users.get(room_id);
                let mut user_changes = BTreeMap::new();
                for (user_id, user) in users {
                    if previous_users.and_then(|users| users.get(user_id)) != Some(user) {
                        user_changes.insert(user_id.clone(), Some(user.clone()));
                    }
                }
                if let Some(previous_users) = previous_users {
                    for user_id in previous_users.keys() {
                        if !users.contains_key(user_id) {
                            user_changes.insert(user_id.clone(), None);
                        }
                    }
                }
                if !user_changes.is_empty() {
                    room_changes.insert(room_id.clone(), Some(user_changes));
                }
            }
            for room_id in previous.profile.room_users.keys() {
                if !next.profile.room_users.contains_key(room_id) {
                    room_changes.insert(room_id.clone(), None);
                }
            }
            changed.profile_room_users_by_room = (!room_changes.is_empty()).then_some(room_changes);
        }
        if previous.profile.local_aliases != next.profile.local_aliases {
            let user_ids = previous
                .profile
                .local_aliases
                .keys()
                .chain(next.profile.local_aliases.keys())
                .cloned()
                .collect::<BTreeSet<_>>();
            let mut alias_changes = BTreeMap::new();
            for user_id in user_ids {
                if previous.profile.local_aliases.get(&user_id)
                    != next.profile.local_aliases.get(&user_id)
                {
                    alias_changes.insert(
                        user_id.clone(),
                        next.profile.local_aliases.get(&user_id).cloned(),
                    );
                }
            }
            changed.profile_local_aliases_by_id =
                (!alias_changes.is_empty()).then_some(alias_changes);
        }
        if previous.profile.ignored_user_ids != next.profile.ignored_user_ids {
            let user_ids = previous
                .profile
                .ignored_user_ids
                .iter()
                .chain(next.profile.ignored_user_ids.iter())
                .cloned()
                .collect::<BTreeSet<_>>();
            let mut ignored_changes = BTreeMap::new();
            for user_id in user_ids {
                let was_ignored = previous.profile.ignored_user_ids.contains(&user_id);
                let is_ignored = next.profile.ignored_user_ids.contains(&user_id);
                if was_ignored != is_ignored {
                    ignored_changes.insert(user_id, is_ignored);
                }
            }
            changed.profile_ignored_user_ids_by_id =
                (!ignored_changes.is_empty()).then_some(ignored_changes);
        }
        if previous.profile.local_alias_update != next.profile.local_alias_update {
            changed.profile_local_alias_update = Some(next.profile.local_alias_update.clone());
        }
        if previous.profile.ignored_user_update != next.profile.ignored_user_update {
            changed.profile_ignored_user_update = Some(next.profile.ignored_user_update.clone());
        }
        if previous.profile.update != next.profile.update {
            changed.profile_update = Some(next.profile.update.clone());
        }
    }
    changed_slice!(space_members);
    changed_slice!(sync);
    changed_slice!(navigation);
    if previous.spaces != next.spaces {
        let order_is_stable = ordered_ids_are_subsequence(
            previous.spaces.iter().map(|space| &space.space_id),
            next.spaces.iter().map(|space| &space.space_id),
        );
        if order_is_stable {
            let mut space_changes = BTreeMap::new();
            for space in &next.spaces {
                if previous
                    .spaces
                    .iter()
                    .find(|old| old.space_id == space.space_id)
                    != Some(space)
                {
                    space_changes.insert(space.space_id.clone(), Some(space.clone()));
                }
            }
            for space in &previous.spaces {
                if !next
                    .spaces
                    .iter()
                    .any(|current| current.space_id == space.space_id)
                {
                    space_changes.insert(space.space_id.clone(), None);
                }
            }
            changed.spaces_by_id = (!space_changes.is_empty()).then_some(space_changes);
        } else {
            changed.spaces = Some(next.spaces.clone());
        }
    }
    if previous.rooms != next.rooms {
        let order_is_stable = ordered_ids_are_subsequence(
            previous.rooms.iter().map(|room| &room.room_id),
            next.rooms.iter().map(|room| &room.room_id),
        );
        if order_is_stable {
            let mut room_changes = BTreeMap::new();
            for room in &next.rooms {
                if previous
                    .rooms
                    .iter()
                    .find(|old| old.room_id == room.room_id)
                    != Some(room)
                {
                    room_changes.insert(room.room_id.clone(), Some(room.clone()));
                }
            }
            for room in &previous.rooms {
                if !next
                    .rooms
                    .iter()
                    .any(|current| current.room_id == room.room_id)
                {
                    room_changes.insert(room.room_id.clone(), None);
                }
            }
            changed.rooms_by_id = (!room_changes.is_empty()).then_some(room_changes);
        } else {
            changed.rooms = Some(next.rooms.clone());
        }
    }
    if previous.invites != next.invites {
        let order_is_stable = ordered_ids_are_subsequence(
            previous.invites.iter().map(|invite| &invite.room_id),
            next.invites.iter().map(|invite| &invite.room_id),
        );
        if order_is_stable {
            let mut invite_changes = BTreeMap::new();
            for invite in &next.invites {
                if previous
                    .invites
                    .iter()
                    .find(|old| old.room_id == invite.room_id)
                    != Some(invite)
                {
                    invite_changes.insert(invite.room_id.clone(), Some(invite.clone()));
                }
            }
            for invite in &previous.invites {
                if !next
                    .invites
                    .iter()
                    .any(|current| current.room_id == invite.room_id)
                {
                    invite_changes.insert(invite.room_id.clone(), None);
                }
            }
            changed.invites_by_id = (!invite_changes.is_empty()).then_some(invite_changes);
        } else {
            changed.invites = Some(next.invites.clone());
        }
    }
    changed_slice!(invite_workflow);
    changed_slice!(room_list);
    if previous.room_notification_settings != next.room_notification_settings {
        let room_ids = previous
            .room_notification_settings
            .keys()
            .chain(next.room_notification_settings.keys())
            .cloned()
            .collect::<std::collections::BTreeSet<_>>();
        let mut changes = BTreeMap::new();
        for room_id in room_ids {
            if previous.room_notification_settings.get(&room_id)
                != next.room_notification_settings.get(&room_id)
            {
                changes.insert(
                    room_id.clone(),
                    next.room_notification_settings.get(&room_id).cloned(),
                );
            }
        }
        changed.room_notification_settings_by_id = (!changes.is_empty()).then_some(changes);
    }
    if previous.room_interactions != next.room_interactions {
        let room_ids = previous
            .room_interactions
            .keys()
            .chain(next.room_interactions.keys())
            .cloned()
            .collect::<std::collections::BTreeSet<_>>();
        let mut changes = BTreeMap::new();
        for room_id in room_ids {
            if previous.room_interactions.get(&room_id) != next.room_interactions.get(&room_id) {
                changes.insert(
                    room_id.clone(),
                    next.room_interactions.get(&room_id).cloned(),
                );
            }
        }
        changed.room_interactions_by_id = (!changes.is_empty()).then_some(changes);
    }
    changed_slice!(directory);
    changed_slice!(room_management);
    changed_slice!(mention_candidates);
    if previous.activity != next.activity {
        match activity_row_deltas(&previous.activity, &next.activity) {
            Some((recent, unread)) if recent.is_some() || unread.is_some() => {
                changed.activity_recent_rows_by_id = recent;
                changed.activity_unread_rows_by_id = unread;
            }
            Some(_) => {}
            None => changed.activity = Some(next.activity.clone()),
        }
    }
    changed_slice!(timeline);
    changed_slice!(thread);
    changed_slice!(thread_attention);
    changed_slice!(threads_list);
    changed_slice!(focused_context);
    changed_slice!(search);
    if previous.search_crawler.last_active != next.search_crawler.last_active {
        changed.search_crawler_last_active = Some(next.search_crawler.last_active.clone());
    }
    if previous.search_crawler.rooms != next.search_crawler.rooms {
        let room_ids = previous
            .search_crawler
            .rooms
            .keys()
            .chain(next.search_crawler.rooms.keys())
            .cloned()
            .collect::<std::collections::BTreeSet<_>>();
        let mut changes = BTreeMap::new();
        for room_id in room_ids {
            if previous.search_crawler.rooms.get(&room_id)
                != next.search_crawler.rooms.get(&room_id)
            {
                changes.insert(
                    room_id.clone(),
                    next.search_crawler.rooms.get(&room_id).cloned(),
                );
            }
        }
        changed.search_crawler_rooms_by_id = (!changes.is_empty()).then_some(changes);
    }
    changed_slice!(files_view);
    changed_slice!(basic_operation);
    if previous.live_signals.presence != next.live_signals.presence {
        let user_ids = previous
            .live_signals
            .presence
            .keys()
            .chain(next.live_signals.presence.keys())
            .cloned()
            .collect::<BTreeSet<_>>();
        let mut changes = BTreeMap::new();
        for user_id in user_ids {
            if previous.live_signals.presence.get(&user_id)
                != next.live_signals.presence.get(&user_id)
            {
                changes.insert(
                    user_id.clone(),
                    next.live_signals.presence.get(&user_id).copied(),
                );
            }
        }
        changed.live_signals_presence_by_user = (!changes.is_empty()).then_some(changes);
    }
    if previous.live_signals.rooms != next.live_signals.rooms {
        let room_ids = previous
            .live_signals
            .rooms
            .keys()
            .chain(next.live_signals.rooms.keys())
            .cloned()
            .collect::<BTreeSet<_>>();
        let mut room_changes = BTreeMap::new();
        let mut receipt_changes = BTreeMap::new();
        let mut metadata_changes = BTreeMap::new();
        for room_id in room_ids {
            match (
                previous.live_signals.rooms.get(&room_id),
                next.live_signals.rooms.get(&room_id),
            ) {
                (None, Some(room)) => {
                    room_changes.insert(room_id, Some(room.clone()));
                }
                (Some(_), None) => {
                    room_changes.insert(room_id, None);
                }
                (Some(previous), Some(next)) => {
                    if previous.fully_read_event_id != next.fully_read_event_id
                        || previous.typing_user_ids != next.typing_user_ids
                        || previous.typing_users != next.typing_users
                    {
                        metadata_changes.insert(
                            room_id.clone(),
                            Some(koushi_protocol::RoomLiveSignalMetadata {
                                fully_read_event_id: next.fully_read_event_id.clone(),
                                typing_user_ids: next.typing_user_ids.clone(),
                                typing_users: next.typing_users.clone(),
                            }),
                        );
                    }
                    if previous.receipts_by_event != next.receipts_by_event {
                        let event_ids = previous
                            .receipts_by_event
                            .keys()
                            .chain(next.receipts_by_event.keys())
                            .cloned()
                            .collect::<BTreeSet<_>>();
                        let mut events = BTreeMap::new();
                        for event_id in event_ids {
                            if previous.receipts_by_event.get(&event_id)
                                != next.receipts_by_event.get(&event_id)
                            {
                                events.insert(
                                    event_id.clone(),
                                    next.receipts_by_event.get(&event_id).cloned(),
                                );
                            }
                        }
                        if !events.is_empty() {
                            receipt_changes.insert(room_id, events);
                        }
                    }
                }
                (None, None) => {}
            }
        }
        changed.live_signals_rooms = (!room_changes.is_empty()).then_some(room_changes);
        changed.live_signals_receipts_by_room_event =
            (!receipt_changes.is_empty()).then_some(receipt_changes);
        changed.live_signals_room_metadata_by_id =
            (!metadata_changes.is_empty()).then_some(metadata_changes);
    }
    changed_slice!(e2ee_trust);
    changed_slice!(local_encryption);
    changed_slice!(native_attention);
    changed_slice!(cjk_text_policy);
    changed_slice!(errors);

    if previous.navigation.active_space_id != next.navigation.active_space_id
        || previous.navigation.space_order != next.navigation.space_order
        || previous.navigation.space_local_presentations
            != next.navigation.space_local_presentations
        || previous.settings.values.room_list_sort != next.settings.values.room_list_sort
        || previous.spaces != next.spaces
        || previous.rooms != next.rooms
        || previous.invites != next.invites
        || previous.room_notification_settings != next.room_notification_settings
    {
        let previous_sidebar = compose_sidebar_for_state(previous);
        let next_sidebar = compose_sidebar_for_state(next);
        if previous_sidebar != next_sidebar {
            changed.sidebar = Some(next_sidebar);
        }
    }

    if changed.is_empty() {
        return None;
    }

    Some(StateDelta {
        generation,
        changed,
    })
}

fn ordered_ids_are_subsequence<'a>(
    previous: impl Iterator<Item = &'a String>,
    next: impl Iterator<Item = &'a String>,
) -> bool {
    let previous = previous.map(String::as_str).collect::<Vec<_>>();
    let mut start = 0;
    for id in next {
        let Some(relative_index) = previous[start..]
            .iter()
            .position(|candidate| *candidate == id.as_str())
        else {
            return false;
        };
        start += relative_index + 1;
    }
    true
}

fn activity_row_deltas(
    previous: &ActivityState,
    next: &ActivityState,
) -> Option<(
    Option<BTreeMap<String, Option<ActivityRow>>>,
    Option<BTreeMap<String, Option<ActivityRow>>>,
)> {
    let (
        ActivityState::Open {
            active_tab: previous_tab,
            recent: previous_recent,
            unread: previous_unread,
            mark_read: previous_mark_read,
        },
        ActivityState::Open {
            active_tab: next_tab,
            recent: next_recent,
            unread: next_unread,
            mark_read: next_mark_read,
        },
    ) = (previous, next)
    else {
        return None;
    };

    if previous_tab != next_tab || previous_mark_read != next_mark_read {
        return None;
    }

    let recent = activity_stream_row_deltas(previous_recent, next_recent)?;
    let unread = activity_stream_row_deltas(previous_unread, next_unread)?;
    Some((recent, unread))
}

fn activity_stream_row_deltas(
    previous: &koushi_state::ActivityStream,
    next: &koushi_state::ActivityStream,
) -> Option<Option<BTreeMap<String, Option<ActivityRow>>>> {
    if previous.next_batch != next.next_batch || previous.resolution != next.resolution {
        return None;
    }

    let previous_keys = previous
        .rows
        .iter()
        .map(activity_row_key)
        .collect::<Vec<_>>();
    let next_keys = next.rows.iter().map(activity_row_key).collect::<Vec<_>>();
    if previous_keys == next_keys && keys_are_unique(&next_keys) {
        let mut changes = BTreeMap::new();
        for (previous_row, next_row) in previous.rows.iter().zip(&next.rows) {
            if previous_row != next_row {
                changes.insert(activity_row_key(next_row), Some(next_row.clone()));
            }
        }
        return Some((!changes.is_empty()).then_some(changes));
    }

    None
}

fn keys_are_unique(keys: &[String]) -> bool {
    keys.iter().collect::<BTreeSet<_>>().len() == keys.len()
}

fn activity_row_key(row: &ActivityRow) -> String {
    match &row.event_id {
        Some(event_id) => format!("event:{event_id}"),
        None => format!("room-unread:{}", row.room_id),
    }
}

fn audit_app_state_delta_slices(state: &AppState) {
    let AppState {
        session: _,
        session_lock_reason: _,
        secure_backup_gate: _,
        sliding_sync_account_epoch: _,
        sliding_sync_capability: _,
        current_session_status: _,
        auth: _,
        account_management_url: _,
        account_management: _,
        account_management_capabilities: _,
        soft_logout_reauth: _,
        qr_login: _,
        settings: _,
        link_preview_settings: _,
        room_preferences: _,
        profile: _,
        space_members: _,
        sync: _,
        sync_generation: _,
        navigation: _,
        spaces: _,
        rooms: _,
        invites: _,
        invite_workflow: _,
        room_list: _,
        room_notification_settings: _,
        room_interactions: _,
        composer_drafts: _,
        scheduled_sends: _,
        upload_staging: _,
        media_gallery: _,
        directory: _,
        room_management: _,
        mention_candidates: _,
        activity: _,
        timeline: _,
        thread: _,
        thread_attention: _,
        threads_list: _,
        thread_root_projections: _,
        focused_context: _,
        search: _,
        search_crawler: _,
        files_view: _,
        basic_operation: _,
        live_signals: _,
        e2ee_trust: _,
        local_encryption: _,
        native_attention: _,
        native_attention_context: _,
        cjk_text_policy: _,
        errors: _,
        device_cleanup: _,
    } = state;
}

#[cfg(test)]
mod tests;
