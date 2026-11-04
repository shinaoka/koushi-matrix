//! Test-only control for the sync observer's first-response handoff (#1634).
//!
//! The nightly aggregate lane could leave the app reporting `starting` while
//! the sync service kept committing, because the sync lifecycle used to be
//! closed by the room-list reconciliation. That reconciliation is an in-process
//! round trip to the RoomActor, so a test cannot make it hang from the
//! homeserver side without relying on an SDK race.
//!
//! This module holds that round trip open for one account runtime, so a test
//! can assert that a committed response alone closes the sync lifecycle. The
//! hold is keyed by the runtime's `SyncDiagnosticsOwner`, so tests that run
//! concurrently in one process do not observe each other's holds, and it is
//! absent from default builds.

#![cfg(any(test, feature = "test-hooks"))]

use std::{collections::BTreeMap, sync::Mutex};

use tokio::sync::oneshot;

use crate::SyncDiagnosticsOwner;

static HELD_RECONCILES: Mutex<BTreeMap<u64, oneshot::Receiver<()>>> = Mutex::new(BTreeMap::new());

fn held_reconciles() -> std::sync::MutexGuard<'static, BTreeMap<u64, oneshot::Receiver<()>>> {
    HELD_RECONCILES
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Hold every room-list reconciliation of `owner` until the returned sender is
/// used or dropped.
pub(crate) fn hold_live_room_list_reconcile(owner: SyncDiagnosticsOwner) -> oneshot::Sender<()> {
    let (release, held) = oneshot::channel();
    held_reconciles().insert(owner.get(), held);
    release
}

/// Wait while this runtime's room-list reconciliation is held. Returns
/// immediately when it is not.
pub(crate) async fn wait_for_held_live_room_list_reconcile(owner: SyncDiagnosticsOwner) {
    let held = held_reconciles().remove(&owner.get());
    if let Some(held) = held {
        let _ = held.await;
    }
}
