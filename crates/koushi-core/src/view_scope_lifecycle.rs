use std::{
    collections::{HashMap, HashSet, VecDeque},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
};

use koushi_protocol::{
    RuntimeConnectionId,
    view::{
        ReaderWindowLimit, ReaderWindowTarget, ViewModel, ViewRetirement, ViewRevision, ViewScopeId,
    },
};
use tokio::sync::Notify;

use crate::view_budget::{ViewBudget, ViewReservation};

mod mailbox;
mod model;
mod producer;
mod profiles;
mod readers;
pub(crate) use producer::ProducerCompletion;
pub(crate) use readers::{ChargedRaw, ReaderWork};

static NEXT_ID: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Eq, PartialEq)]
pub enum ScopeError {
    InactiveSession,
    SourceUnavailable,
    SourceRetired,
    Capacity,
    Closed,
    CounterExhausted,
    NotOwned,
    NotRetired,
    ProducerRunning,
    InvalidModel,
    InvalidRevision,
}

fn mint_id(counter: &AtomicU64) -> Result<u64, ScopeError> {
    counter
        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
            value.checked_add(1)
        })
        .map(|value| value + 1)
        .map_err(|_| ScopeError::CounterExhausted)
}

#[derive(Clone, Default)]
pub(crate) struct ViewScopeRegistry {
    state: Arc<Mutex<RegistryState>>,
    budget: ViewBudget,
    stopped: Arc<Notify>,
    reader_work: Arc<Notify>,
}

/// Owned by the runtime task, including cancellation before its first poll.
pub(crate) struct ViewRuntimeLifetime(pub(crate) ViewScopeRegistry);

impl Drop for ViewRuntimeLifetime {
    fn drop(&mut self) {
        self.0.shutdown();
    }
}

#[derive(Default)]
struct RegistryState {
    closed: bool,
    scopes: HashMap<ViewScopeId, Entry>,
    reader_queue: VecDeque<ViewScopeId>,
    profile_readers: HashMap<String, HashSet<ViewScopeId>>,
    room_profile_readers: HashMap<RoomProfileKey, HashSet<ViewScopeId>>,
    thumbnail_readers: HashMap<String, HashSet<ViewScopeId>>,
    reader_sources: HashMap<ReaderSourceKey, HashSet<ViewScopeId>>,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) struct ReaderSourceKey {
    account_key: String,
    room_id: String,
    event_id: String,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) struct RoomProfileKey {
    room_id: String,
    user_id: String,
}

impl RoomProfileKey {
    fn new(room_id: &str, user_id: &str) -> Self {
        Self {
            room_id: room_id.to_owned(),
            user_id: user_id.to_owned(),
        }
    }
}

impl ReaderSourceKey {
    fn new(account_key: &str, room_id: &str, event_id: &str) -> Self {
        Self {
            account_key: account_key.to_owned(),
            room_id: room_id.to_owned(),
            event_id: event_id.to_owned(),
        }
    }

    fn from_source(source: &koushi_protocol::view::ReceiptSourceRef) -> Self {
        Self::new(
            &source.timeline.key.account_key.0,
            source.timeline.key.room_id(),
            &source.event_id,
        )
    }

    fn retained_bytes(&self) -> Option<usize> {
        let key_bytes = self
            .account_key
            .len()
            .checked_add(self.room_id.len())?
            .checked_add(self.event_id.len())?;
        std::mem::size_of::<Self>()
            .checked_add(key_bytes)?
            .checked_mul(2)?
            .checked_add(std::mem::size_of::<ViewScopeId>())
    }
}

struct Entry {
    owner: u64,
    control: Arc<Control>,
    profile_dependencies: Option<profiles::ProfileRegistration>,
    reader_source: Option<ReaderSourceKey>,
    reader_source_bytes: Option<ViewReservation>,
    _slot: ViewReservation,
}

pub(crate) enum ScopeDelivery {
    Model {
        revision: ViewRevision,
        model: Arc<model::PreparedModel>,
    },
    Retired(ViewRetirement),
}

struct Control {
    retired: Mutex<Option<ViewRetirement>>,
    mailbox: Mutex<mailbox::Mailbox>,
    producer: Mutex<Option<crate::runtime::AbortOnDrop<()>>>,
    reader: Mutex<Option<readers::ReaderRequest>>,
    wake: Notify,
    _bytes: ViewReservation,
}

impl Control {
    fn retire(&self, reason: ViewRetirement) {
        let (producer, reader) = {
            let mut retired = self.retired.lock().expect("view control poisoned");
            if retired.is_some() {
                return;
            }
            *retired = Some(reason);
            self.mailbox.lock().expect("view mailbox poisoned").clear();
            self.wake.notify_one();
            (
                self.producer.lock().expect("view producer poisoned").take(),
                self.reader.lock().expect("reader request poisoned").take(),
            )
        };
        // Aborting can drop the worker's weak completion guard. Never do that
        // under the retirement or producer locks that guard itself may acquire.
        drop((producer, reader));
    }
}

struct Consumer {
    id: u64,
    #[cfg(test)]
    origin: RuntimeConnectionId,
    closed: AtomicBool,
    #[cfg(test)]
    retired: Notify,
    registry: ViewScopeRegistry,
    _bytes: ViewReservation,
}

#[derive(Clone)]
pub(crate) struct ViewConsumer(Arc<Consumer>);

pub(crate) struct OwnedViewScope {
    id: ViewScopeId,
    consumer: Arc<Consumer>,
    control: Arc<Control>,
    retirement_delivered: bool,
}

impl ViewScopeRegistry {
    pub(crate) fn consumer(&self, origin: RuntimeConnectionId) -> Result<ViewConsumer, ScopeError> {
        let state = self.state.lock().expect("view registry poisoned");
        #[cfg(not(test))]
        let _ = origin;
        if state.closed {
            return Err(ScopeError::Closed);
        }
        let bytes = self
            .budget
            .reserve_bytes(std::mem::size_of::<Consumer>())
            .ok_or(ScopeError::Capacity)?;
        Ok(ViewConsumer(Arc::new(Consumer {
            id: mint_id(&NEXT_ID)?,
            #[cfg(test)]
            origin,
            closed: AtomicBool::new(false),
            #[cfg(test)]
            retired: Notify::new(),
            registry: self.clone(),
            _bytes: bytes,
        })))
    }

    pub(crate) fn publish_current(
        &self,
        scope: ViewScopeId,
        model: ViewModel,
        resources: Vec<crate::timeline::ReaderAvatarResource>,
        source: &crate::timeline::ResolvedReceiptWindow,
    ) -> Result<ViewRevision, ScopeError> {
        source.source_revision()?;
        if !source.matches_model(&model) {
            return Err(ScopeError::InvalidModel);
        }
        let prepared = Arc::new(model::prepare(model, &self.budget, resources)?);
        let (revision, replaced) = source
            .commit_if_current(|| self.commit_prepared(scope, &prepared))
            .ok_or(ScopeError::SourceUnavailable)??;
        drop(replaced);
        Ok(revision)
    }

    // Source-independent mailbox fixtures only; production publication must be fenced.
    #[cfg(test)]
    pub(crate) fn publish(
        &self,
        scope: ViewScopeId,
        model: ViewModel,
        resources: Vec<crate::timeline::ReaderAvatarResource>,
    ) -> Result<ViewRevision, ScopeError> {
        let prepared = Arc::new(model::prepare(model, &self.budget, resources)?);
        let (revision, replaced) = self.commit_prepared(scope, &prepared)?;
        // Drop payloads only after all commit (and future source-fence) guards leave.
        drop(replaced);
        Ok(revision)
    }

    // Source fencing may wrap this short operation, never serialization/preparation.
    fn commit_prepared(
        &self,
        scope: ViewScopeId,
        prepared: &Arc<model::PreparedModel>,
    ) -> Result<(ViewRevision, Option<Arc<model::PreparedModel>>), ScopeError> {
        let state = self.state.lock().expect("view registry poisoned");
        let entry = state.scopes.get(&scope).ok_or(ScopeError::Closed)?;
        let retired = entry.control.retired.lock().expect("view control poisoned");
        if retired.is_some() {
            return Err(ScopeError::Closed);
        }
        if let ViewModel::ReaderReady(window) = &prepared.model {
            let request = entry
                .control
                .reader
                .lock()
                .expect("reader request poisoned");
            if !request
                .as_ref()
                .is_some_and(|request| request.accepts(window))
            {
                return Err(ScopeError::InvalidModel);
            }
        }
        let committed = entry
            .control
            .mailbox
            .lock()
            .expect("view mailbox poisoned")
            .publish(prepared)?;
        entry.control.wake.notify_one();
        Ok(committed)
    }

    pub(crate) fn retire(&self, scope: ViewScopeId, reason: ViewRetirement) {
        let state = self.state.lock().expect("view registry poisoned");
        if let Some(entry) = state.scopes.get(&scope) {
            entry.control.retire(reason);
        }
    }

    /// All current view kinds are session-bound Matrix views. Keep host
    /// consumers alive while retiring their old-session scopes synchronously.
    pub(crate) fn retire_session(&self) {
        let controls: Vec<_> = {
            let mut state = self.state.lock().expect("view registry poisoned");
            state.reader_queue.clear();
            state.profile_readers.clear();
            state.room_profile_readers.clear();
            state.thumbnail_readers.clear();
            state.reader_sources.clear();
            state
                .scopes
                .values()
                .map(|entry| entry.control.clone())
                .collect()
        };
        for control in controls {
            control.retire(ViewRetirement::SessionRetired);
        }
    }

    pub(crate) fn shutdown(&self) {
        let mut state = self.state.lock().expect("view registry poisoned");
        state.closed = true;
        self.stopped.notify_waiters();
        for entry in state.scopes.values() {
            entry.control.retire(ViewRetirement::RuntimeStopped);
        }
        state.scopes.clear();
        state.reader_queue.clear();
        state.profile_readers.clear();
        state.room_profile_readers.clear();
        state.thumbnail_readers.clear();
        state.reader_sources.clear();
    }

    fn retire_consumer(&self, id: u64) {
        let mut state = self.state.lock().expect("view registry poisoned");
        let ids: Vec<_> = state
            .scopes
            .iter()
            .filter_map(|(scope, entry)| (entry.owner == id).then_some(*scope))
            .collect();
        let entries: Vec<_> = ids
            .into_iter()
            .filter_map(|scope| state.remove_scope(scope))
            .collect();
        drop(state);
        for entry in entries {
            entry.control.retire(ViewRetirement::ConsumerRetired);
        }
    }
}

impl ViewConsumer {
    #[cfg(test)]
    pub(crate) async fn cancelled(&self) {
        // Create waiters before checking flags: retirement may precede first poll.
        let retired = self.0.retired.notified();
        let stopped = self.0.registry.stopped.notified();
        if self.0.closed.load(Ordering::Acquire)
            || self
                .0
                .registry
                .state
                .lock()
                .expect("view registry poisoned")
                .closed
        {
            return;
        }
        tokio::select! { _ = retired => {}, _ = stopped => {} }
    }

    #[cfg(test)]
    pub(crate) fn is_current_for(
        &self,
        registry: &ViewScopeRegistry,
        origin: RuntimeConnectionId,
    ) -> bool {
        self.0.origin == origin
            && Arc::ptr_eq(&self.0.registry.state, &registry.state)
            && !self.0.closed.load(Ordering::Acquire)
            && !registry
                .state
                .lock()
                .expect("view registry poisoned")
                .closed
    }

    // Source/session admission belongs to the AppActor before opening this lifecycle record.
    pub(crate) fn open(&self) -> Result<OwnedViewScope, ScopeError> {
        let mut state = self
            .0
            .registry
            .state
            .lock()
            .expect("view registry poisoned");
        if state.closed || self.0.closed.load(Ordering::Acquire) {
            return Err(ScopeError::Closed);
        }
        let slot = self
            .0
            .registry
            .budget
            .reserve_scope(0)
            .ok_or(ScopeError::Capacity)?;
        let bytes = self
            .0
            .registry
            .budget
            .reserve_bytes(std::mem::size_of::<Control>() + std::mem::size_of::<Entry>())
            .ok_or(ScopeError::Capacity)?;
        let id = ViewScopeId(mint_id(&NEXT_ID)?);
        let control = Arc::new(Control {
            retired: Mutex::new(None),
            mailbox: Mutex::new(mailbox::Mailbox::default()),
            producer: Mutex::new(None),
            reader: Mutex::new(None),
            wake: Notify::new(),
            _bytes: bytes,
        });
        state.scopes.insert(
            id,
            Entry {
                owner: self.0.id,
                control: control.clone(),
                profile_dependencies: None,
                reader_source: None,
                reader_source_bytes: None,
                _slot: slot,
            },
        );
        Ok(OwnedViewScope {
            id,
            consumer: self.0.clone(),
            control,
            retirement_delivered: false,
        })
    }

    #[cfg(test)]
    pub(crate) fn retire(&self) {
        self.0.closed.store(true, Ordering::Release);
        self.0.retired.notify_waiters();
        self.0.registry.retire_consumer(self.0.id);
    }

    /// Authorize against installed metadata; copy transport bytes only after returning.
    pub fn resource(
        &self,
        id: ViewScopeId,
        revision: ViewRevision,
        source_ref: &str,
    ) -> Result<Option<crate::renderable_thumbnail::RenderableThumbnailLease>, ScopeError> {
        let state = self
            .0
            .registry
            .state
            .lock()
            .expect("view registry poisoned");
        if state.closed || self.0.closed.load(Ordering::Acquire) {
            return Err(ScopeError::Closed);
        }
        let entry = state
            .scopes
            .get(&id)
            .filter(|entry| entry.owner == self.0.id)
            .ok_or(ScopeError::NotOwned)?;
        let retired = entry.control.retired.lock().expect("view control poisoned");
        if retired.is_some() {
            return Err(ScopeError::Closed);
        }
        entry
            .control
            .mailbox
            .lock()
            .expect("view mailbox poisoned")
            .resource(revision, source_ref)
    }

    pub fn ack_model(&self, id: ViewScopeId, revision: ViewRevision) -> Result<(), ScopeError> {
        let state = self
            .0
            .registry
            .state
            .lock()
            .expect("view registry poisoned");
        let entry = state
            .scopes
            .get(&id)
            .filter(|entry| entry.owner == self.0.id)
            .ok_or(ScopeError::NotOwned)?;
        let retired = entry.control.retired.lock().expect("view control poisoned");
        if retired.is_some() {
            return Err(ScopeError::Closed);
        }
        entry
            .control
            .mailbox
            .lock()
            .expect("view mailbox poisoned")
            .ack(revision)?;
        entry.control.wake.notify_one();
        Ok(())
    }

    #[cfg(test)]
    pub fn ack_retirement(&self, id: ViewScopeId) -> Result<(), ScopeError> {
        let mut state = self
            .0
            .registry
            .state
            .lock()
            .expect("view registry poisoned");
        let entry = state
            .scopes
            .get(&id)
            .filter(|entry| entry.owner == self.0.id)
            .ok_or(ScopeError::NotOwned)?;
        if entry
            .control
            .retired
            .lock()
            .expect("view control poisoned")
            .is_none()
        {
            return Err(ScopeError::NotRetired);
        }
        state.remove_scope(id);
        Ok(())
    }
}

impl Drop for Consumer {
    fn drop(&mut self) {
        self.registry.retire_consumer(self.id);
    }
}

impl OwnedViewScope {
    pub(crate) fn id(&self) -> ViewScopeId {
        self.id
    }

    #[cfg(test)]
    pub(crate) fn is_live(&self) -> bool {
        self.control
            .retired
            .lock()
            .expect("view control poisoned")
            .is_none()
    }

    pub(crate) async fn next_delivery(&mut self) -> Option<ScopeDelivery> {
        if self.retirement_delivered {
            return None;
        }
        loop {
            let notified = self.control.wake.notified();
            {
                let retired = self.control.retired.lock().expect("view control poisoned");
                if let Some(reason) = *retired {
                    self.retirement_delivered = true;
                    return Some(ScopeDelivery::Retired(reason));
                }
                if let Some((revision, model)) = self
                    .control
                    .mailbox
                    .lock()
                    .expect("view mailbox poisoned")
                    .take()
                {
                    return Some(ScopeDelivery::Model { revision, model });
                }
            }
            notified.await;
        }
    }

    #[cfg(test)]
    pub(crate) fn ack_retirement(&self) -> Result<(), ScopeError> {
        ViewConsumer(self.consumer.clone()).ack_retirement(self.id)
    }

    pub(crate) fn update_reader_window(
        &self,
        installed_revision: ViewRevision,
        sequence: u64,
        target: ReaderWindowTarget,
        limit: ReaderWindowLimit,
    ) -> Result<(), ScopeError> {
        ViewConsumer(self.consumer.clone()).update_reader_window(
            self.id,
            installed_revision,
            sequence,
            target,
            limit,
        )
    }
}

impl Drop for OwnedViewScope {
    fn drop(&mut self) {
        let entry = {
            let mut state = self
                .consumer
                .registry
                .state
                .lock()
                .expect("view registry poisoned");
            state.remove_scope(self.id)
        };
        // Temporary upgraded observers may outlive the owner. Close explicitly,
        // rather than relying on the final Control allocation being dropped.
        // The registry lock is released before aborting work or dropping data.
        self.control.retire(ViewRetirement::ScopeClosed);
        drop(entry);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn model_backpressure_coalesces_and_never_blocks_retirement() {
        for reason in [
            ViewRetirement::SourceUnavailable,
            ViewRetirement::Capacity,
            ViewRetirement::CounterExhausted,
            ViewRetirement::ProducerFailed,
        ] {
            let registry = ViewScopeRegistry::default();
            let consumer = registry.consumer(RuntimeConnectionId(1)).unwrap();
            let mut scope = consumer.open().unwrap();
            let source = serde_json::from_value(serde_json::json!({
                "key": {"account_key": "account", "kind": {"Room": {"room_id": "!r:example.org"}}},
                "projection_request_id": {"connection_id": "1", "sequence": "2"},
                "generation": "3", "event_id": "$event"
            }))
            .unwrap();
            let model = koushi_protocol::view::ViewModel::ReaderLoading { source };
            let first = registry
                .publish(scope.id, model.clone(), Vec::new())
                .unwrap();
            assert!(
                matches!(scope.next_delivery().await, Some(ScopeDelivery::Model { revision, .. }) if revision == first)
            );
            let skipped = registry
                .publish(scope.id, model.clone(), Vec::new())
                .unwrap();
            let latest = registry
                .publish(scope.id, model.clone(), Vec::new())
                .unwrap();
            assert_eq!(
                consumer.ack_model(scope.id, skipped),
                Err(ScopeError::InvalidRevision)
            );
            assert!(
                tokio::time::timeout(std::time::Duration::from_millis(10), scope.next_delivery())
                    .await
                    .is_err()
            );
            consumer.ack_model(scope.id, first).unwrap();
            assert!(
                matches!(scope.next_delivery().await, Some(ScopeDelivery::Model { revision, .. }) if revision == latest)
            );
            consumer.ack_model(scope.id, first).unwrap();
            assert!(
                tokio::time::timeout(std::time::Duration::from_millis(10), scope.next_delivery())
                    .await
                    .is_err()
            );
            consumer.ack_model(scope.id, latest).unwrap();
            assert_eq!(
                consumer.ack_model(scope.id, skipped),
                Err(ScopeError::InvalidRevision)
            );
            let unacked = registry.publish(scope.id, model, Vec::new()).unwrap();
            assert!(
                matches!(scope.next_delivery().await, Some(ScopeDelivery::Model { revision, .. }) if revision == unacked)
            );
            registry.retire(scope.id, reason);
            assert!(matches!(
                scope.next_delivery().await,
                Some(ScopeDelivery::Retired(observed)) if observed == reason
            ));
            assert_eq!(
                consumer.ack_model(scope.id, latest),
                Err(ScopeError::Closed)
            );
        }
    }

    #[tokio::test]
    async fn transferred_scope_keeps_owner_and_retirement_holds_slot_until_ack() {
        let registry = ViewScopeRegistry::default();
        let first = registry.consumer(RuntimeConnectionId(1)).unwrap();
        let second = registry.consumer(RuntimeConnectionId(2)).unwrap();
        let transferred = first.open().unwrap();
        let other = second.open().unwrap();
        let id = transferred.id;
        drop(first);
        assert!(transferred.is_live());
        assert_eq!(transferred.consumer.origin, RuntimeConnectionId(1));
        assert_eq!(second.ack_retirement(id), Err(ScopeError::NotOwned));
        let waiter = tokio::spawn(async move {
            let mut transferred = transferred;
            let delivery = transferred.next_delivery().await;
            (transferred, delivery)
        });
        tokio::task::yield_now().await;
        registry.retire(id, ViewRetirement::SourceUnavailable);
        let (transferred, reason) = tokio::time::timeout(std::time::Duration::from_secs(1), waiter)
            .await
            .unwrap()
            .unwrap();
        assert!(matches!(
            reason,
            Some(ScopeDelivery::Retired(ViewRetirement::SourceUnavailable))
        ));
        assert!(!transferred.is_live());
        assert!(other.is_live());
        let remaining: Vec<_> = (0..62).map(|_| second.open().unwrap()).collect();
        assert!(matches!(second.open(), Err(ScopeError::Capacity)));
        // Delivering terminal control alone does not release the tombstone's slot.
        assert!(matches!(second.open(), Err(ScopeError::Capacity)));
        transferred.ack_retirement().unwrap();
        let replacement = second.open().unwrap();
        assert_ne!(replacement.id, id);
        drop((remaining, replacement));
        second.retire();
        assert!(!other.is_live());
        assert!(matches!(second.open(), Err(ScopeError::Closed)));
        registry.shutdown();
        assert!(matches!(
            registry.consumer(RuntimeConnectionId(3)),
            Err(ScopeError::Closed)
        ));
    }

    #[test]
    fn identifiers_do_not_wrap_or_repeat_across_registries() {
        let exhausted = AtomicU64::new(u64::MAX);
        assert_eq!(mint_id(&exhausted), Err(ScopeError::CounterExhausted));
        let first = ViewScopeRegistry::default()
            .consumer(RuntimeConnectionId(1))
            .unwrap()
            .open()
            .unwrap();
        let second = ViewScopeRegistry::default()
            .consumer(RuntimeConnectionId(1))
            .unwrap()
            .open()
            .unwrap();
        assert_ne!(first.id, second.id);
    }
}
