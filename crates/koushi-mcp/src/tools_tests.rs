//! Tests for the tool schemas and argument validation kept out of the
//! module body (the inline-module ceiling is 200 lines).

use super::*;

#[test]
fn definitions_are_unique_and_valid() {
    let tools = definitions();
    assert_eq!(tools.len(), 15);
    let mut names: Vec<&str> = tools.iter().map(|tool| tool.name).collect();
    names.sort_unstable();
    names.dedup();
    assert_eq!(names.len(), tools.len());
    for tool in &tools {
        assert_eq!(tool.input_schema["type"], "object");
        assert!(tool.input_schema.get("properties").is_some());
    }
}

#[test]
fn account_scoped_tools_require_account_id() {
    for name in [
        "list_rooms",
        "search_messages",
        "wait_for_events",
        "read_room_messages",
        "read_recent_activity",
        "get_message",
        "react_to_message",
        "unreact_to_message",
        "edit_message",
        "delete_message",
        "send_thread_reply",
        "draft_message",
    ] {
        let tool = find(name).expect("tool");
        let required = tool.input_schema["required"].as_array().expect("required");
        assert!(required.iter().any(|value| value == "account_id"), "{name}");
    }
}

#[test]
fn validation_rejects_missing_and_unknown_arguments() {
    assert!(validate_arguments("list_accounts", &json!({})).is_ok());
    assert!(validate_arguments("search_messages", &json!({"account_id": "a"})).is_err());
    assert!(
        validate_arguments(
            "search_messages",
            &json!({"account_id": "a", "query": "x", "bogus": 1})
        )
        .is_err()
    );
    assert!(
        validate_arguments("search_messages", &json!({"account_id": "", "query": "x"})).is_err()
    );
    assert!(
        validate_arguments("search_messages", &json!({"account_id": "a", "query": "x"})).is_ok()
    );
}

#[test]
fn wait_for_events_publishes_its_coverage_and_cursor_contract() {
    let tool = find("wait_for_events").expect("tool");
    for phrase in [
        "not a message log",
        "coalesce",
        "Invites are not reported",
        "only sets a baseline",
        "at most 500",
        "not removed when a room is read",
        "not that no message was posted",
        "resync_required",
        "read_room_messages",
    ] {
        assert!(tool.description.contains(phrase), "{phrase}");
    }
    let schema = tool.output_schema.expect("output schema");
    assert_eq!(
        schema["properties"]["coverage"]["enum"],
        json!(["sampled_room_changes"])
    );
    // Typed outputs: read_recent_activity, read_room_messages, wait_for_events,
    // get_message, the five message actions (#1628), and draft_message (#1644).
    let others_typed = definitions()
        .iter()
        .filter(|tool| tool.output_schema.is_some())
        .count();
    assert_eq!(others_typed, 10);
}

#[test]
fn send_message_describes_its_attachment_contract() {
    let tool = find("send_message").expect("tool");
    for phrase in [
        "absolute local file paths",
        "attachments permission",
        "caption",
    ] {
        assert!(tool.description.contains(phrase), "{phrase}");
    }
    let path = &tool.input_schema["properties"]["attachments"]["items"]["properties"]["path"];
    assert!(
        path["description"]
            .as_str()
            .is_some_and(|text| text.starts_with("Absolute"))
    );
}

#[test]
fn read_only_flags_match_the_write_tools() {
    let writes: Vec<&str> = definitions()
        .iter()
        .filter(|tool| !tool.read_only)
        .map(|tool| tool.name)
        .collect();
    assert_eq!(
        writes,
        vec![
            "send_message",
            "react_to_message",
            "unreact_to_message",
            "edit_message",
            "delete_message",
            "send_thread_reply",
            "draft_message",
        ]
    );
}

#[test]
fn draft_message_never_sends_and_declares_its_permission_and_conflict() {
    let tool = find("draft_message").expect("draft_message");
    assert!(!tool.read_only, "a draft changes the user's composer");
    for phrase in [
        "Nothing is sent",
        "draft permission",
        "separate from send",
        "conflict",
        "does not switch",
    ] {
        assert!(tool.description.contains(phrase), "{phrase}");
    }
    let properties = tool.input_schema["properties"]
        .as_object()
        .expect("properties");
    assert!(properties.contains_key("thread_root_event_id"));
    assert!(!properties.contains_key("attachments"));
    assert!(!properties.contains_key("reply_to_event_id"));
    assert!(!properties.contains_key("client_request_id"));
    assert!(
        validate_arguments(
            "draft_message",
            &json!({"account_id": "a", "room_id": "!r", "body": "hi"})
        )
        .is_ok()
    );
    assert!(
        validate_arguments(
            "draft_message",
            &json!({"account_id": "a", "room_id": "!r", "body": "hi", "attachments": []})
        )
        .is_err()
    );
    let output = tool.output_schema.expect("typed output");
    assert_eq!(output["properties"]["status"]["const"], "staged");
}

#[test]
fn message_action_tools_declare_their_permission_and_idempotency_contract() {
    let get_message = find("get_message").expect("get_message");
    assert!(get_message.read_only);
    let schema = get_message.output_schema.expect("typed output");
    for field in [
        "body",
        "content_available",
        "reactions",
        "relations_truncated",
        "edited",
        "thread_root_event_id",
        "reply_to_event_id",
    ] {
        assert!(schema["properties"].get(field).is_some(), "{field}");
    }
    for phrase in [
        "latest valid edit",
        "bounded relations page",
        "cannot decrypt",
    ] {
        assert!(get_message.description.contains(phrase), "{phrase}");
    }

    for name in [
        "react_to_message",
        "unreact_to_message",
        "edit_message",
        "delete_message",
    ] {
        let tool = find(name).expect(name);
        assert!(!tool.read_only, "{name}");
        assert!(tool.output_schema.is_some(), "{name}");
        assert!(
            tool.description.contains("message actions permission"),
            "{name}"
        );
        assert!(
            tool.input_schema["required"]
                .as_array()
                .expect("required")
                .iter()
                .any(|value| value == "account_id"),
            "{name}"
        );
    }
    for name in ["edit_message", "send_thread_reply"] {
        let tool = find(name).expect(name);
        assert!(
            tool.input_schema["required"]
                .as_array()
                .expect("required")
                .iter()
                .any(|value| value == "client_request_id"),
            "{name}"
        );
    }
    let reply = find("send_thread_reply").expect("thread reply");
    assert!(!reply.read_only);
    assert!(reply.description.contains("send permission"));
    assert!(reply.description.contains("does not post twice"));
    assert!(reply.description.contains("cannot be recalled"));
    assert!(
        reply.input_schema["properties"]
            .get("attachments")
            .is_none(),
        "thread replies do not accept attachments in v1"
    );

    let edit = find("edit_message").expect("edit");
    assert!(edit.description.contains("media payload"));
    assert!(edit.description.contains("echoes client_request_id"));
    assert!(edit.description.contains("not_permitted"));
    let delete = find("delete_message").expect("delete");
    assert!(delete.description.contains("already_redacted"));
    let react = find("react_to_message").expect("react");
    assert!(react.description.contains("Set semantics"));
    assert_eq!(
        react.input_schema["properties"]["key"]["minLength"], 1,
        "reaction keys are never empty"
    );
}
#[test]
fn recent_activity_discovery_is_read_only_and_describes_bounded_history() {
    let tool = find("read_recent_activity").expect("Recent tool");
    assert!(tool.read_only);
    assert!(tool.description.contains("read"));
    assert!(tool.output_schema.is_some());
    assert_eq!(tool.input_schema["properties"]["limit"]["minimum"], 1);
    assert_eq!(tool.input_schema["properties"]["limit"]["maximum"], 100);
}
