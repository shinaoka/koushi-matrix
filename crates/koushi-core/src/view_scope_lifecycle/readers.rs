use koushi_protocol::view::{
    ReaderWindow, ReaderWindowLimit, ReaderWindowTarget, ReceiptSourceRef, ViewRevision,
    ViewScopeId,
};

use super::{OwnedViewScope, ReaderSourceKey, ScopeError, ViewConsumer, model};
use crate::view_budget::ViewReservation;

#[cfg(test)]
mod tests;

mod raw;
mod scheduling;
pub(crate) use raw::ChargedRaw;

#[derive(Clone, Copy, Eq, PartialEq)]
enum Phase {
    Idle,
    Queued,
    Running,
}

pub(crate) struct ReaderWork {
    pub(crate) scope: koushi_protocol::view::ViewScopeId,
    pub(crate) source: ReceiptSourceRef,
    pub(crate) start: u64,
    pub(crate) limit: ReaderWindowLimit,
    pub(crate) window_sequence: u64,
    pub(crate) dependency_revision: u64,
    pub(crate) reservation: ViewReservation,
    pub(crate) raw: Option<std::sync::Arc<ChargedRaw>>,
    control: std::sync::Weak<super::Control>,
    run_id: u64,
}

impl ReaderWork {
    pub(crate) fn spawn<F, Fut>(self, produce: F) -> Result<(), ScopeError>
    where
        F: FnOnce(Self, super::ProducerCompletion) -> Fut + Send + 'static,
        Fut: std::future::Future<Output = ()> + Send + 'static,
    {
        let control = self.control.upgrade().ok_or(ScopeError::Closed)?;
        control.spawn_producer(move |completion| produce(self, completion))
    }
}

impl Drop for ReaderWork {
    fn drop(&mut self) {
        if let Some(control) = self.control.upgrade() {
            let unfinished = control
                .reader
                .lock()
                .expect("reader request poisoned")
                .as_ref()
                .is_some_and(|reader| {
                    reader.phase == Phase::Running && reader.run_id == self.run_id
                });
            if unfinished {
                control.retire(koushi_protocol::view::ViewRetirement::ProducerFailed);
            }
        }
    }
}

struct ObservedReaderAvatars {
    context: koushi_state::AvatarDemandContext,
    visible: Vec<String>,
    prefetch: Vec<String>,
    _bytes: ViewReservation,
}

impl ObservedReaderAvatars {
    fn retain(
        context: &koushi_state::AvatarDemandContext,
        visible: &[String],
        prefetch: &[String],
        budget: &crate::view_budget::ViewBudget,
    ) -> Result<Self, ScopeError> {
        // The inline record is already charged in Control's ReaderRequest.
        // Reserve its backing strings/vectors before retaining copies.
        let bytes =
            visible
                .iter()
                .chain(prefetch)
                .try_fold(context.account_id.len(), |bytes, id| {
                    bytes
                        .checked_add(std::mem::size_of::<String>())
                        .and_then(|bytes| bytes.checked_add(id.len()))
                        .ok_or(ScopeError::Capacity)
                })?;
        let reservation = budget.reserve_bytes(bytes).ok_or(ScopeError::Capacity)?;
        Ok(Self {
            context: context.clone(),
            visible: visible.to_vec(),
            prefetch: prefetch.to_vec(),
            _bytes: reservation,
        })
    }
}

/// The scope's request, not a second copy of timeline/receipt ordering.
pub(super) struct ReaderRequest {
    source: ReceiptSourceRef,
    start: u64,
    limit: ReaderWindowLimit,
    window_sequence: u64,
    dependency_revision: u64,
    phase: Phase,
    run_id: u64,
    dirty: bool,
    source_dirty: bool,
    accepted_raw: Option<std::sync::Arc<ChargedRaw>>,
    avatar_observation: Option<ObservedReaderAvatars>,
    _bytes: ViewReservation,
}

impl ReaderRequest {
    pub(super) fn refresh_avatar_demand(
        &self,
        scope: ViewScopeId,
        current: &super::ChargedAvatarDemand,
        rows: &super::model::InstalledRows,
        budget: &crate::view_budget::ViewBudget,
    ) -> Result<Option<super::ChargedAvatarDemand>, ScopeError> {
        let Some(observation) = &self.avatar_observation else {
            return Ok(None);
        };
        if &observation.context != current.context() || !current.scope_ids().any(|id| id == scope.0)
        {
            return Ok(None);
        }
        let mut next = current.clone();
        if !next.refresh(
            scope.0,
            rows.resolve_avatar_resources(&observation.visible),
            rows.resolve_avatar_resources(&observation.prefetch),
            budget,
        )? {
            return Ok(None);
        }
        Ok(Some(next))
    }
    pub(super) fn accepts(&self, window: &ReaderWindow) -> bool {
        window.source == self.source
            && window.window_sequence == self.window_sequence
            && window.dependency_revision == self.dependency_revision
            && window.start == self.start.min(window.total_count)
            && window.rows.len() as u64
                == (window.total_count - window.start).min(u64::from(self.limit.get()))
    }
}

impl ViewConsumer {
    pub fn update_reader_window(
        &self,
        scope: ViewScopeId,
        installed_revision: ViewRevision,
        sequence: u64,
        target: ReaderWindowTarget,
        limit: ReaderWindowLimit,
    ) -> Result<(), ScopeError> {
        let mut state = self
            .0
            .registry
            .state
            .lock()
            .expect("view registry poisoned");
        let enqueue = {
            let entry = state
                .scopes
                .get(&scope)
                .filter(|entry| entry.owner == self.0.id)
                .ok_or(ScopeError::NotOwned)?;
            if entry
                .control
                .retired
                .lock()
                .expect("view control poisoned")
                .is_some()
            {
                return Err(ScopeError::Closed);
            }
            if entry
                .control
                .mailbox
                .lock()
                .expect("view mailbox poisoned")
                .installed_revision()
                != Some(installed_revision)
            {
                return Err(ScopeError::InvalidRevision);
            }
            let mut reader = entry
                .control
                .reader
                .lock()
                .expect("reader request poisoned");
            let reader = reader.as_mut().ok_or(ScopeError::Closed)?;
            if sequence < reader.window_sequence {
                return Err(ScopeError::InvalidRevision);
            }
            if sequence == reader.window_sequence
                && (reader.limit != limit
                    || !matches!(target, ReaderWindowTarget::Index { start } if start == reader.start))
            {
                return Err(ScopeError::InvalidRevision);
            }
            let start = match target {
                ReaderWindowTarget::Index { start } => start,
                ReaderWindowTarget::Anchor { user_id } => entry
                    .control
                    .mailbox
                    .lock()
                    .expect("view mailbox poisoned")
                    .anchor_index(installed_revision, &user_id)?
                    .ok_or(ScopeError::InvalidModel)?,
            };
            reader.start = start;
            reader.limit = limit;
            reader.window_sequence = sequence;
            reader.dirty = true;
            reader.source_dirty = false;
            if reader.phase == Phase::Idle {
                reader.phase = Phase::Queued;
                true
            } else {
                false
            }
        };
        if enqueue {
            state.reader_queue.push_back(scope);
            self.0.registry.reader_work.notify_one();
        }
        Ok(())
    }

    pub(crate) fn observe_current_reader_avatars(
        &self,
        id: koushi_protocol::view::ViewScopeId,
        revision: ViewRevision,
        sequence: u64,
        visible: &[String],
        prefetch: &[String],
    ) -> Result<(), ScopeError> {
        let context = self
            .0
            .registry
            .state
            .lock()
            .expect("view registry poisoned")
            .avatar_demand
            .as_ref()
            .ok_or(ScopeError::InactiveSession)?
            .context()
            .clone();
        self.observe_reader_avatars(id, revision, sequence, &context, visible, prefetch)
    }

    /// Commit resolved reader demand. Context comes from AppActor's current
    /// session, never from deserialized host input; host inputs are IDs/revisions.
    pub(crate) fn observe_reader_avatars(
        &self,
        id: koushi_protocol::view::ViewScopeId,
        revision: ViewRevision,
        sequence: u64,
        context: &koushi_state::AvatarDemandContext,
        visible: &[String],
        prefetch: &[String],
    ) -> Result<(), ScopeError> {
        if visible.len() > koushi_state::AVATAR_VISIBLE_CAPACITY
            || prefetch.len() > koushi_state::AVATAR_PREFETCH_CAPACITY
        {
            return Err(ScopeError::Capacity);
        }
        self.with_live_reader_avatar_source(id, revision, |rows| {
            // The acknowledged model grants identity access, not authority to
            // restore resource bindings superseded by a newer Rust projection.
            for user_id in visible.iter().chain(prefetch) {
                rows.avatar_mxc(user_id)?;
            }
            let observation =
                ObservedReaderAvatars::retain(context, visible, prefetch, &self.0.registry.budget)?;
            let mut state = self
                .0
                .registry
                .state
                .lock()
                .expect("view registry poisoned");
            let control = state
                .scopes
                .get(&id)
                .filter(|entry| entry.owner == self.0.id)
                .map(|entry| entry.control.clone())
                .ok_or(ScopeError::NotOwned)?;
            let retired = control.retired.lock().expect("view control poisoned");
            if state.closed
                || self.0.closed.load(std::sync::atomic::Ordering::Acquire)
                || retired.is_some()
            {
                return Err(ScopeError::Closed);
            }
            if !control
                .reader
                .lock()
                .expect("reader request poisoned")
                .as_ref()
                .is_some_and(|reader| {
                    reader.source.timeline.key.account_key.0 == context.account_id
                })
            {
                return Err(ScopeError::InactiveSession);
            }
            let current = {
                let mailbox = control.mailbox.lock().expect("view mailbox poisoned");
                if mailbox.installed_revision() != Some(revision) {
                    return Err(ScopeError::InvalidRevision);
                }
                mailbox
                    .current_rows()
                    .ok_or(ScopeError::SourceUnavailable)?
            };
            let visible = current.resolve_avatar_resources(visible);
            let prefetch = current.resolve_avatar_resources(prefetch);
            // Only AppActor may establish a session context. If it cleared or
            // changed during resolution, do not recreate the captured context.
            let mut next = state
                .avatar_demand
                .as_deref()
                .ok_or(ScopeError::InactiveSession)?
                .clone();
            next.replace(
                context,
                id.0,
                sequence,
                visible,
                prefetch,
                &self.0.registry.budget,
            )?;
            control
                .reader
                .lock()
                .expect("reader request poisoned")
                .as_mut()
                .ok_or(ScopeError::Closed)?
                .avatar_observation = Some(observation);
            state.avatar_demand = Some(std::sync::Arc::new(next));
            self.0.registry.reader_work.notify_one();
            Ok(())
        })
    }

    /// Reader-only observation commit. The accepted raw source owns its charge;
    /// hold source authority and recheck the installed model before the callback.
    pub(crate) fn with_live_reader_avatar_source<R>(
        &self,
        id: koushi_protocol::view::ViewScopeId,
        revision: ViewRevision,
        commit: impl FnOnce(&super::model::InstalledRows) -> Result<R, ScopeError>,
    ) -> Result<R, ScopeError> {
        let installed = self.avatar_source(id, revision)?;
        let raw = {
            let state = self
                .0
                .registry
                .state
                .lock()
                .expect("view registry poisoned");
            let entry = state.scopes.get(&id).ok_or(ScopeError::Closed)?;
            let reader = entry
                .control
                .reader
                .lock()
                .expect("reader request poisoned");
            reader
                .as_ref()
                .and_then(|reader| reader.accepted_raw.clone())
                .ok_or(ScopeError::SourceUnavailable)?
        };
        raw.raw
            .commit_if_current(|| {
                let current = self.avatar_source(id, revision)?;
                if !std::sync::Arc::ptr_eq(&installed, &current) {
                    return Err(ScopeError::InvalidRevision);
                }
                commit(&current)
            })
            .ok_or(ScopeError::SourceUnavailable)?
    }

    pub fn open_reader(
        &self,
        source: ReceiptSourceRef,
        start: u64,
        limit: ReaderWindowLimit,
    ) -> Result<OwnedViewScope, ScopeError> {
        let source_key = ReaderSourceKey::from_source(&source);
        let bytes = self
            .0
            .registry
            .budget
            .reserve_bytes(model::encoded_bytes(&source)?)
            .ok_or(ScopeError::Capacity)?;
        let scope = self.open()?;
        {
            let retired = scope.control.retired.lock().expect("view control poisoned");
            if retired.is_some() {
                return Err(ScopeError::Closed);
            }
            *scope
                .control
                .reader
                .lock()
                .expect("reader request poisoned") = Some(ReaderRequest {
                source,
                start,
                limit,
                window_sequence: 0,
                dependency_revision: 1,
                phase: Phase::Idle,
                run_id: 0,
                dirty: false,
                source_dirty: false,
                accepted_raw: None,
                avatar_observation: None,
                _bytes: bytes,
            });
        }
        self.0
            .registry
            .register_reader_source(scope.id(), source_key)?;
        self.0.registry.dirty_reader(scope.id(), true)?;
        Ok(scope)
    }
}
