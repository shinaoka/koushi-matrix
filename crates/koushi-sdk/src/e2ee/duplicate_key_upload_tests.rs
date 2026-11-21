//! The duplicate one-time-key upload signal (#1780).
//!
//! The SDK notification carries public key material, so the product may only
//! learn that a rejection happened. These tests pin the forwarding rules of the
//! payload-free signal, including the case the SDK produces when it cannot parse
//! the server's message.

use std::str::FromStr;

use matrix_sdk::encryption::DuplicateOneTimeKeyErrorMessage;
use tokio::sync::{broadcast, mpsc};

use super::{forward_duplicate_key_upload_errors, reported_duplicate_one_time_key_upload};
use crate::MatrixClientSession;
use koushi_state::SessionInfo;

/// The shape the SDK parser accepts, with synthetic key material taken from the
/// SDK's own parser test.
fn parsed_message() -> Option<DuplicateOneTimeKeyErrorMessage> {
    // The SDK's own parser test message, verbatim: its signature map key has to
    // stay `@example:matrix.org` for the SDK's parser to accept the JSON, and
    // nothing here is a real account (the key material is the SDK test's).
    let message = concat!(
        "One time key signed_curve25519:AAAAAAAAAAA already exists. ",
        r#"Old key: {"key":"dBcZBzQaiQYWf6rBPh2QypIOB/dxSoTeyaFaxNNbeHs","#,
        r#""signatures":{"@example:matrix.org":{"ed25519:AAAAAAAAAA":""#,
        r#"Fk45zHAbrd+1j9wZXLjL2Y/+DU/Mnz9yuvlfYBOOT7qExN2Jdud+5BAuNs8nZ/caS4wTF39Kg3zQpzaGERoCBg"}}};"#,
        r#" new key: {'key': 'CY0TWVK1/Kj3ZADuBcGe3UKvpT+IKAPMUsMeJhSDqno', "#,
        r#"'signatures': {'@example:matrix.org': {'ed25519:AAAAAAAAAA': "#,
        r#"'BQ9Gp0p+6srF+c8OyruqKKd9R4yaub3THYAyyBB/7X/rG8BwcAqFynzl1aGyFYun4Q+087a5OSiglCXI+/kQAA'}}}"#
    );
    Some(
        DuplicateOneTimeKeyErrorMessage::from_str(message)
            .expect("the SDK parser accepts its own message shape"),
    )
}

#[tokio::test]
async fn a_rejection_is_forwarded_as_a_payload_free_signal() {
    let (sender, mut signals) = mpsc::channel(1);
    let (notifications, receiver) = broadcast::channel(1);
    let task = tokio::spawn(forward_duplicate_key_upload_errors(receiver, sender));

    notifications
        .send(parsed_message())
        .expect("subscriber is present");
    assert_eq!(signals.recv().await, Some(()));

    // The SDK also reports a rejection whose message did not parse. The
    // rejection still happened, so the signal is raised the same way.
    notifications.send(None).expect("subscriber is present");
    assert_eq!(signals.recv().await, Some(()));

    drop(notifications);
    assert_eq!(signals.recv().await, None, "forwarding stops with the SDK");
    task.await.expect("forwarding task ends cleanly");
}

#[tokio::test]
async fn a_lagged_subscriber_still_reports_the_rejection() {
    let (sender, mut signals) = mpsc::channel(4);
    // Capacity one, two notifications before the forwarder reads: the next read
    // reports `Lagged`, which is a skipped notification, not a lost rejection.
    let (notifications, receiver) = broadcast::channel(1);
    let task = tokio::spawn(forward_duplicate_key_upload_errors(receiver, sender));

    notifications.send(parsed_message()).expect("subscriber");
    notifications.send(parsed_message()).expect("subscriber");
    drop(notifications);

    let mut reported = 0;
    while signals.recv().await.is_some() {
        reported += 1;
    }
    // One or two, depending on whether the forwarder read between the sends; the
    // lag itself must not swallow both.
    assert!(
        (1..=2).contains(&reported),
        "reported {reported} rejections"
    );
    task.await.expect("forwarding task ends cleanly");
}

/// The SDK publishes the rejection once per crypto store and records that it
/// did, so a device that went out of sync in an earlier run is only visible
/// through the stored fact.
#[tokio::test]
async fn the_stored_rejection_is_visible_without_a_live_notification() {
    let client = matrix_sdk::Client::builder()
        .homeserver_url("https://example.invalid")
        .build()
        .await
        .expect("client builds");
    let session = MatrixClientSession::from_client_for_testing(
        client.clone(),
        SessionInfo {
            homeserver: "https://example.invalid".to_owned(),
            user_id: "@example:example.invalid".to_owned(),
            device_id: "EXAMPLE".to_owned(),
            authentication_method: koushi_state::SessionAuthenticationMethod::Unknown,
        },
    );

    assert!(!reported_duplicate_one_time_key_upload(&session).await);

    client
        .state_store()
        .set_kv_data(
            matrix_sdk::store::StateStoreDataKey::OneTimeKeyAlreadyUploaded,
            matrix_sdk::store::StateStoreDataValue::OneTimeKeyAlreadyUploaded,
        )
        .await
        .expect("the state store accepts the SDK's own marker");

    assert!(reported_duplicate_one_time_key_upload(&session).await);
}
