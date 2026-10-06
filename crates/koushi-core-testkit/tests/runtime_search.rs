use koushi_core::runtime::CoreRuntime;
use koushi_protocol::command::{CoreCommand, SearchCommand, SearchScope};
use koushi_state::{SearchState, SessionState};

mod support;
use support::restore_ready_actions;

#[tokio::test]
async fn search_query_projects_search_state_before_routing() {
    let (runtime, _stores) = CoreRuntime::start_isolated();
    let mut connection = runtime.attach();

    runtime.inject_actions(restore_ready_actions()).await;

    support::wait_for_state_event(&mut connection, |state| {
        matches!(state.session, SessionState::Ready(_))
    })
    .await;

    let request_id = connection.next_request_id();
    connection
        .command(CoreCommand::Search(SearchCommand::Query {
            request_id,
            query: "Alpha".to_owned(),
            scope: SearchScope::AllRooms,
            room_filter: koushi_state::SearchRoomFilter::AllRooms,
        }))
        .await
        .expect("submit");

    let result = support::wait_for_state_event(&mut connection, |state| {
        !matches!(state.search, SearchState::Closed)
    })
    .await;

    match result.search {
        SearchState::Searching {
            request_id: rid, ..
        }
        | SearchState::Failed {
            request_id: rid, ..
        }
        | SearchState::Results {
            request_id: rid, ..
        } => {
            assert_eq!(rid, request_id.sequence);
        }
        other => panic!("expected search state to project, got {other:?}"),
    }
}
