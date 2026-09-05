use super::{SendFlowWaiter, projection_timeline_item};
use koushi_protocol::{
    event::{CoreEvent, TimelineDiff, TimelineEvent, TimelineItemId, TimelineSendState},
    ids::{
        AccountKey, RequestId, RuntimeConnectionId, TimelineBatchId, TimelineGeneration,
        TimelineKey,
    },
};

fn key() -> TimelineKey {
    TimelineKey::room(AccountKey("fixture".into()), "!room:example.invalid")
}
fn request(sequence: u64) -> RequestId {
    RequestId {
        connection_id: RuntimeConnectionId(1),
        sequence,
    }
}
fn waiter() -> SendFlowWaiter {
    SendFlowWaiter::new(request(1), key(), "client-txn", "Synthetic send")
}
fn item(local: bool) -> koushi_protocol::event::TimelineItem {
    let mut item = projection_timeline_item("$sent:example.invalid", false);
    item.body = Some("Synthetic send".into());
    item.send_state = Some(if local {
        TimelineSendState::Sending
    } else {
        TimelineSendState::Sent
    });
    if local {
        item.id = TimelineItemId::Transaction {
            transaction_id: "sdk-txn".into(),
        };
    }
    item
}
fn completion(sequence: u64) -> CoreEvent {
    CoreEvent::Timeline(TimelineEvent::SendCompleted {
        request_id: request(sequence),
        key: key(),
        transaction_id: "client-txn".into(),
        event_id: "$sent:example.invalid".into(),
    })
}
fn initial(key: TimelineKey, local: bool) -> CoreEvent {
    CoreEvent::Timeline(TimelineEvent::InitialItems {
        request_id: None,
        cause_request_id: None,
        key,
        actor_generation: 1,
        generation: TimelineGeneration(1),
        items: vec![item(local)],
    })
}

#[test]
fn reset_local_echo_then_remote_set_proves_the_same_send() {
    let mut waiter = waiter();
    waiter
        .observe(CoreEvent::Timeline(TimelineEvent::ItemsUpdated {
            key: key(),
            generation: TimelineGeneration(1),
            batch_id: TimelineBatchId(1),
            diffs: vec![
                TimelineDiff::Reset {
                    items: vec![item(true)],
                },
                TimelineDiff::Set {
                    index: 0,
                    item: item(false),
                },
            ],
        }))
        .unwrap();
    waiter.observe(completion(1)).unwrap();
    assert!(waiter.is_complete());
    assert_eq!(waiter.finish().unwrap().sdk_transaction_id, "sdk-txn");
}

#[test]
fn initial_local_echo_requires_matching_key_and_completion_request() {
    let mut waiter = waiter();
    waiter
        .observe(initial(
            TimelineKey::room(AccountKey("fixture".into()), "!other:example.invalid"),
            true,
        ))
        .unwrap();
    waiter.observe(completion(2)).unwrap();
    assert!(!waiter.is_complete());
    waiter.observe(initial(key(), true)).unwrap();
    assert!(!waiter.is_complete());
    waiter.observe(completion(1)).unwrap();
    assert!(waiter.is_complete());
}

#[test]
fn a_remote_only_sent_snapshot_is_not_local_echo_evidence() {
    let mut waiter = waiter();
    waiter.observe(initial(key(), false)).unwrap();
    waiter.observe(completion(1)).unwrap();
    assert!(!waiter.is_complete());
    assert!(waiter.status_summary().contains("local_echo=false"));
}

#[test]
fn mismatched_client_transaction_errors_do_not_disclose_identifiers() {
    let mut waiter = waiter();
    let error = waiter
        .observe(CoreEvent::Timeline(TimelineEvent::SendCompleted {
            request_id: request(1),
            key: key(),
            transaction_id: "wrong-secret-txn".into(),
            event_id: "$sent:example.invalid".into(),
        }))
        .unwrap_err();
    assert!(!error.contains("wrong-secret-txn"));
    assert!(!error.contains("client-txn"));
}
