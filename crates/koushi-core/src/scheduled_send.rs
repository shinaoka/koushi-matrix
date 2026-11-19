#[cfg(test)]
use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::time::Duration;

use koushi_state::ScheduledSendCapability;
use matrix_sdk::ruma::api::FeatureFlag;

pub(crate) const MSC4140_FEATURE: &str = "org.matrix.msc4140";
pub(crate) const LOCAL_SCHEDULED_SEND_RETRY_DELAY: Duration = Duration::from_secs(30);
/// #1973: how long after a server-handle item's due time Core first asks the
/// homeserver whether it sent the delayed event, so a punctual server has
/// already finalised it.
pub(crate) const SERVER_SCHEDULED_SEND_CHECK_GRACE: Duration = Duration::from_secs(2);
/// How long Core waits before asking again when the homeserver still holds the
/// delayed event as scheduled, or could not be asked.
pub(crate) const SERVER_SCHEDULED_SEND_RECHECK_DELAY: Duration = Duration::from_secs(10);

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

pub(crate) fn local_scheduled_send_retry_at_ms() -> u64 {
    crate::time::current_epoch_ms()
        .saturating_add(LOCAL_SCHEDULED_SEND_RETRY_DELAY.as_millis() as u64)
}

pub(crate) fn server_scheduled_send_recheck_at_ms() -> u64 {
    crate::time::current_epoch_ms()
        .saturating_add(SERVER_SCHEDULED_SEND_RECHECK_DELAY.as_millis() as u64)
}

/// Whether the homeserver's listing of the user's delayed events still holds
/// `delay_id` as scheduled. An entry that already carries a sent event id or a
/// send error is finalised, not scheduled.
pub(crate) fn delayed_event_still_scheduled(
    delayed_events: &[matrix_sdk::ruma::api::client::delayed_events::DelayedEventData],
    delay_id: &str,
) -> bool {
    delayed_events.iter().any(|event| {
        event.delay_id == delay_id && event.event_id.is_none() && event.error.is_none()
    })
}

pub(crate) fn scheduled_send_transaction_id(scheduled_id: &str) -> String {
    let sanitized = scheduled_id
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_') {
                ch
            } else {
                '-'
            }
        })
        .collect::<String>();
    format!("desktop-{sanitized}")
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

    use super::{
        MSC4140_FEATURE, capability_from_unstable_features, delayed_event_still_scheduled,
        scheduled_send_transaction_id, server_delay_timeout,
    };

    fn listed_delayed_events(
        entries: serde_json::Value,
    ) -> Vec<matrix_sdk::ruma::api::client::delayed_events::DelayedEventData> {
        serde_json::from_value(entries).expect("synthetic delayed-event listing")
    }

    #[test]
    fn delayed_event_listing_decides_whether_a_server_item_is_still_scheduled() {
        // #1973: shape of a Tuwunel 2.0 / Synapse listing entry.
        let scheduled = listed_delayed_events(serde_json::json!([{
            "delay_id": "delay-pending",
            "room_id": "!room:example.test",
            "type": "m.room.message",
            "content": { "msgtype": "m.text", "body": "synthetic" },
            "delay": 2000,
            "running_since": 1_900_000_000_000_u64
        }]));
        assert!(delayed_event_still_scheduled(&scheduled, "delay-pending"));
        assert!(!delayed_event_still_scheduled(&scheduled, "delay-sent"));
        assert!(!delayed_event_still_scheduled(&[], "delay-pending"));

        let finalised = listed_delayed_events(serde_json::json!([{
            "delay_id": "delay-pending",
            "room_id": "!room:example.test",
            "type": "m.room.message",
            "content": { "msgtype": "m.text", "body": "synthetic" },
            "delay": 2000,
            "running_since": 1_900_000_000_000_u64,
            "event_id": "$sent:example.test"
        }]));
        assert!(!delayed_event_still_scheduled(&finalised, "delay-pending"));
    }

    #[test]
    fn scheduled_send_transaction_id_is_deterministic_and_matrix_safe() {
        assert_eq!(
            scheduled_send_transaction_id("scheduled 12/34"),
            "desktop-scheduled-12-34"
        );
    }

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
