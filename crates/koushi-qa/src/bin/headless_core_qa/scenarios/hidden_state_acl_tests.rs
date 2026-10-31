use super::*;
use crate::contracts::{withheld_projection_items_updated, withheld_projection_test_item};
use koushi_core::runtime::CoreConnectionTestControl;
use koushi_protocol::ids::TimelineGeneration;

const MESSAGE_ID: &str = "$message:example.invalid";

fn key() -> TimelineKey {
    TimelineKey::room(
        koushi_core::AccountKey("@reader:example.invalid".to_owned()),
        "!room:example.invalid",
    )
}

fn acl_ids() -> Vec<String> {
    (0..ACL_UPDATE_COUNT)
        .map(|index| format!("$acl-{index}:example.invalid"))
        .collect()
}

fn snapshot(ids: &[String], with_message: bool) -> Vec<TimelineItem> {
    let mut items: Vec<_> = ids
        .iter()
        .map(|id| {
            let mut item = withheld_projection_test_item(id, "");
            item.is_hidden = true;
            item
        })
        .collect();
    if with_message {
        items.push(withheld_projection_test_item(MESSAGE_ID, MESSAGE_BODY));
    }
    items
}

async fn reply(control: &mut CoreConnectionTestControl, items: Vec<TimelineItem>) {
    let Some(CoreCommand::Timeline(TimelineCommand::Subscribe {
        request_id,
        key: event_key,
        ..
    })) = control.recv_command().await
    else {
        panic!("expected a correlated replay request");
    };
    assert_eq!(event_key, key());
    control.send_event(CoreEvent::Timeline(TimelineEvent::InitialItems {
        request_id: None,
        cause_request_id: Some(request_id),
        key: event_key,
        actor_generation: 1,
        generation: TimelineGeneration(0),
        items,
    }));
}

#[test]
fn replay_needs_message_and_all_distinct_acl_events_in_one_snapshot() {
    let ids = acl_ids();
    let acl_set = ids.iter().map(String::as_str).collect();
    assert!(!acl_replay_ready(
        &snapshot(&ids, false),
        MESSAGE_ID,
        &acl_set
    ));
    let mut short = snapshot(&ids[..120], true);
    assert!(!acl_replay_ready(&short, MESSAGE_ID, &acl_set));
    short.extend(snapshot(&ids[..5], false));
    assert!(!acl_replay_ready(&short, MESSAGE_ID, &acl_set));
    assert!(acl_replay_ready(
        &snapshot(&ids, true),
        MESSAGE_ID,
        &acl_set
    ));
}

#[tokio::test(start_paused = true)]
async fn recovery_requires_a_complete_correlated_replay_not_a_historical_union() {
    let ids = acl_ids();
    let acl_set = ids.iter().map(String::as_str).collect();
    let (mut reader, mut control) = CoreConnection::new_for_testing(32);
    let publish = async {
        reply(&mut control, snapshot(&ids[..120], true)).await;
        control.send_event(withheld_projection_items_updated(
            key(),
            snapshot(&ids[120..121], false).remove(0),
        ));
        // Replacing the snapshot with all ACLs but no message must not reuse
        // the message observed in the previous snapshot.
        reply(&mut control, snapshot(&ids, false)).await;
        control.send_event(withheld_projection_items_updated(
            key(),
            withheld_projection_test_item(MESSAGE_ID, MESSAGE_BODY),
        ));
        reply(&mut control, snapshot(&ids, true)).await;
    };
    let mut observation = AclObservation::default();
    let timeline_key = key();
    let (result, ()) = tokio::join!(
        wait_for_acl_replay(
            &mut reader,
            &timeline_key,
            MESSAGE_ID,
            &acl_set,
            &mut observation
        ),
        publish,
    );
    let replay = result.expect("autonomous recovery supplies one complete replay");
    assert!(acl_replay_ready(&replay, MESSAGE_ID, &acl_set));
    assert_eq!(observation.seen.len(), ACL_UPDATE_COUNT);
    observation.require_all_hidden().unwrap();
}

#[tokio::test(start_paused = true)]
async fn visible_acl_before_a_complete_hidden_replay_is_rejected() {
    let ids = acl_ids();
    let acl_set = ids.iter().map(String::as_str).collect();
    let (mut reader, mut control) = CoreConnection::new_for_testing(32);
    let publish = async {
        control.send_event(withheld_projection_items_updated(
            key(),
            withheld_projection_test_item(&ids[0], "Synthetic incorrectly visible ACL"),
        ));
        reply(&mut control, snapshot(&ids, true)).await;
    };
    let mut observation = AclObservation::default();
    let timeline_key = key();
    let (result, ()) = tokio::join!(
        wait_for_acl_replay(
            &mut reader,
            &timeline_key,
            MESSAGE_ID,
            &acl_set,
            &mut observation
        ),
        publish,
    );
    assert!(result.unwrap_err().contains("exported as visible rows"));
}

#[tokio::test(start_paused = true)]
async fn snapshot_and_event_waits_share_one_replay_deadline() {
    let ids = acl_ids();
    let acl_set = ids.iter().map(String::as_str).collect();
    let (mut reader, mut control) = CoreConnection::new_for_testing(32);
    let fixture_ids = ids.clone();
    let publish = tokio::spawn(async move {
        tokio::time::sleep(EVENT_TIMEOUT / 4).await;
        reply(&mut control, snapshot(&fixture_ids[..120], true)).await;
        tokio::time::sleep(EVENT_TIMEOUT / 4).await;
        control.send_event(withheld_projection_items_updated(
            key(),
            snapshot(&fixture_ids[..1], false).remove(0),
        ));
        tokio::time::sleep(EVENT_TIMEOUT / 4).await;
        reply(&mut control, snapshot(&fixture_ids[..120], true)).await;
        tokio::time::sleep(EVENT_TIMEOUT / 8).await;
        control.send_event(withheld_projection_items_updated(
            key(),
            snapshot(&fixture_ids[..1], false).remove(0),
        ));
        // A fresh budget for the next snapshot would incorrectly outlive the
        // original deadline; no reply is supplied within that deadline.
        tokio::time::sleep(EVENT_TIMEOUT).await;
        reply(&mut control, snapshot(&fixture_ids, true)).await;
    });
    let started = tokio::time::Instant::now();
    let error = wait_for_acl_replay(
        &mut reader,
        &key(),
        MESSAGE_ID,
        &acl_set,
        &mut AclObservation::default(),
    )
    .await
    .expect_err("incomplete replay must time out");
    let elapsed = started.elapsed();
    publish.abort();
    assert_eq!(elapsed, EVENT_TIMEOUT);
    assert!(error.contains("before the deadline"), "{error}");
    assert!(error.contains("acl=120"), "{error}");
}

#[tokio::test(start_paused = true)]
async fn first_replay_request_is_also_deadline_bounded() {
    let ids = acl_ids();
    let acl_set = ids.iter().map(String::as_str).collect();
    let (mut reader, _control) = CoreConnection::new_for_testing(32);
    // Fill command capacity so even submission, not only response reception,
    // must be covered by the replay deadline.
    reader
        .command(CoreCommand::Timeline(TimelineCommand::Subscribe {
            request_id: reader.next_request_id(),
            key: key(),
            initial_backfill: koushi_protocol::command::InitialBackfillPolicy::Disabled,
        }))
        .await
        .unwrap();
    let started = tokio::time::Instant::now();
    let result = wait_for_acl_replay(
        &mut reader,
        &key(),
        MESSAGE_ID,
        &acl_set,
        &mut AclObservation::default(),
    )
    .await;
    assert!(result.is_err());
    assert_eq!(started.elapsed(), EVENT_TIMEOUT);
}
