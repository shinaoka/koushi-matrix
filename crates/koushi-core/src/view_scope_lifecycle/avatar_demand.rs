use super::ScopeError;
use crate::view_budget::{ViewBudget, ViewReservation};
use koushi_state::{AvatarDemandContext, AvatarDemandError, AvatarDemandState};
use std::{collections::BTreeMap, ops::Deref, sync::Arc};

/// Scope payloads and their reservations travel together through the watch.
/// Cloning shares both; closing the current scope cannot uncharge an old value.
#[derive(Clone)]
pub(crate) struct ChargedAvatarDemand {
    state: AvatarDemandState,
    charges: BTreeMap<u64, Arc<ViewReservation>>,
}

impl Deref for ChargedAvatarDemand {
    type Target = AvatarDemandState;
    fn deref(&self) -> &Self::Target {
        &self.state
    }
}

fn reserve(
    state: &AvatarDemandState,
    scope: u64,
    budget: &ViewBudget,
) -> Result<Arc<ViewReservation>, ScopeError> {
    // Retained payload plus logical map-entry/context accounting, not a heap
    // estimate. Map cardinality is independently bounded by the scope budget.
    let bytes = state
        .scope_payload_bytes(scope)
        .and_then(|bytes| bytes.checked_add(2 * std::mem::size_of::<(u64, Arc<ViewReservation>)>()))
        .and_then(|bytes| bytes.checked_add(state.context().account_id.capacity()))
        .ok_or(ScopeError::Capacity)?;
    budget
        .reserve_bytes(bytes)
        .map(Arc::new)
        .ok_or(ScopeError::Capacity)
}

fn map_error(error: AvatarDemandError) -> ScopeError {
    match error {
        AvatarDemandError::SessionChanged => ScopeError::InactiveSession,
        AvatarDemandError::Closed => ScopeError::Closed,
        AvatarDemandError::StaleObservation => ScopeError::InvalidRevision,
        AvatarDemandError::Capacity => ScopeError::Capacity,
    }
}

impl ChargedAvatarDemand {
    pub(crate) fn refresh(
        &mut self,
        scope: u64,
        visible: Vec<Option<String>>,
        prefetch: Vec<Option<String>>,
        budget: &ViewBudget,
    ) -> Result<bool, ScopeError> {
        let mut next = self.state.clone();
        if !next.refresh(scope, visible, prefetch).map_err(map_error)? {
            return Ok(false);
        }
        let charge = reserve(&next, scope, budget)?;
        self.state = next;
        self.charges.insert(scope, charge);
        Ok(true)
    }
    pub(crate) fn new(state: AvatarDemandState, budget: &ViewBudget) -> Result<Self, ScopeError> {
        let mut charges = BTreeMap::new();
        for scope in state.scope_ids() {
            charges.insert(scope, reserve(&state, scope, budget)?);
        }
        Ok(Self { state, charges })
    }

    pub(crate) fn close(&mut self, scope: u64) -> bool {
        let changed = self.state.close(scope);
        self.charges.remove(&scope);
        changed
    }

    pub(crate) fn replace(
        &mut self,
        context: &AvatarDemandContext,
        scope: u64,
        revision: u64,
        visible: Vec<Option<String>>,
        prefetch: Vec<Option<String>>,
        budget: &ViewBudget,
    ) -> Result<(), ScopeError> {
        // Copy only the bounded scope map; unchanged payloads and charges share
        // their owners. Do not install a new revision until reservation succeeds.
        let mut next = self.state.clone();
        next.open(scope).map_err(map_error)?;
        next.replace(context, scope, revision, visible, prefetch)
            .map_err(map_error)?;
        let charge = reserve(&next, scope, budget)?;
        self.state = next;
        self.charges.insert(scope, charge);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use koushi_state::{AvatarDemandContext, AvatarDemandState};

    fn demand() -> AvatarDemandState {
        let context = AvatarDemandContext {
            account_id: "@synthetic:example.invalid".into(),
            session_generation: 1,
        };
        let mut state = AvatarDemandState::new(context.clone());
        state.open(1).unwrap();
        state
            .replace(
                &context,
                1,
                1,
                vec![Some("mxc://example.invalid/one".into())],
                vec![],
            )
            .unwrap();
        state
    }

    #[test]
    fn retired_payload_stays_charged_until_the_last_snapshot_releases_it() {
        let budget = crate::view_budget::ViewBudget::default();
        let original = std::sync::Arc::new(ChargedAvatarDemand::new(demand(), &budget).unwrap());
        let bytes: usize = original.charges.values().map(|charge| charge.bytes()).sum();
        let _remaining = budget.reserve_bytes(256 * 1024 * 1024 - bytes).unwrap();
        let mut next = original.as_ref().clone();
        next.close(1);
        assert!(next.resources_by_priority().is_empty());
        assert!(budget.reserve_bytes(1).is_none());
        drop(original);
        assert!(budget.reserve_bytes(bytes).is_some());
    }

    #[test]
    fn runtime_shutdown_releases_its_avatar_publication() {
        let registry = super::super::ViewScopeRegistry::default();
        let publication = ChargedAvatarDemand::new(demand(), &registry.budget).unwrap();
        let bytes: usize = publication
            .charges
            .values()
            .map(|charge| charge.bytes())
            .sum();
        registry.state.lock().unwrap().avatar_demand = Some(std::sync::Arc::new(publication));
        let _remaining = registry
            .budget
            .reserve_bytes(256 * 1024 * 1024 - bytes)
            .unwrap();
        registry.shutdown();
        assert!(
            registry.budget.reserve_bytes(bytes).is_some(),
            "stopped runtime must release its publication even while connection registry handles survive"
        );
    }

    #[test]
    fn failed_scope_replacement_preserves_demand_and_revision() {
        let budget = crate::view_budget::ViewBudget::default();
        let mut current = ChargedAvatarDemand::new(demand(), &budget).unwrap();
        let bytes: usize = current.charges.values().map(|charge| charge.bytes()).sum();
        let remaining = budget.reserve_bytes(256 * 1024 * 1024 - bytes).unwrap();
        let context = current.context().clone();
        assert_eq!(
            current.refresh(
                1,
                vec![Some("mxc://example.invalid/one".into())],
                vec![],
                &budget
            ),
            Ok(false)
        );
        assert_eq!(
            current.refresh(
                1,
                vec![Some("mxc://example.invalid/two".into())],
                vec![],
                &budget
            ),
            Err(ScopeError::Capacity)
        );
        assert_eq!(
            current.replace(
                &context,
                1,
                2,
                vec![Some("mxc://example.invalid/two".into())],
                vec![],
                &budget
            ),
            Err(super::super::ScopeError::Capacity)
        );
        assert_eq!(
            current.resources_by_priority(),
            ["mxc://example.invalid/one"]
        );
        drop(remaining);
        assert_eq!(
            current.refresh(
                1,
                vec![Some("mxc://example.invalid/two".into())],
                vec![],
                &budget
            ),
            Ok(true)
        );
        current
            .replace(
                &context,
                1,
                2,
                vec![Some("mxc://example.invalid/two".into())],
                vec![],
                &budget,
            )
            .unwrap();
        assert_eq!(
            current.resources_by_priority(),
            ["mxc://example.invalid/two"]
        );
    }
}
