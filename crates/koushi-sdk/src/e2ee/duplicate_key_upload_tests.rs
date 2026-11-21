//! The duplicate one-time-key upload signal (#1780).
//!
//! The SDK notification carries public key material, so the product may only
//! learn that a rejection happened. These tests pin the forwarding rules of the
//! payload-free signal, including the case the SDK produces when it cannot parse
//! the server's message.

use std::str::FromStr;

use matrix_sdk::encryption::DuplicateOneTimeKeyErrorMessage;
use tokio::sync::{broadcast, mpsc};

use super::forward_duplicate_key_upload_errors;

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
    let (sender, mut signals) = mpsc::channel(2);
    let (notifications, receiver) = broadcast::channel(1);
    let task = tokio::spawn(forward_duplicate_key_upload_errors(receiver, sender));

    notifications.send(parsed_message()).expect("subscriber");
    assert_eq!(signals.recv().await, Some(()));
    drop(notifications);
    assert_eq!(signals.recv().await, None);
    task.await.expect("forwarding task ends cleanly");
}
