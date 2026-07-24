use core::fmt;

use serde::{Deserialize, Deserializer, Serialize, Serializer, de};

#[derive(Clone, Copy, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ComposerDraftRevision(u128);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ComposerDraftRevisionError {
    InvalidWire,
    Exhausted,
}

impl ComposerDraftRevision {
    pub const ZERO: Self = Self(0);
    pub const MAX: Self = Self(u128::MAX);

    pub const fn from_u64(value: u64) -> Self {
        Self(value as u128)
    }

    pub const fn is_zero(self) -> bool {
        self.0 == 0
    }

    pub fn parse_wire(value: &str) -> Result<Self, ComposerDraftRevisionError> {
        let canonical = value == "0"
            || (!value.is_empty()
                && value.len() <= 39
                && !value.starts_with('0')
                && value.bytes().all(|byte| byte.is_ascii_digit()));
        if !canonical {
            return Err(ComposerDraftRevisionError::InvalidWire);
        }

        value
            .parse::<u128>()
            .map(Self)
            .map_err(|_| ComposerDraftRevisionError::InvalidWire)
    }

    pub fn checked_successor(
        authoritative: Self,
        submitted: Self,
    ) -> Result<Self, ComposerDraftRevisionError> {
        authoritative
            .0
            .max(submitted.0)
            .checked_add(1)
            .map(Self)
            .ok_or(ComposerDraftRevisionError::Exhausted)
    }

    pub fn to_wire_string(self) -> String {
        self.0.to_string()
    }
}

impl From<u64> for ComposerDraftRevision {
    fn from(value: u64) -> Self {
        Self::from_u64(value)
    }
}

impl fmt::Debug for ComposerDraftRevision {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("ComposerDraftRevision(REDACTED)")
    }
}

impl Serialize for ComposerDraftRevision {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.to_wire_string())
    }
}

impl<'de> Deserialize<'de> for ComposerDraftRevision {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct RevisionVisitor;

        impl de::Visitor<'_> for RevisionVisitor {
            type Value = ComposerDraftRevision;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("a canonical decimal composer revision string")
            }

            fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                ComposerDraftRevision::parse_wire(value)
                    .map_err(|_| E::custom("invalid composer revision"))
            }
        }

        deserializer.deserialize_str(RevisionVisitor)
    }
}
