use serde::{Deserialize, Serialize};

/// A Rust-resolved preview; availability is deliberately not represented.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoomAddressPreview {
    pub localpart: String,
    pub full_alias: Option<String>,
    pub error: Option<RoomAddressError>,
}

impl std::fmt::Debug for RoomAddressPreview {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RoomAddressPreview")
            .field("localpart", &"[redacted]")
            .field("has_full_alias", &self.full_alias.is_some())
            .field("error", &self.error)
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
