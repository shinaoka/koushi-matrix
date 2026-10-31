//! The advertised room-page result preserves unreadable message envelopes.

use koushi_mcp::tools;
use serde_json::json;

#[test]
fn room_messages_schema_declares_envelopes_and_content_availability() {
    let tool = tools::find("read_room_messages").expect("tool");
    assert!(tool.read_only);
    let schema = tool.output_schema.expect("typed room-message page");
    assert_eq!(schema["type"], "object");
    assert_eq!(schema["required"], json!(["room_id", "messages", "next"]));
    assert_eq!(
        schema["properties"]["next"]["type"],
        json!(["string", "null"])
    );
    let item = &schema["properties"]["messages"]["items"];
    assert_eq!(item["properties"]["content_available"]["type"], "boolean");
    assert_eq!(
        item["properties"]["body"]["type"],
        json!(["string", "null"])
    );
    for field in [
        "event_id",
        "sender_id",
        "timestamp_ms",
        "body",
        "kind",
        "is_encrypted",
        "is_redacted",
        "content_available",
    ] {
        assert!(item["properties"].get(field).is_some(), "{field}");
        assert!(
            item["required"]
                .as_array()
                .expect("required")
                .contains(&json!(field)),
            "{field}"
        );
    }
}
