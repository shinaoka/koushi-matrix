//! The profile store: `mcp/profiles.json` in the app data directory (#2132, D3).
//!
//! A profile is an owner-created identity bound to exactly one account. Its row
//! carries the binding (immutable), its grants, and the SHA-256 verifier of its
//! secret. The secret itself is written once to an `O_EXCL` `0600` file the
//! plugin reads and is never stored, logged, or returned.
//!
//! Only the low-frequency owner path writes this file: [`create`], [`revoke`]
//! and [`set_grant`], each a read-modify-write under an exclusive file lock that
//! re-reads inside the lock, so a stale writer in another process cannot
//! resurrect a revoked profile. No IPC method, the acknowledged cursor, and the
//! request path never write it.
//!
//! Like the grant store it fails closed: a missing, unreadable, malformed, or
//! newer file authorises nothing, and the app never writes over a file it could
//! not parse. Every request re-reads it, so a revocation applies to the next
//! request.

use std::path::{Path, PathBuf};

use koushi_diagnostics::{DiagnosticEvent, DiagnosticField, DiagnosticLevel};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};

use crate::endpoint;
use crate::grants::{Need, StoreStatus};
use crate::profile_state::{self, Baseline, ProfileState};
use crate::subject::ProfileId;

pub(crate) const FILE_NAME: &str = "profiles.json";
const LOCK_NAME: &str = "profiles.json.lock";
const SECRET_DIR: &str = "profiles";
const SCHEMA: u64 = 1;
/// Characters of owner text kept for a profile label.
const LABEL_CHARS: usize = 80;

/// The five permissions a profile can hold for its bound account.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ProfileGrants {
    pub read: bool,
    pub send: bool,
    pub attachments: bool,
    pub message_actions: bool,
    pub draft: bool,
}

impl ProfileGrants {
    pub fn allows(&self, need: Need) -> bool {
        match need {
            Need::Read => self.read,
            Need::Send => self.send,
            Need::Attachments => self.attachments,
            Need::MessageActions => self.message_actions,
            Need::Draft => self.draft,
        }
    }

    pub fn set(&mut self, need: Need, allowed: bool) {
        match need {
            Need::Read => self.read = allowed,
            Need::Send => self.send = allowed,
            Need::Attachments => self.attachments = allowed,
            Need::MessageActions => self.message_actions = allowed,
            Need::Draft => self.draft = allowed,
        }
    }

    pub fn any(&self) -> bool {
        Need::ALL.iter().any(|need| self.allows(*need))
    }
}

/// One stored profile. The verifier is not secret-equivalent but is still kept
/// out of `Debug`.
#[derive(Clone, PartialEq)]
pub(crate) struct Profile {
    pub(crate) id: ProfileId,
    pub(crate) label: String,
    pub(crate) account_id: String,
    pub(crate) grants: ProfileGrants,
    verifier: [u8; 32],
    pub(crate) created_at_ms: u64,
    extra: Map<String, Value>,
}

impl std::fmt::Debug for Profile {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Profile")
            .field("id", &self.id)
            .field("grants", &self.grants)
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Debug, PartialEq)]
enum Entry {
    Profile(Profile),
    /// A row with no usable profile id. It authorises nothing and is written
    /// back unchanged.
    Raw(Value),
}

/// The parsed file.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Store {
    entries: Vec<Entry>,
    extra: Map<String, Value>,
}

impl Store {
    pub(crate) fn profiles(&self) -> impl Iterator<Item = &Profile> {
        self.entries.iter().filter_map(|entry| match entry {
            Entry::Profile(profile) => Some(profile),
            Entry::Raw(_) => None,
        })
    }

    pub(crate) fn find(&self, id: &ProfileId) -> Option<&Profile> {
        self.profiles().find(|profile| &profile.id == id)
    }

    fn to_json(&self) -> Value {
        let mut root = self.extra.clone();
        root.insert("schema".to_owned(), json!(SCHEMA));
        let rows = self
            .entries
            .iter()
            .map(|entry| match entry {
                Entry::Raw(value) => value.clone(),
                Entry::Profile(profile) => {
                    let mut row = profile.extra.clone();
                    row.insert("profile_id".to_owned(), json!(profile.id.as_str()));
                    row.insert("label".to_owned(), json!(profile.label));
                    row.insert("account_id".to_owned(), json!(profile.account_id));
                    row.insert(
                        "grants".to_owned(),
                        json!({
                            "read": profile.grants.read,
                            "send": profile.grants.send,
                            "attachments": profile.grants.attachments,
                            "message_actions": profile.grants.message_actions,
                            "draft": profile.grants.draft,
                        }),
                    );
                    row.insert("secret_verifier".to_owned(), json!(hex(&profile.verifier)));
                    row.insert("created_at_ms".to_owned(), json!(profile.created_at_ms));
                    Value::Object(row)
                }
            })
            .collect();
        root.insert("profiles".to_owned(), Value::Array(rows));
        Value::Object(root)
    }
}

/// The result of reading the file.
#[derive(Clone, Debug)]
pub(crate) struct Loaded {
    pub(crate) status: StoreStatus,
    pub(crate) store: Store,
}

impl Loaded {
    pub(crate) fn writable(&self) -> bool {
        matches!(self.status, StoreStatus::Ok | StoreStatus::Missing)
    }
}

fn record(reason: &'static str) {
    koushi_diagnostics::record(
        DiagnosticEvent::new(DiagnosticLevel::Warn, "mcp.profiles", "configuration")
            .field(DiagnosticField::token("reason", reason)),
    );
}

pub(crate) fn profiles_path(data_dir: &Path) -> PathBuf {
    data_dir.join("mcp").join(FILE_NAME)
}

pub(crate) fn secret_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("mcp").join(SECRET_DIR)
}

/// Where a profile's secret file is.
pub(crate) fn secret_path(data_dir: &Path, id: &ProfileId) -> PathBuf {
    secret_dir(data_dir).join(format!("{}.secret", id.as_str()))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn unhex_32(text: &str) -> Option<[u8; 32]> {
    if text.len() != 64 || !text.is_ascii() {
        return None;
    }
    let mut out = [0u8; 32];
    for (index, slot) in out.iter_mut().enumerate() {
        *slot = u8::from_str_radix(&text[index * 2..index * 2 + 2], 16).ok()?;
    }
    Some(out)
}

fn digest(secret: &str) -> [u8; 32] {
    Sha256::digest(secret.as_bytes()).into()
}

/// Reads the store. Anything but a readable file of the known schema yields an
/// empty store and a status saying why.
pub(crate) fn load(data_dir: &Path) -> Loaded {
    let failed = |status: StoreStatus, reason: &'static str| {
        record(reason);
        Loaded {
            status,
            store: Store::default(),
        }
    };
    let bytes = match std::fs::read(profiles_path(data_dir)) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Loaded {
                status: StoreStatus::Missing,
                store: Store::default(),
            };
        }
        Err(_) => return failed(StoreStatus::Unreadable, "profiles_read_failed"),
    };
    let Ok(Value::Object(mut root)) = serde_json::from_slice::<Value>(&bytes) else {
        return failed(StoreStatus::Malformed, "profiles_parse_failed");
    };
    match root.remove("schema").and_then(|value| value.as_u64()) {
        Some(schema) if (1..=SCHEMA).contains(&schema) => {}
        Some(_) => {
            return failed(
                StoreStatus::UnsupportedSchema,
                "profiles_schema_unsupported",
            );
        }
        None => return failed(StoreStatus::Malformed, "profiles_parse_failed"),
    }
    let rows = match root.remove("profiles") {
        None => Vec::new(),
        Some(Value::Array(rows)) => rows,
        Some(_) => return failed(StoreStatus::Malformed, "profiles_parse_failed"),
    };
    let mut entries = Vec::with_capacity(rows.len());
    for row in rows {
        match parse_row(row) {
            Ok(entry) => entries.push(entry),
            Err(()) => return failed(StoreStatus::Malformed, "profiles_parse_failed"),
        }
    }
    Loaded {
        status: StoreStatus::Ok,
        store: Store {
            entries,
            extra: root,
        },
    }
}

fn take_flag(object: &mut Map<String, Value>, key: &str) -> Result<bool, ()> {
    match object.remove(key) {
        None => Ok(false),
        Some(Value::Bool(flag)) => Ok(flag),
        Some(_) => Err(()),
    }
}

fn parse_row(value: Value) -> Result<Entry, ()> {
    let Value::Object(mut row) = value else {
        return Err(());
    };
    let Some(id) = row
        .get("profile_id")
        .and_then(Value::as_str)
        .and_then(ProfileId::parse)
    else {
        record("profiles_entry_ignored");
        return Ok(Entry::Raw(Value::Object(row)));
    };
    row.remove("profile_id");
    let Some(Value::String(label)) = row.remove("label") else {
        return Err(());
    };
    let Some(Value::String(account_id)) = row.remove("account_id") else {
        return Err(());
    };
    if account_id.trim().is_empty() {
        return Err(());
    }
    let Some(Value::Object(mut flags)) = row.remove("grants") else {
        return Err(());
    };
    let grants = ProfileGrants {
        read: take_flag(&mut flags, "read")?,
        send: take_flag(&mut flags, "send")?,
        attachments: take_flag(&mut flags, "attachments")?,
        message_actions: take_flag(&mut flags, "message_actions")?,
        draft: take_flag(&mut flags, "draft")?,
    };
    let Some(verifier) = row
        .remove("secret_verifier")
        .and_then(|value| value.as_str().and_then(unhex_32))
    else {
        return Err(());
    };
    let Some(created_at_ms) = row.remove("created_at_ms").and_then(|value| value.as_u64()) else {
        return Err(());
    };
    Ok(Entry::Profile(Profile {
        id,
        label,
        account_id,
        grants,
        verifier,
        created_at_ms,
        extra: row,
    }))
}

/// Flushes a directory's entries (a rename, a create, an unlink) to disk, so a
/// power loss cannot roll back a revocation while keeping its secret file's
/// removal, or the reverse. A no-op where a directory cannot be opened.
fn sync_dir(directory: &Path) {
    #[cfg(unix)]
    if let Ok(handle) = std::fs::File::open(directory) {
        let _ = handle.sync_all();
    }
    #[cfg(not(unix))]
    let _ = directory;
}

/// Writes the store: a private temporary file in the same directory, then an
/// atomic rename over the old file.
fn save(data_dir: &Path, store: &Store) -> std::io::Result<()> {
    use std::io::Write;
    let dir = endpoint::ensure_dir(data_dir)?;
    let bytes = serde_json::to_vec_pretty(&store.to_json())
        .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?;
    // `tempfile` creates the file 0600 on Unix.
    let mut temporary = tempfile::NamedTempFile::new_in(&dir)?;
    temporary.write_all(&bytes)?;
    temporary.write_all(b"\n")?;
    temporary.as_file().sync_all()?;
    temporary
        .persist(profiles_path(data_dir))
        .map_err(|error| error.error)?;
    sync_dir(&dir);
    Ok(())
}

/// Why an owner action on the store did not happen.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProfileError {
    /// The file exists but could not be understood, so it is left untouched.
    StoreUnusable(StoreStatus),
    /// No profile has that id.
    NotFound,
    /// The grant named an account the profile is not bound to.
    AccountMismatch,
    /// The label is empty once cleaned.
    InvalidLabel,
    /// The account is not an agent account.
    NotAgentAccount,
    /// The account is not exposed by this host.
    AccountUnavailable,
    /// A file could not be written.
    Io,
}

/// A created profile. The secret is not here: it is in the file at
/// `secret_path`, which the plugin reads.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CreatedProfile {
    pub profile_id: ProfileId,
    pub secret_path: PathBuf,
}

/// How long an owner action waits for the store lock before giving up.
const LOCK_DEADLINE: std::time::Duration = std::time::Duration::from_secs(5);

/// Runs `change` holding the exclusive profile-store lock. The lock is a
/// separate file so the atomic rename of `profiles.json` never moves it. The
/// wait is bounded: another process holding the lock for longer than
/// [`LOCK_DEADLINE`] fails the action with `TimedOut` rather than hanging the
/// caller. The wait blocks the calling thread, so async callers run this in
/// `spawn_blocking`.
fn with_lock<T>(data_dir: &Path, change: impl FnOnce() -> T) -> std::io::Result<T> {
    let dir = endpoint::ensure_dir(data_dir)?;
    let lock = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(dir.join(LOCK_NAME))?;
    let started = std::time::Instant::now();
    loop {
        match lock.try_lock() {
            Ok(()) => break,
            Err(std::fs::TryLockError::WouldBlock) => {
                if started.elapsed() >= LOCK_DEADLINE {
                    return Err(std::io::ErrorKind::TimedOut.into());
                }
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            Err(std::fs::TryLockError::Error(error)) => return Err(error),
        }
    }
    let result = change();
    // Dropping the handle releases the lock; unlock first so it is explicit.
    let _ = lock.unlock();
    Ok(result)
}

/// A fresh secret file: `O_CREAT|O_EXCL` with mode `0600` at open time, in a
/// `0700` directory, written and synced. An existing path is refused.
fn write_secret_file(path: &Path, secret: &str) -> std::io::Result<()> {
    use std::io::Write;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path)?;
    let written = file
        .write_all(secret.as_bytes())
        .and_then(|()| file.write_all(b"\n"))
        .and_then(|()| file.sync_all());
    if written.is_err() {
        drop(file);
        let _ = std::fs::remove_file(path);
    } else if let Some(parent) = path.parent() {
        sync_dir(parent);
    }
    written
}

/// Creates a profile bound to `account_id`.
///
/// Everything after choosing the id runs under the store lock, so a sweep in
/// another process cannot remove the secret file between its write and the
/// row. The order inside is fixed: the secret file first (so no row can exist
/// without a secret the plugin can read), then the baseline, then the verifier
/// row. A crash after the file leaves an orphan that authenticates nothing and
/// [`sweep_orphans`] removes. A failure after the file removes it again.
pub(crate) fn create(
    data_dir: &Path,
    label: &str,
    account_id: &str,
    grants: ProfileGrants,
    baseline: Option<Baseline>,
    now_ms: u64,
) -> Result<CreatedProfile, ProfileError> {
    let label = crate::consent::sanitize(label.trim(), LABEL_CHARS);
    if label.trim().is_empty() {
        return Err(ProfileError::InvalidLabel);
    }
    let mut id_bytes = [0u8; 16];
    let mut secret_bytes = [0u8; 32];
    endpoint::fill_random(&mut id_bytes).map_err(|_| ProfileError::Io)?;
    endpoint::fill_random(&mut secret_bytes).map_err(|_| ProfileError::Io)?;
    let id = ProfileId::from_random(id_bytes);
    let secret = endpoint::base64url(&secret_bytes);

    with_lock(data_dir, || {
        let mut loaded = load(data_dir);
        if !loaded.writable() {
            return Err(ProfileError::StoreUnusable(loaded.status));
        }
        endpoint::ensure_private_dir(&secret_dir(data_dir)).map_err(|_| ProfileError::Io)?;
        let secret_file = secret_path(data_dir, &id);
        write_secret_file(&secret_file, &secret).map_err(|_| ProfileError::Io)?;

        let cleanup = || {
            let _ = std::fs::remove_file(secret_path(data_dir, &id));
            profile_state::remove(data_dir, &id);
        };
        if let Some(baseline) = baseline {
            let state = ProfileState {
                baseline: Some(baseline),
                ..ProfileState::default()
            };
            if profile_state::save(data_dir, &id, &state).is_err() {
                cleanup();
                return Err(ProfileError::Io);
            }
        }
        loaded.store.entries.push(Entry::Profile(Profile {
            id: id.clone(),
            label,
            account_id: account_id.to_owned(),
            grants,
            verifier: digest(&secret),
            created_at_ms: now_ms,
            extra: Map::new(),
        }));
        if save(data_dir, &loaded.store).is_err() {
            cleanup();
            return Err(ProfileError::Io);
        }
        Ok(CreatedProfile {
            profile_id: id.clone(),
            secret_path: secret_file,
        })
    })
    .map_err(|_| ProfileError::Io)?
}

/// Revokes a profile: the verifier row is deleted first, so authentication
/// stops, and the secret file and state are removed after. Returns whether a
/// row existed.
pub(crate) fn revoke(data_dir: &Path, id: &ProfileId) -> Result<bool, ProfileError> {
    let removed = with_lock(data_dir, || {
        let mut loaded = load(data_dir);
        if !loaded.writable() {
            return Err(ProfileError::StoreUnusable(loaded.status));
        }
        let before = loaded.store.entries.len();
        loaded
            .store
            .entries
            .retain(|entry| !matches!(entry, Entry::Profile(profile) if &profile.id == id));
        let removed = before != loaded.store.entries.len();
        if removed {
            save(data_dir, &loaded.store).map_err(|_| ProfileError::Io)?;
        }
        Ok(removed)
    })
    .map_err(|_| ProfileError::Io)??;
    let _ = std::fs::remove_file(secret_path(data_dir, id));
    sync_dir(&secret_dir(data_dir));
    profile_state::remove(data_dir, id);
    Ok(removed)
}

/// Turns one permission of a profile on or off. The account must be the one
/// the profile is bound to: the binding never moves.
pub(crate) fn set_grant(
    data_dir: &Path,
    id: &ProfileId,
    account_id: &str,
    need: Need,
    allowed: bool,
) -> Result<(), ProfileError> {
    with_lock(data_dir, || {
        let mut loaded = load(data_dir);
        if !loaded.writable() {
            return Err(ProfileError::StoreUnusable(loaded.status));
        }
        let Some(Entry::Profile(profile)) = loaded
            .store
            .entries
            .iter_mut()
            .find(|entry| matches!(entry, Entry::Profile(profile) if &profile.id == id))
        else {
            return Err(ProfileError::NotFound);
        };
        if profile.account_id != account_id {
            return Err(ProfileError::AccountMismatch);
        }
        profile.grants.set(need, allowed);
        save(data_dir, &loaded.store).map_err(|_| ProfileError::Io)
    })
    .map_err(|_| ProfileError::Io)?
}

/// What reading one profile found.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Lookup {
    Found(Box<Profile>),
    /// The store is readable and has no such row: the profile was revoked.
    Gone,
    /// The store could not be read or understood. Nothing is authorised, but
    /// this is not a revocation.
    Unusable,
}

pub(crate) fn lookup_checked(data_dir: &Path, id: &ProfileId) -> Lookup {
    let loaded = load(data_dir);
    if loaded.status != StoreStatus::Ok {
        return Lookup::Unusable;
    }
    match loaded.store.find(id) {
        Some(profile) => Lookup::Found(Box::new(profile.clone())),
        None => Lookup::Gone,
    }
}

/// The stored profile, or `None` when it is gone or the file cannot be
/// trusted. Fails closed.
pub(crate) fn lookup(data_dir: &Path, id: &ProfileId) -> Option<Profile> {
    match lookup_checked(data_dir, id) {
        Lookup::Found(profile) => Some(*profile),
        Lookup::Gone | Lookup::Unusable => None,
    }
}

/// Verifies a presented id and secret. An unknown id, a malformed id, an
/// unreadable store and a wrong secret all take the same path (the presented
/// secret is hashed and compared in constant time against a verifier, a fixed
/// dummy when there is no row) and return `None`, so neither the result nor
/// its timing says which ids exist.
pub(crate) fn authenticate(data_dir: &Path, presented_id: &str, secret: &str) -> Option<Profile> {
    // The digest of a fixed text no secret can have been generated as.
    static DUMMY: std::sync::OnceLock<[u8; 32]> = std::sync::OnceLock::new();
    let dummy = DUMMY.get_or_init(|| digest("koushi profile verifier: no such profile"));
    // The store is read for a malformed id too, so that read is not what
    // tells a well-formed id from a malformed one.
    let loaded = load(data_dir);
    let row = ProfileId::parse(presented_id).and_then(|id| {
        (loaded.status == StoreStatus::Ok)
            .then(|| loaded.store.find(&id).cloned())
            .flatten()
    });
    let expected = row.as_ref().map_or(dummy, |row| &row.verifier);
    let matched = endpoint::constant_time_eq(&digest(secret), expected);
    if matched { row } else { None }
}

/// Records a rejected profile connection: a token-only diagnostic (never the
/// presented id or secret), and, for a profile that exists, the time in its
/// state file for the settings page. The caller runs it after the rejection
/// was written and the stream closed, off the connection task, so the peer
/// does not wait for it. No lockout: the real plugin always authenticates with
/// the right secret.
pub(crate) fn note_rejected(data_dir: &Path, presented_id: &str) {
    koushi_diagnostics::record(
        DiagnosticEvent::new(DiagnosticLevel::Warn, "mcp.profiles", "authentication")
            .field(DiagnosticField::token("reason", "profile_rejected")),
    );
    if let Some(id) = ProfileId::parse(presented_id)
        && lookup(data_dir, &id).is_some()
    {
        profile_state::note_rejected(data_dir, &id, now_ms());
    }
}

pub(crate) fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as u64)
        .unwrap_or(0)
}

/// Removes secret files and state files that have no profile row: what a crash
/// between the secret file and the row leaves, and what a revoke from another
/// process may leave. It runs under the store lock, the same lock `create`
/// holds from the secret file to the row, so it never removes a file whose row
/// is about to be written. Only a store that parsed (`Ok`) names orphans: a
/// missing, unreadable or newer file removes nothing, since without rows every
/// secret would look orphaned.
pub(crate) fn sweep_orphans(data_dir: &Path) {
    let _ = with_lock(data_dir, || {
        let loaded = load(data_dir);
        if loaded.status != StoreStatus::Ok {
            return;
        }
        for (directory, extension) in [
            (secret_dir(data_dir), "secret"),
            (profile_state::dir(data_dir), "json"),
        ] {
            let Ok(entries) = std::fs::read_dir(&directory) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|value| value.to_str()) != Some(extension) {
                    continue;
                }
                let Some(id) = path
                    .file_stem()
                    .and_then(|stem| stem.to_str())
                    .and_then(ProfileId::parse)
                else {
                    continue;
                };
                if loaded.store.find(&id).is_none() {
                    let _ = std::fs::remove_file(&path);
                }
            }
        }
    });
}

#[cfg(test)]
#[path = "profiles_tests.rs"]
mod tests;
