use std::fmt;

use koushi_state::{AttachmentFilter, AttachmentScope, AttachmentSort, SearchRoomFilter};

use crate::ids::RequestId;

pub enum SearchCommand {
    Query {
        request_id: RequestId,
        query: String,
        scope: SearchScope,
        room_filter: SearchRoomFilter,
    },
    Attachments {
        request_id: RequestId,
        scope: AttachmentScope,
        filter: AttachmentFilter,
        sort: AttachmentSort,
    },
    StartHistoryCrawl {
        request_id: RequestId,
        room_id: String,
        settings: koushi_state::SearchCrawlerSettings,
    },
    StopHistoryCrawl {
        request_id: RequestId,
        room_id: String,
    },
}

#[derive(Clone)]
pub enum ThreadsListCommand {
    Open {
        request_id: RequestId,
        scope: koushi_state::ThreadsListScope,
        room_ids: Vec<String>,
    },
    Close {
        request_id: RequestId,
    },
    Paginate {
        request_id: RequestId,
        scope: koushi_state::ThreadsListScope,
    },
}

#[derive(Clone, Eq, PartialEq)]
pub enum SearchScope {
    AllRooms,
    CurrentRoom { room_id: String },
    CurrentSpace { space_id: String },
}

// #1979: the scope names one room or space, so its Debug is a placeholder.
impl fmt::Debug for SearchScope {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::AllRooms => "AllRooms",
            Self::CurrentRoom { .. } => "CurrentRoom { room_id: RoomId(..) }",
            Self::CurrentSpace { .. } => "CurrentSpace { space_id: SpaceId(..) }",
        })
    }
}

// #1979: `room_ids` and the scope name real rooms; count them instead.
impl fmt::Debug for ThreadsListCommand {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Open {
                request_id,
                scope,
                room_ids,
            } => formatter
                .debug_struct("Open")
                .field("request_id", request_id)
                .field("scope", scope)
                .field("room_ids_len", &room_ids.len())
                .finish(),
            Self::Close { request_id } => formatter
                .debug_struct("Close")
                .field("request_id", request_id)
                .finish(),
            Self::Paginate { request_id, scope } => formatter
                .debug_struct("Paginate")
                .field("request_id", request_id)
                .field("scope", scope)
                .finish(),
        }
    }
}

fn search_room_filter_debug(filter: &SearchRoomFilter) -> (&'static str, usize) {
    match filter {
        SearchRoomFilter::AllRooms => ("all_rooms", 0),
        SearchRoomFilter::OnlyRooms(room_ids) => ("only_rooms", room_ids.len()),
    }
}

// Search queries can quote message content; redact like bodies.
impl fmt::Debug for SearchCommand {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Query {
                request_id,
                scope,
                room_filter,
                ..
            } => formatter
                .debug_struct("Query")
                .field("request_id", request_id)
                .field("query", &"SearchQuery(..)")
                .field("scope", scope)
                .field("room_filter", &search_room_filter_debug(room_filter))
                .finish(),
            Self::Attachments {
                request_id,
                scope,
                filter,
                sort,
            } => formatter
                .debug_struct("Attachments")
                .field("request_id", request_id)
                .field("scope", scope)
                .field("filter", filter)
                .field("sort", sort)
                .finish(),
            Self::StartHistoryCrawl {
                request_id,
                room_id: _,
                settings,
            } => formatter
                .debug_struct("StartHistoryCrawl")
                .field("request_id", request_id)
                .field("room_id", &"RoomId(..)")
                .field("settings", settings)
                .finish(),
            Self::StopHistoryCrawl {
                request_id,
                room_id: _,
            } => formatter
                .debug_struct("StopHistoryCrawl")
                .field("request_id", request_id)
                .field("room_id", &"RoomId(..)")
                .finish(),
        }
    }
}
