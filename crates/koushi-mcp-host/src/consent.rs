//! In-app approval of MCP assistants (#1536).
//!
//! A tool call that needs a permission the grant store does not hold becomes a
//! pending approval, keyed by (client name, account, permission), and waits
//! for the user's answer in Koushi. Allow writes exactly that permission to
//! the store; Deny is remembered for the rest of this bridge run.
//!
//! The broker is process-wide and toolkit-independent. A host publishes its
//! state through [`ConsentBroker::subscribe`] and forwards the user's answers;
//! it holds no grant logic itself. Lock discipline: one `std::sync::Mutex` for
//! broker state with no await and no file I/O under it, and one async writer
//! lock that serializes every write of `connections.json`.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use koushi_mcp::wire::{ErrorCode, WireError};
use serde::Serialize;
use tokio::sync::{oneshot, watch};
use tokio::time::Instant;

use crate::grants::{self, Need, Store, StoreStatus};
use crate::registration::{self, Registration};

/// How long a tool call waits for the user before answering
/// `consent_required`. The launcher allows 70 s per call and Codex's default
/// tool timeout is 60 s.
pub const APPROVAL_WAIT: Duration = Duration::from_secs(40);
/// How long an unanswered approval stays on screen.
pub const APPROVAL_LIFETIME: Duration = Duration::from_secs(5 * 60);
/// Pending approvals one client name may hold at once.
pub const PENDING_PER_CLIENT: usize = 4;
/// Pending approvals across all clients.
pub const PENDING_TOTAL: usize = 16;
/// Characters of an outgoing message shown as approval context.
pub const PREVIEW_CHARS: usize = 280;
/// Client names remembered for their version and last use.
const SEEN_LIMIT: usize = 64;
/// Granularity of the "last used" time, so routine calls do not republish.
const LAST_USED_GRANULARITY_MS: u64 = 30_000;

/// Whether this host can ask the user.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConsentMode {
    /// Missing permissions become pending approvals answered in the app.
    Interactive,
    /// No consent surface: a missing permission fails at once with
    /// `consent_required` and nothing is created. Used by headless hosts.
    FailClosed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BridgeStatus {
    Disabled,
    Running,
    StartFailed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LauncherUnavailable {
    /// This build does not contain the launcher.
    NotBundled,
    /// macOS runs the app from a disk image or a translocated copy, so the
    /// launcher path would change; Koushi must be moved to Applications.
    Translocated,
    /// The launcher exists only at a temporary path (an AppImage mount) and
    /// could not be copied to a stable one.
    Ephemeral,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum LauncherStatus {
    Available { path: String },
    Unavailable { reason: LauncherUnavailable },
}

/// The assistant-declared identity from the connection handshake. It is not
/// verified: any process of the same OS user can claim any name.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ClientIdentity {
    pub name: String,
    pub version: String,
}

/// What the dialog shows besides the client, account, and permission.
#[derive(Clone, Default, Eq, PartialEq)]
pub struct ConsentContext {
    /// The target room's label, only when the room resolved to a joined room.
    pub room_label: Option<String>,
    /// The beginning of the message a send is waiting to deliver.
    pub message_preview: Option<String>,
}

impl std::fmt::Debug for ConsentContext {
    // The room label and message are user content; only their presence shows.
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ConsentContext")
            .field("room_label", &self.room_label.is_some())
            .field("message_preview", &self.message_preview.is_some())
            .finish()
    }
}

#[derive(Clone, Eq, PartialEq, Serialize)]
pub struct PendingApproval {
    pub request_id: u64,
    pub client_name: String,
    pub client_version: String,
    pub account_id: String,
    pub need: Need,
    pub room_label: Option<String>,
    pub message_preview: Option<String>,
    /// Whether a tool call is still waiting for this answer.
    pub waiting: bool,
    /// How many tool calls wait for this answer. Allow releases all of them;
    /// the preview shows the most recent one.
    pub waiting_calls: usize,
}

#[derive(Clone, Eq, PartialEq, Serialize)]
pub struct AssistantAccount {
    pub account_id: String,
    pub read: bool,
    pub send: bool,
    pub attachments: bool,
    /// Permissions denied for this account during this bridge run.
    pub denied: Vec<Need>,
}

#[derive(Clone, Eq, PartialEq, Serialize)]
pub struct AssistantView {
    pub client_name: String,
    pub client_version: Option<String>,
    pub last_used_ms: Option<u64>,
    pub accounts: Vec<AssistantAccount>,
}

/// Everything the settings page and the approval dialog render.
#[derive(Clone, Eq, PartialEq, Serialize)]
pub struct McpAssistantsState {
    pub bridge: BridgeStatus,
    pub store: StoreStatus,
    pub launcher: LauncherStatus,
    pub pending: Vec<PendingApproval>,
    pub assistants: Vec<AssistantView>,
    pub registrations: Vec<Registration>,
    /// Client names whose launcher speaks an older bridge protocol, so they
    /// must be registered again (or the Claude Desktop extension recreated).
    pub outdated_launchers: Vec<String>,
}

impl std::fmt::Debug for McpAssistantsState {
    // Names, account ids, room labels, and previews stay out of logs.
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("McpAssistantsState")
            .field("bridge", &self.bridge)
            .field("store", &self.store)
            .field(
                "launcher",
                &matches!(self.launcher, LauncherStatus::Available { .. }),
            )
            .field("pending", &self.pending.len())
            .field("assistants", &self.assistants.len())
            .finish()
    }
}

/// The result of answering an approval.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AnswerOutcome {
    Applied,
    /// The request was already answered, expired, or never existed.
    Stale,
    /// Allow could not be saved; the request stays pending.
    StoreFailed,
}

/// A permission change from the settings page that could not be saved.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StoreError(pub StoreStatus);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Outcome {
    Allowed,
    Denied,
    StoreFailed,
    Unavailable,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct Key {
    client: String,
    account: String,
    need: Need,
}

struct Pending {
    request_id: u64,
    key: Key,
    client_version: String,
    room_label: Option<String>,
    message_preview: Option<String>,
    expires_at: Instant,
    waiters: usize,
    senders: Vec<oneshot::Sender<Outcome>>,
}

impl Pending {
    fn resolve(&mut self, outcome: Outcome) {
        for sender in self.senders.drain(..) {
            let _ = sender.send(outcome);
        }
    }
}

struct Seen {
    version: String,
    last_used_ms: u64,
}

struct Inner {
    next_request_id: u64,
    pending: Vec<Pending>,
    denied: HashSet<Key>,
    seen: HashMap<String, Seen>,
    outdated: std::collections::BTreeSet<String>,
    bridge: BridgeStatus,
    launcher: LauncherStatus,
    store_status: StoreStatus,
    store: Store,
    /// Bumped by every write of the store. A read that started before a write
    /// must not replace what that write installed (a revocation would briefly
    /// come undone).
    store_generation: u64,
}

impl Inner {
    /// Adopts a store read that began at `generation`, unless a write landed
    /// since. Returns whether the visible state changed.
    fn adopt(&mut self, generation: u64, loaded: grants::Loaded) -> bool {
        if self.store_generation != generation {
            return false;
        }
        let changed = self.store_status != loaded.status || self.store != loaded.store;
        self.store_status = loaded.status;
        self.store = loaded.store;
        changed
    }

    /// Installs what a successful write produced.
    fn written(&mut self, store: Store) {
        self.store_generation += 1;
        self.store_status = StoreStatus::Ok;
        self.store = store;
    }
}

/// The process-wide approval broker.
pub struct ConsentBroker {
    data_dir: PathBuf,
    mode: ConsentMode,
    inner: Mutex<Inner>,
    writer: tokio::sync::Mutex<()>,
    state: watch::Sender<McpAssistantsState>,
}

impl ConsentBroker {
    pub fn new(data_dir: PathBuf, mode: ConsentMode) -> Arc<Self> {
        let loaded = grants::load(&data_dir);
        let inner = Inner {
            next_request_id: 1,
            pending: Vec::new(),
            denied: HashSet::new(),
            seen: HashMap::new(),
            outdated: std::collections::BTreeSet::new(),
            bridge: BridgeStatus::Disabled,
            launcher: LauncherStatus::Unavailable {
                reason: LauncherUnavailable::NotBundled,
            },
            store_status: loaded.status,
            store: loaded.store,
            store_generation: 0,
        };
        let (state, _) = watch::channel(view(&inner));
        Arc::new(Self {
            data_dir,
            mode,
            inner: Mutex::new(inner),
            writer: tokio::sync::Mutex::new(()),
            state,
        })
    }

    pub fn mode(&self) -> ConsentMode {
        self.mode
    }

    /// The grant store as it is on disk now.
    pub(crate) fn store(&self) -> Store {
        grants::load(&self.data_dir).store
    }

    /// The current state. Every change is also published here, in order.
    pub fn subscribe(&self) -> watch::Receiver<McpAssistantsState> {
        self.state.subscribe()
    }

    /// Re-reads the grant store (a person may have edited it) and returns the
    /// refreshed state.
    pub fn refresh(&self) -> McpAssistantsState {
        let generation = self.lock().store_generation;
        let loaded = grants::load(&self.data_dir);
        self.update(|inner| {
            inner.adopt(generation, loaded);
        })
    }

    /// Records a launcher that authenticated but speaks another bridge
    /// protocol. The name is cleared once that client completes a handshake on
    /// the current protocol. Bounded like the other per-name memory.
    pub fn note_outdated_launcher(&self, client_name: &str) {
        self.update(|inner| {
            if inner.outdated.len() < SEEN_LIMIT || inner.outdated.contains(client_name) {
                inner.outdated.insert(client_name.to_owned());
            }
        });
    }

    /// A client completed a handshake on the current protocol.
    pub fn note_current_launcher(&self, client_name: &str) {
        let mut inner = self.lock();
        if inner.outdated.remove(client_name) {
            self.publish(&inner);
        }
    }

    pub fn set_bridge_status(&self, bridge: BridgeStatus) {
        self.update(|inner| inner.bridge = bridge);
    }

    pub fn set_launcher(&self, launcher: LauncherStatus) {
        self.update(|inner| inner.launcher = launcher);
    }

    /// The bridge stopped or was turned off: every waiting call answers
    /// `koushi_unavailable`, open approvals close, and denials are forgotten.
    pub fn reset(&self) {
        self.update(|inner| {
            for mut pending in inner.pending.drain(..) {
                pending.resolve(Outcome::Unavailable);
            }
            inner.denied.clear();
        });
    }

    /// Closes approvals that outlived [`APPROVAL_LIFETIME`].
    pub fn sweep(&self) {
        let now = Instant::now();
        let mut inner = self.lock();
        let before = inner.pending.len();
        inner.pending.retain_mut(|pending| {
            let live = pending.expires_at > now;
            if !live {
                pending.resolve(Outcome::Unavailable);
            }
            live
        });
        if inner.pending.len() != before {
            self.publish(&inner);
        }
    }

    /// Checks a permission, asking the user when the store lacks it.
    ///
    /// The caller has already resolved `account_id` to a live account.
    pub async fn authorize(
        &self,
        client: &ClientIdentity,
        account_id: &str,
        need: Need,
        context: ConsentContext,
    ) -> Result<(), WireError> {
        let generation = self.lock().store_generation;
        let loaded = grants::load(&self.data_dir);
        let key = Key {
            client: client.name.clone(),
            account: account_id.to_owned(),
            need,
        };
        let receiver = {
            let mut inner = self.lock();
            let mut changed = inner.adopt(generation, loaded);
            changed |= note_use(&mut inner, client);
            if inner.store.allows(&key.client, &key.account, need) {
                if changed {
                    self.publish(&inner);
                }
                return Ok(());
            }
            if self.mode == ConsentMode::FailClosed {
                return Err(consent_required(client, need));
            }
            if inner.denied.contains(&key) {
                if changed {
                    self.publish(&inner);
                }
                return Err(WireError::new(
                    ErrorCode::NotPermitted,
                    format!(
                        "{} was denied {} access in Koushi",
                        client.name,
                        need_word(need)
                    ),
                ));
            }
            let (sender, receiver) = oneshot::channel();
            let now = Instant::now();
            let room_label = context.room_label.map(|label| sanitize(&label, 120));
            let message_preview = context
                .message_preview
                .map(|text| sanitize(&text, PREVIEW_CHARS));
            let request_id = match inner
                .pending
                .iter_mut()
                .find(|pending| pending.key == key && pending.expires_at > now)
            {
                Some(pending) => {
                    // A retry or a second call joins; the dialog shows what
                    // the newest caller is waiting to do.
                    pending.waiters += 1;
                    pending.senders.push(sender);
                    pending.room_label = room_label;
                    pending.message_preview = message_preview;
                    pending.request_id
                }
                None => {
                    let per_client = inner
                        .pending
                        .iter()
                        .filter(|pending| pending.key.client == key.client)
                        .count();
                    if per_client >= PENDING_PER_CLIENT || inner.pending.len() >= PENDING_TOTAL {
                        if changed {
                            self.publish(&inner);
                        }
                        return Err(WireError::new(
                            ErrorCode::Busy,
                            "too many approvals are waiting in Koushi",
                        ));
                    }
                    let request_id = inner.next_request_id;
                    inner.next_request_id += 1;
                    inner.pending.push(Pending {
                        request_id,
                        key: key.clone(),
                        client_version: client.version.clone(),
                        room_label,
                        message_preview,
                        expires_at: now + APPROVAL_LIFETIME,
                        waiters: 1,
                        senders: vec![sender],
                    });
                    request_id
                }
            };
            self.publish(&inner);
            Waiter {
                broker: self,
                request_id,
                receiver,
            }
        };
        receiver.wait(client, need).await
    }

    /// Answers a pending approval by its id.
    pub async fn answer(&self, request_id: u64, allow: bool) -> AnswerOutcome {
        // Deny is serialized with writes too, so an Allow and a Deny for the
        // same request cannot both apply.
        let _writer = self.writer.lock().await;
        if !allow {
            let mut inner = self.lock();
            let Some(index) = live_index(&inner, request_id) else {
                return AnswerOutcome::Stale;
            };
            let mut pending = inner.pending.remove(index);
            pending.resolve(Outcome::Denied);
            inner.denied.insert(pending.key);
            self.publish(&inner);
            return AnswerOutcome::Applied;
        }

        let key = {
            let inner = self.lock();
            match live_index(&inner, request_id) {
                Some(index) => inner.pending[index].key.clone(),
                None => return AnswerOutcome::Stale,
            }
        };
        let saved = self.write_store(|store| store.set(&key.client, &key.account, key.need, true));
        let mut inner = self.lock();
        let index = live_index(&inner, request_id);
        match saved {
            Ok(store) => {
                inner.written(store);
                inner.denied.remove(&key);
                // Settle every approval for this key, including one created
                // after the answered one expired during the write.
                inner.pending.retain_mut(|pending| {
                    let settled = pending.key == key;
                    if settled {
                        pending.resolve(Outcome::Allowed);
                    }
                    !settled
                });
                self.publish(&inner);
                AnswerOutcome::Applied
            }
            Err(status) => {
                inner.store_status = status;
                if let Some(index) = index {
                    inner.pending[index].resolve(Outcome::StoreFailed);
                }
                self.publish(&inner);
                AnswerOutcome::StoreFailed
            }
        }
    }

    /// Turns one permission on or off from the settings page. Turning it on
    /// also clears a remembered denial and settles a matching approval.
    pub async fn set_permission(
        &self,
        client_name: &str,
        account_id: &str,
        need: Need,
        allowed: bool,
    ) -> Result<(), StoreError> {
        let _writer = self.writer.lock().await;
        let saved = self.write_store(|store| store.set(client_name, account_id, need, allowed));
        let key = Key {
            client: client_name.to_owned(),
            account: account_id.to_owned(),
            need,
        };
        let mut inner = self.lock();
        match saved {
            Ok(store) => {
                inner.written(store);
                if allowed {
                    inner.denied.remove(&key);
                    if let Some(index) = inner.pending.iter().position(|pending| pending.key == key)
                    {
                        inner.pending.remove(index).resolve(Outcome::Allowed);
                    }
                }
                self.publish(&inner);
                Ok(())
            }
            Err(status) => {
                inner.store_status = status;
                self.publish(&inner);
                Err(StoreError(status))
            }
        }
    }

    /// Removes an assistant entirely: its grants, its open approvals, and its
    /// remembered denials.
    pub async fn remove_client(&self, client_name: &str) -> Result<(), StoreError> {
        let _writer = self.writer.lock().await;
        let saved = self.write_store(|store| {
            store.remove(client_name);
        });
        let mut inner = self.lock();
        match saved {
            Ok(store) => {
                inner.written(store);
                inner.denied.retain(|key| key.client != client_name);
                inner.pending.retain_mut(|pending| {
                    let keep = pending.key.client != client_name;
                    if !keep {
                        pending.resolve(Outcome::Denied);
                    }
                    keep
                });
                inner.seen.remove(client_name);
                self.publish(&inner);
                Ok(())
            }
            Err(status) => {
                inner.store_status = status;
                self.publish(&inner);
                Err(StoreError(status))
            }
        }
    }

    /// Reads, changes, and writes the store. Called with the writer lock held
    /// and the state lock released.
    fn write_store(&self, change: impl FnOnce(&mut Store)) -> Result<Store, StoreStatus> {
        let mut loaded = grants::load(&self.data_dir);
        if !loaded.writable() {
            return Err(loaded.status);
        }
        change(&mut loaded.store);
        grants::save(&self.data_dir, &loaded.store).map_err(|_| StoreStatus::WriteFailed)?;
        Ok(loaded.store)
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        // A panic under the lock leaves plain data; keep serving.
        self.inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn update(&self, change: impl FnOnce(&mut Inner)) -> McpAssistantsState {
        let mut inner = self.lock();
        change(&mut inner);
        self.publish(&inner)
    }

    fn publish(&self, inner: &Inner) -> McpAssistantsState {
        let state = view(inner);
        self.state.send_if_modified(|current| {
            if *current == state {
                false
            } else {
                *current = state.clone();
                true
            }
        });
        state
    }

    /// Called when a waiting call goes away (answered, timed out, or its
    /// connection closed). The approval stays open; its preview is cleared
    /// once nobody waits for that message any more.
    fn release(&self, request_id: u64) {
        let mut inner = self.lock();
        if let Some(pending) = inner
            .pending
            .iter_mut()
            .find(|pending| pending.request_id == request_id)
        {
            pending.waiters = pending.waiters.saturating_sub(1);
            if pending.waiters == 0 {
                pending.message_preview = None;
            }
            self.publish(&inner);
        }
    }
}

/// A call waiting for an answer. Dropping it (a timeout, or the connection
/// closing mid-wait) releases its place without running the tool.
struct Waiter<'a> {
    broker: &'a ConsentBroker,
    request_id: u64,
    receiver: oneshot::Receiver<Outcome>,
}

impl Waiter<'_> {
    async fn wait(mut self, client: &ClientIdentity, need: Need) -> Result<(), WireError> {
        let outcome = tokio::time::timeout(APPROVAL_WAIT, &mut self.receiver).await;
        match outcome {
            Ok(Ok(Outcome::Allowed)) => Ok(()),
            Ok(Ok(Outcome::Denied)) => Err(WireError::new(
                ErrorCode::NotPermitted,
                format!(
                    "{} was denied {} access in Koushi",
                    client.name,
                    need_word(need)
                ),
            )),
            Ok(Ok(Outcome::StoreFailed)) => Err(WireError::new(
                ErrorCode::Internal,
                "Koushi could not save the approval",
            )),
            Ok(Ok(Outcome::Unavailable)) | Ok(Err(_)) => Err(WireError::new(
                ErrorCode::KoushiUnavailable,
                "the Koushi bridge stopped before the approval was answered",
            )),
            Err(_) => Err(WireError::new(
                ErrorCode::ConsentRequired,
                format!(
                    "waiting for approval in Koushi timed out; approve {} for {} in Koushi, then retry",
                    need_word(need),
                    client.name
                ),
            )),
        }
    }
}

impl Drop for Waiter<'_> {
    fn drop(&mut self) {
        self.broker.release(self.request_id);
    }
}

fn consent_required(client: &ClientIdentity, need: Need) -> WireError {
    WireError::new(
        ErrorCode::ConsentRequired,
        format!(
            "no Koushi grant gives {} {} access",
            client.name,
            need_word(need)
        ),
    )
}

fn need_word(need: Need) -> &'static str {
    match need {
        Need::Read => "read",
        Need::Send => "send",
        Need::Attachments => "attachment",
        Need::MessageActions => "message actions",
    }
}

fn live_index(inner: &Inner, request_id: u64) -> Option<usize> {
    let now = Instant::now();
    inner
        .pending
        .iter()
        .position(|pending| pending.request_id == request_id && pending.expires_at > now)
}

/// Records the client's version and last use. Returns whether the visible
/// state changed.
fn note_use(inner: &mut Inner, client: &ClientIdentity) -> bool {
    let now_ms = now_ms();
    match inner.seen.get_mut(&client.name) {
        Some(seen)
            if seen.version == client.version
                && now_ms.saturating_sub(seen.last_used_ms) < LAST_USED_GRANULARITY_MS =>
        {
            false
        }
        _ => {
            if inner.seen.len() >= SEEN_LIMIT && !inner.seen.contains_key(&client.name) {
                prune_seen(inner);
            }
            inner.seen.insert(
                client.name.clone(),
                Seen {
                    version: client.version.clone(),
                    last_used_ms: now_ms,
                },
            );
            true
        }
    }
}

/// Self-declared names are free to invent, so names that hold no grant, no
/// denial, and no approval are forgotten once the map is full.
fn prune_seen(inner: &mut Inner) {
    let Inner {
        seen,
        store,
        denied,
        pending,
        ..
    } = inner;
    seen.retain(|name, _| {
        store.client(name).is_some()
            || denied.iter().any(|key| &key.client == name)
            || pending.iter().any(|request| &request.key.client == name)
    });
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as u64)
        .unwrap_or(0)
}

/// Text the assistant chose, made safe to show: control, bidirectional,
/// invisible formatting, and tag characters removed (they could hide or
/// reorder text), length capped.
pub fn sanitize(text: &str, max_chars: usize) -> String {
    let mut out = String::new();
    let mut count = 0;
    for character in text.chars() {
        let formatting = matches!(
            character,
            // Zero-width joiners stay: emoji sequences and some scripts need them.
            '\u{200B}'
                | '\u{200E}'
                | '\u{200F}'
                | '\u{061C}'
                | '\u{2028}'..='\u{202E}'
                | '\u{2060}'..='\u{2069}'
                | '\u{FEFF}'
                | '\u{E0000}'..='\u{E007F}'
        );
        if formatting {
            continue;
        }
        let character = if character.is_control() {
            ' '
        } else {
            character
        };
        if count == max_chars {
            out.push('…');
            break;
        }
        out.push(character);
        count += 1;
    }
    out
}

fn view(inner: &Inner) -> McpAssistantsState {
    let pending = inner
        .pending
        .iter()
        .map(|pending| PendingApproval {
            request_id: pending.request_id,
            client_name: pending.key.client.clone(),
            client_version: pending.client_version.clone(),
            account_id: pending.key.account.clone(),
            need: pending.key.need,
            room_label: pending.room_label.clone(),
            message_preview: pending.message_preview.clone(),
            waiting: pending.waiters > 0,
            waiting_calls: pending.waiters,
        })
        .collect();

    let mut assistants: Vec<AssistantView> = Vec::new();
    let mut names: Vec<&str> = Vec::new();
    for grant in inner.store.clients() {
        if !names.contains(&grant.client_name.as_str()) {
            names.push(&grant.client_name);
        }
    }
    let mut denied: Vec<&Key> = inner.denied.iter().collect();
    denied.sort_by(|a, b| (&a.client, &a.account).cmp(&(&b.client, &b.account)));
    for key in &denied {
        if !names.contains(&key.client.as_str()) {
            names.push(&key.client);
        }
    }
    for name in names {
        let mut accounts: Vec<AssistantAccount> = inner
            .store
            .client(name)
            .map(|grant| {
                grant
                    .accounts
                    .iter()
                    .map(|account| AssistantAccount {
                        account_id: account.account_id.clone(),
                        read: account.read,
                        send: account.send,
                        attachments: account.attachments,
                        denied: Vec::new(),
                    })
                    .collect()
            })
            .unwrap_or_default();
        for key in denied.iter().filter(|key| key.client == name) {
            let row = match accounts
                .iter_mut()
                .position(|row| row.account_id == key.account)
            {
                Some(index) => &mut accounts[index],
                None => {
                    accounts.push(AssistantAccount {
                        account_id: key.account.clone(),
                        read: false,
                        send: false,
                        attachments: false,
                        denied: Vec::new(),
                    });
                    accounts.last_mut().expect("just pushed")
                }
            };
            row.denied.push(key.need);
        }
        for row in &mut accounts {
            row.denied
                .sort_by_key(|need| Need::ALL.iter().position(|n| n == need));
        }
        let seen = inner.seen.get(name);
        assistants.push(AssistantView {
            client_name: name.to_owned(),
            client_version: seen.map(|seen| seen.version.clone()),
            last_used_ms: seen.map(|seen| seen.last_used_ms),
            accounts,
        });
    }

    let registrations = match &inner.launcher {
        LauncherStatus::Available { path } => {
            registration::registrations(path, registration::Platform::current())
        }
        LauncherStatus::Unavailable { .. } => Vec::new(),
    };

    McpAssistantsState {
        bridge: inner.bridge,
        store: inner.store_status,
        launcher: inner.launcher.clone(),
        pending,
        assistants,
        registrations,
        outdated_launchers: inner.outdated.iter().cloned().collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inner(generation: u64) -> Inner {
        Inner {
            next_request_id: 1,
            pending: Vec::new(),
            denied: HashSet::new(),
            seen: HashMap::new(),
            outdated: std::collections::BTreeSet::new(),
            bridge: BridgeStatus::Running,
            launcher: LauncherStatus::Unavailable {
                reason: LauncherUnavailable::NotBundled,
            },
            store_status: StoreStatus::Ok,
            store: Store::default(),
            store_generation: generation,
        }
    }

    #[test]
    fn a_read_that_started_before_a_write_does_not_replace_it() {
        let mut granted = Store::default();
        granted.set("pi", "account:@alice:example.invalid", Need::Read, true);
        let stale = grants::Loaded {
            status: StoreStatus::Ok,
            store: granted.clone(),
        };

        // A revocation wrote generation 1 while this read was in flight.
        let mut state = inner(1);
        assert!(!state.adopt(0, stale.clone()));
        assert!(
            !state
                .store
                .allows("pi", "account:@alice:example.invalid", Need::Read)
        );

        // The same read with no intervening write is adopted.
        assert!(state.adopt(1, stale));
        assert!(
            state
                .store
                .allows("pi", "account:@alice:example.invalid", Need::Read)
        );
    }
}
