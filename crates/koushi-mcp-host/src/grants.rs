//! The grant store: `mcp/connections.json` in the app data directory.
//!
//! The app writes it when the user answers an approval or changes a
//! permission, and every tool call re-reads it, so a hand edit or a revocation
//! applies on the next call. Schema 3 keeps permissions per account; schema 1
//! (one set of flags for a list of accounts) and schema 2 (no
//! `message_actions` flag) are still read and are upgraded by the next write.
//!
//! The store fails closed: a missing, unreadable, malformed, or newer file
//! grants nothing, and the app never writes over a file it could not parse.

use std::path::{Path, PathBuf};

use koushi_diagnostics::{DiagnosticEvent, DiagnosticField, DiagnosticLevel};
use serde::Serialize;
use serde_json::{Map, Value, json};

pub(crate) const FILE_NAME: &str = "connections.json";
const SCHEMA: u64 = 3;

/// A permission an assistant can hold for one account.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Need {
    Read,
    Send,
    Attachments,
    MessageActions,
}

impl Need {
    pub const ALL: [Need; 4] = [
        Need::Read,
        Need::Send,
        Need::Attachments,
        Need::MessageActions,
    ];

    fn field(self) -> &'static str {
        match self {
            Need::Read => "read",
            Need::Send => "send",
            Need::Attachments => "attachments",
            Need::MessageActions => "message_actions",
        }
    }
}

/// What the last read of the file found.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StoreStatus {
    Ok,
    Missing,
    Malformed,
    Unreadable,
    UnsupportedSchema,
    WriteFailed,
}

/// One account's permissions for one assistant.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct AccountGrant {
    pub(crate) account_id: String,
    pub(crate) read: bool,
    pub(crate) send: bool,
    pub(crate) attachments: bool,
    pub(crate) message_actions: bool,
    extra: Map<String, Value>,
}

impl AccountGrant {
    pub(crate) fn allows(&self, need: Need) -> bool {
        match need {
            Need::Read => self.read,
            Need::Send => self.send,
            Need::Attachments => self.attachments,
            Need::MessageActions => self.message_actions,
        }
    }

    fn set(&mut self, need: Need, allowed: bool) {
        match need {
            Need::Read => self.read = allowed,
            Need::Send => self.send = allowed,
            Need::Attachments => self.attachments = allowed,
            Need::MessageActions => self.message_actions = allowed,
        }
    }
}

/// One assistant's grants. Unknown keys are kept so a rewrite does not drop
/// what a newer build or a person added.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ClientGrant {
    pub(crate) client_name: String,
    pub(crate) accounts: Vec<AccountGrant>,
    extra: Map<String, Value>,
}

#[derive(Clone, Debug, PartialEq)]
enum Entry {
    Client(ClientGrant),
    /// An entry with no usable client name. It grants nothing and is written
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
    /// The grant that applies to a client: the first entry with its name.
    pub(crate) fn client(&self, client_name: &str) -> Option<&ClientGrant> {
        self.clients()
            .find(|grant| grant.client_name == client_name)
    }

    pub(crate) fn clients(&self) -> impl Iterator<Item = &ClientGrant> {
        self.entries.iter().filter_map(|entry| match entry {
            Entry::Client(grant) => Some(grant),
            Entry::Raw(_) => None,
        })
    }

    pub(crate) fn allows(&self, client_name: &str, account_id: &str, need: Need) -> bool {
        self.account(client_name, account_id)
            .is_some_and(|grant| grant.allows(need))
    }

    pub(crate) fn account(&self, client_name: &str, account_id: &str) -> Option<&AccountGrant> {
        self.client(client_name)?
            .accounts
            .iter()
            .find(|grant| grant.account_id == account_id)
    }

    pub(crate) fn set(&mut self, client_name: &str, account_id: &str, need: Need, allowed: bool) {
        let index = match self.entries.iter().position(
            |entry| matches!(entry, Entry::Client(grant) if grant.client_name == client_name),
        ) {
            Some(index) => index,
            None => {
                self.entries.push(Entry::Client(ClientGrant {
                    client_name: client_name.to_owned(),
                    accounts: Vec::new(),
                    extra: Map::new(),
                }));
                self.entries.len() - 1
            }
        };
        let Entry::Client(grant) = &mut self.entries[index] else {
            unreachable!("position matched a client entry");
        };
        match grant
            .accounts
            .iter_mut()
            .find(|account| account.account_id == account_id)
        {
            Some(account) => account.set(need, allowed),
            None => {
                let mut account = AccountGrant {
                    account_id: account_id.to_owned(),
                    ..AccountGrant::default()
                };
                account.set(need, allowed);
                grant.accounts.push(account);
            }
        }
    }

    /// Removes every entry for the client, so a later duplicate cannot take
    /// over the removed one's place.
    pub(crate) fn remove(&mut self, client_name: &str) -> bool {
        let before = self.entries.len();
        self.entries.retain(
            |entry| !matches!(entry, Entry::Client(grant) if grant.client_name == client_name),
        );
        before != self.entries.len()
    }

    fn to_json(&self) -> Value {
        let mut root = self.extra.clone();
        root.insert("schema".to_owned(), json!(SCHEMA));
        let connections = self
            .entries
            .iter()
            .map(|entry| match entry {
                Entry::Raw(value) => value.clone(),
                Entry::Client(grant) => {
                    let mut object = grant.extra.clone();
                    object.insert("client_name".to_owned(), json!(grant.client_name));
                    let accounts = grant
                        .accounts
                        .iter()
                        .map(|account| {
                            let mut object = account.extra.clone();
                            object.insert("account_id".to_owned(), json!(account.account_id));
                            for need in Need::ALL {
                                object.insert(need.field().to_owned(), json!(account.allows(need)));
                            }
                            Value::Object(object)
                        })
                        .collect();
                    object.insert("accounts".to_owned(), Value::Array(accounts));
                    Value::Object(object)
                }
            })
            .collect();
        root.insert("connections".to_owned(), Value::Array(connections));
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
    /// The app may write only over a file it understood, or none at all.
    pub(crate) fn writable(&self) -> bool {
        matches!(self.status, StoreStatus::Ok | StoreStatus::Missing)
    }
}

/// Records a store problem as a token event. Paths and parse errors stay out
/// of diagnostics: they name user data.
fn record(reason: &'static str) {
    koushi_diagnostics::record(
        DiagnosticEvent::new(DiagnosticLevel::Warn, "mcp.grants", "configuration")
            .field(DiagnosticField::token("reason", reason)),
    );
}

pub(crate) fn grants_path(data_dir: &Path) -> PathBuf {
    data_dir.join("mcp").join(FILE_NAME)
}

/// Reads the store. Anything but a readable file of a known schema yields an
/// empty store and a status saying why.
pub(crate) fn load(data_dir: &Path) -> Loaded {
    let failed = |status: StoreStatus, reason: &'static str| {
        record(reason);
        Loaded {
            status,
            store: Store::default(),
        }
    };
    let bytes = match std::fs::read(grants_path(data_dir)) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Loaded {
                status: StoreStatus::Missing,
                store: Store::default(),
            };
        }
        Err(_) => return failed(StoreStatus::Unreadable, "grants_read_failed"),
    };
    let Ok(Value::Object(mut root)) = serde_json::from_slice::<Value>(&bytes) else {
        return failed(StoreStatus::Malformed, "grants_parse_failed");
    };
    let schema = match root.remove("schema") {
        None => 1,
        Some(value) => match value.as_u64() {
            Some(schema) => schema,
            None => return failed(StoreStatus::Malformed, "grants_parse_failed"),
        },
    };
    if !(1..=SCHEMA).contains(&schema) {
        return failed(StoreStatus::UnsupportedSchema, "grants_schema_unsupported");
    }
    let connections = match root.remove("connections") {
        None => Vec::new(),
        Some(Value::Array(connections)) => connections,
        Some(_) => return failed(StoreStatus::Malformed, "grants_parse_failed"),
    };
    let mut entries = Vec::with_capacity(connections.len());
    for connection in connections {
        let parsed = if schema == 1 {
            parse_v1(connection)
        } else {
            parse_v2(connection)
        };
        match parsed {
            Ok(entry) => entries.push(entry),
            Err(()) => return failed(StoreStatus::Malformed, "grants_parse_failed"),
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

fn take_name(object: &mut Map<String, Value>) -> Option<String> {
    match object.get("client_name") {
        Some(Value::String(name)) if !name.trim().is_empty() => {
            let name = name.clone();
            object.remove("client_name");
            Some(name)
        }
        _ => None,
    }
}

/// Schema 1: `{client_name, accounts: [id], allow_read, allow_send,
/// allow_attachments}`. Each listed account gets the entry's flags. An entry
/// without a name or accounts granted nothing and still grants nothing.
fn parse_v1(value: Value) -> Result<Entry, ()> {
    let Value::Object(mut object) = value else {
        return Err(());
    };
    let Some(client_name) = take_name(&mut object) else {
        record("grants_entry_ignored");
        return Ok(Entry::Raw(Value::Object(object)));
    };
    let ids = match object.remove("accounts") {
        None => Vec::new(),
        Some(Value::Array(ids)) => ids,
        Some(_) => return Err(()),
    };
    let read = take_flag(&mut object, "allow_read")?;
    let send = take_flag(&mut object, "allow_send")?;
    let attachments = take_flag(&mut object, "allow_attachments")?;
    let mut accounts = Vec::new();
    for id in ids {
        let Value::String(account_id) = id else {
            return Err(());
        };
        if account_id.trim().is_empty()
            || accounts
                .iter()
                .any(|a: &AccountGrant| a.account_id == account_id)
        {
            continue;
        }
        accounts.push(AccountGrant {
            account_id,
            read,
            send,
            attachments,
            message_actions: false,
            extra: Map::new(),
        });
    }
    if accounts.is_empty() {
        record("grants_entry_ignored");
    }
    Ok(Entry::Client(ClientGrant {
        client_name,
        accounts,
        extra: object,
    }))
}

fn parse_v2(value: Value) -> Result<Entry, ()> {
    let Value::Object(mut object) = value else {
        return Err(());
    };
    let Some(client_name) = take_name(&mut object) else {
        record("grants_entry_ignored");
        return Ok(Entry::Raw(Value::Object(object)));
    };
    let rows = match object.remove("accounts") {
        None => Vec::new(),
        Some(Value::Array(rows)) => rows,
        Some(_) => return Err(()),
    };
    let mut accounts = Vec::new();
    for row in rows {
        let Value::Object(mut row) = row else {
            return Err(());
        };
        let Some(Value::String(account_id)) = row.remove("account_id") else {
            return Err(());
        };
        let read = take_flag(&mut row, "read")?;
        let send = take_flag(&mut row, "send")?;
        let attachments = take_flag(&mut row, "attachments")?;
        // Schema 2 rows have no `message_actions` key; a missing flag is false.
        let message_actions = take_flag(&mut row, "message_actions")?;
        accounts.push(AccountGrant {
            account_id,
            read,
            send,
            attachments,
            message_actions,
            extra: row,
        });
    }
    Ok(Entry::Client(ClientGrant {
        client_name,
        accounts,
        extra: object,
    }))
}

/// Writes the store as schema 3: a private temporary file in the same
/// directory, then an atomic rename over the old file.
pub(crate) fn save(data_dir: &Path, store: &Store) -> std::io::Result<()> {
    use std::io::Write;
    let dir = crate::endpoint::ensure_dir(data_dir)?;
    let bytes = serde_json::to_vec_pretty(&store.to_json())
        .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?;
    // `tempfile` creates the file 0600 on Unix.
    let mut temporary = tempfile::NamedTempFile::new_in(&dir)?;
    temporary.write_all(&bytes)?;
    temporary.write_all(b"\n")?;
    temporary.as_file().sync_all()?;
    temporary
        .persist(grants_path(data_dir))
        .map_err(|error| error.error)?;
    Ok(())
}

#[cfg(test)]
#[path = "grants_tests.rs"]
mod tests;
