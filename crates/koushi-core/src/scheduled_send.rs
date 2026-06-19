#[cfg(test)]
use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::time::Duration;
use std::time::{SystemTime, UNIX_EPOCH};

use koushi_state::ScheduledSendCapability;
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

#[cfg(test)]
mod tests {
    use koushi_state::ScheduledSendCapability;
    use matrix_sdk::ruma::api::FeatureFlag;

    use super::{MSC4140_FEATURE, capability_from_unstable_features, server_delay_timeout};

    #[test]
    fn capability_detects_msc4140_server_support() {
        let features = [FeatureFlag::from(MSC4140_FEATURE)].into_iter().collect();

        assert_eq!(
            capability_from_unstable_features(&features),
            ScheduledSendCapability::ServerDelayedEvents
        );
    }

    #[test]
    fn capability_falls_back_when_msc4140_is_absent_or_disabled() {
        let absent = [FeatureFlag::from("org.matrix.something_else")]
            .into_iter()
            .collect();
        let disabled = std::collections::BTreeMap::from([(MSC4140_FEATURE.to_owned(), false)]);

        assert_eq!(
            capability_from_unstable_features(&absent),
            ScheduledSendCapability::LocalFallback
        );
        assert_eq!(
            super::capability_from_versions_flags(&disabled),
            ScheduledSendCapability::LocalFallback
        );
    }

    #[test]
    fn server_timeout_uses_target_delta_without_private_data() {
        assert_eq!(
            server_delay_timeout(1_500, 1_000),
            std::time::Duration::from_millis(500)
        );
        assert_eq!(
            server_delay_timeout(1_000, 1_500),
            std::time::Duration::from_millis(0)
        );
    }
}
