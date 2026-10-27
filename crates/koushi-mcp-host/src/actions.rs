//! Idempotency ledger for MCP message actions (#1628).
//!
//! `client_request_id` is the caller's retry key. The SDK send queue mints its
//! own transaction id, and an edit has no queue entry at all, so nothing
//! downstream can deduplicate a retry. This ledger remembers, per (client,
//! account, target, kind, client_request_id), whether an identical request is
//! in flight or already settled, and answers a repeat without submitting
//! again.
//!
//! It holds the tool's own result object: no message body, reaction key, or
//! event content is stored, and entries expire. It is process-local, so a
//! fresh bridge run starts empty; a retry after a restart may therefore act
//! again, which the tool descriptions say.

use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// How long a settled or in-flight request is remembered.
pub(crate) const LEDGER_TTL: Duration = Duration::from_secs(600);

/// Maximum remembered requests across all clients.
pub(crate) const LEDGER_LIMIT: usize = 128;

#[derive(Clone)]
enum Entry {
    /// Claimed and still running. Never forgotten or evicted, because forgetting
    /// it would let an identical retry act a second time.
    InFlight,
    Settled {
        at: Instant,
        result: serde_json::Value,
    },
}

/// What a caller should do about a repeated `client_request_id`.
pub(crate) enum Replay {
    /// The same request already settled; return this result again.
    Settled(serde_json::Value),
    /// The same request is still running.
    InFlight,
    /// Nothing remembered, so the caller may act.
    Fresh,
}

#[derive(Default)]
pub(crate) struct ActionLedger {
    entries: Mutex<LedgerState>,
}

#[derive(Default)]
struct LedgerState {
    entries: HashMap<String, Entry>,
    order: VecDeque<String>,
}

impl ActionLedger {
    pub(crate) fn key(
        client: &str,
        account_id: &str,
        target: &str,
        kind: &str,
        client_request_id: &str,
    ) -> String {
        format!("{client}\u{1f}{account_id}\u{1f}{target}\u{1f}{kind}\u{1f}{client_request_id}")
    }

    /// Looks a request up and, when it is fresh, claims it as in flight.
    pub(crate) fn begin(&self, key: String) -> Replay {
        let mut state = self.entries.lock().expect("action ledger lock");
        state.expire();
        match state.entries.get(&key) {
            Some(Entry::Settled { result, .. }) => Replay::Settled(result.clone()),
            Some(Entry::InFlight) => Replay::InFlight,
            None => {
                state.insert(key, Entry::InFlight);
                Replay::Fresh
            }
        }
    }

    /// Records the settled result of a claimed request.
    pub(crate) fn settle(&self, key: &str, result: serde_json::Value) {
        let mut state = self.entries.lock().expect("action ledger lock");
        state.expire();
        if state.entries.contains_key(key) {
            state.entries.insert(
                key.to_owned(),
                Entry::Settled {
                    at: Instant::now(),
                    result,
                },
            );
        }
    }

    /// Releases a claimed request that did not settle, so a retry may act.
    pub(crate) fn release(&self, key: &str) {
        let mut state = self.entries.lock().expect("action ledger lock");
        if matches!(state.entries.get(key), Some(Entry::InFlight)) {
            state.entries.remove(key);
            state.order.retain(|existing| existing != key);
        }
    }
}

impl LedgerState {
    fn expire(&mut self) {
        let now = Instant::now();
        self.entries.retain(|_key, entry| {
            // A settled request forgets after the TTL; a still-running one is
            // never forgotten, because forgetting it would let an identical
            // retry act a second time.
            match entry {
                Entry::InFlight => true,
                Entry::Settled { at, .. } => now.duration_since(*at) < LEDGER_TTL,
            }
        });
        self.order.retain(|key| self.entries.contains_key(key));
    }

    fn insert(&mut self, key: String, entry: Entry) {
        self.entries.insert(key.clone(), entry);
        self.order.push_back(key);
        while self.entries.len() > LEDGER_LIMIT {
            let Some(oldest) = self.order.pop_front() else {
                break;
            };
            // Never evict a request that is still running.
            if matches!(self.entries.get(&oldest), Some(Entry::InFlight)) {
                self.order.push_back(oldest);
                continue;
            }
            self.entries.remove(&oldest);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_settled_request_replays_its_result_and_a_release_frees_a_failed_one() {
        let ledger = ActionLedger::default();
        let key = ActionLedger::key("pi", "account:@a:x", "!room", "edit", "uuid-1");

        assert!(matches!(ledger.begin(key.clone()), Replay::Fresh));
        assert!(
            matches!(ledger.begin(key.clone()), Replay::InFlight),
            "a second identical call must not submit while the first is running"
        );
        ledger.settle(&key, serde_json::json!({"edit_event_id": "$edit"}));
        match ledger.begin(key.clone()) {
            Replay::Settled(result) => assert_eq!(result["edit_event_id"], "$edit"),
            _ => panic!("a settled request replays its result"),
        }

        let other = ActionLedger::key("pi", "account:@a:x", "!room", "edit", "uuid-2");
        assert!(matches!(ledger.begin(other.clone()), Replay::Fresh));
        ledger.release(&other);
        assert!(
            matches!(ledger.begin(other), Replay::Fresh),
            "a request that did not settle may be retried"
        );
    }

    #[test]
    fn the_ledger_is_bounded_and_another_client_does_not_replay() {
        let ledger = ActionLedger::default();
        for index in 0..LEDGER_LIMIT + 8 {
            let key = ActionLedger::key("pi", "account:@a:x", "!room", "reply", &index.to_string());
            assert!(matches!(ledger.begin(key.clone()), Replay::Fresh));
            ledger.settle(&key, serde_json::json!({"index": index}));
        }
        {
            let state = ledger.entries.lock().expect("lock");
            assert!(state.entries.len() <= LEDGER_LIMIT);
        }

        let mine = ActionLedger::key("pi", "account:@a:x", "!room", "reply", "uuid-9");
        ledger.settle(&mine, serde_json::json!({}));
        let other_client = ActionLedger::key("codex", "account:@a:x", "!room", "reply", "uuid-9");
        assert!(
            matches!(ledger.begin(other_client), Replay::Fresh),
            "one client's retry key must not settle another client's call"
        );
    }
}
