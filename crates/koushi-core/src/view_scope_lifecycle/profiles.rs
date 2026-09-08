use super::{Entry, ReaderSourceKey, RegistryState, RoomProfileKey, ScopeError, ViewScopeRegistry};
use crate::view_budget::ViewReservation;
use koushi_protocol::view::ViewScopeId;
use std::collections::HashSet;

pub(super) struct ProfileRegistration {
    pub(super) room_id: String,
    pub(super) users: Vec<String>,
    pub(super) thumbnail_sources: Vec<String>,
    _bytes: ViewReservation,
}

impl ProfileRegistration {
    pub(super) fn prepare(
        room_id: String,
        users: impl Iterator<Item = String>,
        thumbnail_sources: impl Iterator<Item = String>,
        builder: &mut ViewReservation,
    ) -> Result<Self, ScopeError> {
        let mut users: Vec<_> = users.collect();
        users.sort_unstable();
        users.dedup();
        let mut thumbnail_sources: Vec<_> = thumbnail_sources.collect();
        thumbnail_sources.sort_unstable();
        thumbnail_sources.dedup();
        // Conservatively charge per-scope keys plus index keys/membership even
        // when another scope already shares an index key. This is data accounting.
        let initial = std::mem::size_of::<Self>()
            .checked_add(room_id.len())
            .and_then(|total| {
                room_id
                    .len()
                    .checked_mul(users.len())
                    .and_then(|room_index| total.checked_add(room_index))
            })
            .and_then(|total| {
                users
                    .len()
                    .checked_mul(std::mem::size_of::<String>())
                    .and_then(|user_index| total.checked_add(user_index))
            })
            .ok_or(ScopeError::Capacity)?;
        let bytes = users
            .iter()
            .try_fold(initial, |total, user| {
                total.checked_add(user.len().checked_mul(2)?)?.checked_add(
                    2 * std::mem::size_of::<String>() + std::mem::size_of::<ViewScopeId>(),
                )
            })
            .and_then(|total| {
                thumbnail_sources.iter().try_fold(total, |total, source| {
                    total
                        .checked_add(source.len().checked_mul(2)?)?
                        .checked_add(2 * std::mem::size_of::<String>())
                })
            })
            .ok_or(ScopeError::Capacity)?;
        let bytes = builder.split_bytes(bytes).ok_or(ScopeError::Capacity)?;
        Ok(Self {
            room_id,
            users,
            thumbnail_sources,
            _bytes: bytes,
        })
    }
}

impl RegistryState {
    pub(super) fn replace_profiles(
        &mut self,
        id: ViewScopeId,
        registration: ProfileRegistration,
    ) -> Option<ProfileRegistration> {
        let entry = self
            .scopes
            .get_mut(&id)
            .expect("validated scope remains under registry lock");
        let previous = entry.profile_dependencies.replace(registration);
        if let Some(previous) = &previous {
            for user in &previous.users {
                if let Some(scopes) = self.profile_readers.get_mut(user) {
                    scopes.remove(&id);
                    if scopes.is_empty() {
                        self.profile_readers.remove(user);
                    }
                }
                let key = RoomProfileKey::new(&previous.room_id, user);
                if let Some(scopes) = self.room_profile_readers.get_mut(&key) {
                    scopes.remove(&id);
                    if scopes.is_empty() {
                        self.room_profile_readers.remove(&key);
                    }
                }
            }
            for source in &previous.thumbnail_sources {
                if let Some(scopes) = self.thumbnail_readers.get_mut(source) {
                    scopes.remove(&id);
                    if scopes.is_empty() {
                        self.thumbnail_readers.remove(source);
                    }
                }
            }
        }
        let current = self.scopes[&id].profile_dependencies.as_ref().unwrap();
        for user in &current.users {
            self.profile_readers
                .entry(user.clone())
                .or_default()
                .insert(id);
            self.room_profile_readers
                .entry(RoomProfileKey::new(&current.room_id, user))
                .or_default()
                .insert(id);
        }
        for source in &current.thumbnail_sources {
            self.thumbnail_readers
                .entry(source.clone())
                .or_default()
                .insert(id);
        }
        previous
    }

    pub(super) fn remove_scope(&mut self, id: ViewScopeId) -> Option<Entry> {
        self.reader_queue.retain(|queued| *queued != id);
        let entry = self.scopes.remove(&id)?;
        if let Some(source) = &entry.reader_source
            && let Some(scopes) = self.reader_sources.get_mut(source)
        {
            scopes.remove(&id);
            if scopes.is_empty() {
                self.reader_sources.remove(source);
            }
        }
        if let Some(registration) = &entry.profile_dependencies {
            for user in &registration.users {
                if let Some(scopes) = self.profile_readers.get_mut(user) {
                    scopes.remove(&id);
                    if scopes.is_empty() {
                        self.profile_readers.remove(user);
                    }
                }
                let key = RoomProfileKey::new(&registration.room_id, user);
                if let Some(scopes) = self.room_profile_readers.get_mut(&key) {
                    scopes.remove(&id);
                    if scopes.is_empty() {
                        self.room_profile_readers.remove(&key);
                    }
                }
            }
            for source in &registration.thumbnail_sources {
                if let Some(scopes) = self.thumbnail_readers.get_mut(source) {
                    scopes.remove(&id);
                    if scopes.is_empty() {
                        self.thumbnail_readers.remove(source);
                    }
                }
            }
        }
        Some(entry)
    }
}

impl ViewScopeRegistry {
    pub(crate) fn register_reader_source(
        &self,
        id: ViewScopeId,
        source: ReaderSourceKey,
    ) -> Result<(), ScopeError> {
        let bytes = source
            .retained_bytes()
            .and_then(|bytes| self.budget.reserve_bytes(bytes))
            .ok_or(ScopeError::Capacity)?;
        let mut state = self.state.lock().expect("view registry poisoned");
        let (previous, previous_bytes) = {
            let entry = state.scopes.get_mut(&id).ok_or(ScopeError::Closed)?;
            (
                entry.reader_source.replace(source.clone()),
                entry.reader_source_bytes.replace(bytes),
            )
        };
        if let Some(previous) = previous
            && let Some(scopes) = state.reader_sources.get_mut(&previous)
        {
            scopes.remove(&id);
            if scopes.is_empty() {
                state.reader_sources.remove(&previous);
            }
        }
        state.reader_sources.entry(source).or_default().insert(id);
        drop(state);
        drop(previous_bytes);
        Ok(())
    }

    pub(crate) fn reader_receipt_source_changed(
        &self,
        account_key: &str,
        room_id: &str,
        event_id: &str,
    ) {
        let source = super::ReaderSourceKey::new(account_key, room_id, event_id);
        let scopes = {
            let state = self.state.lock().expect("view registry poisoned");
            state
                .reader_sources
                .get(&source)
                .cloned()
                .unwrap_or_default()
        };
        for id in scopes {
            let _ = self.dirty_reader(id, true);
        }
    }

    pub(crate) fn reader_profiles_changed(&self, users: &[String]) {
        let scopes: HashSet<_> = {
            let state = self.state.lock().expect("view registry poisoned");
            users
                .iter()
                .filter_map(|user| state.profile_readers.get(user))
                .flat_map(|scopes| scopes.iter().copied())
                .collect()
        };
        for id in scopes {
            let _ = self.dirty_reader(id, false);
        }
    }

    pub(crate) fn reader_room_profiles_changed(&self, room_id: &str, users: &[String]) {
        let scopes: HashSet<_> = {
            let state = self.state.lock().expect("view registry poisoned");
            users
                .iter()
                .filter_map(|user| {
                    state
                        .room_profile_readers
                        .get(&RoomProfileKey::new(room_id, user))
                })
                .flat_map(|scopes| scopes.iter().copied())
                .collect()
        };
        for id in scopes {
            let _ = self.dirty_reader(id, false);
        }
    }

    pub(crate) fn reader_avatar_thumbnail_changed(&self, mxc_uri: &str) {
        let scopes = {
            let state = self.state.lock().expect("view registry poisoned");
            state
                .thumbnail_readers
                .get(mxc_uri)
                .cloned()
                .unwrap_or_default()
        };
        for id in scopes {
            let _ = self.dirty_reader(id, false);
        }
    }
}
