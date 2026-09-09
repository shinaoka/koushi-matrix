use std::sync::{Arc, Mutex};

use koushi_state::VIEW_SCOPE_CAPACITY as MAX_SCOPES;
const MAX_BYTES: usize = 256 * 1024 * 1024;

/// Shared admission ledger for charged model/raw/builder/control data, not a heap estimate.
#[derive(Clone, Default)]
pub(crate) struct ViewBudget(Arc<Mutex<Usage>>);

#[derive(Default)]
struct Usage {
    scopes: usize,
    bytes: usize,
}

/// Move into the retained artifact; release only when that artifact's last owner drops.
pub(crate) struct ViewReservation {
    budget: ViewBudget,
    scope: bool,
    bytes: usize,
}

impl ViewBudget {
    pub(crate) fn reserve_scope(&self, control_bytes: usize) -> Option<ViewReservation> {
        self.reserve(true, control_bytes)
    }

    pub(crate) fn reserve_bytes(&self, bytes: usize) -> Option<ViewReservation> {
        self.reserve(false, bytes)
    }

    fn reserve(&self, scope: bool, bytes: usize) -> Option<ViewReservation> {
        let mut usage = self.0.lock().expect("view budget poisoned");
        if scope && usage.scopes == MAX_SCOPES {
            return None;
        }
        let total = usage.bytes.checked_add(bytes)?;
        if total > MAX_BYTES {
            return None;
        }
        usage.bytes = total;
        usage.scopes += usize::from(scope);
        Some(ViewReservation {
            budget: self.clone(),
            scope,
            bytes,
        })
    }
}

impl ViewReservation {
    pub(crate) fn bytes(&self) -> usize {
        self.bytes
    }

    /// Transfer part of an admitted builder charge without a release/reacquire gap.
    pub(crate) fn split_bytes(&mut self, bytes: usize) -> Option<Self> {
        self.bytes = self.bytes.checked_sub(bytes)?;
        Some(Self {
            budget: self.budget.clone(),
            scope: false,
            bytes,
        })
    }

    /// Adjust a builder's reservation without releasing its old charge on failure.
    pub(crate) fn resize_bytes(&mut self, bytes: usize) -> bool {
        let mut usage = self.budget.0.lock().expect("view budget poisoned");
        let Some(total) = (usage.bytes - self.bytes).checked_add(bytes) else {
            return false;
        };
        if total > MAX_BYTES {
            return false;
        }
        usage.bytes = total;
        self.bytes = bytes;
        true
    }
}

impl Drop for ViewReservation {
    fn drop(&mut self) {
        let mut usage = self.budget.0.lock().expect("view budget poisoned");
        usage.scopes -= usize::from(self.scope);
        usage.bytes -= self.bytes;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reservations_remain_charged_until_released_and_resize_is_atomic() {
        let budget = ViewBudget::default();
        let scopes: Vec<_> = (0..64).map(|_| budget.reserve_scope(1).unwrap()).collect();
        assert!(budget.reserve_scope(1).is_none());
        let mut payload = budget.reserve_bytes(256 * 1024 * 1024 - 64).unwrap();
        assert!(budget.reserve_bytes(1).is_none());
        assert!(!payload.resize_bytes(usize::MAX));
        assert!(budget.reserve_bytes(1).is_none());
        assert!(payload.resize_bytes(32));
        let remaining = budget.reserve_bytes(256 * 1024 * 1024 - 32 - 64).unwrap();
        let held = std::sync::Arc::new(payload);
        let transferred = held.clone();
        drop(held);
        assert!(budget.reserve_bytes(1).is_none());
        drop(transferred);
        let restored = budget.reserve_bytes(32).unwrap();
        drop((restored, remaining, scopes));
        assert!(budget.reserve_scope(1).is_some());
        assert!(budget.reserve_bytes(256 * 1024 * 1024).is_some());
    }
}
