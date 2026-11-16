use std::collections::BTreeMap;

use crate::{DiagnosticEvent, DiagnosticField, record};

/// Owner-held state that records a keyed diagnostic only when its fields
/// differ from the last record made for the same key (#1915).
///
/// High-volume per-entity traces (for example one unread record per room per
/// room-list snapshot) would otherwise evict every other record from the
/// bounded ring within seconds. The key identifies the entity in memory only;
/// it is never recorded, so it may be a raw identifier.
///
/// One entry per key ever recorded by its owner until the owner `forget`s it
/// or is replaced: a session-owned ledger is bounded by the rooms that session
/// traced, not by the rooms that still exist.
#[derive(Default)]
pub struct DiagnosticChangeLedger {
    last_fields: BTreeMap<String, Vec<DiagnosticField>>,
}

impl DiagnosticChangeLedger {
    /// Record `event` unless the last event recorded for `key` had the same
    /// fields.
    pub fn record_if_changed(&mut self, key: &str, event: DiagnosticEvent) {
        if self
            .last_fields
            .get(key)
            .is_some_and(|fields| *fields == event.fields)
        {
            return;
        }
        self.last_fields
            .insert(key.to_owned(), event.fields.clone());
        record(event);
    }

    /// Forget `key`, so its next event is recorded even if it repeats the
    /// last one. Owners call this when the entity leaves the traced set: a
    /// later return is a new transition worth recording.
    pub fn forget(&mut self, key: &str) {
        self.last_fields.remove(key);
    }
}
