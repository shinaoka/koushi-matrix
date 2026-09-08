use serde::{Deserialize, Serialize};

use super::{
    ReceiptSourceRef, ResolvedReaderAnchor, TimelineViewSource, ViewRevision, ViewScopeId,
};

/// Valid whole Unix milliseconds, encoded as a canonical decimal string.
///
/// ```
/// use koushi_protocol::view::ReceiptTimestampMillis;
/// let value = ReceiptTimestampMillis::try_from(42).unwrap();
/// assert_eq!(value.get(), 42);
/// assert_eq!(serde_json::to_string(&value).unwrap(), "\"42\"");
/// ```
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub struct ReceiptTimestampMillis(#[serde(with = "crate::u64_decimal_string")] u64);

impl ReceiptTimestampMillis {
    /// Return an exactly representable native Date input.
    pub fn get(self) -> u64 {
        self.0
    }
}

impl TryFrom<u64> for ReceiptTimestampMillis {
    type Error = &'static str;
    fn try_from(value: u64) -> Result<Self, Self::Error> {
        if value <= 8_640_000_000_000_000 {
            Ok(Self(value))
        } else {
            Err("receipt timestamp exceeds supported date range")
        }
    }
}

impl<'de> Deserialize<'de> for ReceiptTimestampMillis {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = crate::u64_decimal_string::deserialize(deserializer)?;
        Self::try_from(value).map_err(serde::de::Error::custom)
    }
}

/// Final native formatter locale; adapters do not choose a fallback.
///
/// ```
/// use koushi_protocol::view::ReceiptTimestampLocale;
/// assert_eq!(serde_json::to_string(&ReceiptTimestampLocale::Ja).unwrap(), "\"ja\"");
/// ```
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ReceiptTimestampLocale {
    En,
    Ja,
}

impl From<koushi_state::CatalogLocale> for ReceiptTimestampLocale {
    /// Resolve the same fixed native-formatting policy used by receipt rows.
    ///
    /// ```
    /// use koushi_protocol::view::ReceiptTimestampLocale;
    /// assert_eq!(ReceiptTimestampLocale::from(koushi_state::CatalogLocale::Pseudo), ReceiptTimestampLocale::En);
    /// ```
    fn from(locale: koushi_state::CatalogLocale) -> Self {
        match locale {
            koushi_state::CatalogLocale::Ja => Self::Ja,
            koushi_state::CatalogLocale::En | koushi_state::CatalogLocale::Pseudo => Self::En,
        }
    }
}

/// Bounded scalar presentation data, using native medium-date/short-time style.
///
/// ```
/// use koushi_protocol::view::{ReceiptTimestamp, ReceiptTimestampLocale};
/// let value = ReceiptTimestamp::from_sdk(Some(0), koushi_state::CatalogLocale::Pseudo).unwrap();
/// assert_eq!(value.locale, ReceiptTimestampLocale::En);
/// assert!(ReceiptTimestamp::from_sdk(None, koushi_state::CatalogLocale::En).is_none());
/// ```
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ReceiptTimestamp {
    pub unix_ms: ReceiptTimestampMillis,
    pub locale: ReceiptTimestampLocale,
}

impl ReceiptTimestamp {
    /// Resolve current Rust locale policy; unusable SDK timestamps have no display value.
    pub fn from_sdk(value: Option<u64>, locale: koushi_state::CatalogLocale) -> Option<Self> {
        Some(Self {
            unix_ms: value?.try_into().ok()?,
            locale: locale.into(),
        })
    }
}

/// A Rust-selected display row; no raw avatar URL, filesystem path or image bytes.
///
/// ```
/// use koushi_protocol::view::ReaderRow;
/// let row = ReaderRow {
///     user_id: "@reader:example.org".into(), display_label: "Reader".into(),
///     original_display_label: "Reader".into(), initials: "R".into(),
///     timestamp: None, avatar: None,
/// };
/// assert_eq!(serde_json::to_value(row).unwrap()["display_label"], "Reader");
/// ```
#[derive(Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct ReaderRow {
    pub user_id: String,
    pub display_label: String,
    pub original_display_label: String,
    pub initials: String,
    pub timestamp: Option<ReceiptTimestamp>,
    #[serde(with = "avatar_wire")]
    pub avatar: Option<koushi_state::AvatarThumbnailState>,
}

impl std::fmt::Debug for ReaderRow {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("ReaderRow(..)")
    }
}

/// Exact Rust counts and compact rows (all through four, three from five).
///
/// ```
/// use koushi_protocol::view::ReceiptCompactSummary;
/// fn has_overflow(summary: &ReceiptCompactSummary) -> bool {
///     summary.overflow_count != 0 // consumers do not infer totals from mounted rows
/// }
/// ```
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ReceiptCompactSummary {
    pub source: ReceiptSourceRef,
    pub total_count: u64,
    pub overflow_count: u64,
    pub readers: Vec<ReaderRow>,
}

/// A bounded prepared window and its accepted input/dependency versions.
///
/// ```
/// use koushi_protocol::view::ReaderWindow;
/// fn row_ids(window: &ReaderWindow) -> impl Iterator<Item = &str> {
///     window.rows.iter().map(|row| row.user_id.as_str())
/// }
/// ```
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ReaderWindow {
    pub source: ReceiptSourceRef,
    pub total_count: u64,
    pub start: u64,
    pub rows: Vec<ReaderRow>,
    #[serde(with = "crate::u64_decimal_string")]
    pub window_sequence: u64,
    #[serde(with = "crate::u64_decimal_string")]
    pub source_revision: u64,
    #[serde(with = "crate::u64_decimal_string")]
    pub dependency_revision: u64,
    pub resolved_anchor: ResolvedReaderAnchor,
}

/// Closed product models: unavailable sources are never represented as empty rows.
///
/// ```
/// use koushi_protocol::view::ViewModel;
/// fn is_loading(model: &ViewModel) -> bool {
///     matches!(model, ViewModel::ReaderLoading { .. })
/// }
/// ```
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ViewModel {
    TimelineReceipts {
        source: TimelineViewSource,
        summaries: Vec<ReceiptCompactSummary>,
    },
    ReaderLoading {
        source: ReceiptSourceRef,
    },
    ReaderReady(ReaderWindow),
}

/// Terminal lifecycle reasons, not capacity rejection or a replacement data model.
///
/// ```
/// use koushi_protocol::view::ViewRetirement;
/// assert_eq!(serde_json::to_value(ViewRetirement::RuntimeStopped).unwrap(), "runtimeStopped");
/// ```
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ViewRetirement {
    /// This scope was explicitly closed; its logical consumer may remain live.
    ScopeClosed,
    /// Required scoped work could not be admitted within the hard data budget.
    Capacity,
    /// A checked scope/source/dependency counter cannot advance further.
    CounterExhausted,
    /// The owned producer ended unexpectedly before completing its work.
    ProducerFailed,
    SourceUnavailable,
    SessionRetired,
    ConsumerRetired,
    RuntimeStopped,
}

/// Separate model/control payloads; senders must not block retirement on model ACK.
///
/// ```
/// use koushi_protocol::view::{ViewDelivery, ViewRetirement, ViewScopeId};
/// let retired = ViewDelivery::Retired { scope: ViewScopeId(1), reason: ViewRetirement::ConsumerRetired };
/// assert_eq!(serde_json::to_value(retired).unwrap()["kind"], "retired");
/// for (reason, wire) in [
///     (ViewRetirement::ScopeClosed, "scopeClosed"),
///     (ViewRetirement::Capacity, "capacity"),
///     (ViewRetirement::CounterExhausted, "counterExhausted"),
///     (ViewRetirement::ProducerFailed, "producerFailed"),
/// ] {
///     assert_eq!(serde_json::to_value(reason).unwrap(), wire);
///     assert_eq!(serde_json::from_value::<ViewRetirement>(serde_json::json!(wire)).unwrap(), reason);
/// }
/// ```
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ViewDelivery {
    Model {
        scope: ViewScopeId,
        revision: ViewRevision,
        model: ViewModel,
    },
    Retired {
        scope: ViewScopeId,
        reason: ViewRetirement,
    },
}

mod avatar_wire {
    use koushi_state::{AvatarThumbnailFailureKind, AvatarThumbnailState};
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    #[derive(Serialize, Deserialize)]
    #[serde(
        remote = "AvatarThumbnailState",
        tag = "kind",
        rename_all = "camelCase"
    )]
    enum DecimalAvatar {
        NotRequested,
        Loading {
            #[serde(with = "crate::u64_decimal_string")]
            request_id: u64,
        },
        Ready {
            source_ref: String,
            width: Option<u64>,
            height: Option<u64>,
            mime_type: Option<String>,
        },
        Failed {
            #[serde(with = "crate::u64_decimal_string")]
            request_id: u64,
            #[serde(rename = "failureKind")]
            kind: AvatarThumbnailFailureKind,
        },
    }

    #[derive(Serialize)]
    struct Borrowed<'a>(#[serde(with = "DecimalAvatar")] &'a AvatarThumbnailState);
    #[derive(Deserialize)]
    struct Owned(#[serde(with = "DecimalAvatar")] AvatarThumbnailState);

    pub(super) fn serialize<S: Serializer>(
        value: &Option<AvatarThumbnailState>,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        value.as_ref().map(Borrowed).serialize(serializer)
    }

    pub(super) fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Option<AvatarThumbnailState>, D::Error> {
        Option::<Owned>::deserialize(deserializer).map(|value| value.map(|owned| owned.0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn receipt_timestamp_has_lossless_validated_wire_and_rust_locale() {
        for value in [0, 1, 8_640_000_000_000_000] {
            let stamp =
                ReceiptTimestamp::from_sdk(Some(value), koushi_state::CatalogLocale::Ja).unwrap();
            let wire = serde_json::to_value(&stamp).unwrap();
            assert_eq!(wire["unix_ms"], value.to_string());
            assert_eq!(wire["locale"], "ja");
            assert_eq!(
                serde_json::from_value::<ReceiptTimestamp>(wire).unwrap(),
                stamp
            );
        }
        for locale in [
            koushi_state::CatalogLocale::En,
            koushi_state::CatalogLocale::Pseudo,
        ] {
            assert_eq!(
                ReceiptTimestamp::from_sdk(Some(0), locale).unwrap().locale,
                ReceiptTimestampLocale::En
            );
        }
        assert!(ReceiptTimestamp::from_sdk(None, koushi_state::CatalogLocale::En).is_none());
        assert!(
            ReceiptTimestamp::from_sdk(Some(u64::MAX), koushi_state::CatalogLocale::En).is_none()
        );
        for wire in [
            r#""01""#,
            r#""-1""#,
            r#"1"#,
            r#""8640000000000001""#,
            r#""NaN""#,
        ] {
            assert!(
                serde_json::from_str::<ReceiptTimestampMillis>(wire).is_err(),
                "{wire}"
            );
        }
        assert!(
            serde_json::from_str::<ReceiptTimestamp>(r#"{"unix_ms":"0","locale":"pseudo"}"#)
                .is_err()
        );
    }

    #[test]
    fn loading_and_retirement_cannot_be_mistaken_for_empty_ready() {
        let source: super::super::ReceiptSourceRef = serde_json::from_value(serde_json::json!({
            "key": { "account_key": "private-account", "kind": { "Room": { "room_id": "!private:example.org" } } },
            "projection_request_id": { "connection_id": "1", "sequence": "2" },
            "generation": "3", "event_id": "$private-event"
        })).unwrap();
        let loading = ViewDelivery::Model {
            scope: super::super::ViewScopeId(1),
            revision: super::super::ViewRevision(u64::MAX),
            model: ViewModel::ReaderLoading {
                source: source.clone(),
            },
        };
        let wire = serde_json::to_value(&loading).unwrap();
        assert_eq!(wire["kind"], "model");
        assert_eq!(wire["revision"], u64::MAX.to_string());
        assert_eq!(wire["model"]["kind"], "readerLoading");
        assert!(wire["model"].get("rows").is_none());
        assert_eq!(
            serde_json::from_value::<ViewDelivery>(wire).unwrap(),
            loading
        );
        assert!(!format!("{loading:?}").contains("private"));
        let ready = ViewModel::ReaderReady(ReaderWindow {
            source,
            total_count: 1500,
            start: 500,
            rows: vec![ReaderRow {
                user_id: "@private:example.org".into(),
                display_label: "private name".into(),
                original_display_label: "private original".into(),
                initials: "PN".into(),
                timestamp: ReceiptTimestamp::from_sdk(Some(42), koushi_state::CatalogLocale::En),
                avatar: Some(koushi_state::AvatarThumbnailState::Loading {
                    request_id: u64::MAX,
                }),
            }],
            window_sequence: u64::MAX,
            source_revision: u64::MAX,
            dependency_revision: u64::MAX,
            resolved_anchor: ResolvedReaderAnchor::Row {
                user_id: "@private:example.org".into(),
                index: 500,
            },
        });
        let wire = serde_json::to_value(&ready).unwrap();
        assert_eq!(wire["kind"], "readerReady");
        for field in ["window_sequence", "source_revision", "dependency_revision"] {
            assert_eq!(wire[field], u64::MAX.to_string());
        }
        assert_eq!(
            wire["rows"][0]["avatar"]["request_id"],
            u64::MAX.to_string()
        );
        assert_eq!(serde_json::from_value::<ViewModel>(wire).unwrap(), ready);
        assert!(!format!("{ready:?}").contains("private"));
        let retired = ViewDelivery::Retired {
            scope: super::super::ViewScopeId(1),
            reason: ViewRetirement::SourceUnavailable,
        };
        let wire = serde_json::to_value(&retired).unwrap();
        assert_eq!(wire["kind"], "retired");
        assert!(wire.get("model").is_none());
        assert_eq!(
            serde_json::from_value::<ViewDelivery>(wire).unwrap(),
            retired
        );
    }
}
