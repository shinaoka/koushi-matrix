use std::fmt;

use serde::{Deserialize, Serialize};

use super::errors::OperationFailureKind;

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct DirectoryState {
    pub query: DirectoryQueryState,
    pub join: DirectoryJoinState,
}

#[derive(Clone, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum DirectoryQueryState {
    #[default]
    Closed,
    Querying {
        request_id: u64,
        query: DirectoryQuery,
    },
    Results {
        request_id: u64,
        query: DirectoryQuery,
        rooms: Vec<DirectoryRoomSummary>,
        next_batch: Option<String>,
    },
    Failed {
        request_id: u64,
        query: DirectoryQuery,
        #[serde(rename = "failureKind")]
        kind: OperationFailureKind,
    },
}

impl fmt::Debug for DirectoryQueryState {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Closed => formatter.write_str("Closed"),
            Self::Querying { request_id, query } => formatter
                .debug_struct("Querying")
                .field("request_id", request_id)
                .field("query", query)
                .finish(),
            Self::Results {
                request_id,
                query,
                rooms,
                next_batch,
            } => formatter
                .debug_struct("Results")
                .field("request_id", request_id)
                .field("query", query)
                .field("rooms", rooms)
                .field("next_batch", &next_batch.as_ref().map(|_| "PageToken(..)"))
                .finish(),
            Self::Failed {
                request_id,
                query,
                kind,
            } => formatter
                .debug_struct("Failed")
                .field("request_id", request_id)
                .field("query", query)
                .field("kind", kind)
                .finish(),
        }
    }
}

#[derive(Clone, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum DirectoryJoinState {
    #[default]
    Idle,
    Joining {
        request_id: u64,
        alias: String,
        via_server: Option<String>,
    },
    Failed {
        request_id: u64,
        alias: String,
        via_server: Option<String>,
        #[serde(rename = "failureKind")]
        kind: OperationFailureKind,
    },
}

impl fmt::Debug for DirectoryJoinState {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Idle => formatter.write_str("Idle"),
            Self::Joining {
                request_id,
                via_server,
                ..
            } => formatter
                .debug_struct("Joining")
                .field("request_id", request_id)
                .field("alias", &"RoomAlias(..)")
                .field("via_server", &via_server.as_ref().map(|_| "ServerName(..)"))
                .finish(),
            Self::Failed {
                request_id,
                via_server,
                kind,
                ..
            } => formatter
                .debug_struct("Failed")
                .field("request_id", request_id)
                .field("alias", &"RoomAlias(..)")
                .field("via_server", &via_server.as_ref().map(|_| "ServerName(..)"))
                .field("kind", kind)
                .finish(),
        }
    }
}

#[derive(Clone, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct DirectoryQuery {
    pub term: Option<String>,
    pub server_name: Option<String>,
    pub limit: Option<u32>,
    #[serde(default)]
    pub since: Option<String>,
}

impl fmt::Debug for DirectoryQuery {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DirectoryQuery")
            .field("term", &self.term.as_ref().map(|_| "QueryText(..)"))
            .field(
                "server_name",
                &self.server_name.as_ref().map(|_| "ServerName(..)"),
            )
            .field("limit", &self.limit)
            .field("since", &self.since.as_ref().map(|_| "PageToken(..)"))
            .finish()
    }
}

#[derive(Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct DirectoryRoomSummary {
    pub room_id: String,
    pub canonical_alias: Option<String>,
    pub name: String,
    pub topic: Option<String>,
    pub avatar_url: Option<String>,
    pub joined_members: u64,
    pub world_readable: bool,
    pub guest_can_join: bool,
}

impl fmt::Debug for DirectoryRoomSummary {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DirectoryRoomSummary")
            .field("room_id", &"RoomId(..)")
            .field(
                "canonical_alias",
                &self.canonical_alias.as_ref().map(|_| "RoomAlias(..)"),
            )
            .field("name", &"RoomName(..)")
            .field("topic", &self.topic.as_ref().map(|_| "RoomTopic(..)"))
            .field(
                "avatar_url",
                &self.avatar_url.as_ref().map(|_| "MxcUri(..)"),
            )
            .field("joined_members", &self.joined_members)
            .field("world_readable", &self.world_readable)
            .field("guest_can_join", &self.guest_can_join)
            .finish()
    }
}
