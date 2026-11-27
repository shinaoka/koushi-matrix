use super::*;
use std::os::unix::fs::PermissionsExt;

const ACCOUNT: &str = "account:@agent:example.invalid";

fn read_only() -> ProfileGrants {
    ProfileGrants {
        read: true,
        ..ProfileGrants::default()
    }
}

fn make(data: &Path) -> (CreatedProfile, String) {
    let created = create(data, "Reader", ACCOUNT, read_only(), None, 1).expect("created");
    let secret = std::fs::read_to_string(&created.secret_path)
        .expect("secret file")
        .trim()
        .to_owned();
    (created, secret)
}

#[test]
fn creation_writes_a_verifier_and_a_private_secret_file_once() {
    let dir = tempfile::tempdir().expect("tempdir");
    let (created, secret) = make(dir.path());

    // The secret is a 256-bit value in a 0600 file inside a 0700 directory.
    assert_eq!(secret.len(), 43, "32 bytes in unpadded base64url");
    let file = std::fs::metadata(&created.secret_path).expect("metadata");
    assert_eq!(file.permissions().mode() & 0o777, 0o600);
    let directory = std::fs::metadata(secret_dir(dir.path())).expect("metadata");
    assert_eq!(directory.permissions().mode() & 0o777, 0o700);

    // The store holds the SHA-256 verifier and never the secret.
    let stored = std::fs::read_to_string(profiles_path(dir.path())).expect("profiles.json");
    assert!(!stored.contains(&secret));
    assert!(stored.contains(&hex(&digest(&secret))));
    let row = lookup(dir.path(), &created.profile_id).expect("row");
    assert_eq!(
        (row.account_id.as_str(), row.grants),
        (ACCOUNT, read_only())
    );

    // `O_EXCL`: an existing path is refused and left as it was.
    let again = write_secret_file(&created.secret_path, "other");
    assert_eq!(
        again.expect_err("exists").kind(),
        std::io::ErrorKind::AlreadyExists
    );
    assert_eq!(
        std::fs::read_to_string(&created.secret_path)
            .unwrap()
            .trim(),
        secret
    );
}

#[test]
fn only_the_right_secret_authenticates() {
    let dir = tempfile::tempdir().expect("tempdir");
    let (created, secret) = make(dir.path());
    let id = created.profile_id.as_str();
    assert!(authenticate(dir.path(), id, &secret).is_some());
    for (id, secret) in [
        (id, "wrong"),
        (id, ""),
        (id, &secret[..secret.len() - 1]),
        ("prof_00000000000000000000000000000000", secret.as_str()),
        ("not-an-id", secret.as_str()),
        ("", ""),
    ] {
        assert!(
            authenticate(dir.path(), id, secret).is_none(),
            "{id} {secret}"
        );
    }
}

#[test]
fn revocation_removes_the_row_first_and_then_the_file_and_state() {
    let dir = tempfile::tempdir().expect("tempdir");
    let (created, secret) = make(dir.path());
    // Even when the secret file cannot be unlinked (a directory stands in for
    // it), the verifier row is already gone, so authentication has stopped.
    std::fs::remove_file(&created.secret_path).unwrap();
    std::fs::create_dir(&created.secret_path).unwrap();
    std::fs::write(created.secret_path.join("keep"), "x").unwrap();
    assert_eq!(revoke(dir.path(), &created.profile_id), Ok(true));
    assert!(lookup(dir.path(), &created.profile_id).is_none());
    assert!(authenticate(dir.path(), created.profile_id.as_str(), &secret).is_none());

    let (second, _) = make(dir.path());
    assert_eq!(revoke(dir.path(), &second.profile_id), Ok(true));
    assert!(!second.secret_path.exists());
    assert_eq!(revoke(dir.path(), &second.profile_id), Ok(false));
}

#[test]
fn an_owner_revoke_racing_stale_writers_never_resurrects_the_profile() {
    let dir = tempfile::tempdir().expect("tempdir");
    let data = dir.path().to_owned();
    let (created, _) = make(&data);
    let id = created.profile_id.clone();
    let writers: Vec<_> = (0..4)
        .map(|index| {
            let (data, id) = (data.clone(), id.clone());
            std::thread::spawn(move || {
                let mut outcomes = Vec::new();
                for round in 0..30 {
                    let need = Need::ALL[(index + round) % Need::ALL.len()];
                    outcomes.push(set_grant(&data, &id, ACCOUNT, need, round % 2 == 0));
                }
                outcomes
            })
        })
        .collect();
    std::thread::sleep(std::time::Duration::from_millis(5));
    assert_eq!(revoke(&data, &id), Ok(true));
    for writer in writers {
        for outcome in writer.join().expect("writer") {
            assert!(matches!(outcome, Ok(()) | Err(ProfileError::NotFound)));
        }
    }
    // Every writer re-reads under the lock, so none brought the row back.
    assert!(lookup(&data, &id).is_none());
    let stored = std::fs::read_to_string(profiles_path(&data)).expect("profiles.json");
    assert!(!stored.contains(id.as_str()));
}

#[test]
fn a_grant_edit_keeps_the_binding_and_refuses_another_account() {
    let dir = tempfile::tempdir().expect("tempdir");
    let (created, _) = make(dir.path());
    let id = &created.profile_id;
    assert_eq!(set_grant(dir.path(), id, ACCOUNT, Need::Send, true), Ok(()));
    assert_eq!(
        set_grant(
            dir.path(),
            id,
            "account:@other:example.invalid",
            Need::Send,
            true
        ),
        Err(ProfileError::AccountMismatch)
    );
    let row = lookup(dir.path(), id).expect("row");
    assert!(row.grants.send && row.grants.read && !row.grants.draft);
    assert_eq!(row.account_id, ACCOUNT, "the binding never moves");
}

#[test]
fn an_unusable_store_authorises_nothing_and_is_never_overwritten() {
    for (text, status) in [
        ("{not json", StoreStatus::Malformed),
        (
            r#"{"schema":99,"profiles":[]}"#,
            StoreStatus::UnsupportedSchema,
        ),
        (r#"{"profiles":[]}"#, StoreStatus::Malformed),
        (r#"{"schema":1,"profiles":{}}"#, StoreStatus::Malformed),
        (r#"{"schema":1,"profiles":[7]}"#, StoreStatus::Malformed),
        (
            r#"{"schema":1,"profiles":[{"profile_id":"prof_00000000000000000000000000000000"}]}"#,
            StoreStatus::Malformed,
        ),
    ] {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir_all(dir.path().join("mcp")).unwrap();
        std::fs::write(profiles_path(dir.path()), text).unwrap();
        assert_eq!(load(dir.path()).status, status, "{text}");
        assert!(
            lookup(
                dir.path(),
                &ProfileId::parse("prof_00000000000000000000000000000000").unwrap()
            )
            .is_none()
        );
        let created = create(dir.path(), "x", ACCOUNT, read_only(), None, 1);
        assert_eq!(created, Err(ProfileError::StoreUnusable(status)), "{text}");
        assert_eq!(
            std::fs::read_to_string(profiles_path(dir.path())).unwrap(),
            text
        );
        assert!(
            !secret_dir(dir.path()).exists()
                || std::fs::read_dir(secret_dir(dir.path()))
                    .unwrap()
                    .next()
                    .is_none(),
            "no secret file is left behind a refused create"
        );
    }
    let dir = tempfile::tempdir().expect("tempdir");
    assert_eq!(load(dir.path()).status, StoreStatus::Missing);
}

#[test]
fn unknown_keys_and_unusable_rows_survive_a_rewrite() {
    let dir = tempfile::tempdir().expect("tempdir");
    let (created, _) = make(dir.path());
    let mut value: Value =
        serde_json::from_slice(&std::fs::read(profiles_path(dir.path())).unwrap()).unwrap();
    value["future"] = json!({"kept": true});
    value["profiles"][0]["note"] = json!("by a newer build");
    value["profiles"]
        .as_array_mut()
        .unwrap()
        .push(json!({"profile_id": "garbage", "x": 1}));
    std::fs::write(profiles_path(dir.path()), value.to_string()).unwrap();

    set_grant(dir.path(), &created.profile_id, ACCOUNT, Need::Draft, true).expect("edit");
    let after: Value =
        serde_json::from_slice(&std::fs::read(profiles_path(dir.path())).unwrap()).unwrap();
    assert_eq!(after["future"], json!({"kept": true}));
    assert_eq!(after["profiles"][0]["note"], "by a newer build");
    assert_eq!(
        after["profiles"][1],
        json!({"profile_id": "garbage", "x": 1})
    );
}

#[test]
fn a_crash_between_the_secret_and_the_row_leaves_an_orphan_that_is_swept() {
    let dir = tempfile::tempdir().expect("tempdir");
    let (kept, secret) = make(dir.path());
    // A crash after the secret file (and baseline) but before the row.
    let orphan = ProfileId::parse("prof_11111111111111111111111111111111").unwrap();
    std::fs::create_dir_all(secret_dir(dir.path())).unwrap();
    write_secret_file(&secret_path(dir.path(), &orphan), "orphan-secret").unwrap();
    let state = ProfileState::default();
    profile_state::save(dir.path(), &orphan, &state).unwrap();
    assert!(authenticate(dir.path(), orphan.as_str(), "orphan-secret").is_none());

    sweep_orphans(dir.path());
    assert!(!secret_path(dir.path(), &orphan).exists());
    assert!(!profile_state::path(dir.path(), &orphan).exists());
    assert!(
        kept.secret_path.exists(),
        "a profile with a row is untouched"
    );
    assert!(authenticate(dir.path(), kept.profile_id.as_str(), &secret).is_some());

    // With an unreadable store nothing can be called an orphan.
    write_secret_file(&secret_path(dir.path(), &orphan), "x").unwrap();
    std::fs::write(profiles_path(dir.path()), "{broken").unwrap();
    sweep_orphans(dir.path());
    assert!(secret_path(dir.path(), &orphan).exists());
}

#[test]
fn the_baseline_is_kept_in_the_state_file_and_not_in_the_store() {
    let dir = tempfile::tempdir().expect("tempdir");
    let baseline = Baseline {
        feed_generation: "abcd".to_owned(),
        head_cursor: "mf1.abcd.7".to_owned(),
    };
    let created = create(
        dir.path(),
        "R",
        ACCOUNT,
        read_only(),
        Some(baseline.clone()),
        1,
    )
    .expect("created");
    let state = profile_state::load(dir.path(), &created.profile_id);
    assert_eq!(state.baseline, Some(baseline));
    let stored = std::fs::read_to_string(profiles_path(dir.path())).unwrap();
    assert!(!stored.contains("mf1.abcd.7"));
    let file = std::fs::metadata(profile_state::path(dir.path(), &created.profile_id)).unwrap();
    assert_eq!(file.permissions().mode() & 0o777, 0o600);

    // A rejection writes only the state file, coalesced to one per second.
    let before = std::fs::read(profiles_path(dir.path())).unwrap();
    note_rejected(dir.path(), created.profile_id.as_str());
    let first = profile_state::load(dir.path(), &created.profile_id)
        .last_rejected_ms
        .expect("recorded");
    note_rejected(dir.path(), created.profile_id.as_str());
    assert_eq!(
        profile_state::load(dir.path(), &created.profile_id).last_rejected_ms,
        Some(first)
    );
    assert_eq!(std::fs::read(profiles_path(dir.path())).unwrap(), before);
    assert_eq!(
        profile_state::load(dir.path(), &created.profile_id)
            .baseline
            .map(|b| b.head_cursor),
        Some("mf1.abcd.7".to_owned()),
        "a rejection keeps the baseline"
    );
}

#[test]
fn a_label_is_cleaned_and_bounded_and_the_row_debug_hides_the_verifier() {
    let dir = tempfile::tempdir().expect("tempdir");
    let created = create(
        dir.path(),
        &format!("  Evil\u{202E}name\n{}", "x".repeat(200)),
        ACCOUNT,
        read_only(),
        None,
        1,
    )
    .expect("created");
    let row = lookup(dir.path(), &created.profile_id).expect("row");
    assert!(!row.label.contains('\u{202E}') && !row.label.contains('\n'));
    assert!(row.label.chars().count() <= LABEL_CHARS + 1);
    assert!(!format!("{row:?}").contains(&hex(&row.verifier)));
    assert!(ProfileId::parse("prof_../../../../etc/passwd/0000000000").is_none());
}
