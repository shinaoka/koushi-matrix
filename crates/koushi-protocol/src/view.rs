//! Toolkit-independent identities for owned, bounded view subscriptions.

use serde::{Deserialize, Serialize};

mod models;
pub use models::*;

/// An opaque subscription identifier, not authority to access another consumer's view.
///
/// The Core registry must validate ownership and liveness on every operation.
/// JSON uses a canonical decimal string, preserving all 64 bits in native and web hosts.
///
/// ```
/// use koushi_protocol::view::ViewScopeId;
/// let id = ViewScopeId(u64::MAX);
/// let wire = serde_json::to_string(&id).unwrap();
/// assert_eq!(serde_json::from_str::<ViewScopeId>(&wire).unwrap(), id);
/// ```
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct ViewScopeId(#[serde(with = "crate::u64_decimal_string")] pub u64);

/// A scope-local installed-model revision; model ACK is not visibility evidence.
///
/// ```
/// use koushi_protocol::view::ViewRevision;
/// assert_eq!(serde_json::to_string(&ViewRevision(42)).unwrap(), "\"42\"");
/// ```
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
pub struct ViewRevision(#[serde(with = "crate::u64_decimal_string")] pub u64);

/// The observed timeline owner, before Core resolves its private generation lease.
///
/// Replays preserve the projection request identity. A source reference does not
/// authorize access; the subscription registry must validate its live owner.
/// Existing Rust identity types are reused with lossless scoped wire encodings.
///
/// ```
/// use koushi_protocol::{AccountKey, RequestId, RuntimeConnectionId, TimelineGeneration, TimelineKey};
/// use koushi_protocol::view::TimelineViewSource;
/// let source = TimelineViewSource {
///     key: TimelineKey::room(AccountKey("account".into()), "!room:example.org"),
///     projection_request_id: RequestId { connection_id: RuntimeConnectionId(1), sequence: 2 },
///     generation: TimelineGeneration(3),
/// };
/// let wire = serde_json::to_string(&source).unwrap();
/// assert_eq!(serde_json::from_str::<TimelineViewSource>(&wire).unwrap(), source);
/// ```
#[derive(Clone, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct TimelineViewSource {
    pub key: crate::TimelineKey,
    #[serde(with = "DecimalRequestId")]
    pub projection_request_id: crate::RequestId,
    #[serde(with = "DecimalTimelineGeneration")]
    pub generation: crate::TimelineGeneration,
}

impl std::fmt::Debug for TimelineViewSource {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("TimelineViewSource")
            .field("key", &"TimelineKey(..)")
            .field("projection_request_id", &self.projection_request_id)
            .field("generation", &self.generation)
            .finish()
    }
}

/// An event in one observed timeline source, not an independently authorized locator.
///
/// ```
/// use koushi_protocol::view::ReceiptSourceRef;
/// let source: ReceiptSourceRef = serde_json::from_value(serde_json::json!({
///     "key": { "account_key": "account", "kind": { "Room": { "room_id": "!r:example.org" } } },
///     "projection_request_id": { "connection_id": "1", "sequence": "2" },
///     "generation": "3", "event_id": "$event"
/// })).unwrap();
/// assert_eq!(source.event_id, "$event");
/// ```
#[derive(Clone, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct ReceiptSourceRef {
    #[serde(flatten)]
    pub timeline: TimelineViewSource,
    pub event_id: String,
}

impl std::fmt::Debug for ReceiptSourceRef {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ReceiptSourceRef")
            .field("timeline", &self.timeline)
            .field("event_id", &"EventId(..)")
            .finish()
    }
}

/// A requested window capacity, constrained to one through 256 rows.
///
/// ```
/// use koushi_protocol::view::ReaderWindowLimit;
/// assert!(ReaderWindowLimit::try_from(32).is_ok());
/// assert!(ReaderWindowLimit::try_from(257).is_err());
/// ```
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "u16", into = "u16")]
pub struct ReaderWindowLimit(u16);

impl ReaderWindowLimit {
    /// Read the validated capacity.
    ///
    /// ```
    /// use koushi_protocol::view::ReaderWindowLimit;
    /// assert_eq!(ReaderWindowLimit::try_from(32).unwrap().get(), 32);
    /// ```
    pub const fn get(self) -> u16 {
        self.0
    }
}

impl TryFrom<u16> for ReaderWindowLimit {
    type Error = &'static str;

    fn try_from(value: u16) -> Result<Self, Self::Error> {
        if (1..=256).contains(&value) {
            Ok(Self(value))
        } else {
            Err("reader window capacity must be between 1 and 256")
        }
    }
}

impl From<ReaderWindowLimit> for u16 {
    fn from(value: ReaderWindowLimit) -> Self {
        value.0
    }
}

/// Rust resolves an index or an identity from the installed window, never pixels.
///
/// ```
/// use koushi_protocol::view::ReaderWindowTarget;
/// let target = ReaderWindowTarget::Index { start: 50 };
/// assert_eq!(serde_json::to_value(target).unwrap()["kind"], "index");
/// ```
#[derive(Clone, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ReaderWindowTarget {
    Index { start: u64 },
    Anchor { user_id: String },
}

impl std::fmt::Debug for ReaderWindowTarget {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Index { start } => formatter
                .debug_struct("Index")
                .field("start", start)
                .finish(),
            Self::Anchor { .. } => formatter.write_str("Anchor { user_id: .. }"),
        }
    }
}

/// A revision-qualified request on an already-owned reader subscription.
///
/// Core additionally validates scope ownership, installed revision, sequence and
/// anchor membership; parsing this DTO alone does not authorize the request.
///
/// ```
/// use koushi_protocol::view::{ReaderWindowLimit, ReaderWindowRequest, ReaderWindowTarget, ViewRevision};
/// let request = ReaderWindowRequest {
///     installed_revision: ViewRevision(1), sequence: 2,
///     target: ReaderWindowTarget::Index { start: 0 },
///     limit: ReaderWindowLimit::try_from(32).unwrap(),
/// };
/// assert_eq!(serde_json::to_value(request).unwrap()["sequence"], "2");
/// ```
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ReaderWindowRequest {
    pub installed_revision: ViewRevision,
    #[serde(with = "crate::u64_decimal_string")]
    pub sequence: u64,
    pub target: ReaderWindowTarget,
    pub limit: ReaderWindowLimit,
}

/// Visible/prefetch identities from an acknowledged reader model. Core validates
/// ownership, live source, current session, sequence and both window bounds.
#[derive(Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct ReaderAvatarObservation {
    pub installed_revision: ViewRevision,
    #[serde(with = "crate::u64_decimal_string")]
    pub sequence: u64,
    pub visible_user_ids: Vec<String>,
    pub prefetch_user_ids: Vec<String>,
}

impl std::fmt::Debug for ReaderAvatarObservation {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ReaderAvatarObservation")
            .field("installed_revision", &self.installed_revision)
            .field("sequence", &self.sequence)
            .field("visible_count", &self.visible_user_ids.len())
            .field("prefetch_count", &self.prefetch_user_ids.len())
            .finish()
    }
}

/// Rust's explicit anchor result; absence never implies an estimated scroll jump.
///
/// ```
/// use koushi_protocol::view::ResolvedReaderAnchor;
/// let result = ResolvedReaderAnchor::NoSurvivingInstalledRow;
/// assert_eq!(serde_json::to_value(result).unwrap()["kind"], "noSurvivingInstalledRow");
/// ```
#[derive(Clone, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ResolvedReaderAnchor {
    Row { user_id: String, index: u64 },
    NoSurvivingInstalledRow,
    NotRequested,
}

impl std::fmt::Debug for ResolvedReaderAnchor {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Row { index, .. } => formatter
                .debug_struct("Row")
                .field("user_id", &"UserId(..)")
                .field("index", index)
                .finish(),
            Self::NoSurvivingInstalledRow => formatter.write_str("NoSurvivingInstalledRow"),
            Self::NotRequested => formatter.write_str("NotRequested"),
        }
    }
}

#[derive(Serialize, Deserialize)]
#[serde(remote = "crate::RequestId")]
struct DecimalRequestId {
    #[serde(with = "DecimalConnectionId")]
    connection_id: crate::RuntimeConnectionId,
    #[serde(with = "crate::u64_decimal_string")]
    sequence: u64,
}

#[derive(Serialize, Deserialize)]
#[serde(remote = "crate::RuntimeConnectionId")]
struct DecimalConnectionId(#[serde(with = "crate::u64_decimal_string")] u64);

#[derive(Serialize, Deserialize)]
#[serde(remote = "crate::TimelineGeneration")]
struct DecimalTimelineGeneration(#[serde(with = "crate::u64_decimal_string")] u64);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn window_request_limits_and_sequences_are_validated() {
        let request = ReaderWindowRequest {
            installed_revision: ViewRevision(1),
            sequence: u64::MAX,
            target: ReaderWindowTarget::Anchor {
                user_id: "@private:example.org".into(),
            },
            limit: ReaderWindowLimit::try_from(256).unwrap(),
        };
        let wire = serde_json::to_value(&request).unwrap();
        assert_eq!(wire["sequence"], u64::MAX.to_string());
        assert_eq!(wire["limit"], 256);
        assert_eq!(
            serde_json::from_value::<ReaderWindowRequest>(wire.clone()).unwrap(),
            request
        );
        assert!(!format!("{request:?}").contains("@private:example.org"));
        for invalid in [0, 257, 65536] {
            let mut invalid_wire = wire.clone();
            invalid_wire["limit"] = invalid.into();
            assert!(serde_json::from_value::<ReaderWindowRequest>(invalid_wire).is_err());
        }
        assert_eq!(ReaderWindowLimit::try_from(1).unwrap().get(), 1);
    }

    #[test]
    fn source_identity_is_exact_and_debug_is_private() {
        let source = TimelineViewSource {
            key: crate::TimelineKey::room(
                crate::AccountKey("private-account".into()),
                "!private:example.org",
            ),
            projection_request_id: crate::RequestId {
                connection_id: crate::RuntimeConnectionId(u64::MAX),
                sequence: u64::MAX,
            },
            generation: crate::TimelineGeneration(u64::MAX),
        };
        let receipt_source = ReceiptSourceRef {
            timeline: source.clone(),
            event_id: "$private-event".into(),
        };
        let receipt_wire = serde_json::to_value(&receipt_source).unwrap();
        assert_eq!(receipt_wire["event_id"], "$private-event");
        assert!(receipt_wire.get("timeline").is_none());
        assert_eq!(
            serde_json::from_value::<ReceiptSourceRef>(receipt_wire).unwrap(),
            receipt_source
        );
        assert!(!format!("{receipt_source:?}").contains("$private-event"));
        let wire = serde_json::to_value(&source).unwrap();
        assert_eq!(wire["generation"], u64::MAX.to_string());
        assert_eq!(
            wire["projection_request_id"]["connection_id"],
            u64::MAX.to_string()
        );
        assert_eq!(
            wire["projection_request_id"]["sequence"],
            u64::MAX.to_string()
        );
        assert_eq!(
            serde_json::from_value::<TimelineViewSource>(wire).unwrap(),
            source
        );
        let debug = format!("{source:?}");
        assert!(!debug.contains("private-account"));
        assert!(!debug.contains("!private:example.org"));
    }

    #[test]
    fn avatar_observations_keep_counters_lossless_and_debug_private() {
        let observation = ReaderAvatarObservation {
            installed_revision: ViewRevision(u64::MAX),
            sequence: u64::MAX,
            visible_user_ids: vec!["@private:example.invalid".into()],
            prefetch_user_ids: vec![],
        };
        let wire = serde_json::to_value(&observation).unwrap();
        assert_eq!(wire["sequence"], u64::MAX.to_string());
        assert_eq!(wire["installed_revision"], u64::MAX.to_string());
        assert_eq!(
            serde_json::from_value::<ReaderAvatarObservation>(wire.clone()).unwrap(),
            observation
        );
        assert!(!format!("{observation:?}").contains("@private"));
        let mut invalid = wire;
        invalid["sequence"] = serde_json::json!(1);
        assert!(serde_json::from_value::<ReaderAvatarObservation>(invalid).is_err());
    }

    #[test]
    fn view_identities_round_trip_without_javascript_number_loss() {
        let scope = ViewScopeId(u64::MAX);
        let revision = ViewRevision(u64::MAX);
        let encoded = "\"18446744073709551615\"";
        assert_eq!(serde_json::to_string(&scope).unwrap(), encoded);
        assert_eq!(serde_json::from_str::<ViewScopeId>(encoded).unwrap(), scope);
        assert_eq!(serde_json::to_string(&revision).unwrap(), encoded);
        assert_eq!(
            serde_json::from_str::<ViewRevision>(encoded).unwrap(),
            revision
        );
        for invalid in [
            "1",
            "\"01\"",
            "\"+1\"",
            "\"-1\"",
            "\"18446744073709551616\"",
        ] {
            assert!(serde_json::from_str::<ViewScopeId>(invalid).is_err());
            assert!(serde_json::from_str::<ViewRevision>(invalid).is_err());
        }
    }
}
