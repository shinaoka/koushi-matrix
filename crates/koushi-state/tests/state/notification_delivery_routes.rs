//! #1460: a settings file written before per-destination routes loads conservatively.
//!
//! Every rule can only narrow what leaves the device: a switched-off channel or sink
//! gets no targets, only the sink the old consent named keeps message text, and
//! nothing becomes the entire-message mode.

use koushi_state::{
    NotificationDeliveryContent, NotificationDeliverySettings, NotificationSinkConfig,
};
use serde_json::json;

fn load(value: serde_json::Value) -> NotificationDeliverySettings {
    let mut settings: NotificationDeliverySettings =
        serde_json::from_value(value).expect("a legacy block loads");
    settings.canonicalize();
    settings
}

fn legacy_sink(id: &str, destination: &str, enabled: bool) -> serde_json::Value {
    json!({
        "id": id,
        "enabled": enabled,
        "destination": destination,
        "from_address": "notice@example.com",
        "sender_name": "Koushi",
        "config": { "kind": "resend", "endpoint": "https://api.resend.com/emails" },
    })
}

#[test]
fn an_enabled_legacy_channel_copies_its_targets_and_timing_to_enabled_sinks() {
    let settings = load(json!({
        "enabled": true,
        "direct_messages": false,
        "mentions": true,
        "keywords": false,
        "only_when_unfocused": false,
        "dwell_secs": 120,
        "min_interval_secs": 600,
        "watched": [{ "kind": "room", "room_id": "!watched:example.org" }],
        "sinks": [
            legacy_sink("home", "me@example.com", true),
            legacy_sink("off", "other@example.com", false),
        ],
    }));

    let home = settings.sink("home").expect("home");
    assert!(!home.targets.direct_messages);
    assert!(home.targets.mentions);
    assert!(!home.targets.keywords);
    assert!(home.targets.watches_room("!watched:example.org"));
    assert_eq!(home.timing.dwell_secs, 120);
    assert_eq!(home.timing.min_interval_secs, 600);
    assert!(!home.timing.only_when_unfocused);
    assert_eq!(home.content, NotificationDeliveryContent::NamesOnly);

    // A sink that was switched off stays configured but sends for nothing.
    let off = settings.sink("off").expect("off");
    assert!(!off.targets.has_any());
}

#[test]
fn a_disabled_legacy_channel_leaves_every_sink_without_targets() {
    for channel in [json!(false), serde_json::Value::Null] {
        let mut block = json!({
            "mentions": true,
            "sinks": [legacy_sink("home", "me@example.com", true)],
        });
        if !channel.is_null() {
            block["enabled"] = channel;
        }
        let settings = load(block);
        let home = settings.sink("home").expect("home");
        assert!(!home.targets.has_any());
    }
}

#[test]
fn only_the_sink_the_old_consent_named_keeps_a_prefix_and_none_becomes_full() {
    let settings = load(json!({
        "enabled": true,
        "prefix_chars": 140,
        "prefix_consent": {
            "sink_id": " HOME ",
            "destination": "me@example.com",
            "endpoint": "https://api.resend.com/emails",
            "accepted_at_ms": 7,
        },
        "sinks": [
            legacy_sink("home", "me@example.com", true),
            legacy_sink("work", "work@example.com", true),
        ],
    }));

    let home = settings.sink("home").expect("home");
    assert_eq!(
        home.effective_content(),
        NotificationDeliveryContent::Prefix { graphemes: 140 }
    );
    let consent = home.content_consent.as_ref().expect("migrated consent");
    assert_eq!(consent.sink_kind, "resend");
    assert_eq!(consent.recipient, "me@example.com");
    assert_eq!(consent.accepted_at_ms, 7);

    let work = settings.sink("work").expect("work");
    assert_eq!(work.content, NotificationDeliveryContent::NamesOnly);
    assert_eq!(work.content_consent, None);
    for sink in &settings.sinks {
        assert_ne!(sink.content, NotificationDeliveryContent::Full);
    }
}

#[test]
fn an_old_consent_for_another_address_or_endpoint_is_not_migrated() {
    for (destination, endpoint) in [
        ("old@example.com", "https://api.resend.com/emails"),
        ("me@example.com", "https://relay.example.com/send"),
    ] {
        let settings = load(json!({
            "enabled": true,
            "prefix_consent": {
                "sink_id": "home",
                "destination": destination,
                "endpoint": endpoint,
                "accepted_at_ms": 1,
            },
            "sinks": [legacy_sink("home", "me@example.com", true)],
        }));
        let home = settings.sink("home").expect("home");
        assert_eq!(home.content, NotificationDeliveryContent::NamesOnly);
        assert_eq!(home.content_consent, None);
    }
}

#[test]
fn a_current_file_round_trips_without_running_the_migration() {
    let mut settings = load(json!({
        "enabled": true,
        "sinks": [legacy_sink("home", "me@example.com", true)],
    }));
    let home = settings.sinks.get_mut(0).expect("home");
    home.targets.direct_messages = false;
    home.content = NotificationDeliveryContent::Full;
    home.content_consent = Some(home.consent_for_current(9));
    home.timing.dwell_secs = 60;

    let stored = serde_json::to_value(&settings).expect("serialize");
    assert!(
        stored.get("enabled").is_none(),
        "no legacy field is written"
    );
    assert!(stored["sinks"][0].get("enabled").is_none());
    let reloaded = load(stored);
    assert_eq!(reloaded, settings);
    assert_eq!(
        reloaded.sink("home").expect("home").effective_content(),
        NotificationDeliveryContent::Full
    );
}

#[test]
fn a_serverchan_destination_loads_with_its_default_endpoint() {
    let settings = load(json!({
        "sinks": [{
            "id": "wechat",
            "config": { "kind": "serverChan", "endpoint": "" },
            "targets": { "direct_messages": true, "mentions": true, "keywords": false, "watched": [] },
        }],
    }));
    let wechat = settings.sink("wechat").expect("wechat");
    assert_eq!(
        wechat.config,
        NotificationSinkConfig::ServerChan {
            endpoint: "https://sctapi.ftqq.com".to_owned()
        }
    );
    assert!(wechat.targets.direct_messages);
}

#[test]
fn a_migrated_prefix_never_exceeds_the_old_bound() {
    let settings = load(json!({
        "enabled": true,
        "prefix_chars": 450,
        "prefix_consent": {
            "sink_id": "home",
            "destination": "me@example.com",
            "endpoint": "https://api.resend.com/emails",
            "accepted_at_ms": 1,
        },
        "sinks": [legacy_sink("home", "me@example.com", true)],
    }));
    assert_eq!(
        settings.sink("home").expect("home").effective_content(),
        NotificationDeliveryContent::Prefix { graphemes: 200 }
    );
}
