use super::*;
use matrix_sdk::{
    deserialized_responses::TimelineEvent,
    ruma::{events::AnySyncTimelineEvent, room_id, serde::Raw},
    test_utils::mocks::MatrixMockServer,
};
use serde_json::{Value, json};

fn wire_event(event_type: &str, content: Value, state: bool, index: usize) -> Value {
    let mut event = json!({
        "type": event_type, "content": content,
        "event_id": format!("$policy-{index}:example.invalid"),
        "sender": "@alice:example.invalid", "origin_server_ts": 1,
    });
    if state {
        event["state_key"] = if event_type.starts_with("m.space.") {
            json!("!related:example.invalid")
        } else if event_type == "m.call.member" {
            json!("@alice:example.invalid")
        } else {
            json!("")
        };
    }
    event
}

#[tokio::test]
async fn sdk_wire_state_inventory_has_deliberate_projection() {
    let server = MatrixMockServer::new().await;
    let client = server.client_builder().build().await;
    let room = server
        .sync_joined_room(&client, room_id!("!policy:example.invalid"))
        .await;
    let cases = [
        (
            "m.room.create",
            json!({"creator":"@alice:example.invalid"}),
            Some(TimelineNoticeI18nKey::RoomCreate),
        ),
        (
            "m.room.power_levels",
            json!({}),
            Some(TimelineNoticeI18nKey::RoomPowerLevels),
        ),
        (
            "m.room.guest_access",
            json!({"guest_access":"forbidden"}),
            Some(TimelineNoticeI18nKey::RoomGuestAccess),
        ),
        (
            "m.room.encryption",
            json!({"algorithm":"m.megolm.v1.aes-sha2"}),
            Some(TimelineNoticeI18nKey::RoomEncryption),
        ),
        (
            "m.room.join_rules",
            json!({"join_rule":"invite"}),
            Some(TimelineNoticeI18nKey::RoomJoinRules),
        ),
        (
            "m.room.history_visibility",
            json!({"history_visibility":"shared"}),
            Some(TimelineNoticeI18nKey::RoomHistoryVisibility),
        ),
        (
            "m.room.pinned_events",
            json!({"pinned":[]}),
            Some(TimelineNoticeI18nKey::RoomPinnedEvents),
        ),
        (
            "m.room.name",
            json!({"name":"Synthetic room"}),
            Some(TimelineNoticeI18nKey::RoomNameSet),
        ),
        (
            "m.space.parent",
            json!({"via":["example.invalid"]}),
            Some(TimelineNoticeI18nKey::SpaceParent),
        ),
        (
            "m.room.topic",
            json!({"topic":"<synthetic topic>"}),
            Some(TimelineNoticeI18nKey::RoomTopicSet),
        ),
        (
            "m.room.avatar",
            json!({"url":"mxc://example.invalid/avatar"}),
            Some(TimelineNoticeI18nKey::RoomAvatarChanged),
        ),
        (
            "m.room.tombstone",
            json!({"body":"Replacement", "replacement_room":"!new:example.invalid"}),
            Some(TimelineNoticeI18nKey::RoomUpgraded),
        ),
        (
            "m.room.third_party_invite",
            json!({"display_name":"Synthetic guest","public_key":"key","key_validity_url":"https://example.invalid/key"}),
            Some(TimelineNoticeI18nKey::RoomThirdPartyInvite),
        ),
        (
            "m.room.canonical_alias",
            json!({"alias":"#synthetic:example.invalid"}),
            None,
        ),
        ("m.space.child", json!({"via":["example.invalid"]}), None),
        ("m.room.server_acl", json!({"allow":["*"]}), None),
        (
            "m.policy.rule.room",
            json!({"entity":"!room:example.invalid","recommendation":"m.ban","reason":"Synthetic"}),
            None,
        ),
        (
            "m.policy.rule.server",
            json!({"entity":"example.invalid","recommendation":"m.ban","reason":"Synthetic"}),
            None,
        ),
        (
            "m.policy.rule.user",
            json!({"entity":"@user:example.invalid","recommendation":"m.ban","reason":"Synthetic"}),
            None,
        ),
        ("m.room.image_pack", json!({"images":{}}), None),
        (
            "m.room.policy",
            json!({"via":"example.invalid","public_keys":{"ed25519":"a2V5"}}),
            None,
        ),
        (
            "org.matrix.msc1763.retention",
            json!({"max_lifetime":1000}),
            None,
        ),
        ("m.room.retention", json!({}), None),
        (
            "io.element.functional_members",
            json!({"service_members":[]}),
            None,
        ),
        ("m.member_hints", json!({"service_members":[]}), None),
        ("m.call.member", json!({}), None),
        ("m.room.language", json!({}), None),
        ("m.room.presence_sharing", json!({}), None),
        ("im.vector.modular.widgets", json!({}), None),
        ("org.example.future", json!({}), None),
    ];
    for (i, (ty, content, expected)) in cases.into_iter().enumerate() {
        let raw: Raw<AnySyncTimelineEvent> =
            Raw::from_json_string(wire_event(ty, content, true, i).to_string()).unwrap();
        let parsed = raw.deserialize().expect(ty);
        assert!(
            matrix_sdk_ui::timeline::default_event_filter(
                &parsed,
                &matrix_sdk::ruma::RoomVersionId::V10.rules().unwrap()
            ),
            "{ty}"
        );
        let content = TimelineItemContent::from_event(&room, TimelineEvent::from_plaintext(raw))
            .await
            .expect(ty);
        let projection = message_projection_from_timeline_content(&content);
        assert_eq!(
            projection.notice_i18n.as_ref().map(|n| n.key),
            expected,
            "{ty}"
        );
        assert_eq!(projection.body.is_some(), expected.is_some(), "{ty}");
        assert!(
            !projection
                .body
                .as_deref()
                .unwrap_or_default()
                .contains("Unsupported event:")
        );
        if ty == "m.room.tombstone" {
            assert_eq!(
                projection
                    .notice_i18n
                    .unwrap()
                    .replacement_room_id
                    .as_deref(),
                Some("!new:example.invalid")
            );
        }
    }
}

#[test]
fn topic_and_avatar_changes_preserve_removal_and_redaction_meaning() {
    use matrix_sdk::ruma::events::room::{
        avatar::RoomAvatarEventContent, topic::RoomTopicEventContent,
    };
    let topic = |current: &str, previous: Option<&str>| {
        AnyOtherStateEventContentChange::RoomTopic(StateEventContentChange::Original {
            content: RoomTopicEventContent::new(current.into()),
            prev_content: previous
                .map(|topic| serde_json::from_value(json!({"topic":topic})).unwrap()),
        })
    };
    for (current, previous, expected) in [
        ("New", None, TimelineNoticeI18nKey::RoomTopicSet),
        ("New", Some("Old"), TimelineNoticeI18nKey::RoomTopicChanged),
        ("", Some("Old"), TimelineNoticeI18nKey::RoomTopicRemoved),
    ] {
        assert_eq!(
            other_state_projection(&topic(current, previous))
                .notice_i18n
                .unwrap()
                .key,
            expected
        );
    }
    let removed = AnyOtherStateEventContentChange::RoomAvatar(StateEventContentChange::Original {
        content: RoomAvatarEventContent::new(),
        prev_content: None,
    });
    assert_eq!(
        other_state_projection(&removed).notice_i18n.unwrap().key,
        TimelineNoticeI18nKey::RoomAvatarRemoved
    );
    let redacted = AnyOtherStateEventContentChange::RoomTopic(StateEventContentChange::Redacted(
        serde_json::from_value(json!({})).unwrap(),
    ));
    assert!(other_state_projection(&redacted).body.is_none());
}

#[test]
fn malformed_calls_and_decryption_have_distinct_localized_notices() {
    use matrix_sdk_ui::timeline::MsgLikeContent;
    let error = || Arc::new(serde_json::from_str::<Value>("{").unwrap_err());
    for (content, expected) in [
        (
            TimelineItemContent::FailedToParseState {
                event_type: "m.room.encryption".into(),
                state_key: String::new(),
                error: error(),
            },
            TimelineNoticeI18nKey::MalformedEvent,
        ),
        (
            TimelineItemContent::FailedToParseMessageLike {
                event_type: "m.room.message".into(),
                error: error(),
            },
            TimelineNoticeI18nKey::MalformedEvent,
        ),
        (TimelineItemContent::CallInvite, TimelineNoticeI18nKey::Call),
        (
            TimelineItemContent::RtcNotification {
                call_intent: None,
                declined_by: vec![],
                active_call_info: None,
            },
            TimelineNoticeI18nKey::Call,
        ),
        (
            TimelineItemContent::MsgLike(MsgLikeContent::unable_to_decrypt(
                EncryptedMessage::Unknown,
            )),
            TimelineNoticeI18nKey::UnableToDecrypt,
        ),
    ] {
        assert_eq!(
            message_projection_from_timeline_content(&content)
                .notice_i18n
                .unwrap()
                .key,
            expected
        );
    }
    assert!(
        message_projection_from_timeline_content(&TimelineItemContent::MsgLike(
            MsgLikeContent::redacted()
        ))
        .body
        .is_none()
    );
}

#[test]
fn housekeeping_stays_hidden_after_ignore_and_unignore() {
    let projection = state_event_notice_projection("org.example.future");
    let mut item = super::super::test_support::timeline_item(
        "$hidden:example.invalid",
        None,
        "@alice:example.invalid",
        false,
    );
    item.body = projection.body;
    item.sender = Some("@alice:example.invalid".into());
    for ignored in [
        std::collections::BTreeSet::new(),
        std::collections::BTreeSet::from(["@alice:example.invalid".into()]),
        std::collections::BTreeSet::new(),
    ] {
        apply_timeline_item_visibility(&mut item, false, &ignored);
        assert!(item.is_hidden);
        assert!(!has_user_visible_content(&item));
    }
}

#[tokio::test]
async fn sdk_message_filter_and_partial_content_policy_are_preserved() {
    let server = MatrixMockServer::new().await;
    let client = server.client_builder().build().await;
    let room = server
        .sync_joined_room(&client, room_id!("!messages:example.invalid"))
        .await;
    let messages = [
        json!({"msgtype":"m.text","body":"Text"}),
        json!({"msgtype":"m.notice","body":"Notice"}),
        json!({"msgtype":"m.emote","body":"Emote"}),
        json!({"msgtype":"m.image","body":"Image","url":"mxc://example.invalid/image"}),
        json!({"msgtype":"m.file","body":"File","url":"mxc://example.invalid/file"}),
        json!({"msgtype":"m.audio","body":"Audio","url":"mxc://example.invalid/audio"}),
        json!({"msgtype":"m.video","body":"Video","url":"mxc://example.invalid/video"}),
        json!({"msgtype":"m.location","body":"Location","geo_uri":"geo:0,0"}),
        json!({"msgtype":"m.server_notice","body":"Server notice","server_notice_type":"m.server_notice.usage_limit_reached"}),
        json!({"msgtype":"m.key.verification.request","body":"Verification","from_device":"TEST","methods":["m.sas.v1"],"to":"@bob:example.invalid"}),
    ];
    for (i, content) in messages.into_iter().enumerate() {
        let expected_body = content["body"].as_str().unwrap().to_owned();
        let raw: Raw<AnySyncTimelineEvent> =
            Raw::from_json_string(wire_event("m.room.message", content, false, i).to_string())
                .unwrap();
        assert!(matrix_sdk_ui::timeline::default_event_filter(
            &raw.deserialize().unwrap(),
            &matrix_sdk::ruma::RoomVersionId::V10.rules().unwrap()
        ));
        let content = TimelineItemContent::from_event(&room, TimelineEvent::from_plaintext(raw))
            .await
            .unwrap();
        let projection = message_projection_from_timeline_content(&content);
        let display = projection.body.as_deref().or_else(|| {
            projection
                .media
                .as_ref()
                .map(|media| media.filename.as_str())
        });
        assert_eq!(display, Some(expected_body.as_str()));
        assert!(projection.notice_i18n.is_none());
    }
    for ty in ["org.example.unknown", "m.poll.start", "m.room.message"] {
        let content = if ty == "m.room.message" {
            json!({"msgtype":"org.example.unknown","body":"Unknown"})
        } else if ty == "m.poll.start" {
            json!({"m.poll":{"question":{"m.text":[{"body":"Synthetic question"}]},"kind":"m.poll.disclosed","max_selections":1,"answers":[{"m.id":"a","m.text":[{"body":"A"}]},{"m.id":"b","m.text":[{"body":"B"}]}]},"m.text":[{"body":"Synthetic question"}]})
        } else {
            json!({})
        };
        let raw: Raw<AnySyncTimelineEvent> =
            Raw::from_json_string(wire_event(ty, content, false, 40).to_string()).unwrap();
        assert!(
            !matrix_sdk_ui::timeline::default_event_filter(
                &raw.deserialize().unwrap(),
                &matrix_sdk::ruma::RoomVersionId::V10.rules().unwrap()
            ),
            "{ty}"
        );
    }
    // Focused/reply lookups can still encounter custom message-like content.
    use matrix_sdk_ui::timeline::{MsgLikeContent, MsgLikeKind, OtherMessageLike};
    let other = MsgLikeContent::redacted().with_kind(MsgLikeKind::Other(
        OtherMessageLike::from_event_type("org.example.unknown".into()),
    ));
    let projection = message_projection_from_timeline_content(&TimelineItemContent::MsgLike(other));
    assert_eq!(
        projection.notice_i18n.unwrap().key,
        TimelineNoticeI18nKey::UnsupportedMessage
    );
}

#[tokio::test]
async fn hidden_alias_keeps_sdk_identity_in_live_and_paginated_timelines() {
    use futures_util::StreamExt;
    use matrix_sdk::test_utils::mocks::RoomMessagesResponseTemplate;
    use matrix_sdk_test::JoinedRoomBuilder;
    use matrix_sdk_ui::timeline::TimelineFocus;
    let server = MatrixMockServer::new().await;
    let client = server.client_builder().build().await;
    client.event_cache().subscribe().unwrap();
    let room_id = room_id!("!live-policy:example.invalid");
    let room = server.sync_joined_room(&client, room_id).await;
    let alias: Raw<AnySyncTimelineEvent> = Raw::from_json_string(
        wire_event(
            "m.room.canonical_alias",
            json!({"alias":"#synthetic:example.invalid"}),
            true,
            80,
        )
        .to_string(),
    )
    .unwrap();
    let message: Raw<AnySyncTimelineEvent> = Raw::from_json_string(
        wire_event(
            "m.room.message",
            json!({"msgtype":"m.text","body":"Synthetic message"}),
            false,
            81,
        )
        .to_string(),
    )
    .unwrap();
    server
        .sync_room(
            &client,
            JoinedRoomBuilder::new(room_id)
                .set_timeline_prev_batch("before-policy")
                .add_timeline_event(alias.clone())
                .add_timeline_event(message.clone()),
        )
        .await;
    let history: Vec<Raw<matrix_sdk::ruma::events::AnyTimelineEvent>> = [
        wire_event(
            "m.room.message",
            json!({"msgtype":"m.text","body":"Synthetic older message"}),
            false,
            71,
        ),
        wire_event(
            "m.room.canonical_alias",
            json!({"alias":"#older:example.invalid"}),
            true,
            70,
        ),
    ]
    .into_iter()
    .map(|mut event| {
        event["room_id"] = json!(room_id.as_str());
        Raw::from_json_string(event.to_string()).unwrap()
    })
    .collect();
    server
        .mock_room_messages()
        .ok(RoomMessagesResponseTemplate::default().events(history))
        .mount()
        .await;
    let timeline = super::super::relay::koushi_timeline_builder(
        &room,
        TimelineFocus::Live {
            hide_threaded_events: true,
        },
    )
    .build()
    .await
    .unwrap();
    let key = TimelineKey::room(
        koushi_protocol::ids::AccountKey("@alice:example.invalid".into()),
        room_id.to_string(),
    );
    let check = |items: &eyeball_im::Vector<Arc<SdkTimelineItem>>| {
        let alias = items
            .iter()
            .find(|item| {
                item.as_event()
                    .and_then(|event| event.event_id())
                    .is_some_and(|id| id.as_str() == "$policy-80:example.invalid")
            })
            .unwrap();
        let dto = sdk_item_to_timeline_item(&key, alias, None);
        assert!(dto.is_hidden);
        assert!(dto.body.is_none());
        assert!(
            matches!(&dto.id, TimelineItemId::Event {event_id} if event_id == "$policy-80:example.invalid")
        );
        assert!(items.iter().any(|item| {
            item.as_event()
                .is_some_and(|event| event.content().as_message().is_some())
        }));
    };
    let initial = timeline.items().await;
    check(&initial);
    timeline.paginate_backwards(10).await.unwrap();
    let paginated = timeline.items().await;
    check(&paginated);
    let older_alias = paginated
        .iter()
        .find(|item| {
            item.as_event()
                .and_then(|event| event.event_id())
                .is_some_and(|id| id.as_str() == "$policy-70:example.invalid")
        })
        .unwrap();
    assert!(sdk_item_to_timeline_item(&key, older_alias, None).is_hidden);
    let (_, mut updates) = timeline.subscribe().await;
    let alias2: Raw<AnySyncTimelineEvent> = Raw::from_json_string(
        wire_event(
            "m.room.canonical_alias",
            json!({"alias":"#changed:example.invalid"}),
            true,
            82,
        )
        .to_string(),
    )
    .unwrap();
    server
        .sync_room(
            &client,
            JoinedRoomBuilder::new(room_id).add_timeline_event(alias2),
        )
        .await;
    tokio::time::timeout(std::time::Duration::from_secs(5), updates.next())
        .await
        .unwrap()
        .unwrap();
    let latest = timeline.items().await;
    let new_alias = latest
        .iter()
        .find(|item| {
            item.as_event()
                .and_then(|event| event.event_id())
                .is_some_and(|id| id.as_str() == "$policy-82:example.invalid")
        })
        .unwrap();
    assert!(sdk_item_to_timeline_item(&key, new_alias, None).is_hidden);
}

#[tokio::test]
async fn poll_and_live_location_interim_notices_use_typed_sdk_content() {
    let server = MatrixMockServer::new().await;
    let client = server.client_builder().build().await;
    let room = server
        .sync_joined_room(&client, room_id!("!deferred:example.invalid"))
        .await;
    for (index, (ty, content, state, key)) in [
        ("org.matrix.msc3381.poll.start", json!({"org.matrix.msc3381.poll.start":{"question":{"org.matrix.msc1767.text":"Synthetic question"},"kind":"org.matrix.msc3381.poll.disclosed","max_selections":1,"answers":[{"id":"a","org.matrix.msc1767.text":"A"},{"id":"b","org.matrix.msc1767.text":"B"}]},"org.matrix.msc1767.text":"Synthetic question"}), false, TimelineNoticeI18nKey::Poll),
        ("org.matrix.msc3672.beacon_info", json!({"live":true,"org.matrix.msc3488.ts":1,"timeout":60000}), true, TimelineNoticeI18nKey::LiveLocation),
        ("m.beacon_info", json!({"live":true,"org.matrix.msc3488.ts":1,"timeout":60000}), true, TimelineNoticeI18nKey::LiveLocation),
    ].into_iter().enumerate() {
        let mut wire = wire_event(ty, content, state, 100+index);
        if state { wire["state_key"] = json!("@alice:example.invalid"); }
        let raw: Raw<AnySyncTimelineEvent> = Raw::from_json_string(wire.to_string()).unwrap();
        assert!(matrix_sdk_ui::timeline::default_event_filter(&raw.deserialize().expect(ty), &matrix_sdk::ruma::RoomVersionId::V10.rules().unwrap()));
        let content = TimelineItemContent::from_event(&room, TimelineEvent::from_plaintext(raw)).await.expect(ty);
        assert_eq!(message_projection_from_timeline_content(&content).notice_i18n.unwrap().key, key);
    }
}
