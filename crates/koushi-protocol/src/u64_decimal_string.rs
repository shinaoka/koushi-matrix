use serde::{Deserialize, Deserializer, Serializer, de::Error as _};

pub(crate) fn serialize<S>(value: &u64, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    serializer.serialize_str(&value.to_string())
}

pub(crate) fn deserialize<'de, D>(deserializer: D) -> Result<u64, D::Error>
where
    D: Deserializer<'de>,
{
    let encoded = String::deserialize(deserializer)?;
    let parsed = encoded
        .parse::<u64>()
        .map_err(|_| D::Error::custom("expected a canonical unsigned decimal string"))?;
    if parsed.to_string() != encoded {
        return Err(D::Error::custom(
            "expected a canonical unsigned decimal string",
        ));
    }
    Ok(parsed)
}
