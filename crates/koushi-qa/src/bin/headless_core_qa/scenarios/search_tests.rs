use super::crawler_state_counts;
use crate::{SearchCrawlerFailureKind, SearchCrawlerRoomState};
use std::collections::BTreeMap;

#[test]
fn crawler_state_counts_bucket_every_room_state() {
    let mut rooms = BTreeMap::new();
    rooms.insert("a".to_owned(), SearchCrawlerRoomState::Idle);
    rooms.insert("b".to_owned(), SearchCrawlerRoomState::Queued);
    rooms.insert(
        "c".to_owned(),
        SearchCrawlerRoomState::Running {
            processed: 3,
            indexed: 2,
        },
    );
    rooms.insert(
        "d".to_owned(),
        SearchCrawlerRoomState::Completed { indexed: 4 },
    );
    rooms.insert(
        "e".to_owned(),
        SearchCrawlerRoomState::Failed {
            kind: SearchCrawlerFailureKind::RoomNotFound,
        },
    );

    assert_eq!(crawler_state_counts(&rooms), [1, 1, 1, 1, 1]);
    assert_eq!(crawler_state_counts(&BTreeMap::new()), [0, 0, 0, 0, 0]);
}
