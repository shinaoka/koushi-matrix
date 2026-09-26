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

/// Suggest an alternative to an address that is in use (#1006): a trailing
/// `-N` is incremented, otherwise `-2` is appended. It is only a suggestion:
/// nothing is claimed about its availability until it is checked.
pub fn suggest_alternative_room_alias_localpart(localpart: &str) -> String {
    if let Some((stem, number)) = localpart.rsplit_once('-')
        && !stem.is_empty()
        && !number.is_empty()
        && number.chars().all(|character| character.is_ascii_digit())
        && let Ok(value) = number.parse::<u64>()
        && let Some(next) = value.checked_add(1)
    {
        return format!("{stem}-{next}");
    }
    format!("{localpart}-2")
}

/// Advisory availability of the create dialog's current address (#1006).
///
/// It reflects one homeserver lookup and never reserves the address; room
/// creation remains authoritative. A settlement is admitted only for the
/// in-flight `request_id` and address, so a result for an earlier draft never
/// replaces a newer check.
#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum RoomAddressAvailabilityState {
    #[default]
    Idle,
    Checking {
        request_id: u64,
        full_alias: String,
    },
    Checked {
        request_id: u64,
        full_alias: String,
        availability: RoomAddressAvailability,
        /// Offered only for an address in use; not checked yet.
        suggestion: Option<RoomAddressSuggestion>,
    },
}

impl std::fmt::Debug for RoomAddressAvailabilityState {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Idle => formatter.write_str("Idle"),
            Self::Checking { request_id, .. } => formatter
                .debug_struct("Checking")
                .field("request_id", request_id)
                .field("full_alias", &"[redacted]")
                .finish(),
            Self::Checked {
                request_id,
                availability,
                suggestion,
                ..
            } => formatter
                .debug_struct("Checked")
                .field("request_id", request_id)
                .field("full_alias", &"[redacted]")
                .field("availability", availability)
                .field("has_suggestion", &suggestion.is_some())
                .finish(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RoomAddressAvailability {
    /// The homeserver had no room for the address when asked.
    Available,
    /// The address resolved to a room.
    InUse,
    /// The lookup failed or timed out; nothing is known.
    Unknown,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoomAddressSuggestion {
    pub localpart: String,
    pub full_alias: String,
}

impl std::fmt::Debug for RoomAddressSuggestion {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("RoomAddressSuggestion([redacted])")
    }
}
