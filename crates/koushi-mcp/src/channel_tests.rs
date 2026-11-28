use super::*;

fn row() -> Value {
    json!({
        "cursor": "mf1.a.1",
        "account_id": "account:@agent:example.invalid",
        "room_id": "!dm:example.invalid",
        "event_id": "$e1",
        "sender_id": "@owner:example.invalid",
        "timestamp_ms": 1,
        "thread_root_event_id": "$root",
        "kind": "mention",
    })
}

#[test]
fn a_row_becomes_one_channel_notification_with_identifier_only_meta() {
    let message = notification_for_row(&row(), Some("hello agent".to_owned()));
    assert_eq!(message["method"], "notifications/claude/channel");
    assert!(message.get("id").is_none(), "a notification has no id");
    assert_eq!(message["params"]["content"], "hello agent");
    let meta = message["params"]["meta"].as_object().expect("meta");
    let mut keys: Vec<_> = meta.keys().map(String::as_str).collect();
    keys.sort_unstable();
    assert_eq!(
        keys,
        ["event_id", "kind", "room_id", "sender", "thread_root"]
    );
    for key in meta.keys() {
        assert!(key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'));
    }
    assert!(meta.values().all(Value::is_string));
    assert_eq!(meta["sender"], "@owner:example.invalid");
    assert_eq!(meta["thread_root"], "$root");
}

#[test]
fn absent_fields_are_omitted_not_nulled() {
    let mut invite = row();
    invite["event_id"] = Value::Null;
    invite["thread_root_event_id"] = Value::Null;
    invite["kind"] = json!("invite");
    let message = notification_for_row(&invite, None);
    let meta = message["params"]["meta"].as_object().expect("meta");
    assert!(!meta.contains_key("event_id") && !meta.contains_key("thread_root"));
    assert_eq!(meta["kind"], "invite");
    let content = message["params"]["content"].as_str().expect("content");
    assert!(content.contains("get_message"), "{content}");
}

#[test]
fn long_text_is_cut_on_a_character_boundary() {
    let text = "é".repeat(MAX_CONTENT_BYTES);
    let message = notification_for_row(&row(), Some(text));
    let content = message["params"]["content"].as_str().expect("content");
    assert!(content.len() < MAX_CONTENT_BYTES + 100);
    assert!(content.ends_with("get_message]"));
}

#[test]
fn a_redelivered_cursor_is_seen_once_and_the_memory_is_bounded() {
    let mut seen = Seen::new();
    assert!(seen.insert("c1"));
    assert!(!seen.insert("c1"));
    for index in 0..SEEN_CAPACITY + 10 {
        seen.insert(&format!("x{index}"));
    }
    assert_eq!(seen.set.len(), SEEN_CAPACITY);
    assert!(seen.insert("c1"), "the oldest entry was forgotten");
}

#[test]
fn status_notes_use_a_status_kind_and_no_ids() {
    let message = status_notification("profile_busy", PROFILE_BUSY_TEXT);
    assert_eq!(message["params"]["meta"], json!({"kind": "profile_busy"}));
    assert!(
        message["params"]["content"]
            .as_str()
            .expect("content")
            .contains("request tools keep working")
    );
}
