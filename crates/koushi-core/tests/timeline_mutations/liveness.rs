use super::fixture::{Fixture, LIVENESS, REACTION, TARGET, is_pending};
use crate::timeline::test_support::fake_rid;
use koushi_protocol::command::TimelineCommand;
use koushi_state::ComposerDocument;
use std::sync::{Arc, Condvar, Mutex};
use tokio::sync::oneshot;
use wiremock::{
    Mock, ResponseTemplate,
    matchers::{method, path_regex},
};

/// The responder holds a real HTTP response behind a barrier. These tests use
/// four runtime workers so one blocked mock responder cannot park Core.
struct HeldHttp {
    entered: oneshot::Receiver<()>,
    barrier: Arc<(Mutex<bool>, Condvar)>,
}
impl Drop for HeldHttp {
    fn drop(&mut self) {
        let (lock, wake) = &*self.barrier;
        *lock.lock().unwrap() = true;
        wake.notify_all();
    }
}
impl HeldHttp {
    async fn response(
        fixture: &Fixture,
        verb: &'static str,
        path: &'static str,
        body: serde_json::Value,
        status: u16,
    ) -> Self {
        let barrier = Arc::new((Mutex::new(false), Condvar::new()));
        let waiting = barrier.clone();
        let (entered_tx, entered) = oneshot::channel();
        let entered_tx = Mutex::new(Some(entered_tx));
        Mock::given(method(verb))
            .and(path_regex(path))
            .respond_with(move |_: &wiremock::Request| {
                if let Some(sender) = entered_tx.lock().unwrap().take() {
                    let _ = sender.send(());
                }
                let (lock, wake) = &*waiting;
                let mut released = lock.lock().unwrap();
                while !*released {
                    released = wake.wait(released).unwrap();
                }
                ResponseTemplate::new(status).set_body_json(body.clone())
            })
            .mount(fixture.server.server())
            .await;
        Self { entered, barrier }
    }
    async fn mount(
        fixture: &Fixture,
        verb: &'static str,
        path: &'static str,
        body: serde_json::Value,
    ) -> Self {
        Self::response(fixture, verb, path, body, 200).await
    }
    async fn wait(&mut self) {
        tokio::time::timeout(LIVENESS, &mut self.entered)
            .await
            .expect("SDK must reach held HTTP")
            .unwrap();
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn held_remote_reaction_removal_publishes_pending_items() {
    let mut fixture = Fixture::new(false).await;
    let mut held = HeldHttp::mount(
        &fixture,
        "PUT",
        "/redact/",
        serde_json::json!({ "event_id": "$redaction:example.invalid" }),
    )
    .await;
    fixture
        .command(TimelineCommand::RedactReaction {
            request_id: fake_rid(10),
            key: fixture.key.clone(),
            event_id: TARGET.into(),
            reaction_key: "👍".into(),
            reaction_event_id: REACTION.into(),
        })
        .await;
    held.wait().await;
    fixture.pending().await;
    fixture.wait_for(is_pending).await;
    unrelated_diff_and_send_terminal_progress(&mut fixture).await;
    drop(held);
    fixture.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn held_remote_edit_preparation_publishes_pending_items() {
    let mut fixture = Fixture::new(true).await;
    let mut held = HeldHttp::mount(
        &fixture,
        "GET",
        "/event/",
        serde_json::json!({
            "event_id": TARGET, "type": "m.room.message", "sender": matrix_sdk_test::User::Alice.as_str(),
            "room_id": "!mutation:example.invalid", "origin_server_ts": 1,
            "content": { "msgtype": "m.text", "body": "Synthetic original" }
        }),
    )
    .await;
    fixture
        .command(TimelineCommand::EditText {
            request_id: fake_rid(11),
            key: fixture.key.clone(),
            event_id: TARGET.into(),
            document: ComposerDocument::from_plain_text("Synthetic edit"),
        })
        .await;
    held.wait().await;
    fixture.pending().await;
    fixture.wait_for(is_pending).await;
    unrelated_diff_and_send_terminal_progress(&mut fixture).await;
    drop(held);
    fixture.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn client_timeout_keeps_sdk_reaction_rollback_alive() {
    use koushi_protocol::{
        event::{CoreEvent, TimelineDiff, TimelineEvent},
        failure::{CoreFailure, TimelineFailureKind},
    };
    let mut fixture = Fixture::with_deadline(false, Some(std::time::Duration::from_secs(3))).await;
    let mut held = HeldHttp::response(
        &fixture,
        "PUT",
        "/redact/",
        serde_json::json!({ "errcode": "M_FORBIDDEN", "error": "Synthetic refusal" }),
        403,
    )
    .await;
    fixture
        .command(TimelineCommand::RedactReaction {
            request_id: fake_rid(12),
            key: fixture.key.clone(),
            event_id: TARGET.into(),
            reaction_key: "👍".into(),
            reaction_event_id: REACTION.into(),
        })
        .await;
    held.wait().await;
    fixture.wait_for(|event| matches!(event, CoreEvent::OperationFailed { request_id, failure: CoreFailure::TimelineOperationFailed { kind: TimelineFailureKind::Sdk } } if *request_id == fake_rid(12))).await;
    // A failed HTTP response must still run SDK rollback after the client deadline.
    drop(held);
    fixture.wait_for(|event| matches!(event, CoreEvent::Timeline(TimelineEvent::ItemsUpdated { diffs, .. }) if diffs.iter().any(|diff| matches!(diff, TimelineDiff::Set { item, .. } if item.reactions.iter().any(|reaction| reaction.my_reaction_event_id.as_deref() == Some(REACTION)))))).await;
    while let Ok(event) = fixture.events.try_recv() {
        assert!(
            !matches!(event, CoreEvent::OperationFailed { request_id, .. } if request_id == fake_rid(12)),
            "late SDK failure cannot settle twice"
        );
    }
    fixture.stop().await;
}

async fn unrelated_diff_and_send_terminal_progress(fixture: &mut Fixture) {
    use crate::timeline::actor::TimelineActorMessage;
    use crate::timeline::outbound_send::{
        SendCompletionObservation, apply_send_completion_observation_and_handoff,
    };
    use koushi_protocol::event::{CoreEvent, TimelineDiff, TimelineEvent};
    apply_send_completion_observation_and_handoff(
        &fixture.coordinator,
        &fixture.terminal_ingress,
        fixture.key.room_id(),
        SendCompletionObservation::Sent {
            sdk_transaction_id: super::fixture::PENDING.into(),
            event_id: "$pending-sent:example.invalid".into(),
        },
    );
    fixture.wait_for_stage("send_terminal", |event| matches!(event, CoreEvent::Timeline(TimelineEvent::SendCompleted { request_id, .. }) if *request_id == fake_rid(700))).await;
    // Existing relay-input seam: exercise the real actor's diff application,
    // rather than merely inspecting the manager's retained projection.
    let (acknowledged, ack) = oneshot::channel();
    fixture
        .actor
        .send(TimelineActorMessage::TestInjectDiff {
            diffs: vec![eyeball_im::VectorDiff::Clear],
            projections: Default::default(),
            acknowledged,
        })
        .await
        .unwrap();
    tokio::time::timeout(LIVENESS, ack)
        .await
        .expect("incoming diff progresses before HTTP release")
        .unwrap();
    fixture.wait_for_stage("incoming_diff", |event| matches!(event, CoreEvent::Timeline(TimelineEvent::ItemsUpdated { diffs, .. }) if diffs.iter().any(|diff| matches!(diff, TimelineDiff::Clear | TimelineDiff::Remove { .. })))).await;
}
