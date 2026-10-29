//! #1676: the live-tail latch reports once, only after Core's projection holds
//! the SDK's final row, whichever of the completion and the final batch comes
//! first.

use koushi_protocol::event::{TimelineItem, TimelineItemId, TimelineMessageActions};

use super::FocusedLiveTail;

fn item(id: TimelineItemId) -> TimelineItem {
    TimelineItem {
        request_state: None,
        id,
        sender: None,
        sender_label: None,
        sender_avatar: None,
        body: None,
        notice_i18n: None,
        message_kind: Default::default(),
        spoiler_spans: Vec::new(),
        timestamp_ms: None,
        in_reply_to_event_id: None,
        formatted: None,
        reply_quote: None,
        thread_root: None,
        thread_summary: None,
        media: None,
        link_previews: None,
        link_ranges: Vec::new(),
        mentioned_user_ids: Vec::new(),
        mentions_room: false,
        reactions: Vec::new(),
        can_react: false,
        is_redacted: false,
        is_hidden: false,
        can_redact: false,
        is_edited: false,
        can_edit: false,
        actions: TimelineMessageActions::default(),
        send_state: None,
        unable_to_decrypt: None,
        display_metadata: None,
    }
}

fn row(event_id: &str) -> TimelineItem {
    item(TimelineItemId::Event {
        event_id: event_id.to_owned(),
    })
}

#[test]
fn completion_after_the_final_batch_latches_at_once() {
    let mut tail = FocusedLiveTail::default();
    let committed = [row("$context"), row("$final")];
    tail.expect("$final".to_owned());
    assert_eq!(tail.commit(&committed), Some("$final".to_owned()));
    assert_eq!(tail.latched(), Some("$final"));
    assert_eq!(tail.commit(&committed), None, "reported once");
}

#[test]
fn completion_before_the_final_batch_waits_for_its_row() {
    let mut tail = FocusedLiveTail::default();
    tail.expect("$final".to_owned());
    assert_eq!(tail.commit(&[row("$context")]), None);
    assert_eq!(tail.latched(), None);
    assert_eq!(
        tail.commit(&[row("$context"), row("$final")]),
        Some("$final".to_owned())
    );
}

#[test]
fn no_report_before_the_forward_end() {
    let mut tail = FocusedLiveTail::default();
    assert_eq!(tail.commit(&[row("$final")]), None);
}

#[test]
fn a_latched_tail_survives_a_repeated_end() {
    let mut tail = FocusedLiveTail::default();
    tail.expect("$final".to_owned());
    tail.commit(&[row("$final")]);
    tail.expect("$other".to_owned());
    assert_eq!(tail.latched(), Some("$final"));
    assert_eq!(tail.commit(&[row("$other")]), None);
}

#[test]
fn synthetic_rows_after_the_tail_do_not_hide_it() {
    let mut tail = FocusedLiveTail::default();
    tail.expect("$final".to_owned());
    let synthetic = item(TimelineItemId::Synthetic {
        synthetic_id: "day-divider".to_owned(),
    });
    assert_eq!(
        tail.commit(&[row("$final"), synthetic]),
        Some("$final".to_owned())
    );
}

/// The SDK's final row is the tail even when the final page ends with an
/// event that never becomes a row, and the retained-cache lookup after the
/// build makes no second `/context` request.
#[tokio::test]
async fn final_tail_is_the_sdk_final_row_after_a_trailing_reaction() {
    use matrix_sdk::event_cache::EventFocusThreadMode;
    use matrix_sdk::ruma::{event_id, room_id};
    use matrix_sdk::test_utils::mocks::MatrixMockServer;
    use matrix_sdk_test::{ALICE, event_factory::EventFactory};
    use matrix_sdk_ui::timeline::{TimelineEventFocusThreadMode, TimelineFocus};
    use wiremock::matchers::{method, path_regex, query_param};
    use wiremock::{Mock, ResponseTemplate};

    let server = MatrixMockServer::new().await;
    let client = server.client_builder().build().await;
    client
        .event_cache()
        .subscribe()
        .expect("event cache subscription");
    let room_id = room_id!("!live-tail:example.org");
    let target = event_id!("$anchor:example.org");
    let final_row = event_id!("$final:example.org");
    let room = server.sync_joined_room(&client, room_id).await;
    let factory = EventFactory::new().room(room_id).sender(&ALICE);
    let anchor = factory.text_msg("anchor").event_id(target).into_event();
    Mock::given(method("GET"))
        .and(path_regex(r"/context/"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "event": anchor.into_raw().json(),
            "events_before": [],
            "events_after": [],
            "state": [],
            "start": "before-anchor",
            "end": "after-anchor",
        })))
        .expect(1)
        .mount(server.server())
        .await;
    let message = factory.text_msg("final").event_id(final_row).into_event();
    let reaction = factory
        .reaction(final_row, "👍")
        .event_id(event_id!("$reaction:example.org"))
        .into_event();
    Mock::given(method("GET"))
        .and(path_regex(r"/messages"))
        .and(query_param("dir", "f"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "chunk": [message.into_raw().json(), reaction.into_raw().json()],
            "start": "after-anchor",
        })))
        .mount(server.server())
        .await;

    let timeline = super::super::relay::koushi_timeline_builder(
        &room,
        TimelineFocus::Event {
            target: target.to_owned(),
            num_context_events: 20,
            thread_mode: TimelineEventFocusThreadMode::Automatic {
                hide_threaded_events: false,
            },
        },
    )
    .build()
    .await
    .expect("focused timeline");
    let (cache, _handles) = client
        .event_cache()
        .event_focused(
            room_id,
            target,
            EventFocusThreadMode::Automatic {
                hide_threaded_events: false,
            },
            20,
        )
        .await
        .expect("memoized focused cache");
    assert!(
        timeline.paginate_forwards(20).await.expect("forward page"),
        "the final page reaches the end"
    );

    assert_eq!(
        super::final_tail(&timeline, &cache, std::time::Duration::from_secs(10)).await,
        Some(final_row.to_string())
    );
}
