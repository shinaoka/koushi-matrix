use super::super::test_support::{file_store_actor, make_key_id};
use super::super::*;
use super::CoreFailure;
use super::*;
use tempfile::tempdir;

#[test]
fn atomic_read_state_outbox_replace_overwrites_an_existing_file() {
    let directory = tempdir().expect("tempdir");
    let path = directory.path().join("outbox.enc");

    koushi_store::atomic_replace_file(&path, b"first", false).expect("first atomic write");
    koushi_store::atomic_replace_file(&path, b"second", false).expect("replacement atomic write");

    assert_eq!(std::fs::read(path).expect("read replacement"), b"second");
}

#[test]
fn read_state_outbox_round_trips_encrypted_without_plaintext_identifiers() {
    use crate::read_state::{ReadStateEngine, ReadStateKey, ReadTarget, ReadWaiterId};

    let data_dir = tempdir().expect("tempdir");
    let cred_dir = tempdir().expect("tempdir");
    let key_id = make_key_id();
    let actor = file_store_actor(&data_dir, &cred_dir);
    let room_id = "!secret-room:test.example.com";
    let event_id = "$secret-event:test.example.com";
    let mut engine = ReadStateEngine::new(7);
    engine.admit(
        7,
        ReadStateKey::PublicUnthreaded {
            room_id: room_id.to_owned(),
        },
        ReadTarget::new(event_id.to_owned()),
        ReadWaiterId::new(1),
    );
    let snapshot = engine.persistence_snapshot();

    actor
        .save_read_state_outbox(&key_id, &snapshot)
        .expect("save encrypted read-state outbox");

    let path = actor.account_read_state_outbox_file(&key_id);
    let bytes = std::fs::read(&path).expect("read encrypted outbox");
    assert!(
        !bytes
            .windows(room_id.len())
            .any(|window| window == room_id.as_bytes())
    );
    assert!(
        !bytes
            .windows(event_id.len())
            .any(|window| window == event_id.as_bytes())
    );
    assert_eq!(
        actor
            .load_read_state_outbox(&key_id)
            .expect("load encrypted outbox"),
        snapshot
    );
    assert!(
        std::fs::read_dir(path.parent().expect("outbox parent"))
            .expect("read outbox parent")
            .all(|entry| !entry
                .expect("directory entry")
                .file_name()
                .to_string_lossy()
                .ends_with(".tmp"))
    );
}

#[test]
fn read_state_outbox_fails_closed_for_wrong_key_and_corruption() {
    use crate::read_state::{ReadStateEngine, ReadStateKey, ReadTarget, ReadWaiterId};

    let data_dir = tempdir().expect("tempdir");
    let first_cred_dir = tempdir().expect("tempdir");
    let second_cred_dir = tempdir().expect("tempdir");
    let key_id = make_key_id();
    let first = file_store_actor(&data_dir, &first_cred_dir);
    let second = file_store_actor(&data_dir, &second_cred_dir);
    let mut engine = ReadStateEngine::new(9);
    engine.admit(
        9,
        ReadStateKey::FullyReadAndPrivateUnthreaded {
            room_id: "!secret-room:test.example.com".to_owned(),
        },
        ReadTarget::new("$secret-event:test.example.com".to_owned()),
        ReadWaiterId::new(1),
    );
    first
        .save_read_state_outbox(&key_id, &engine.persistence_snapshot())
        .expect("save encrypted read-state outbox");

    second
        .credential_backend()
        .save(&key_id, &koushi_key::LocalUnlockSecret::generate())
        .expect("create a different unlock secret");
    assert!(matches!(
        second.load_read_state_outbox(&key_id),
        Err(CoreFailure::StoreUnavailable)
    ));

    let path = first.account_read_state_outbox_file(&key_id);
    let mut bytes = std::fs::read(&path).expect("read encrypted outbox");
    *bytes.last_mut().expect("encrypted payload byte") ^= 1;
    std::fs::write(&path, bytes).expect("write corrupt outbox");
    assert!(matches!(
        first.load_read_state_outbox(&key_id),
        Err(CoreFailure::StoreUnavailable)
    ));
}

#[test]
fn empty_read_state_snapshot_removes_the_durable_outbox() {
    let data_dir = tempdir().expect("tempdir");
    let cred_dir = tempdir().expect("tempdir");
    let key_id = make_key_id();
    let actor = file_store_actor(&data_dir, &cred_dir);
    let path = actor.account_read_state_outbox_file(&key_id);
    let snapshot = crate::read_state::ReadPersistenceSnapshot::default();

    actor
        .save_read_state_outbox(&key_id, &snapshot)
        .expect("empty snapshot delete is idempotent");

    assert!(!path.exists());
    assert_eq!(
        actor
            .load_read_state_outbox(&key_id)
            .expect("missing outbox is empty"),
        snapshot
    );
}

fn write_legacy_read_state_outbox(
    actor: &StoreActor,
    key_id: &SessionKeyId,
    entries: Vec<ReadPersistenceV1Entry>,
) {
    let secret = actor
        .load_or_create_unlock_secret(key_id)
        .expect("legacy outbox unlock secret");
    let plaintext = serde_json::to_vec(&(
        READ_STATE_OUTBOX_V1_VERSION,
        ReadPersistenceV1Snapshot { entries },
    ))
    .expect("legacy outbox payload");
    let payload = encrypt_read_state_payload(&secret, READ_STATE_OUTBOX_V1_MAGIC, plaintext)
        .expect("legacy outbox encryption");
    let path = actor.account_read_state_outbox_v1_file(key_id);
    std::fs::create_dir_all(path.parent().expect("legacy outbox parent"))
        .expect("legacy outbox parent");
    std::fs::write(path, payload).expect("legacy outbox write");
}

#[test]
fn v1_migration_conservatively_picks_last_entry_and_cleans_v1() {
    use crate::read_state::ReadStateKey;

    let data_dir = tempdir().expect("tempdir");
    let cred_dir = tempdir().expect("tempdir");
    let key_id = make_key_id();
    let actor = file_store_actor(&data_dir, &cred_dir);
    write_legacy_read_state_outbox(
        &actor,
        &key_id,
        vec![ReadPersistenceV1Entry {
            key: ReadStateKey::PublicUnthreaded {
                room_id: "!migration-room:example.invalid".to_owned(),
            },
            event_ids: vec![
                "$migration-a:example.invalid".to_owned(),
                "$migration-b:example.invalid".to_owned(),
                "$migration-c:example.invalid".to_owned(),
            ],
        }],
    );

    let snapshot = actor.load_read_state_outbox(&key_id).expect("V1 migration");
    assert_eq!(
        snapshot.entries()[0].event_id(),
        "$migration-c:example.invalid"
    );
    assert!(actor.account_read_state_outbox_file(&key_id).exists());
    assert!(!actor.account_read_state_outbox_v1_file(&key_id).exists());
}

#[test]
fn malformed_v1_is_retained_and_does_not_create_v2() {
    let data_dir = tempdir().expect("tempdir");
    let cred_dir = tempdir().expect("tempdir");
    let key_id = make_key_id();
    let actor = file_store_actor(&data_dir, &cred_dir);
    actor
        .load_or_create_unlock_secret(&key_id)
        .expect("malformed V1 unlock secret");
    let v1_path = actor.account_read_state_outbox_v1_file(&key_id);
    std::fs::create_dir_all(v1_path.parent().expect("legacy outbox parent"))
        .expect("legacy outbox parent");
    std::fs::write(&v1_path, b"malformed-v1").expect("malformed V1");

    assert!(matches!(
        actor.load_read_state_outbox(&key_id),
        Err(CoreFailure::StoreUnavailable)
    ));
    assert!(v1_path.exists());
    assert!(!actor.account_read_state_outbox_file(&key_id).exists());
}

#[test]
fn v2_write_failure_keeps_v1_for_a_later_crash_safe_retry() {
    use crate::read_state::{ReadStateEngine, ReadStateKey, ReadTarget, ReadWaiterId};

    let data_dir = tempdir().expect("tempdir");
    let cred_dir = tempdir().expect("tempdir");
    let key_id = make_key_id();
    let actor = file_store_actor(&data_dir, &cred_dir);
    write_legacy_read_state_outbox(
        &actor,
        &key_id,
        vec![ReadPersistenceV1Entry {
            key: ReadStateKey::PublicUnthreaded {
                room_id: "!crash-safe-room:example.invalid".to_owned(),
            },
            event_ids: vec!["$crash-safe:example.invalid".to_owned()],
        }],
    );
    let mut engine = ReadStateEngine::new(1);
    engine.admit(
        1,
        ReadStateKey::PublicUnthreaded {
            room_id: "!crash-safe-room:example.invalid".to_owned(),
        },
        ReadTarget::new("$crash-safe:example.invalid".to_owned()),
        ReadWaiterId::new(1),
    );
    let v2_path = actor.account_read_state_outbox_file(&key_id);
    std::fs::create_dir_all(&v2_path).expect("block V2 replacement with a directory");

    assert!(matches!(
        actor.save_read_state_outbox(&key_id, &engine.persistence_snapshot()),
        Err(CoreFailure::StoreUnavailable)
    ));
    assert!(actor.account_read_state_outbox_v1_file(&key_id).exists());
    assert!(v2_path.is_dir());
}

#[test]
fn valid_v2_load_cleans_a_leftover_v1_file() {
    use crate::read_state::{ReadStateEngine, ReadStateKey, ReadTarget, ReadWaiterId};

    let data_dir = tempdir().expect("tempdir");
    let cred_dir = tempdir().expect("cred_dir");
    let key_id = make_key_id();
    let actor = file_store_actor(&data_dir, &cred_dir);
    let mut engine = ReadStateEngine::new(1);
    engine.admit(
        1,
        ReadStateKey::FullyReadAndPrivateUnthreaded {
            room_id: "!dual-version:example.invalid".to_owned(),
        },
        ReadTarget::new("$v2:example.invalid".to_owned()),
        ReadWaiterId::new(1),
    );
    let snapshot = engine.persistence_snapshot();
    actor
        .save_read_state_outbox(&key_id, &snapshot)
        .expect("V2 save");
    write_legacy_read_state_outbox(
        &actor,
        &key_id,
        vec![ReadPersistenceV1Entry {
            key: ReadStateKey::FullyReadAndPrivateUnthreaded {
                room_id: "!dual-version:example.invalid".to_owned(),
            },
            event_ids: vec!["$old-v1:example.invalid".to_owned()],
        }],
    );

    assert_eq!(
        actor.load_read_state_outbox(&key_id).expect("V2 load"),
        snapshot
    );
    assert!(!actor.account_read_state_outbox_v1_file(&key_id).exists());
}

#[test]
fn stale_read_state_outbox_save_cannot_overwrite_newer_session_generation() {
    use crate::read_state::{ReadStateEngine, ReadStateKey, ReadTarget, ReadWaiterId};

    fn snapshot(event_id: &str) -> crate::read_state::ReadPersistenceSnapshot {
        let mut engine = ReadStateEngine::new(1);
        engine.admit(
            1,
            ReadStateKey::PublicUnthreaded {
                room_id: "!synthetic-room:test.example.com".to_owned(),
            },
            ReadTarget::new(event_id.to_owned()),
            ReadWaiterId::new(1),
        );
        engine.persistence_snapshot()
    }

    let data_dir = tempdir().expect("tempdir");
    let cred_dir = tempdir().expect("tempdir");
    let key_id = make_key_id();
    let actor = file_store_actor(&data_dir, &cred_dir);
    let newer = snapshot("$newer:test.example.com");
    let stale = snapshot("$stale:test.example.com");

    assert!(
        actor
            .save_read_state_outbox_if_current(&key_id, 2, 1, &newer)
            .expect("newer session save")
    );
    assert!(
        !actor
            .save_read_state_outbox_if_current(&key_id, 1, u64::MAX, &stale)
            .expect("stale session is rejected")
    );
    assert_eq!(
        actor
            .load_read_state_outbox(&key_id)
            .expect("load generation-fenced outbox"),
        newer
    );
}

#[test]
fn blocked_outbox_io_does_not_block_invalidation_or_win_after_timeout() {
    let data_dir = tempdir().expect("tempdir");
    let cred_dir = tempdir().expect("tempdir");
    let key_id = make_key_id();
    let actor = file_store_actor(&data_dir, &cred_dir);
    let path = actor.account_read_state_outbox_file(&key_id);
    let writer_path = path.clone();
    let (entered_tx, entered_rx) = std::sync::mpsc::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let writer = std::thread::spawn(move || {
        save_read_state_outbox_generation_fenced(&writer_path, 1, 1, || {
            entered_tx.send(()).expect("announce blocked write");
            let _ = release_rx.recv();
            std::fs::create_dir_all(writer_path.parent().expect("outbox parent directory"))
                .map_err(|_| CoreFailure::StoreUnavailable)?;
            std::fs::write(&writer_path, b"stale").map_err(|_| CoreFailure::StoreUnavailable)
        })
    });
    entered_rx
        .recv_timeout(std::time::Duration::from_secs(60))
        .expect("writer reaches simulated blocking IO");

    let invalidator = actor.clone();
    let invalidation_key = key_id.clone();
    let (invalidated_tx, invalidated_rx) = std::sync::mpsc::channel();
    let invalidation = std::thread::spawn(move || {
        invalidator.invalidate_read_state_outbox_saves(&invalidation_key, 2);
        invalidated_tx.send(()).expect("report invalidation");
    });
    // Liveness bound: the simulated IO is only released after this wait.
    invalidated_rx
        .recv_timeout(std::time::Duration::from_secs(60))
        .expect("invalidation must not wait for keychain or filesystem IO");

    release_tx.send(()).expect("release simulated IO");
    assert!(
        !writer
            .join()
            .expect("writer thread")
            .expect("generation-fenced write"),
        "the invalidated writer must report stale"
    );
    invalidation.join().expect("invalidation thread");
    assert!(
        !path.exists(),
        "a stale writer that finishes after invalidation must not win"
    );
}
