use super::InitialItemsRequestIdentity;
use crate::timeline::test_support::fake_rid;

#[test]
fn committed_replay_retains_actor_projection_identity() {
    let projection_request_id = fake_rid(41);
    let cause_request_id = fake_rid(42);

    let replay = InitialItemsRequestIdentity::replay(projection_request_id, Some(cause_request_id));
    assert_eq!(replay.projection_request_id, Some(projection_request_id));
    assert_eq!(replay.cause_request_id, Some(cause_request_id));
}
