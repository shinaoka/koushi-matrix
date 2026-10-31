use super::{REPLY_COUNT, ThreadUnreadObservation};

#[test]
fn missing_room_summary_cannot_pass_as_zero_unread() {
    let mut observed = ThreadUnreadObservation {
        max_reply_count: REPLY_COUNT,
        thread_replies: (0..REPLY_COUNT)
            .map(|index| format!("$reply-{index}"))
            .collect(),
        ..Default::default()
    };
    observed.observe_snapshot(&koushi_state::AppState::default(), "!missing:example.org");
    assert!(observed.require_loaded_evidence().is_err());
}
