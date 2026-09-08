use super::{Phase, ReaderWork};
use crate::view_scope_lifecycle::{ScopeError, ViewScopeRegistry};
use koushi_protocol::view::{ViewRetirement, ViewScopeId};

impl ViewScopeRegistry {
    /// Locale is a global input to every reader projection, unlike per-user
    /// profile changes. Visit only the bounded live reader registrations.
    pub(crate) fn reader_locale_changed(&self) {
        let scopes: Vec<_> = {
            let state = self.state.lock().expect("view registry poisoned");
            state
                .scopes
                .iter()
                .filter_map(|(id, entry)| {
                    entry
                        .control
                        .reader
                        .lock()
                        .expect("reader request poisoned")
                        .is_some()
                        .then_some(*id)
                })
                .collect()
        };
        for id in scopes {
            let _ = self.dirty_reader(id, false);
        }
    }

    pub(crate) fn reader_work_superseded(&self, work: &ReaderWork) -> bool {
        let state = self.state.lock().expect("view registry poisoned");
        let Some(entry) = state.scopes.get(&work.scope) else {
            return false;
        };
        entry
            .control
            .reader
            .lock()
            .expect("reader request poisoned")
            .as_ref()
            .is_some_and(|reader| {
                reader.dirty
                    || reader.window_sequence != work.window_sequence
                    || reader.dependency_revision != work.dependency_revision
            })
    }

    pub(crate) async fn reader_work_ready(&self) {
        self.reader_work.notified().await;
    }

    /// Source changes require a refetch; dependency changes advance their own
    /// stamp and reuse raw input once the producer's accepted slot is connected.
    pub(crate) fn dirty_reader(&self, id: ViewScopeId, source: bool) -> Result<(), ScopeError> {
        let result = (|| {
            let mut state = self.state.lock().expect("view registry poisoned");
            let enqueue = {
                let entry = state.scopes.get(&id).ok_or(ScopeError::Closed)?;
                let retired = entry.control.retired.lock().expect("view control poisoned");
                if retired.is_some() {
                    return Err(ScopeError::Closed);
                }
                let mut reader = entry
                    .control
                    .reader
                    .lock()
                    .expect("reader request poisoned");
                let reader = reader.as_mut().ok_or(ScopeError::InvalidModel)?;
                if !source {
                    reader.dependency_revision = reader
                        .dependency_revision
                        .checked_add(1)
                        .ok_or(ScopeError::CounterExhausted)?;
                }
                reader.dirty = true;
                reader.source_dirty |= source;
                let enqueue = reader.phase == Phase::Idle;
                if enqueue {
                    reader.phase = Phase::Queued;
                }
                enqueue
            };
            if enqueue {
                state.reader_queue.push_back(id);
                self.reader_work.notify_one();
            }
            Ok(())
        })();
        if result == Err(ScopeError::CounterExhausted) {
            self.retire(id, ViewRetirement::CounterExhausted);
        }
        result
    }

    pub(crate) fn take_reader_work(&self) -> Result<Option<ReaderWork>, ScopeError> {
        let mut failed_scope = None;
        let result = (|| {
            let mut state = self.state.lock().expect("view registry poisoned");
            while let Some(id) = state.reader_queue.pop_front() {
                if !state.reader_queue.is_empty() {
                    self.reader_work.notify_one();
                }
                let Some(entry) = state.scopes.get(&id) else {
                    continue;
                };
                let retired = entry.control.retired.lock().expect("view control poisoned");
                if retired.is_some() {
                    continue;
                }
                let mut reader = entry
                    .control
                    .reader
                    .lock()
                    .expect("reader request poisoned");
                let Some(reader) = reader.as_mut() else {
                    continue;
                };
                if reader.phase != Phase::Queued {
                    continue;
                }
                failed_scope = Some(id);
                let run_id = reader
                    .run_id
                    .checked_add(1)
                    .ok_or(ScopeError::CounterExhausted)?;
                let reservation = self
                    .budget
                    .reserve_bytes(64 * 1024 * 1024)
                    .ok_or(ScopeError::Capacity)?;
                reader.run_id = run_id;
                reader.phase = Phase::Running;
                let work = ReaderWork {
                    scope: id,
                    source: reader.source.clone(),
                    start: reader.start,
                    limit: reader.limit,
                    window_sequence: reader.window_sequence,
                    dependency_revision: reader.dependency_revision,
                    raw: if reader.source_dirty {
                        None
                    } else {
                        reader.accepted_raw.clone()
                    },
                    reservation,
                    control: std::sync::Arc::downgrade(&entry.control),
                    run_id,
                };
                reader.dirty = false;
                reader.source_dirty = false;
                return Ok(Some(work));
            }
            Ok(None)
        })();
        if let Err(error) = &result
            && let Some(id) = failed_scope
        {
            self.retire(
                id,
                if *error == ScopeError::CounterExhausted {
                    ViewRetirement::CounterExhausted
                } else {
                    ViewRetirement::Capacity
                },
            );
        }
        result
    }

    /// Called after completion/rejection handling, never just after sending.
    pub(crate) fn finish_reader_work(&self, work: &ReaderWork) -> Result<(), ScopeError> {
        let id = work.scope;
        let mut state = self.state.lock().expect("view registry poisoned");
        let enqueue = {
            let entry = state.scopes.get(&id).ok_or(ScopeError::Closed)?;
            let retired = entry.control.retired.lock().expect("view control poisoned");
            if retired.is_some() {
                return Err(ScopeError::Closed);
            }
            let mut reader = entry
                .control
                .reader
                .lock()
                .expect("reader request poisoned");
            let reader = reader.as_mut().ok_or(ScopeError::InvalidModel)?;
            if reader.phase != Phase::Running || reader.run_id != work.run_id {
                return Err(ScopeError::InvalidRevision);
            }
            reader.phase = if reader.dirty {
                Phase::Queued
            } else {
                Phase::Idle
            };
            reader.dirty
        };
        if enqueue {
            state.reader_queue.push_back(id);
            self.reader_work.notify_one();
        }
        Ok(())
    }
}
