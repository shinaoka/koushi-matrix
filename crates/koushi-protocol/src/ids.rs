//! Transport-neutral identity DTOs.

use std::fmt;

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct RuntimeConnectionId(pub u64);

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct RequestId {
    pub connection_id: RuntimeConnectionId,
    pub sequence: u64,
}

#[derive(Clone, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct AccountKey(pub String);

impl fmt::Debug for AccountKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("AccountKey(..)")
    }
}

#[derive(Clone, Eq, Hash, PartialEq, Deserialize, Serialize)]
pub struct SessionKeyId {
    pub homeserver: String,
    pub user_id: String,
    pub device_id: String,
}

impl fmt::Debug for SessionKeyId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SessionKeyId")
            .field("homeserver", &"Homeserver(..)")
            .field("user_id", &"UserId(..)")
            .field("device_id", &"DeviceId(..)")
            .finish()
    }
}

#[derive(Clone, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct TimelineKey {
    pub account_key: AccountKey,
    pub kind: TimelineKind,
}

// #1979: boundary identity, so its `Debug` reaches CI logs and issue comments.
impl fmt::Debug for TimelineKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("TimelineKey(..)")
    }
}

impl TimelineKey {
    pub fn room(account_key: AccountKey, room_id: impl Into<String>) -> Self {
        Self {
            account_key,
            kind: TimelineKind::Room {
                room_id: room_id.into(),
            },
        }
    }

    pub fn room_id(&self) -> &str {
        match &self.kind {
            TimelineKind::Room { room_id }
            | TimelineKind::Thread { room_id, .. }
            | TimelineKind::Focused { room_id, .. } => room_id,
        }
    }
}

#[derive(Clone, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub enum TimelineKind {
    Room {
        room_id: String,
    },
    Thread {
        room_id: String,
        root_event_id: String,
    },
    Focused {
        room_id: String,
        event_id: String,
    },
}

impl fmt::Debug for TimelineKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Room { .. } => formatter.write_str("Room { room_id: RoomId(..) }"),
            Self::Thread { .. } => {
                formatter.write_str("Thread { room_id: RoomId(..), root_event_id: EventId(..) }")
            }
            Self::Focused { .. } => {
                formatter.write_str("Focused { room_id: RoomId(..), event_id: EventId(..) }")
            }
        }
    }
}
#[derive(
    Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize,
)]
pub struct TimelineGeneration(pub u64);

#[derive(
    Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize,
)]
pub struct TimelineBatchId(pub u64);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_key_debug_redacts_all_identity_fields() {
        let key = SessionKeyId {
            homeserver: "https://private-homeserver.invalid".to_owned(),
            user_id: "@private-user:example.invalid".to_owned(),
            device_id: "PRIVATE-DEVICE".to_owned(),
        };
        let debug = format!("{key:?}");
        assert!(!debug.contains(&key.homeserver));
        assert!(!debug.contains(&key.user_id));
        assert!(!debug.contains(&key.device_id));
    }

    // #1979: `AccountKey` and `TimelineKey` are boundary identities, so their
    // derived `Debug` copied the account and room ids into CI logs and issue
    // comments.
    #[test]
    fn account_and_timeline_key_debug_redacts_account_and_room_ids() {
        let account = AccountKey("@private-user:example.invalid".to_owned());
        let debug = format!("{account:?}");
        assert_eq!(debug, "AccountKey(..)");

        let room = TimelineKey::room(account.clone(), "!private-room:example.invalid");
        let debug = format!("{room:?}");
        assert_eq!(debug, "TimelineKey(..)");
    }

    #[test]
    fn timeline_kind_debug_redacts_every_identifier() {
        for (kind, secrets, variant) in [
            (
                TimelineKind::Room {
                    room_id: "!private-room:example.invalid".to_owned(),
                },
                vec!["!private-room:example.invalid"],
                "Room {",
            ),
            (
                TimelineKind::Thread {
                    room_id: "!private-room:example.invalid".to_owned(),
                    root_event_id: "$private-root:example.invalid".to_owned(),
                },
                vec![
                    "!private-room:example.invalid",
                    "$private-root:example.invalid",
                ],
                "Thread {",
            ),
            (
                TimelineKind::Focused {
                    room_id: "!private-room:example.invalid".to_owned(),
                    event_id: "$private-focus:example.invalid".to_owned(),
                },
                vec![
                    "!private-room:example.invalid",
                    "$private-focus:example.invalid",
                ],
                "Focused {",
            ),
        ] {
            let debug = format!("{kind:?}");
            assert!(debug.starts_with(variant), "{debug}");
            assert!(debug.contains("RoomId(..)"), "{debug}");
            if !matches!(kind, TimelineKind::Room { .. }) {
                assert!(debug.contains("EventId(..)"), "{debug}");
            }
            for secret in secrets {
                assert!(!debug.contains(secret), "{debug}");
            }
        }
    }
}
