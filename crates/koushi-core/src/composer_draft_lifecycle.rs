use std::{
    collections::{BTreeSet, HashMap},
    fmt,
    sync::{Arc, Mutex, Weak},
};

use koushi_key::SessionKeyId;
use koushi_state::ComposerTarget;
use tokio::sync::watch;

#[derive(Clone, Eq, Hash, PartialEq)]
pub struct ComposerDraftScope {
    pub account: SessionKeyId,
    pub target: ComposerTarget,
}

impl fmt::Debug for ComposerDraftScope {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ComposerDraftScope")
            .field("target_kind", &target_kind(&self.target))
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Copy, Eq, Hash, PartialEq)]
pub struct ComposerRendererGeneration(u64);

impl fmt::Debug for ComposerRendererGeneration {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("ComposerRendererGeneration(..)")
    }
}

#[derive(Clone, Copy, Eq, Hash, PartialEq)]
pub struct ComposerDraftLeaseId(u64);

impl fmt::Debug for ComposerDraftLeaseId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("ComposerDraftLeaseId(..)")
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ComposerDraftLeaseFailure {
    CounterExhausted,
    RendererGenerationRetired,
    LeaseMismatch,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ComposerDraftPermitKind {
    Command,
    Persistence,
}

struct ComposerDraftPermitGuard {
    registry: Weak<ComposerDraftLeaseRegistry>,
    scope: ComposerDraftScope,
    generation: ComposerRendererGeneration,
    kind: ComposerDraftPermitKind,
}

impl fmt::Debug for ComposerDraftPermitGuard {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ComposerDraftPermitGuard")
            .field("target_kind", &target_kind(&self.scope.target))
            .field("generation", &self.generation)
            .field("kind", &self.kind)
            .finish_non_exhaustive()
    }
}

impl Drop for ComposerDraftPermitGuard {
    fn drop(&mut self) {
        if let Some(registry) = self.registry.upgrade() {
            registry.release_permit(&self.scope, self.kind);
        }
    }
}

#[derive(Clone)]
pub struct ComposerDraftCommandPermit {
    guard: Arc<ComposerDraftPermitGuard>,
}

impl fmt::Debug for ComposerDraftCommandPermit {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ComposerDraftCommandPermit")
            .field("kind", &self.guard.kind)
            .finish_non_exhaustive()
    }
}

#[derive(Clone)]
pub struct ComposerDraftPersistencePermit {
    guard: Arc<ComposerDraftPermitGuard>,
}

impl fmt::Debug for ComposerDraftPersistencePermit {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ComposerDraftPersistencePermit")
            .field("kind", &self.guard.kind)
            .finish_non_exhaustive()
    }
}

#[derive(Clone)]
struct LeaseRecord {
    generation: ComposerRendererGeneration,
    scope: ComposerDraftScope,
}

#[derive(Default)]
struct ComposerDraftLeaseRegistryState {
    next_generation: u64,
    next_lease_id: u64,
    live_generation: Option<ComposerRendererGeneration>,
    leases: HashMap<ComposerDraftLeaseId, LeaseRecord>,
    command_permits: HashMap<ComposerDraftScope, usize>,
    persistence_permits: HashMap<ComposerDraftScope, usize>,
    change_generation: u64,
}

pub struct ComposerDraftLeaseRegistry {
    state: Mutex<ComposerDraftLeaseRegistryState>,
    changes: watch::Sender<u64>,
}

impl Default for ComposerDraftLeaseRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for ComposerDraftLeaseRegistry {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let state = self.state.lock().map_err(|_| fmt::Error)?;
        formatter
            .debug_struct("ComposerDraftLeaseRegistry")
            .field("has_live_generation", &state.live_generation.is_some())
            .field("lease_count", &state.leases.len())
            .field("command_target_count", &state.command_permits.len())
            .field("persistence_target_count", &state.persistence_permits.len())
            .finish()
    }
}

impl ComposerDraftLeaseRegistry {
    pub fn new() -> Self {
        let (changes, _) = watch::channel(0);
        Self {
            state: Mutex::new(ComposerDraftLeaseRegistryState::default()),
            changes,
        }
    }

    pub fn subscribe(&self) -> watch::Receiver<u64> {
        self.changes.subscribe()
    }

    pub fn begin_renderer_generation(
        &self,
    ) -> Result<ComposerRendererGeneration, ComposerDraftLeaseFailure> {
        let mut state = self.state.lock().expect("composer lease registry mutex");
        let next = state
            .next_generation
            .checked_add(1)
            .ok_or(ComposerDraftLeaseFailure::CounterExhausted)?;
        state.next_generation = next;
        let generation = ComposerRendererGeneration(next);
        state.live_generation = Some(generation);
        state
            .leases
            .retain(|_, lease| lease.generation == generation);
        Self::notify_locked(&mut state, &self.changes)?;
        Ok(generation)
    }

    pub fn acquire(
        self: &Arc<Self>,
        generation: ComposerRendererGeneration,
        scope: ComposerDraftScope,
    ) -> Result<ComposerDraftLeaseId, ComposerDraftLeaseFailure> {
        let mut state = self.state.lock().expect("composer lease registry mutex");
        if state.live_generation != Some(generation) {
            return Err(ComposerDraftLeaseFailure::RendererGenerationRetired);
        }
        let next = state
            .next_lease_id
            .checked_add(1)
            .ok_or(ComposerDraftLeaseFailure::CounterExhausted)?;
        state.next_lease_id = next;
        let lease_id = ComposerDraftLeaseId(next);
        state
            .leases
            .insert(lease_id, LeaseRecord { generation, scope });
        Self::notify_locked(&mut state, &self.changes)?;
        Ok(lease_id)
    }

    pub fn try_command_permit(
        self: &Arc<Self>,
        generation: ComposerRendererGeneration,
        lease_id: ComposerDraftLeaseId,
        scope: &ComposerDraftScope,
    ) -> Result<ComposerDraftCommandPermit, ComposerDraftLeaseFailure> {
        let mut state = self.state.lock().expect("composer lease registry mutex");
        if state.live_generation != Some(generation) {
            return Err(ComposerDraftLeaseFailure::RendererGenerationRetired);
        }
        let Some(lease) = state.leases.get(&lease_id) else {
            return Err(ComposerDraftLeaseFailure::LeaseMismatch);
        };
        if lease.generation != generation || lease.scope != *scope {
            return Err(ComposerDraftLeaseFailure::LeaseMismatch);
        }
        increment_permit(&mut state.command_permits, scope)?;
        Self::notify_locked(&mut state, &self.changes)?;
        Ok(ComposerDraftCommandPermit {
            guard: Arc::new(ComposerDraftPermitGuard {
                registry: Arc::downgrade(self),
                scope: scope.clone(),
                generation,
                kind: ComposerDraftPermitKind::Command,
            }),
        })
    }

    pub fn release(
        &self,
        generation: ComposerRendererGeneration,
        lease_id: ComposerDraftLeaseId,
    ) -> Result<(), ComposerDraftLeaseFailure> {
        let mut state = self.state.lock().expect("composer lease registry mutex");
        let Some(lease) = state.leases.get(&lease_id) else {
            return Err(ComposerDraftLeaseFailure::LeaseMismatch);
        };
        if lease.generation != generation {
            return Err(ComposerDraftLeaseFailure::LeaseMismatch);
        }
        state.leases.remove(&lease_id);
        Self::notify_locked(&mut state, &self.changes)
    }

    pub fn revoke_generation(&self, generation: ComposerRendererGeneration) {
        let mut state = self.state.lock().expect("composer lease registry mutex");
        if state.live_generation == Some(generation) {
            state.live_generation = None;
        }
        state
            .leases
            .retain(|_, lease| lease.generation != generation);
        let _ = Self::notify_locked(&mut state, &self.changes);
    }

    pub(crate) fn revoke_live_generation(&self) {
        let mut state = self.state.lock().expect("composer lease registry mutex");
        let Some(generation) = state.live_generation.take() else {
            return;
        };
        // Activation records gate new admission only. Already-admitted command
        // and persistence guards live in their independent permit maps and
        // remain protected until the last corresponding RAII guard drops.
        state
            .leases
            .retain(|_, lease| lease.generation != generation);
        let _ = Self::notify_locked(&mut state, &self.changes);
    }

    pub fn persistence_permits(
        self: &Arc<Self>,
        account: &SessionKeyId,
        targets: impl IntoIterator<Item = ComposerTarget>,
    ) -> Result<Vec<ComposerDraftPersistencePermit>, ComposerDraftLeaseFailure> {
        let mut state = self.state.lock().expect("composer lease registry mutex");
        let scopes = targets
            .into_iter()
            .map(|target| ComposerDraftScope {
                account: account.clone(),
                target,
            })
            .collect::<Vec<_>>();
        for (index, scope) in scopes.iter().enumerate() {
            if let Err(error) = increment_permit(&mut state.persistence_permits, scope) {
                for rollback in &scopes[..index] {
                    decrement_permit(&mut state.persistence_permits, rollback);
                }
                return Err(error);
            }
        }
        Self::notify_locked(&mut state, &self.changes)?;
        Ok(scopes
            .into_iter()
            .map(|scope| ComposerDraftPersistencePermit {
                guard: Arc::new(ComposerDraftPermitGuard {
                    registry: Arc::downgrade(self),
                    scope,
                    generation: ComposerRendererGeneration(0),
                    kind: ComposerDraftPermitKind::Persistence,
                }),
            })
            .collect())
    }

    pub fn protected_targets(&self, account: &SessionKeyId) -> BTreeSet<ComposerTarget> {
        let state = self.state.lock().expect("composer lease registry mutex");
        state
            .leases
            .values()
            .map(|lease| &lease.scope)
            .chain(state.command_permits.keys())
            .chain(state.persistence_permits.keys())
            .filter(|scope| scope.account == *account)
            .map(|scope| scope.target.clone())
            .collect()
    }

    fn release_permit(&self, scope: &ComposerDraftScope, kind: ComposerDraftPermitKind) {
        let mut state = self.state.lock().expect("composer lease registry mutex");
        let permits = match kind {
            ComposerDraftPermitKind::Command => &mut state.command_permits,
            ComposerDraftPermitKind::Persistence => &mut state.persistence_permits,
        };
        if permits.contains_key(scope) {
            decrement_permit(permits, scope);
            let _ = Self::notify_locked(&mut state, &self.changes);
        }
    }

    fn notify_locked(
        state: &mut ComposerDraftLeaseRegistryState,
        changes: &watch::Sender<u64>,
    ) -> Result<(), ComposerDraftLeaseFailure> {
        state.change_generation = state
            .change_generation
            .checked_add(1)
            .ok_or(ComposerDraftLeaseFailure::CounterExhausted)?;
        changes.send_replace(state.change_generation);
        Ok(())
    }
}

fn increment_permit(
    permits: &mut HashMap<ComposerDraftScope, usize>,
    scope: &ComposerDraftScope,
) -> Result<(), ComposerDraftLeaseFailure> {
    let count = permits.entry(scope.clone()).or_default();
    *count = count
        .checked_add(1)
        .ok_or(ComposerDraftLeaseFailure::CounterExhausted)?;
    Ok(())
}

fn decrement_permit(permits: &mut HashMap<ComposerDraftScope, usize>, scope: &ComposerDraftScope) {
    if let Some(count) = permits.get_mut(scope) {
        *count = count.saturating_sub(1);
        if *count == 0 {
            permits.remove(scope);
        }
    }
}

fn target_kind(target: &ComposerTarget) -> &'static str {
    match target {
        ComposerTarget::Main { .. } => "main",
        ComposerTarget::Thread { .. } => "thread",
    }
}
