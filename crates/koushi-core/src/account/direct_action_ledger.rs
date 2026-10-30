//! The retry ledger for direct message actions (#1628, #1681, #1660).
//!
//! An out-of-band caller's `client_request_id` is its retry key. The SDK send
//! queue mints its own transaction id, and an edit has no queue entry at all,
//! so nothing downstream can deduplicate a retry. The account actor keeps this
//! ledger, which remembers per (client, action, room, target,
//! `client_request_id`) whether an identical request is running, settled, or
//! ended ambiguously, and answers a repeat without acting again.
//!
//! It holds ids and the action's own result only. Every entry ages out (the
//! rule is in overview "The message-action retry key is Core retry policy"), and
//! the ledger lives with the account runtime, so a retry after a restart or a
//! sign-out may act again.

use std::collections::{HashMap, VecDeque};
use std::future::Future;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::direct_operations::{DirectMessageFailure, DirectMessageRequest, DirectMessageResponse};
use crate::executor::Instant;

/// How long a settled or ambiguous request is remembered.
pub(super) const LEDGER_TTL: Duration = Duration::from_secs(600);

/// Maximum remembered requests per account.
pub(super) const LEDGER_LIMIT: usize = 128;

// A live claim holds one of the account's direct-action slots, so live claims
// alone can never fill the ledger.
const _: () = assert!(LEDGER_LIMIT > super::actor::DIRECT_MESSAGE_ACTION_LIMIT);

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
enum ActionKind {
    Edit,
    ThreadReply,
}

/// The identity of one retryable request on this account.
#[derive(Clone, Eq, Hash, PartialEq)]
pub(super) struct DirectActionKey {
    client: String,
    kind: ActionKind,
    room_id: String,
    target_event_id: String,
    client_request_id: String,
}

impl std::fmt::Debug for DirectActionKey {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("DirectActionKey")
            .field("kind", &self.kind)
            .finish_non_exhaustive()
    }
}

impl DirectActionKey {
    /// The ledger key of a request that carries a retry key; reads, reactions,
    /// and deletes are idempotent on their own and carry none.
    pub(super) fn for_request(request: &DirectMessageRequest) -> Option<Self> {
        let (kind, room_id, target_event_id, retry) = match request {
            DirectMessageRequest::Edit {
                room_id,
                event_id,
                retry,
                ..
            } => (ActionKind::Edit, room_id, event_id, retry),
            DirectMessageRequest::ThreadReply {
                room_id,
                root_event_id,
                retry,
                ..
            } => (ActionKind::ThreadReply, room_id, root_event_id, retry),
            _ => return None,
        };
        let retry = retry.as_ref()?;
        Some(Self {
            client: retry.client.clone(),
            kind,
            room_id: room_id.clone(),
            target_event_id: target_event_id.clone(),
            client_request_id: retry.client_request_id.clone(),
        })
    }
}

/// Proves which claim a completion belongs to, so a late completion of a claim
/// that already aged out cannot overwrite a newer claim of the same key.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct ClaimToken(u64);

/// What the actor does with a request that carries a retry key.
#[derive(Debug)]
pub(super) enum Begin {
    /// Nothing is remembered; the request was claimed and may act.
    Fresh(ClaimToken),
    /// The same request already settled; answer with its result again.
    Replay(Box<DirectMessageResponse>),
    /// The same request is running or ended ambiguously; do not act.
    Pending,
    /// Every remembered entry still guards a retry that may not act again, so
    /// nothing was claimed.
    Full,
}

enum EntryState {
    Running,
    Settled(Box<DirectMessageResponse>),
    Ambiguous,
}

struct Entry {
    state: EntryState,
    token: ClaimToken,
    expires_at: Instant,
}

pub(super) struct DirectActionLedger {
    entries: HashMap<DirectActionKey, Entry>,
    order: VecDeque<DirectActionKey>,
    /// The bound of one action, after which an unsettled claim is ambiguous.
    action_deadline: Duration,
    next_token: u64,
}

impl DirectActionLedger {
    pub(super) fn new(action_deadline: Duration) -> Self {
        Self {
            entries: HashMap::new(),
            order: VecDeque::new(),
            action_deadline,
            next_token: 0,
        }
    }

    /// Looks a request up and, when nothing is remembered, claims it.
    pub(super) fn begin(&mut self, key: DirectActionKey, now: Instant) -> Begin {
        self.expire(now);
        match self.entries.get(&key).map(|entry| &entry.state) {
            Some(EntryState::Settled(response)) => return Begin::Replay(response.clone()),
            Some(EntryState::Running | EntryState::Ambiguous) => return Begin::Pending,
            None => {}
        }
        if self.entries.len() >= LEDGER_LIMIT && !self.evict_oldest_settled() {
            return Begin::Full;
        }
        let token = ClaimToken(self.next_token);
        self.next_token += 1;
        let deadline = now + self.action_deadline;
        self.entries.insert(
            key.clone(),
            Entry {
                state: EntryState::Running,
                token,
                expires_at: deadline + LEDGER_TTL,
            },
        );
        self.order.push_back(key);
        Begin::Fresh(token)
    }

    /// Records how a claimed request ended. Success is replayed, `Unavailable`
    /// (which includes the action deadline) may have reached the homeserver
    /// and is answered `Pending`, and any other failure submitted nothing, so
    /// the key is free at once.
    pub(super) fn finish(
        &mut self,
        key: &DirectActionKey,
        token: ClaimToken,
        outcome: &Result<DirectMessageResponse, DirectMessageFailure>,
        now: Instant,
    ) {
        let Some(entry) = self.entries.get_mut(key) else {
            return;
        };
        if entry.token != token {
            return;
        }
        match outcome {
            Ok(response) => {
                entry.state = EntryState::Settled(Box::new(response.clone()));
                entry.expires_at = now + LEDGER_TTL;
            }
            Err(DirectMessageFailure::Unavailable) => {
                entry.state = EntryState::Ambiguous;
                entry.expires_at = now + LEDGER_TTL;
            }
            Err(_) => self.remove(key),
        }
    }

    fn expire(&mut self, now: Instant) {
        self.entries.retain(|_key, entry| now < entry.expires_at);
        let entries = &self.entries;
        self.order.retain(|key| entries.contains_key(key));
    }

    /// Forgets the oldest settled request, whose retry would then act again,
    /// which is the cost of the bound. A running or ambiguous entry is never
    /// forgotten before it expires, because that retry could send twice.
    fn evict_oldest_settled(&mut self) -> bool {
        let entries = &self.entries;
        let Some(position) = self.order.iter().position(|key| {
            entries
                .get(key)
                .is_some_and(|entry| matches!(entry.state, EntryState::Settled(_)))
        }) else {
            return false;
        };
        if let Some(oldest) = self.order.remove(position) {
            self.entries.remove(&oldest);
        }
        true
    }

    fn remove(&mut self, key: &DirectActionKey) {
        self.entries.remove(key);
        self.order.retain(|existing| existing != key);
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.entries.len()
    }
}

/// A claimed request and the token its completion must present.
pub(super) struct Claim {
    key: DirectActionKey,
    token: ClaimToken,
}

/// A request admitted to run on one of the account's direct-action slots.
pub(super) struct Admitted {
    pub(super) permit: tokio::sync::OwnedSemaphorePermit,
    pub(super) claim: Option<Claim>,
}

type DirectOutcome = Result<DirectMessageResponse, DirectMessageFailure>;

/// Admits one direct message request, or answers it at once.
///
/// The ledger is consulted before a slot is taken, so a settled retry replays
/// even while the account's slots are busy; a fresh claim that then finds no
/// slot is released, so it never outlives this call.
pub(super) fn admit(
    ledger: &Mutex<DirectActionLedger>,
    slots: &Arc<tokio::sync::Semaphore>,
    request: &DirectMessageRequest,
    now: Instant,
) -> Result<Admitted, Box<DirectOutcome>> {
    let claim = match DirectActionKey::for_request(request) {
        None => None,
        Some(key) => {
            let begin = ledger
                .lock()
                .expect("direct action ledger mutex")
                .begin(key.clone(), now);
            match begin {
                Begin::Fresh(token) => Some(Claim { key, token }),
                Begin::Replay(response) => return Err(Box::new(Ok(*response))),
                Begin::Pending => {
                    return Err(Box::new(Err(DirectMessageFailure::DuplicatePending)));
                }
                Begin::Full => return Err(Box::new(Err(DirectMessageFailure::Capacity))),
            }
        }
    };
    match Arc::clone(slots).try_acquire_owned() {
        Ok(permit) => Ok(Admitted { permit, claim }),
        Err(_) => {
            if let Some(claim) = claim {
                ledger.lock().expect("direct action ledger mutex").finish(
                    &claim.key,
                    claim.token,
                    &Err(DirectMessageFailure::Capacity),
                    now,
                );
            }
            Err(Box::new(Err(DirectMessageFailure::Capacity)))
        }
    }
}

/// Runs an admitted request under its deadline and settles its claim with the
/// outcome, whether or not the caller is still waiting. A task aborted before
/// it settles leaves the claim to age out.
pub(super) async fn run_claimed<F>(
    ledger: &Mutex<DirectActionLedger>,
    claim: Option<Claim>,
    deadline: Duration,
    action: F,
) -> DirectOutcome
where
    F: Future<Output = DirectOutcome>,
{
    let outcome = crate::executor::timeout(deadline, action)
        .await
        .unwrap_or(Err(DirectMessageFailure::Unavailable));
    if let Some(claim) = claim {
        ledger.lock().expect("direct action ledger mutex").finish(
            &claim.key,
            claim.token,
            &outcome,
            Instant::now(),
        );
    }
    outcome
}

#[cfg(test)]
#[path = "direct_action_ledger_tests.rs"]
mod tests;
