//! In-app approval of MCP assistants (#1536).
//!
//! A tool call that needs a permission the grant store does not hold becomes a
//! pending approval, keyed by (grant subject, account, permission), and waits
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
use crate::profiles::{self, CreatedProfile, ProfileError, ProfileGrants};
use crate::registration::{self, Registration};

pub use crate::subject::{ClientIdentity, ClientName, GrantSubject, Label, ProfileId};

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
    pub message_actions: bool,
    pub draft: bool,
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
    /// The profile was revoked while the call waited.
    Revoked,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct Key {
    subject: GrantSubject,
    account: String,
    need: Need,
}

struct Pending {
    request_id: u64,
    key: Key,
    /// Text for the dialog. It names the connection and keys nothing.
    label: String,
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
    seen: HashMap<GrantSubject, Seen>,
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
    /// One signal per profile with a live connection, fired when it is
    /// revoked. Entries are created by connections and removed by revocation.
    revocations: Mutex<HashMap<ProfileId, watch::Sender<bool>>>,
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
            revocations: Mutex::new(HashMap::new()),
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
        // A profile's grants are its own row, read here and never under the
        // state lock; a revoked profile or another account fails before any
        // approval can exist.
        let profile_grants = self.profile_grants(client.subject(), account_id)?;
        let key = Key {
            subject: client.subject().clone(),
            account: account_id.to_owned(),
            need,
        };
        let receiver = {
            let mut inner = self.lock();
            let mut changed = inner.adopt(generation, loaded);
            changed |= note_use(&mut inner, client);
            let granted = match &key.subject {
                GrantSubject::Client(name) => inner.store.allows(name.as_str(), &key.account, need),
                GrantSubject::Profile(_) => {
                    profile_grants.is_some_and(|grants| grants.allows(need))
                }
            };
            if granted {
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
                        client.label(),
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
                        .filter(|pending| pending.key.subject == key.subject)
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
                        label: sanitize(&client.label().to_string(), 120),
                        client_version: client.version().to_owned(),
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
        let saved = self.write_grant(&key.subject, &key.account, key.need, true);
        let mut inner = self.lock();
        let index = live_index(&inner, request_id);
        match saved {
            Ok(written) => {
                if let Some(store) = written {
                    inner.written(store);
                }
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
        subject: &GrantSubject,
        account_id: &str,
        need: Need,
        allowed: bool,
    ) -> Result<(), StoreError> {
        let _writer = self.writer.lock().await;
        let saved = self.write_grant(subject, account_id, need, allowed);
        let key = Key {
            subject: subject.clone(),
            account: account_id.to_owned(),
            need,
        };
        let mut inner = self.lock();
        match saved {
            Ok(written) => {
                if let Some(store) = written {
                    inner.written(store);
                }
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

    /// Removes a subject's approvals, remembered denials and last-use record,
    /// and, for an unprofiled client, its grants. A profile's row is not touched
    /// here: revoking a profile is [`ConsentBroker::revoke_profile`].
    pub async fn remove_subject(&self, subject: &GrantSubject) -> Result<(), StoreError> {
        let _writer = self.writer.lock().await;
        let saved = match subject {
            GrantSubject::Client(name) => self
                .write_store(|store| {
                    store.remove(name.as_str());
                })
                .map(Some),
            GrantSubject::Profile(_) => Ok(None),
        };
        let mut inner = self.lock();
        match saved {
            Ok(written) => {
                if let Some(store) = written {
                    inner.written(store);
                }
                inner.denied.retain(|key| &key.subject != subject);
                inner.pending.retain_mut(|pending| {
                    let keep = &pending.key.subject != subject;
                    if !keep {
                        pending.resolve(Outcome::Denied);
                    }
                    keep
                });
                inner.seen.remove(subject);
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

    /// Writes one grant to where the subject's grants live. An unprofiled
    /// client's store is returned so the caller installs it; a profile's row
    /// is its own file and has nothing to install.
    fn write_grant(
        &self,
        subject: &GrantSubject,
        account_id: &str,
        need: Need,
        allowed: bool,
    ) -> Result<Option<Store>, StoreStatus> {
        match subject {
            GrantSubject::Client(name) => self
                .write_store(|store| store.set(name.as_str(), account_id, need, allowed))
                .map(Some),
            GrantSubject::Profile(id) => {
                profiles::set_grant(&self.data_dir, id, account_id, need, allowed)
                    .map(|()| None)
                    .map_err(|error| match error {
                        ProfileError::StoreUnusable(status) => status,
                        _ => StoreStatus::WriteFailed,
                    })
            }
        }
    }

    /// The grants of a profile subject for `account_id`, `None` for an
    /// unprofiled client. A deleted row is `unauthorized/profile_revoked`;
    /// another account is `not_permitted/profile_account`.
    fn profile_grants(
        &self,
        subject: &GrantSubject,
        account_id: &str,
    ) -> Result<Option<ProfileGrants>, WireError> {
        let GrantSubject::Profile(id) = subject else {
            return Ok(None);
        };
        let row = match profiles::lookup_checked(&self.data_dir, id) {
            profiles::Lookup::Found(row) => row,
            profiles::Lookup::Gone => return Err(profile_revoked()),
            profiles::Lookup::Unusable => return Err(profile_store_unusable()),
        };
        if row.account_id != account_id {
            return Err(profile_account());
        }
        Ok(Some(row.grants))
    }

    /// The permissions a subject holds for an account, as the store has them
    /// now. A client reads `connections.json`; a profile reads its row, and
    /// only for its bound account. Never a path from one to the other.
    pub(crate) fn permissions(&self, subject: &GrantSubject, account_id: &str) -> ProfileGrants {
        match subject {
            GrantSubject::Client(name) => self
                .store()
                .account(name.as_str(), account_id)
                .map(|grant| ProfileGrants {
                    read: grant.read,
                    send: grant.send,
                    attachments: grant.attachments,
                    message_actions: grant.message_actions,
                    draft: grant.draft,
                })
                .unwrap_or_default(),
            GrantSubject::Profile(_) => self
                .profile_grants(subject, account_id)
                .ok()
                .flatten()
                .unwrap_or_default(),
        }
    }

    /// Whether the subject holds `need` for the account right now, without
    /// asking.
    pub(crate) fn allows(&self, subject: &GrantSubject, account_id: &str, need: Need) -> bool {
        self.permissions(subject, account_id).allows(need)
    }

    /// Creates a profile bound to an agent account (owner path). The secret is
    /// written once to the returned file and nowhere else.
    pub async fn create_profile(
        &self,
        accounts: &dyn crate::BridgeAccounts,
        label: &str,
        account_id: &str,
        grants: ProfileGrants,
    ) -> Result<CreatedProfile, ProfileError> {
        let Some(account) = accounts
            .accounts()
            .into_iter()
            .find(|account| account.id == account_id)
        else {
            return Err(ProfileError::AccountUnavailable);
        };
        if !account.connection.snapshot().is_agent_account() {
            return Err(ProfileError::NotAgentAccount);
        }
        let baseline = match accounts.mention_feed(account_id) {
            Some(feed) => match feed.wait(None, Duration::ZERO, 1).await {
                Ok(page) => baseline_from_cursor(page.cursor),
                Err(_) => None,
            },
            None => None,
        };
        let _writer = self.writer.lock().await;
        // The store lock can wait on another process; never on this thread.
        let data_dir = self.data_dir.clone();
        let label = label.to_owned();
        let account_id = account_id.to_owned();
        tokio::task::spawn_blocking(move || {
            profiles::create(
                &data_dir,
                &label,
                &account_id,
                grants,
                baseline,
                profiles::now_ms(),
            )
        })
        .await
        .map_err(|_| ProfileError::Io)?
    }

    /// Revokes a profile (owner path): the row goes first, the secret file
    /// after, then every live connection of it is signalled and its approvals
    /// end as `unauthorized/profile_revoked`. A send already on the wire
    /// cannot be recalled.
    pub async fn revoke_profile(&self, id: &ProfileId) -> Result<bool, ProfileError> {
        let _writer = self.writer.lock().await;
        let removed = {
            let data_dir = self.data_dir.clone();
            let id = id.clone();
            tokio::task::spawn_blocking(move || profiles::revoke(&data_dir, &id))
                .await
                .map_err(|_| ProfileError::Io)??
        };
        if let Some(signal) = self
            .revocations
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(id)
        {
            let _ = signal.send(true);
        }
        let subject = GrantSubject::Profile(id.clone());
        let mut inner = self.lock();
        inner.denied.retain(|key| key.subject != subject);
        inner.pending.retain_mut(|pending| {
            let keep = pending.key.subject != subject;
            if !keep {
                pending.resolve(Outcome::Revoked);
            }
            keep
        });
        inner.seen.remove(&subject);
        self.publish(&inner);
        Ok(removed)
    }

    /// When a connection last presented this profile with a wrong secret.
    pub fn profile_last_rejected_ms(&self, id: &ProfileId) -> Option<u64> {
        crate::profile_state::load(&self.data_dir, id).last_rejected_ms
    }

    /// A signal that fires when the profile is revoked. Held by each live
    /// connection of the profile.
    pub(crate) fn watch_revocation(&self, id: &ProfileId) -> watch::Receiver<bool> {
        self.revocations
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .entry(id.clone())
            .or_insert_with(|| watch::channel(false).0)
            .subscribe()
    }

    pub(crate) fn data_dir(&self) -> &std::path::Path {
        &self.data_dir
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
            Ok(Ok(Outcome::Revoked)) => Err(profile_revoked()),
            Ok(Ok(Outcome::Denied)) => Err(WireError::new(
                ErrorCode::NotPermitted,
                format!(
                    "{} was denied {} access in Koushi",
                    client.label(),
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
                    client.label()
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
            client.label(),
            need_word(need)
        ),
    )
}

/// The error for a profile whose row is gone.
pub(crate) fn profile_revoked() -> WireError {
    WireError::with_reason(
        ErrorCode::Unauthorized,
        "profile_revoked",
        "this agent profile was revoked in Koushi",
    )
}

/// The error for a live profile connection when the profile store cannot be
/// read. Not a revocation: the row may well still exist.
pub(crate) fn profile_store_unusable() -> WireError {
    WireError::with_reason(
        ErrorCode::Unauthorized,
        "profile_store_unusable",
        "Koushi cannot read the agent profile store right now",
    )
}

/// The one error for any account other than the profile's own. Identical for
/// an account that exists and one that does not.
pub(crate) fn profile_account() -> WireError {
    WireError::with_reason(
        ErrorCode::NotPermitted,
        "profile_account",
        "this agent profile can use only its own account",
    )
}

/// The creation baseline of a profile from the feed's head cursor
/// (`mf1.<generation>.<seq>`).
fn baseline_from_cursor(cursor: String) -> Option<crate::profile_state::Baseline> {
    let generation = cursor.split('.').nth(1)?.to_owned();
    Some(crate::profile_state::Baseline {
        feed_generation: generation,
        head_cursor: cursor,
    })
}

fn need_word(need: Need) -> &'static str {
    match need {
        Need::Read => "read",
        Need::Send => "send",
        Need::Attachments => "attachment",
        Need::MessageActions => "message actions",
        Need::Draft => "draft",
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
    match inner.seen.get_mut(client.subject()) {
        Some(seen)
            if seen.version == client.version()
                && now_ms.saturating_sub(seen.last_used_ms) < LAST_USED_GRANULARITY_MS =>
        {
            false
        }
        _ => {
            if inner.seen.len() >= SEEN_LIMIT && !inner.seen.contains_key(client.subject()) {
                prune_seen(inner);
            }
            inner.seen.insert(
                client.subject().clone(),
                Seen {
                    version: client.version().to_owned(),
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
    seen.retain(|subject, _| {
        matches!(subject, GrantSubject::Client(name) if store.client(name.as_str()).is_some())
            || denied.iter().any(|key| &key.subject == subject)
            || pending
                .iter()
                .any(|request| &request.key.subject == subject)
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
            client_name: pending.label.clone(),
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
    // Profiles are listed by the profile section, not as assistants by name.
    let mut denied: Vec<(&str, &Key)> = inner
        .denied
        .iter()
        .filter_map(|key| match &key.subject {
            GrantSubject::Client(name) => Some((name.as_str(), key)),
            GrantSubject::Profile(_) => None,
        })
        .collect();
    denied.sort_by(|a, b| (a.0, &a.1.account).cmp(&(b.0, &b.1.account)));
    for (client, _) in &denied {
        if !names.contains(client) {
            names.push(client);
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
                        message_actions: account.message_actions,
                        draft: account.draft,
                        denied: Vec::new(),
                    })
                    .collect()
            })
            .unwrap_or_default();
        for (_, key) in denied.iter().filter(|(client, _)| *client == name) {
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
                        message_actions: false,
                        draft: false,
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
        let seen = inner.seen.get(&GrantSubject::client(name));
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
