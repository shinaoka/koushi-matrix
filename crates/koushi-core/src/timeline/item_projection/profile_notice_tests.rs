use super::*;
use matrix_sdk::{
    deserialized_responses::TimelineEvent,
    ruma::{events::AnySyncTimelineEvent, room_id, serde::Raw},
    test_utils::mocks::MatrixMockServer,
};
use serde_json::json;

#[tokio::test]
async fn profile_changes_project_localizable_notices() {
    let server = MatrixMockServer::new().await;
    let client = server.client_builder().build().await;
    let room = server
        .sync_joined_room(&client, room_id!("!profile:example.invalid"))
        .await;
    for (index, (name, avatar, expected)) in [
        (
            Some("Member 2"),
            Some("mxc://example.invalid/old"),
            "timeline.notice.profileDisplayNameChanged",
        ),
        (
            Some("Member 1"),
            Some("mxc://example.invalid/new"),
            "timeline.notice.profileAvatarChanged",
        ),
        (
            Some("Member 2"),
            Some("mxc://example.invalid/new"),
            "timeline.notice.profileDisplayNameAndAvatarChanged",
        ),
        (
            None,
            Some("mxc://example.invalid/old"),
            "timeline.notice.profileDisplayNameChanged",
        ),
        (
            Some("Member 1"),
            None,
            "timeline.notice.profileAvatarChanged",
        ),
    ]
    .into_iter()
    .enumerate()
    {
        let event = json!({
            "type": "m.room.member", "state_key": "@member:example.invalid",
            "sender": "@member:example.invalid", "event_id": format!("$profile-{index}:example.invalid"),
            "origin_server_ts": 1,
            "content": {"membership": "join", "displayname": name, "avatar_url": avatar},
            "unsigned": {"prev_content": {"membership": "join", "displayname": "Member 1", "avatar_url": "mxc://example.invalid/old"}}
        });
        let raw = Raw::<AnySyncTimelineEvent>::from_json_string(event.to_string()).unwrap();
        let content = TimelineItemContent::from_event(&room, TimelineEvent::from_plaintext(raw))
            .await
            .unwrap();
        assert!(matches!(content, TimelineItemContent::ProfileChange(_)));
        let projection = message_projection_from_timeline_content(&content);
        let notice = projection
            .notice_i18n
            .expect("profile changes must use the message catalog");
        assert_eq!(serde_json::to_value(notice.key).unwrap(), json!(expected));
        assert_eq!(projection.message_kind, TimelineMessageKind::Notice);
        assert!(!projection.body_is_user_content);
    }
}
