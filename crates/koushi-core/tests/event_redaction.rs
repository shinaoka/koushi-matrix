//! Redaction tests for Core events.

#[allow(dead_code)]
mod support;

use support::fake_request_id;

use koushi_core::event::LiveSignalsEvent;
use koushi_core::{AccountKey, CoreEvent, E2eeTrustEvent, TimelineKey};
use koushi_state::{PresenceKind, SasEmoji, VerificationFlowState, VerificationTarget};

#[test]
fn e2ee_trust_events_are_typed_and_debug_redacts_identifiers() {
    let target = VerificationTarget {
        user_id: "@bob:example.test".to_owned(),
        device_id: "BOBDEVICE".to_owned(),
    };
    let event = E2eeTrustEvent::VerificationProgress {
        account_key: AccountKey("@alice:example.test".to_owned()),
        state: VerificationFlowState::SasPresented {
            request_id: 7,
            target,
            emojis: vec![SasEmoji {
                symbol: "🐶".to_owned(),
                description: "Dog".to_owned(),
            }],
        },
    };

    let value = serde_json::to_value(&event).expect("E2EE trust event serializes");
    assert_eq!(value["kind"], "verificationProgress");
    assert_eq!(value["state"]["kind"], "sasPresented");

    let debug = format!("{:?}", CoreEvent::E2eeTrust(event));
    assert!(debug.contains("VerificationProgress"));
    assert!(!debug.contains("@alice:example.test"));
    assert!(!debug.contains("@bob:example.test"));
    assert!(!debug.contains("BOBDEVICE"));
}

#[test]
fn live_signal_events_are_typed_and_debug_redacts_identifiers() {
    let request_id = fake_request_id();
    let key = TimelineKey::room(
        AccountKey("@alice:example.test".to_owned()),
        "!room:example.test",
    );

    let event = LiveSignalsEvent::PresenceUpdated {
        user_id: "@bob:example.test".to_owned(),
        presence: PresenceKind::Away,
    };
    let value = serde_json::to_value(&event).expect("live signal event serializes");
    assert_eq!(value["kind"], "presenceUpdated");
    assert_eq!(value["user_id"], "@bob:example.test");
    let debug_presence = format!("{:?}", CoreEvent::LiveSignals(event));
    assert!(debug_presence.contains("PresenceUpdated"));
    assert!(
        !debug_presence.contains("@bob:example.test"),
        "{debug_presence}"
    );

    let completion = LiveSignalsEvent::ReadReceiptSent {
        request_id,
        key,
        event_id: "$event:example.test".to_owned(),
    };
    let debug = format!("{:?}", CoreEvent::LiveSignals(completion));
    assert!(debug.contains("ReadReceiptSent"));
    assert!(!debug.contains("@alice:example.test"), "{debug}");
    assert!(!debug.contains("!room:example.test"), "{debug}");
    assert!(!debug.contains("$event:example.test"), "{debug}");
}
