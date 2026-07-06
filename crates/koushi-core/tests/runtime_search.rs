use std::time::Duration;

use koushi_core::command::{CoreCommand, SearchCommand, SearchScope};
use koushi_core::event::CoreEvent;
use koushi_core::executor;
use koushi_core::runtime::CoreRuntime;
use koushi_state::{AppAction, SearchState, SessionState};

mod support;
use support::session_info;

#[tokio::test]
async fn search_query_projects_search_state_before_routing() {
    let runtime = CoreRuntime::start();
    let mut connection = runtime.attach();

    runtime
        .inject_actions(vec![AppAction::RestoreSessionSucceeded(session_info())])
        .await;

    loop {
        if matches!(connection.snapshot().session, SessionState::Ready(_)) {
            break;
        }
        executor::sleep(Duration::from_millis(5)).await;
    }

    let request_id = connection.next_request_id();
    connection
        .command(CoreCommand::Search(SearchCommand::Query {
            request_id,
            query: "Alpha".to_owned(),
            scope: SearchScope::AllRooms,
        }))
        .await
        .expect("submit");

    let result = executor::timeout(Duration::from_secs(1), async {
        loop {
            match connection.recv_event().await.expect("event") {
                CoreEvent::StateChanged(snapshot)
                    if !matches!(snapshot.search, SearchState::Closed) =>
                {
                    return snapshot;
                }
                _ => continue,
            }
        }
    })
    .await
    .expect("search submission should publish a non-closed search snapshot");

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
