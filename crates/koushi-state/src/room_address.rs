use serde::{Deserialize, Serialize};

/// A Rust-resolved preview; availability is deliberately not represented.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoomAddressPreview {
    pub localpart: String,
    pub full_alias: Option<String>,
    pub error: Option<RoomAddressError>,
    /// The account's server, whose alias namespace the address is unique in
    /// (shared by every Space on it). `None` until a Ready session exists.
    #[serde(default)]
    pub server_name: Option<String>,
}

impl std::fmt::Debug for RoomAddressPreview {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RoomAddressPreview")
            .field("localpart", &"[redacted]")
            .field("has_full_alias", &self.full_alias.is_some())
            .field("error", &self.error)
            .field("has_server_name", &self.server_name.is_some())
            .finish()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RoomAddressError {
    Empty,
    Invalid,
    NotReady,
}

/// Suggest an editable room-alias local part, without claiming availability.
/// Preserve Unicode letters/numbers (including Japanese); separate name segments
/// with hyphens. An unsuitable name returns empty and requires a manual address.
/// The SDK still validates the complete alias and the server owns availability.
pub fn suggest_room_alias_localpart(name: &str) -> String {
    name.split(|character: char| !character.is_alphanumeric())
        .filter(|part| !part.is_empty())
        .map(str::to_lowercase)
        .collect::<Vec<_>>()
        .join("-")
}

/// Suggest an address for a room created from a Space (#1006):
/// `<space>-<room>`, both normalized as by [`suggest_room_alias_localpart`].
///
/// A Space has no alias namespace of its own; the prefix only makes a
/// collision with an unrelated room on the same server less likely. A Space
/// name with no usable characters falls back to the room-only suggestion, and
/// a room name with none still requires a manual address (empty). The caller
/// falls back to the room-only suggestion when the prefixed alias would
/// exceed Matrix's 255-byte limit.
pub fn suggest_space_room_alias_localpart(space_name: Option<&str>, room_name: &str) -> String {
    let room = suggest_room_alias_localpart(room_name);
    let space = space_name
        .map(suggest_room_alias_localpart)
        .unwrap_or_default();
    if room.is_empty() || space.is_empty() {
        room
    } else {
        format!("{space}-{room}")
    }
}
