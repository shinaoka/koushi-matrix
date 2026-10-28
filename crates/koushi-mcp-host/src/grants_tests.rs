use super::*;

const ALICE: &str = "account:@alice:example.invalid";
const BOB: &str = "account:@bob:example.invalid";

fn write_raw(dir: &Path, text: &str) {
    std::fs::create_dir_all(dir.join("mcp")).expect("mkdir");
    std::fs::write(grants_path(dir), text).expect("write");
}

#[test]
fn a_missing_file_grants_nothing_and_may_be_created() {
    let dir = tempfile::tempdir().expect("tempdir");
    let loaded = load(dir.path());
    assert_eq!(loaded.status, StoreStatus::Missing);
    assert!(loaded.writable());
    assert!(!loaded.store.allows("claude-code", ALICE, Need::Read));
}

#[test]
fn schema_one_gives_each_listed_account_the_entry_flags() {
    let dir = tempfile::tempdir().expect("tempdir");
    write_raw(
        dir.path(),
        r#"{"connections":[{"client_name":"pi","accounts":["account:@alice:example.invalid"],
            "allow_read":true,"allow_send":false,"allow_attachments":true}]}"#,
    );
    let loaded = load(dir.path());
    assert_eq!(loaded.status, StoreStatus::Ok);
    assert!(loaded.store.allows("pi", ALICE, Need::Read));
    assert!(!loaded.store.allows("pi", ALICE, Need::Send));
    assert!(loaded.store.allows("pi", ALICE, Need::Attachments));
    assert!(!loaded.store.allows("pi", BOB, Need::Read));
}

#[test]
fn schema_one_keeps_first_entry_wins() {
    let dir = tempfile::tempdir().expect("tempdir");
    write_raw(
        dir.path(),
        r#"{"schema":1,"connections":[
            {"client_name":"pi","accounts":["account:@alice:example.invalid"],"allow_read":true},
            {"client_name":"pi","accounts":["account:@alice:example.invalid"],"allow_read":true,"allow_send":true}]}"#,
    );
    let loaded = load(dir.path());
    assert!(loaded.store.allows("pi", ALICE, Need::Read));
    assert!(
        !loaded.store.allows("pi", ALICE, Need::Send),
        "a later duplicate must not widen the first entry"
    );
}

#[test]
fn malformed_and_newer_files_grant_nothing_and_are_not_writable() {
    let dir = tempfile::tempdir().expect("tempdir");
    write_raw(dir.path(), "{ not json");
    let loaded = load(dir.path());
    assert_eq!(loaded.status, StoreStatus::Malformed);
    assert!(!loaded.writable());

    write_raw(dir.path(), r#"{"schema":4,"connections":[]}"#);
    let loaded = load(dir.path());
    assert_eq!(loaded.status, StoreStatus::UnsupportedSchema);
    assert!(!loaded.writable());

    write_raw(
        dir.path(),
        r#"{"schema":2,"connections":[{"client_name":"pi","accounts":[{"account_id":"x","read":"yes"}]}]}"#,
    );
    assert_eq!(load(dir.path()).status, StoreStatus::Malformed);
}

#[test]
fn schema_two_rows_default_message_actions_off_and_keep_their_other_flags() {
    let dir = tempfile::tempdir().expect("tempdir");
    write_raw(
        dir.path(),
        r#"{"schema":2,"connections":[{"client_name":"pi","accounts":[
            {"account_id":"account:@alice:example.invalid","read":true,"send":true,"attachments":false}]}]}"#,
    );
    let loaded = load(dir.path());
    assert_eq!(loaded.status, StoreStatus::Ok);
    assert!(loaded.store.allows("pi", ALICE, Need::Read));
    assert!(loaded.store.allows("pi", ALICE, Need::Send));
    assert!(
        !loaded.store.allows("pi", ALICE, Need::MessageActions),
        "schema 2 has no message_actions flag, so it must default to false"
    );
}

#[test]
fn schema_three_round_trips_message_actions_without_widening_another_need() {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut store = Store::default();
    store.set("pi", ALICE, Need::MessageActions, true);
    save(dir.path(), &store).expect("save");

    let written: Value =
        serde_json::from_slice(&std::fs::read(grants_path(dir.path())).expect("read"))
            .expect("json");
    assert_eq!(written["schema"], 3);
    assert_eq!(
        written["connections"][0]["accounts"][0]["message_actions"],
        true
    );
    assert_eq!(written["connections"][0]["accounts"][0]["send"], false);

    let reloaded = load(dir.path());
    assert_eq!(reloaded.status, StoreStatus::Ok);
    assert!(reloaded.store.allows("pi", ALICE, Need::MessageActions));
    assert!(!reloaded.store.allows("pi", ALICE, Need::Send));
}

#[test]
fn a_schema_three_row_without_draft_grants_no_draft() {
    let dir = tempfile::tempdir().expect("tempdir");
    write_raw(
        dir.path(),
        r#"{"schema":3,"connections":[{"client_name":"pi","accounts":[
            {"account_id":"account:@alice:example.invalid","read":true,"send":true,"attachments":false,"message_actions":true}]}]}"#,
    );
    let loaded = load(dir.path());
    assert_eq!(loaded.status, StoreStatus::Ok);
    assert!(loaded.store.allows("pi", ALICE, Need::Send));
    assert!(
        !loaded.store.allows("pi", ALICE, Need::Draft),
        "rows written before #1644 have no draft flag"
    );
}

#[test]
fn draft_round_trips_in_schema_three_without_granting_send() {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut store = Store::default();
    store.set("pi", ALICE, Need::Draft, true);
    save(dir.path(), &store).expect("save");

    let written: Value =
        serde_json::from_slice(&std::fs::read(grants_path(dir.path())).expect("read"))
            .expect("json");
    assert_eq!(written["schema"], 3);
    assert_eq!(written["connections"][0]["accounts"][0]["draft"], true);
    assert_eq!(written["connections"][0]["accounts"][0]["send"], false);

    let reloaded = load(dir.path());
    assert!(reloaded.store.allows("pi", ALICE, Need::Draft));
    assert!(!reloaded.store.allows("pi", ALICE, Need::Send));
}

#[test]
fn setting_one_permission_touches_only_that_account_and_need() {
    let mut store = Store::default();
    store.set("claude-code", ALICE, Need::Read, true);
    assert!(store.allows("claude-code", ALICE, Need::Read));
    assert!(!store.allows("claude-code", ALICE, Need::Send));
    assert!(!store.allows("claude-code", BOB, Need::Read));
    assert!(!store.allows("codex", ALICE, Need::Read));
}

#[test]
fn a_write_upgrades_schema_one_and_keeps_unknown_fields() {
    let dir = tempfile::tempdir().expect("tempdir");
    write_raw(
        dir.path(),
        r#"{"note":"kept","connections":[{"client_name":"pi","label":"kept too",
            "accounts":["account:@alice:example.invalid"],"allow_read":true},
            {"comment":"no name"}]}"#,
    );
    let mut loaded = load(dir.path());
    loaded.store.set("pi", ALICE, Need::Send, true);
    save(dir.path(), &loaded.store).expect("save");

    let written: Value =
        serde_json::from_slice(&std::fs::read(grants_path(dir.path())).expect("read"))
            .expect("json");
    assert_eq!(written["schema"], 3);
    assert_eq!(written["note"], "kept");
    assert_eq!(written["connections"][0]["label"], "kept too");
    assert_eq!(written["connections"][0]["accounts"][0]["read"], true);
    assert_eq!(written["connections"][0]["accounts"][0]["send"], true);
    assert_eq!(written["connections"][1]["comment"], "no name");

    let reloaded = load(dir.path());
    assert_eq!(reloaded.status, StoreStatus::Ok);
    assert!(reloaded.store.allows("pi", ALICE, Need::Send));
}

#[cfg(unix)]
#[test]
fn the_written_file_is_private() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().expect("tempdir");
    let mut store = Store::default();
    store.set("pi", ALICE, Need::Read, true);
    save(dir.path(), &store).expect("save");
    let mode = std::fs::metadata(grants_path(dir.path()))
        .expect("metadata")
        .permissions()
        .mode();
    assert_eq!(mode & 0o777, 0o600);
}

#[test]
fn removing_a_client_removes_every_entry_with_its_name() {
    let dir = tempfile::tempdir().expect("tempdir");
    write_raw(
        dir.path(),
        r#"{"connections":[
            {"client_name":"pi","accounts":["account:@alice:example.invalid"],"allow_read":true},
            {"client_name":"pi","accounts":["account:@alice:example.invalid"],"allow_read":true,"allow_send":true}]}"#,
    );
    let mut loaded = load(dir.path());
    assert!(loaded.store.remove("pi"));
    assert!(!loaded.store.allows("pi", ALICE, Need::Read));
    assert!(loaded.store.client("pi").is_none());
}
