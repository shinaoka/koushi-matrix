#[cfg(test)]
use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::time::Duration;
use std::time::{SystemTime, UNIX_EPOCH};

use matrix_desktop_state::ScheduledSendCapability;
use matrix_sdk::ruma::api::FeatureFlag;

pub(crate) const MSC4140_FEATURE: &str = "org.matrix.msc4140";

pub(crate) fn capability_from_unstable_features(
    features: &BTreeSet<FeatureFlag>,
) -> ScheduledSendCapability {
    if features.contains(&FeatureFlag::from(MSC4140_FEATURE)) {
        ScheduledSendCapability::ServerDelayedEvents
    } else {
        ScheduledSendCapability::LocalFallback
    }
}

#[cfg(test)]
pub(crate) fn capability_from_versions_flags(
    flags: &BTreeMap<String, bool>,
) -> ScheduledSendCapability {
    if flags.get(MSC4140_FEATURE).copied().unwrap_or(false) {
        ScheduledSendCapability::ServerDelayedEvents
    } else {
        ScheduledSendCapability::LocalFallback
    }
}

pub(crate) fn server_delay_timeout(send_at_ms: u64, now_ms: u64) -> Duration {
    Duration::from_millis(send_at_ms.saturating_sub(now_ms))
}

pub(crate) fn current_epoch_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or_default()
}

pub(crate) async fn detect_capability(client: &matrix_sdk::Client) -> ScheduledSendCapability {
    match client.unstable_features().await {
        Ok(features) => capability_from_unstable_features(&features),
        Err(_) => ScheduledSendCapability::LocalFallback,
    }
}
