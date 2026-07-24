use std::collections::{BTreeMap, BTreeSet};

use koushi_state::{
    ComposerDraftPersistenceEntry, ComposerDraftPersistenceProjection, ComposerDraftProtection,
    ComposerDraftRevision, ComposerDraftStore, ComposerTarget,
};
use serde::{Deserialize, Serialize};

const COMPOSER_DRAFT_PAYLOAD_SCHEMA_VERSION: u8 = 2;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ComposerDraftPayloadError {
    Corrupt,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PersistedComposerDraftStoreV2 {
    schema_version: u8,
    rooms: BTreeMap<String, PersistedComposerDraftEntry>,
    threads: BTreeMap<String, BTreeMap<String, PersistedComposerDraftEntry>>,
    quiescent_room_order: Vec<String>,
    quiescent_thread_order: Vec<(String, String)>,
    protected_empty_rooms: Vec<String>,
    protected_empty_threads: Vec<(String, String)>,
}

impl PersistedComposerDraftStoreV2 {
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
) -> PersistedComposerDraftStoreV2 {
    let projection = drafts.persisted_projection(protection);
    PersistedComposerDraftStoreV2 {
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
    drafts: &PersistedComposerDraftStoreV2,
) -> Result<Vec<u8>, ComposerDraftPayloadError> {
    serde_json::to_vec(drafts).map_err(|_| ComposerDraftPayloadError::Corrupt)
}

pub(crate) fn decode_payload_json(
    payload: &[u8],
) -> Result<ComposerDraftStore, ComposerDraftPayloadError> {
    let value = serde_json::from_slice::<serde_json::Value>(payload).map_err(|_| corrupt())?;
    if value.get("schema_version").is_some() {
        let persisted = serde_json::from_value::<PersistedComposerDraftStoreV2>(value)
            .map_err(|_| corrupt())?;
        if persisted.schema_version != COMPOSER_DRAFT_PAYLOAD_SCHEMA_VERSION {
            return Err(corrupt());
        }
        ComposerDraftStore::from_persisted_projection(persisted.into()).map_err(|_| corrupt())
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

impl From<PersistedComposerDraftStoreV2> for ComposerDraftPersistenceProjection {
    fn from(persisted: PersistedComposerDraftStoreV2) -> Self {
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
                            .cloned(),
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
                .cloned();
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
mod tests {
    use super::*;

    const LARGE_LEGACY_REVISION: u64 = 9_007_199_254_740_993;

    #[test]
    fn composer_draft_payload_pre_293_defaults_content_revision_and_clear_token_to_zero() {
        let legacy = br#"{
            "rooms":{"room-legacy":"room body"},
            "threads":{"room-legacy":{"root-legacy":"thread body"}}
        }"#;

        let mut decoded = decode_payload_json(legacy).expect("decode pre-#293 payload");
        let room = decoded.composer_for_room("room-legacy");
        assert_eq!(room.draft, "room body");
        assert!(room.draft_revision.is_zero());
        assert!(room.last_accepted_clear_revision.is_zero());
        let thread = decoded.composer_for_thread("room-legacy", "root-legacy");
        assert_eq!(thread.draft, "thread body");
        assert!(thread.draft_revision.is_zero());
        assert!(thread.last_accepted_clear_revision.is_zero());

        assert!(
            decoded
                .apply_room_draft("room-legacy".to_owned(), "mutated".to_owned(), 1.into())
                .expect("checked mutation")
        );
        let encoded = encode_payload_json(&persisted_projection(
            &decoded,
            &ComposerDraftProtection::default(),
        ))
        .expect("encode v2");
        let reloaded = decode_payload_json(&encoded).expect("reload v2");
        assert_eq!(
            reloaded.rooms.get("room-legacy").map(String::as_str),
            Some("mutated")
        );
        assert_eq!(reloaded.room_revision("room-legacy"), 1.into());
        assert!(
            reloaded
                .composer_for_room("room-legacy")
                .last_accepted_clear_revision
                .is_zero()
        );
    }

    #[test]
    fn composer_draft_payload_issue_293_numeric_u64_migrates_losslessly_to_strings() {
        let legacy = format!(
            r#"{{
                "rooms":{{"room-large":"room body"}},
                "threads":{{"room-large":{{"root-large":"thread body"}}}},
                "room_revisions":{{"room-large":{LARGE_LEGACY_REVISION}}},
                "thread_revisions":{{"room-large":{{"root-large":{LARGE_LEGACY_REVISION}}}}}
            }}"#
        );

        let decoded = decode_payload_json(legacy.as_bytes()).expect("decode #293 payload");
        let encoded = encode_payload_json(&persisted_projection(
            &decoded,
            &ComposerDraftProtection::default(),
        ))
        .expect("encode v2");
        let encoded: serde_json::Value =
            serde_json::from_slice(&encoded).expect("parse encoded v2");

        assert_eq!(
            encoded["rooms"]["room-large"]["revision"],
            serde_json::json!("9007199254740993")
        );
        assert_eq!(
            encoded["threads"]["room-large"]["root-large"]["revision"],
            serde_json::json!("9007199254740993")
        );
    }

    #[test]
    fn composer_draft_payload_legacy_clear_watermarks_migrate_losslessly() {
        let legacy = format!(
            r#"{{
                "room_revisions":{{"room-cleared":{LARGE_LEGACY_REVISION}}},
                "thread_revisions":{{"room-cleared":{{"root-cleared":{LARGE_LEGACY_REVISION}}}}},
                "room_last_accepted_clear_revisions":{{"room-cleared":{LARGE_LEGACY_REVISION}}},
                "thread_last_accepted_clear_revisions":{{"room-cleared":{{"root-cleared":{LARGE_LEGACY_REVISION}}}}},
                "quiescent_room_lru":["room-cleared"],
                "quiescent_thread_lru":[["room-cleared","root-cleared"]]
            }}"#
        );

        let decoded = decode_payload_json(legacy.as_bytes()).expect("decode causal legacy payload");
        assert_eq!(
            decoded
                .composer_for_room("room-cleared")
                .last_accepted_clear_revision,
            ComposerDraftRevision::from_u64(LARGE_LEGACY_REVISION)
        );
        assert_eq!(
            decoded
                .composer_for_thread("room-cleared", "root-cleared")
                .last_accepted_clear_revision,
            ComposerDraftRevision::from_u64(LARGE_LEGACY_REVISION)
        );

        let encoded = encode_payload_json(&persisted_projection(
            &decoded,
            &ComposerDraftProtection::default(),
        ))
        .expect("encode migrated v2");
        let encoded: serde_json::Value =
            serde_json::from_slice(&encoded).expect("parse migrated v2");
        assert_eq!(
            encoded["rooms"]["room-cleared"]["last_accepted_clear_revision"],
            serde_json::json!("9007199254740993")
        );
        assert_eq!(
            encoded["threads"]["room-cleared"]["root-cleared"]["last_accepted_clear_revision"],
            serde_json::json!("9007199254740993")
        );
    }

    #[test]
    fn composer_draft_payload_legacy_lru_preserves_nonlexical_order_and_rejects_invalid_order() {
        let legacy = br#"{
            "room_revisions":{"z-oldest":1,"a-newer":1,"middle-missing-order":1},
            "thread_revisions":{"z-room":{"z-root":1,"a-root":1,"middle-root":1}},
            "quiescent_room_lru":["z-oldest","a-newer"],
            "quiescent_thread_lru":[["z-room","z-root"],["z-room","a-root"]]
        }"#;

        let decoded = decode_payload_json(legacy).expect("decode ordered legacy payload");
        let projection = persisted_projection(&decoded, &ComposerDraftProtection::default());
        assert_eq!(
            projection.quiescent_room_order,
            vec!["z-oldest", "a-newer", "middle-missing-order"]
        );
        assert_eq!(
            projection.quiescent_thread_order,
            vec![
                ("z-room".to_owned(), "z-root".to_owned()),
                ("z-room".to_owned(), "a-root".to_owned()),
                ("z-room".to_owned(), "middle-root".to_owned()),
            ]
        );

        let invalid = [
            br#"{
                "room_revisions":{"room":1},
                "quiescent_room_lru":["room","room"]
            }"#
            .as_slice(),
            br#"{
                "room_revisions":{"room":1},
                "quiescent_room_lru":["unknown"]
            }"#
            .as_slice(),
            br#"{
                "threads":{"room":{"root":"body"}},
                "thread_revisions":{"room":{"root":1}},
                "quiescent_thread_lru":[["room","root"]]
            }"#
            .as_slice(),
            br#"{
                "room_revisionz":{"room":1}
            }"#
            .as_slice(),
        ];
        for payload in invalid {
            assert_eq!(
                decode_payload_json(payload).expect_err("invalid legacy order must fail"),
                ComposerDraftPayloadError::Corrupt
            );
        }
    }

    #[test]
    fn composer_draft_payload_v2_round_trips_bounded_empty_string_rooms() {
        let mut drafts = ComposerDraftStore::default();
        for index in 0..(koushi_state::MAX_PERSISTED_COMPOSER_DRAFT_ROOM_COUNT + 2) {
            let room_id = format!("empty-room-{index:03}");
            drafts.rooms.insert(room_id.clone(), String::new());
            drafts.room_revisions.insert(room_id, 1.into());
        }

        let encoded = encode_payload_json(&persisted_projection(
            &drafts,
            &ComposerDraftProtection::default(),
        ))
        .expect("encode bounded v2");
        let decoded = decode_payload_json(&encoded).expect("self-encoded v2 must decode");

        assert_eq!(
            decoded.room_revisions.len(),
            koushi_state::MAX_PERSISTED_COMPOSER_DRAFT_ROOM_COUNT
        );
        assert!(!decoded.room_revisions.contains_key("empty-room-000"));
        assert!(!decoded.room_revisions.contains_key("empty-room-001"));
        assert!(decoded.room_revisions.contains_key("empty-room-002"));
    }

    #[test]
    fn composer_draft_payload_v2_rejects_noncanonical_overflow_and_duplicate_order_entries() {
        let cases = [
            br#"{
                "schema_version":2,
                "rooms":{"room":{"content":null,"revision":"01","last_accepted_clear_revision":"0"}},
                "threads":{},"quiescent_room_order":["room"],"quiescent_thread_order":[],
                "protected_empty_rooms":[],"protected_empty_threads":[]
            }"#
            .as_slice(),
            br#"{
                "schema_version":2,
                "rooms":{"room":{"content":null,"revision":"340282366920938463463374607431768211456","last_accepted_clear_revision":"0"}},
                "threads":{},"quiescent_room_order":["room"],"quiescent_thread_order":[],
                "protected_empty_rooms":[],"protected_empty_threads":[]
            }"#
            .as_slice(),
            br#"{
                "schema_version":2,
                "rooms":{"room":{"content":null,"revision":"1","last_accepted_clear_revision":"0"}},
                "threads":{},"quiescent_room_order":["room","room"],"quiescent_thread_order":[],
                "protected_empty_rooms":[],"protected_empty_threads":[]
            }"#
            .as_slice(),
            br#"{
                "schema_version":2,
                "rooms":{},
                "threads":{"room":{"root":{"content":null,"revision":"1","last_accepted_clear_revision":"0"}}},
                "quiescent_room_order":[],"quiescent_thread_order":[["room","root"],["room","root"]],
                "protected_empty_rooms":[],"protected_empty_threads":[]
            }"#
            .as_slice(),
        ];

        for payload in cases {
            let error = decode_payload_json(payload).expect_err("invalid v2 must be rejected");
            assert_eq!(error, ComposerDraftPayloadError::Corrupt);
            let debug = format!("{error:?}");
            assert_eq!(debug, "Corrupt");
            assert!(!debug.contains("room"));
            assert!(!debug.contains("340282366920938463463374607431768211456"));
        }
    }
}
