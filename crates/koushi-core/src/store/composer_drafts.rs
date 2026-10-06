use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

#[cfg(any(test, feature = "test-hooks"))]
use super::ComposerDraftIoProbe;
use super::{
    COMPOSER_DRAFTS_FILE_MAGIC, CoreFailure, StoreActor, decode_composer_draft_payload_json,
    encode_composer_draft_payload_json,
};
use koushi_key::LocalUnlockSecret;
use koushi_protocol::SessionKeyId;

use koushi_state::{
    ComposerDocument, ComposerDraftPersistenceEntry, ComposerDraftPersistenceProjection,
    ComposerDraftProtection, ComposerDraftRevision, ComposerDraftStore, ComposerTarget,
};
use serde::{Deserialize, Serialize};

const COMPOSER_DRAFT_PAYLOAD_SCHEMA_VERSION: u8 = 3;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ComposerDraftPayloadError {
    Corrupt,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PersistedComposerDraftStoreV3 {
    schema_version: u8,
    rooms: BTreeMap<String, PersistedComposerDraftEntry>,
    threads: BTreeMap<String, BTreeMap<String, PersistedComposerDraftEntry>>,
    quiescent_room_order: Vec<String>,
    quiescent_thread_order: Vec<(String, String)>,
    protected_empty_rooms: Vec<String>,
    protected_empty_threads: Vec<(String, String)>,
}

impl PersistedComposerDraftStoreV3 {
    pub(crate) fn is_empty(&self) -> bool {
        self.rooms.is_empty() && self.threads.is_empty()
    }

    pub(crate) fn targets(&self) -> BTreeSet<ComposerTarget> {
        self.rooms
            .keys()
            .cloned()
            .map(|room_id| ComposerTarget::Main { room_id })
            .chain(self.threads.iter().flat_map(|(room_id, room_threads)| {
                room_threads
                    .keys()
                    .cloned()
                    .map(|root_event_id| ComposerTarget::Thread {
                        room_id: room_id.clone(),
                        root_event_id,
                    })
            }))
            .collect()
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PersistedComposerDraftEntry {
    content: Option<ComposerDocument>,
    revision: ComposerDraftRevision,
    last_accepted_clear_revision: ComposerDraftRevision,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PersistedComposerDraftStoreV2 {
    schema_version: u8,
    rooms: BTreeMap<String, PersistedComposerDraftEntryV2>,
    threads: BTreeMap<String, BTreeMap<String, PersistedComposerDraftEntryV2>>,
    quiescent_room_order: Vec<String>,
    quiescent_thread_order: Vec<(String, String)>,
    protected_empty_rooms: Vec<String>,
    protected_empty_threads: Vec<(String, String)>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PersistedComposerDraftEntryV2 {
    content: Option<String>,
    revision: ComposerDraftRevision,
    last_accepted_clear_revision: ComposerDraftRevision,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LegacyComposerDraftStoreV1 {
    #[serde(default)]
    rooms: BTreeMap<String, String>,
    #[serde(default)]
    threads: BTreeMap<String, BTreeMap<String, String>>,
    #[serde(default)]
    room_revisions: BTreeMap<String, u64>,
    #[serde(default)]
    thread_revisions: BTreeMap<String, BTreeMap<String, u64>>,
    #[serde(default)]
    room_last_accepted_clear_revisions: BTreeMap<String, u64>,
    #[serde(default)]
    thread_last_accepted_clear_revisions: BTreeMap<String, BTreeMap<String, u64>>,
    #[serde(default)]
    quiescent_room_lru: Vec<String>,
    #[serde(default)]
    quiescent_thread_lru: Vec<(String, String)>,
}

pub(crate) fn persisted_projection(
    drafts: &ComposerDraftStore,
    protection: &ComposerDraftProtection,
) -> PersistedComposerDraftStoreV3 {
    let projection = drafts.persisted_projection(protection);
    PersistedComposerDraftStoreV3 {
        schema_version: COMPOSER_DRAFT_PAYLOAD_SCHEMA_VERSION,
        rooms: projection
            .rooms
            .into_iter()
            .map(|(room_id, entry)| (room_id, entry.into()))
            .collect(),
        threads: projection
            .threads
            .into_iter()
            .map(|(room_id, room_threads)| {
                (
                    room_id,
                    room_threads
                        .into_iter()
                        .map(|(root_event_id, entry)| (root_event_id, entry.into()))
                        .collect(),
                )
            })
            .collect(),
        quiescent_room_order: projection.quiescent_room_order,
        quiescent_thread_order: projection.quiescent_thread_order,
        protected_empty_rooms: projection.protected_empty_rooms,
        protected_empty_threads: projection.protected_empty_threads,
    }
}

pub(crate) fn encode_payload_json(
    drafts: &PersistedComposerDraftStoreV3,
) -> Result<Vec<u8>, ComposerDraftPayloadError> {
    serde_json::to_vec(drafts).map_err(|_| ComposerDraftPayloadError::Corrupt)
}

pub(crate) fn decode_payload_json(
    payload: &[u8],
) -> Result<ComposerDraftStore, ComposerDraftPayloadError> {
    let value = serde_json::from_slice::<serde_json::Value>(payload).map_err(|_| corrupt())?;
    if let Some(schema_version) = value
        .get("schema_version")
        .and_then(serde_json::Value::as_u64)
    {
        let projection = match schema_version {
            3 => serde_json::from_value::<PersistedComposerDraftStoreV3>(value)
                .map(Into::into)
                .map_err(|_| corrupt())?,
            2 => serde_json::from_value::<PersistedComposerDraftStoreV2>(value)
                .map(Into::into)
                .map_err(|_| corrupt())?,
            _ => return Err(corrupt()),
        };
        ComposerDraftStore::from_persisted_projection(projection).map_err(|_| corrupt())
    } else {
        let legacy =
            serde_json::from_value::<LegacyComposerDraftStoreV1>(value).map_err(|_| corrupt())?;
        let projection = ComposerDraftPersistenceProjection::try_from(legacy)?;
        ComposerDraftStore::from_persisted_projection(projection).map_err(|_| corrupt())
    }
}

fn corrupt() -> ComposerDraftPayloadError {
    ComposerDraftPayloadError::Corrupt
}

impl From<ComposerDraftPersistenceEntry> for PersistedComposerDraftEntry {
    fn from(entry: ComposerDraftPersistenceEntry) -> Self {
        Self {
            content: entry.content,
            revision: entry.revision,
            last_accepted_clear_revision: entry.last_accepted_clear_revision,
        }
    }
}

impl From<PersistedComposerDraftEntry> for ComposerDraftPersistenceEntry {
    fn from(entry: PersistedComposerDraftEntry) -> Self {
        Self {
            content: entry.content,
            revision: entry.revision,
            last_accepted_clear_revision: entry.last_accepted_clear_revision,
        }
    }
}

impl From<PersistedComposerDraftStoreV3> for ComposerDraftPersistenceProjection {
    fn from(persisted: PersistedComposerDraftStoreV3) -> Self {
        Self {
            rooms: persisted
                .rooms
                .into_iter()
                .map(|(room_id, entry)| (room_id, entry.into()))
                .collect(),
            threads: persisted
                .threads
                .into_iter()
                .map(|(room_id, room_threads)| {
                    (
                        room_id,
                        room_threads
                            .into_iter()
                            .map(|(root_event_id, entry)| (root_event_id, entry.into()))
                            .collect(),
                    )
                })
                .collect(),
            quiescent_room_order: persisted.quiescent_room_order,
            quiescent_thread_order: persisted.quiescent_thread_order,
            protected_empty_rooms: persisted.protected_empty_rooms,
            protected_empty_threads: persisted.protected_empty_threads,
        }
    }
}

impl From<PersistedComposerDraftStoreV2> for ComposerDraftPersistenceProjection {
    fn from(persisted: PersistedComposerDraftStoreV2) -> Self {
        debug_assert_eq!(persisted.schema_version, 2);
        let convert = |entry: PersistedComposerDraftEntryV2| ComposerDraftPersistenceEntry {
            content: entry.content.map(ComposerDocument::from_plain_text),
            revision: entry.revision,
            last_accepted_clear_revision: entry.last_accepted_clear_revision,
        };
        Self {
            rooms: persisted
                .rooms
                .into_iter()
                .map(|(room_id, entry)| (room_id, convert(entry)))
                .collect(),
            threads: persisted
                .threads
                .into_iter()
                .map(|(room_id, room_threads)| {
                    (
                        room_id,
                        room_threads
                            .into_iter()
                            .map(|(root_event_id, entry)| (root_event_id, convert(entry)))
                            .collect(),
                    )
                })
                .collect(),
            quiescent_room_order: persisted.quiescent_room_order,
            quiescent_thread_order: persisted.quiescent_thread_order,
            protected_empty_rooms: persisted.protected_empty_rooms,
            protected_empty_threads: persisted.protected_empty_threads,
        }
    }
}

impl TryFrom<LegacyComposerDraftStoreV1> for ComposerDraftPersistenceProjection {
    type Error = ComposerDraftPayloadError;

    fn try_from(legacy: LegacyComposerDraftStoreV1) -> Result<Self, Self::Error> {
        let room_ids = legacy
            .rooms
            .keys()
            .chain(legacy.room_revisions.keys())
            .chain(legacy.room_last_accepted_clear_revisions.keys())
            .cloned()
            .collect::<std::collections::BTreeSet<_>>();
        let rooms = room_ids
            .iter()
            .map(|room_id| {
                (
                    room_id.clone(),
                    ComposerDraftPersistenceEntry {
                        content: legacy
                            .rooms
                            .get(room_id)
                            .filter(|content| !content.is_empty())
                            .cloned()
                            .map(ComposerDocument::from_plain_text),
                        revision: legacy
                            .room_revisions
                            .get(room_id)
                            .copied()
                            .map(ComposerDraftRevision::from_u64)
                            .unwrap_or_default(),
                        last_accepted_clear_revision: legacy
                            .room_last_accepted_clear_revisions
                            .get(room_id)
                            .copied()
                            .map(ComposerDraftRevision::from_u64)
                            .unwrap_or_default(),
                    },
                )
            })
            .collect::<BTreeMap<_, _>>();
        let empty_room_ids = room_ids
            .into_iter()
            .filter(|room_id| {
                rooms
                    .get(room_id)
                    .is_some_and(|entry| entry.content.is_none())
            })
            .collect::<BTreeSet<_>>();
        let quiescent_room_order = merge_legacy_order(legacy.quiescent_room_lru, &empty_room_ids)?;

        let thread_targets = legacy
            .threads
            .iter()
            .flat_map(|(room_id, threads)| {
                threads
                    .keys()
                    .map(|root_event_id| (room_id.clone(), root_event_id.clone()))
            })
            .chain(
                legacy
                    .thread_revisions
                    .iter()
                    .flat_map(|(room_id, revisions)| {
                        revisions
                            .keys()
                            .map(|root_event_id| (room_id.clone(), root_event_id.clone()))
                    }),
            )
            .chain(legacy.thread_last_accepted_clear_revisions.iter().flat_map(
                |(room_id, revisions)| {
                    revisions
                        .keys()
                        .map(|root_event_id| (room_id.clone(), root_event_id.clone()))
                },
            ))
            .collect::<std::collections::BTreeSet<_>>();
        let mut threads = BTreeMap::<String, BTreeMap<String, _>>::new();
        let mut empty_thread_targets = BTreeSet::new();
        for (room_id, root_event_id) in thread_targets {
            let content = legacy
                .threads
                .get(&room_id)
                .and_then(|room_threads| room_threads.get(&root_event_id))
                .filter(|content| !content.is_empty())
                .cloned()
                .map(ComposerDocument::from_plain_text);
            if content.is_none() {
                empty_thread_targets.insert((room_id.clone(), root_event_id.clone()));
            }
            let revision = legacy
                .thread_revisions
                .get(&room_id)
                .and_then(|room_threads| room_threads.get(&root_event_id))
                .copied()
                .map(ComposerDraftRevision::from_u64)
                .unwrap_or_default();
            let last_accepted_clear_revision = legacy
                .thread_last_accepted_clear_revisions
                .get(&room_id)
                .and_then(|room_threads| room_threads.get(&root_event_id))
                .copied()
                .map(ComposerDraftRevision::from_u64)
                .unwrap_or_default();
            threads.entry(room_id).or_default().insert(
                root_event_id,
                ComposerDraftPersistenceEntry {
                    content,
                    revision,
                    last_accepted_clear_revision,
                },
            );
        }
        let quiescent_thread_order =
            merge_legacy_order(legacy.quiescent_thread_lru, &empty_thread_targets)?;

        Ok(Self {
            rooms,
            threads,
            quiescent_room_order,
            quiescent_thread_order,
            protected_empty_rooms: Vec::new(),
            protected_empty_threads: Vec::new(),
        })
    }
}

fn merge_legacy_order<T: Clone + Ord>(
    mut saved_order: Vec<T>,
    empty_targets: &BTreeSet<T>,
) -> Result<Vec<T>, ComposerDraftPayloadError> {
    let mut seen = BTreeSet::new();
    if saved_order
        .iter()
        .any(|target| !empty_targets.contains(target) || !seen.insert(target.clone()))
    {
        return Err(corrupt());
    }
    saved_order.extend(empty_targets.difference(&seen).cloned());
    Ok(saved_order)
}

#[cfg(test)]
mod tests;

impl StoreActor {
    pub fn load_composer_drafts(
        &self,
        key_id: &SessionKeyId,
    ) -> Result<ComposerDraftStore, CoreFailure> {
        #[cfg(any(test, feature = "test-hooks"))]
        self.notify_composer_draft_load_started_for_testing();

        (|| {
            let path = self.account_composer_drafts_file(key_id);
            let bytes = match std::fs::read(&path) {
                Ok(bytes) => bytes,
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                    return Ok(ComposerDraftStore::default());
                }
                Err(_) => return Err(CoreFailure::StoreUnavailable),
            };
            decrypt_composer_drafts_payload(&self.load_unlock_secret(key_id)?, &bytes)
        })()
    }

    pub(crate) fn save_composer_drafts(
        &self,
        key_id: &SessionKeyId,
        drafts: &PersistedComposerDraftStoreV3,
    ) -> Result<(), CoreFailure> {
        #[cfg(any(test, feature = "test-hooks"))]
        self.wait_for_composer_draft_save_release_for_testing();
        let result = self.save_composer_drafts_inner(key_id, drafts);
        #[cfg(any(test, feature = "test-hooks"))]
        self.notify_composer_draft_save_completed_for_testing();
        result
    }

    fn save_composer_drafts_inner(
        &self,
        key_id: &SessionKeyId,
        drafts: &PersistedComposerDraftStoreV3,
    ) -> Result<(), CoreFailure> {
        let path = self.account_composer_drafts_file(key_id);
        if drafts.is_empty() {
            match std::fs::remove_file(&path) {
                Ok(()) => return Ok(()),
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(()),
                Err(_) => return Err(CoreFailure::StoreUnavailable),
            }
        }
        let payload =
            encrypt_composer_drafts_payload(&self.load_or_create_unlock_secret(key_id)?, drafts)?;
        #[cfg(test)]
        let fail_before_persist = self
            .composer_draft_replace_fault
            .swap(false, std::sync::atomic::Ordering::AcqRel);
        #[cfg(not(test))]
        let fail_before_persist = false;
        koushi_store::atomic_replace_file(&path, &payload, fail_before_persist)
            .map_err(|_| CoreFailure::StoreUnavailable)
    }

    #[cfg(test)]
    fn fail_next_composer_draft_replace_for_testing(&self) {
        self.composer_draft_replace_fault
            .store(true, std::sync::atomic::Ordering::Release);
    }

    #[cfg(any(test, feature = "test-hooks"))]
    pub(crate) fn install_composer_draft_io_probe(
        &self,
        save_started: tokio::sync::oneshot::Sender<()>,
        save_release: std::sync::mpsc::Receiver<()>,
        save_completed: tokio::sync::oneshot::Sender<()>,
        load_started: tokio::sync::oneshot::Sender<()>,
        load_completed: tokio::sync::oneshot::Sender<()>,
        load_counts: super::ComposerDraftLoadCounts,
    ) {
        *self
            .composer_draft_io_probe
            .lock()
            .expect("composer draft I/O probe mutex") = Some(ComposerDraftIoProbe {
            save_started: Some(save_started),
            save_release: Some(save_release),
            save_completed: Some(save_completed),
            load_started: Some(load_started),
            load_completed: Some(load_completed),
            load_counts,
        });
    }

    #[cfg(any(test, feature = "test-hooks"))]
    fn wait_for_composer_draft_save_release_for_testing(&self) {
        let (started, release) = {
            let mut probe = self
                .composer_draft_io_probe
                .lock()
                .expect("composer draft I/O probe mutex");
            let Some(probe) = probe.as_mut() else {
                return;
            };
            (probe.save_started.take(), probe.save_release.take())
        };
        if let Some(started) = started {
            let _ = started.send(());
        }
        if let Some(release) = release {
            let _ = release.recv();
        }
    }

    #[cfg(any(test, feature = "test-hooks"))]
    fn notify_composer_draft_save_completed_for_testing(&self) {
        let completed = self
            .composer_draft_io_probe
            .lock()
            .expect("composer draft I/O probe mutex")
            .as_mut()
            .and_then(|probe| probe.save_completed.take());
        if let Some(completed) = completed {
            let _ = completed.send(());
        }
    }

    #[cfg(any(test, feature = "test-hooks"))]
    fn notify_composer_draft_load_started_for_testing(&self) {
        let started = {
            let mut probe = self
                .composer_draft_io_probe
                .lock()
                .expect("composer draft I/O probe mutex");
            let Some(probe) = probe.as_mut() else {
                return;
            };
            probe
                .load_counts
                .attempts
                .fetch_add(1, std::sync::atomic::Ordering::Release);
            probe.load_started.take()
        };
        if let Some(started) = started {
            let _ = started.send(());
        }
    }

    /// Counts failed loads per installed probe, so a test can attribute
    /// failures to its own runtime: the `core.composer_draft/load_failed`
    /// diagnostic carries no owner and other runtimes in the process emit it.
    #[cfg(any(test, feature = "test-hooks"))]
    pub(crate) fn notify_composer_draft_load_failed_for_testing(&self) {
        if let Some(probe) = self
            .composer_draft_io_probe
            .lock()
            .expect("composer draft I/O probe mutex")
            .as_ref()
        {
            probe
                .load_counts
                .failures
                .fetch_add(1, std::sync::atomic::Ordering::Release);
        }
    }

    #[cfg(any(test, feature = "test-hooks"))]
    pub(crate) fn notify_composer_draft_load_completed_for_testing(&self) {
        let completed = self
            .composer_draft_io_probe
            .lock()
            .expect("composer draft I/O probe mutex")
            .as_mut()
            .and_then(|probe| probe.load_completed.take());
        if let Some(completed) = completed {
            let _ = completed.send(());
        }
    }

    fn account_composer_drafts_file(&self, key_id: &SessionKeyId) -> PathBuf {
        self.account_root_dir(key_id)
            .join("composer-drafts")
            .join("drafts.v1.enc")
    }
}

fn encrypt_composer_drafts_payload(
    secret: &LocalUnlockSecret,
    drafts: &PersistedComposerDraftStoreV3,
) -> Result<Vec<u8>, CoreFailure> {
    let plaintext =
        encode_composer_draft_payload_json(drafts).map_err(|_| CoreFailure::StoreUnavailable)?;
    encrypt_composer_drafts_plaintext(secret, &plaintext)
}

fn encrypt_composer_drafts_plaintext(
    secret: &LocalUnlockSecret,
    plaintext: &[u8],
) -> Result<Vec<u8>, CoreFailure> {
    let key = secret.derive_composer_drafts_key();
    koushi_store::encrypt_envelope(
        COMPOSER_DRAFTS_FILE_MAGIC,
        key.as_bytes(),
        plaintext,
        usize::MAX,
    )
    .map_err(|_| CoreFailure::StoreUnavailable)
}

#[cfg(test)]
fn encrypt_composer_drafts_fixture_payload(
    secret: &LocalUnlockSecret,
    plaintext: &[u8],
) -> Result<Vec<u8>, CoreFailure> {
    encrypt_composer_drafts_plaintext(secret, plaintext)
}

fn decrypt_composer_drafts_payload(
    secret: &LocalUnlockSecret,
    payload: &[u8],
) -> Result<ComposerDraftStore, CoreFailure> {
    let key = secret.derive_composer_drafts_key();
    let plaintext = koushi_store::decrypt_envelope(
        COMPOSER_DRAFTS_FILE_MAGIC,
        key.as_bytes(),
        payload,
        usize::MAX,
    )
    .map_err(|_| CoreFailure::StoreUnavailable)?;
    decode_composer_draft_payload_json(&plaintext).map_err(|_| CoreFailure::StoreUnavailable)
}

#[cfg(test)]
mod store_tests;
