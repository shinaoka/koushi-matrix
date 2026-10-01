//! Contact security details against real SDK crypto (#1024).
//!
//! The end-to-end tests run several real SDK clients against the matrix-sdk
//! crypto mock homeserver, so every signature is produced and checked by the
//! SDK itself. The template tests craft `/keys/query` responses for the
//! abnormal cases a well-behaved client never uploads.

use std::time::Duration;

use futures_util::StreamExt;
use koushi_state::{
    ContactDeviceSignature, ContactDevicesStatus, ContactIdentityVerification,
    ContactSecurityFailureKind, ContactSecuritySummary, ContactVerificationDirectChat,
    ContactVerificationOffer,
};
use matrix_sdk::{
    Client,
    ruma::{DeviceId, UserId, device_id, user_id},
    test_utils::mocks::MatrixMockServer,
};
use matrix_sdk_test::{
    ruma_response_to_json,
    test_json::keys_query_sets::{KeyQueryResponseTemplate, KeyQueryResponseTemplateDeviceOptions},
};
use serde_json::{Value, json};
use vodozemac::{Curve25519PublicKey, Ed25519SecretKey};
use wiremock::{
    Mock, ResponseTemplate,
    matchers::{method, path_regex},
};

use super::{
    ContactDeviceFacts, ContactIdentityFacts, ContactVerificationFacts, classify_contact_security,
    load_contact_security, observe_contact_security_changes, read_contact_security,
};
use crate::MatrixClientSession;

fn session(client: &Client, homeserver: String) -> MatrixClientSession {
    MatrixClientSession {
        info: koushi_state::SessionInfo {
            homeserver,
            user_id: client.user_id().unwrap().to_string(),
            device_id: client.device_id().unwrap().to_string(),
            authentication_method: koushi_state::SessionAuthenticationMethod::Password,
        },
        client: client.clone(),
        diagnostic_counters: koushi_diagnostics::DiagnosticCounterContext::registered(),
    }
}

async fn crypto_client(
    server: &MatrixMockServer,
    user_id: &UserId,
    device_id: &DeviceId,
) -> Client {
    let client = server
        .client_builder_for_crypto_end_to_end(user_id, device_id)
        .build()
        .await;
    // Upload the device keys.
    server.mock_sync().ok_and_run(&client, |_| {}).await;
    client
}

async fn cross_signed_client(
    server: &MatrixMockServer,
    user_id: &UserId,
    device_id: &DeviceId,
) -> Client {
    let client = crypto_client(server, user_id, device_id).await;
    client
        .encryption()
        .bootstrap_cross_signing(None)
        .await
        .expect("cross-signing bootstrap");
    client
}

async fn verify_contact(viewer: &Client, contact: &UserId) {
    viewer
        .encryption()
        .request_user_identity(contact)
        .await
        .expect("contact identity query")
        .expect("contact identity")
        .verify()
        .await
        .expect("manual identity verification");
}

fn devices(summary: &ContactSecuritySummary) -> (ContactDevicesStatus, u32, u32) {
    (
        summary.devices,
        summary.device_counts.owner_signed,
        summary.device_counts.not_owner_signed,
    )
}

// ── End-to-end with SDK-produced signatures ─────────────────────────────────

#[tokio::test]
async fn owner_signed_devices_and_your_verification_are_independent() {
    let server = MatrixMockServer::new().await;
    server.mock_crypto_endpoints_preset().await;
    let alice_id = user_id!("@alice:example.test");
    let bob_id = user_id!("@bob:example.test");
    let alice = cross_signed_client(&server, alice_id, device_id!("ALICE1")).await;
    let _bob = cross_signed_client(&server, bob_id, device_id!("BOB1")).await;
    let alice_session = session(&alice, server.uri());

    // 1. All devices owner-signed, contact not verified by you.
    let summary = load_contact_security(&alice_session, bob_id.as_str())
        .await
        .expect("load");
    assert_eq!(
        devices(&summary),
        (ContactDevicesStatus::AllOwnerSigned, 1, 0)
    );
    assert_eq!(
        summary.identity,
        ContactIdentityVerification::NotVerifiedByYou
    );
    // Verify user is offered; no DM exists yet, so one would be created.
    assert_eq!(
        summary.verification,
        ContactVerificationOffer::Offered {
            direct_chat: ContactVerificationDirectChat::New
        }
    );

    // 2. All devices owner-signed, contact verified by you.
    verify_contact(&alice, bob_id).await;
    let summary = load_contact_security(&alice_session, bob_id.as_str())
        .await
        .expect("load");
    assert_eq!(
        devices(&summary),
        (ContactDevicesStatus::AllOwnerSigned, 1, 0)
    );
    assert_eq!(summary.identity, ContactIdentityVerification::VerifiedByYou);
    // Not offered again for an already verified contact.
    assert_eq!(summary.verification, ContactVerificationOffer::NotOffered);

    // 3. The contact adds a device they have not confirmed: your verification
    // of them is preserved, and does not confirm the new device.
    let _bob_second = crypto_client(&server, bob_id, device_id!("BOB2")).await;
    let summary = load_contact_security(&alice_session, bob_id.as_str())
        .await
        .expect("load");
    assert_eq!(
        devices(&summary),
        (ContactDevicesStatus::SomeNotOwnerSigned, 1, 1)
    );
    assert_eq!(
        summary.device_signatures,
        vec![
            ContactDeviceSignature::OwnerSigned,
            ContactDeviceSignature::NotOwnerSigned
        ]
    );
    assert_eq!(summary.device_counts.owner_signature_invalid, 0);
    assert_eq!(summary.identity, ContactIdentityVerification::VerifiedByYou);
}

#[tokio::test]
async fn missing_cross_signing_is_not_the_same_as_unsigned_devices() {
    let server = MatrixMockServer::new().await;
    server.mock_crypto_endpoints_preset().await;
    let alice = cross_signed_client(
        &server,
        user_id!("@alice:example.test"),
        device_id!("ALICE1"),
    )
    .await;
    let carol_id = user_id!("@carol:example.test");
    let _carol = crypto_client(&server, carol_id, device_id!("CAROL1")).await;

    let summary = load_contact_security(&session(&alice, server.uri()), carol_id.as_str())
        .await
        .expect("load");
    assert_eq!(summary.devices, ContactDevicesStatus::OwnerIdentityMissing);
    assert_eq!(
        summary.device_signatures,
        vec![ContactDeviceSignature::OwnerIdentityMissing]
    );
    assert_eq!(summary.identity, ContactIdentityVerification::Unknown);
}

#[tokio::test]
async fn identity_change_is_attention_only_after_you_verified_it() {
    let server = MatrixMockServer::new().await;
    server.mock_crypto_endpoints_preset().await;
    let alice = cross_signed_client(
        &server,
        user_id!("@alice:example.test"),
        device_id!("ALICE1"),
    )
    .await;
    let alice_session = session(&alice, server.uri());
    let verified_id = user_id!("@bob:example.test");
    let never_verified_id = user_id!("@dave:example.test");
    let bob = cross_signed_client(&server, verified_id, device_id!("BOB1")).await;
    let dave = cross_signed_client(&server, never_verified_id, device_id!("DAVE1")).await;

    verify_contact(&alice, verified_id).await;
    let before = load_contact_security(&alice_session, verified_id.as_str())
        .await
        .expect("load");
    assert_eq!(before.identity, ContactIdentityVerification::VerifiedByYou);
    let before = load_contact_security(&alice_session, never_verified_id.as_str())
        .await
        .expect("load");
    assert_eq!(
        before.identity,
        ContactIdentityVerification::NotVerifiedByYou
    );

    // Both contacts reset their cross-signing identity.
    bob.encryption()
        .bootstrap_cross_signing(None)
        .await
        .expect("bob identity reset");
    dave.encryption()
        .bootstrap_cross_signing(None)
        .await
        .expect("dave identity reset");

    let verified_after = load_contact_security(&alice_session, verified_id.as_str())
        .await
        .expect("load");
    assert_eq!(
        verified_after.identity,
        ContactIdentityVerification::ChangedAfterVerification
    );
    // Re-verification is offered after an identity change.
    assert!(matches!(
        verified_after.verification,
        ContactVerificationOffer::Offered { .. }
    ));
    // The contact re-signed their device with the new identity, so the device
    // is confirmed again: identity change and device confirmation stay apart.
    assert_eq!(verified_after.devices, ContactDevicesStatus::AllOwnerSigned);

    let never_verified_after = load_contact_security(&alice_session, never_verified_id.as_str())
        .await
        .expect("load");
    assert_eq!(
        never_verified_after.identity,
        ContactIdentityVerification::NotVerifiedByYou
    );
}

/// `/keys/query` answers 200 with the contact's homeserver under `failures`
/// when it could not be reached; the SDK then keeps its cached keys. That is
/// not a fresh retrieval, so it must not re-confirm a cached verification.
#[tokio::test]
async fn keys_query_failure_for_the_contact_server_is_not_a_fresh_retrieval() {
    let server = MatrixMockServer::new().await;
    server.mock_crypto_endpoints_preset().await;
    let alice = cross_signed_client(
        &server,
        user_id!("@alice:example.test"),
        device_id!("ALICE1"),
    )
    .await;
    let alice_session = session(&alice, server.uri());
    let bob_id = user_id!("@bob:example.test");
    let bob = cross_signed_client(&server, bob_id, device_id!("BOB1")).await;
    verify_contact(&alice, bob_id).await;
    let cached = load_contact_security(&alice_session, bob_id.as_str())
        .await
        .expect("load");
    assert_eq!(cached.identity, ContactIdentityVerification::VerifiedByYou);
    assert_eq!(cached.devices, ContactDevicesStatus::AllOwnerSigned);

    // Bob resets his identity while his homeserver is unreachable from ours.
    bob.encryption()
        .bootstrap_cross_signing(None)
        .await
        .expect("bob identity reset");
    // Answers the SDK's single query for each retrieval.
    // (Not a scoped mock: wiremock 0.6 deactivates scoped mocks by an index
    // that its priority sort has reordered.)
    Mock::given(method("POST"))
        .and(path_regex(r"^/_matrix/client/.*/keys/query"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "device_keys": {},
            "failures": {
                "example.test": {
                    "errcode": "M_UNAVAILABLE",
                    "error": "synthetic federation failure",
                },
            },
        })))
        .with_priority(1)
        .up_to_n_times(2)
        .expect(2)
        .mount(server.server())
        .await;
    assert_eq!(
        load_contact_security(&alice_session, bob_id.as_str()).await,
        Err(ContactSecurityFailureKind::Network)
    );
    // Verify user does not act on the unconfirmed cached identity either.
    assert!(matches!(
        crate::request_user_verification(&alice_session, bob_id.as_str()).await,
        Err(crate::E2eeTrustError::Classified(
            crate::E2eeTrustFailureKind::Network
        ))
    ));

    // Once the server answers, the change is shown.
    let fresh = load_contact_security(&alice_session, bob_id.as_str())
        .await
        .expect("load");
    assert_eq!(
        fresh.identity,
        ContactIdentityVerification::ChangedAfterVerification
    );
}

#[tokio::test]
async fn partial_key_query_failure_does_not_project_stale_verified_identity() {
    let server = MatrixMockServer::new().await;
    server.mock_crypto_endpoints_preset().await;
    let alice = cross_signed_client(
        &server,
        user_id!("@alice:example.test"),
        device_id!("ALICE1"),
    )
    .await;
    let alice_session = session(&alice, server.uri());
    let bob_id = user_id!("@bob:example.test");
    let bob = cross_signed_client(&server, bob_id, device_id!("BOB1")).await;
    verify_contact(&alice, bob_id).await;
    assert_eq!(
        load_contact_security(&alice_session, bob_id.as_str())
            .await
            .expect("initial load")
            .identity,
        ContactIdentityVerification::VerifiedByYou
    );

    bob.encryption()
        .bootstrap_cross_signing(None)
        .await
        .expect("bob identity reset");
    Mock::given(method("POST"))
        .and(path_regex(r"^/_matrix/client/.*/keys/query"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "device_keys": {},
            "failures": {
                "example.test": {
                    "errcode": "M_UNAVAILABLE",
                    "error": "synthetic federation failure",
                },
            },
        })))
        .with_priority(1)
        .up_to_n_times(1)
        .expect(1)
        .mount(server.server())
        .await;

    assert_eq!(
        load_contact_security(&alice_session, bob_id.as_str()).await,
        Err(ContactSecurityFailureKind::Network)
    );
}

#[tokio::test]
async fn key_store_changes_are_observed_and_reread_without_network() {
    let server = MatrixMockServer::new().await;
    server.mock_crypto_endpoints_preset().await;
    let alice = cross_signed_client(
        &server,
        user_id!("@alice:example.test"),
        device_id!("ALICE1"),
    )
    .await;
    let alice_session = session(&alice, server.uri());
    let bob_id = user_id!("@bob:example.test");
    let _bob = cross_signed_client(&server, bob_id, device_id!("BOB1")).await;

    let mut changes = observe_contact_security_changes(&alice_session)
        .await
        .expect("observe");
    let _bob_second = crypto_client(&server, bob_id, device_id!("BOB2")).await;
    // Any path that updates the store (sync-driven or explicit) notifies.
    alice
        .encryption()
        .request_user_identity(bob_id)
        .await
        .expect("query");
    tokio::time::timeout(Duration::from_secs(5), changes.next())
        .await
        .expect("a key-store change notification")
        .expect("stream open");
    let summary = read_contact_security(&alice_session, bob_id.as_str())
        .await
        .expect("store read");
    assert_eq!(summary.device_counts.total, 2);
    assert_eq!(summary.devices, ContactDevicesStatus::SomeNotOwnerSigned);
}

/// A direct chat that appears after User info opened (for example the one a
/// failed first Verify user attempt created) is observed through `m.direct`,
/// so the confirmation step stops saying there is no direct chat.
#[tokio::test]
async fn a_new_direct_chat_is_observed_and_reread() {
    let server = MatrixMockServer::new().await;
    server.mock_crypto_endpoints_preset().await;
    let alice = cross_signed_client(
        &server,
        user_id!("@alice:example.test"),
        device_id!("ALICE1"),
    )
    .await;
    let alice_session = session(&alice, server.uri());
    let bob_id = user_id!("@bob:example.test");
    let _bob = cross_signed_client(&server, bob_id, device_id!("BOB1")).await;
    let before = load_contact_security(&alice_session, bob_id.as_str())
        .await
        .expect("load");
    assert_eq!(
        before.verification,
        ContactVerificationOffer::Offered {
            direct_chat: ContactVerificationDirectChat::New
        }
    );

    let mut changes = observe_contact_security_changes(&alice_session)
        .await
        .expect("observe");
    let dm = matrix_sdk::ruma::room_id!("!dm:example.test");
    server
        .mock_sync()
        .ok_and_run(&alice, |builder| {
            builder
                .add_joined_room(matrix_sdk_test::JoinedRoomBuilder::new(dm))
                .add_custom_global_account_data(json!({
                    "type": "m.direct",
                    "content": { bob_id.as_str(): [dm.as_str()] },
                }));
        })
        .await;
    tokio::time::timeout(Duration::from_secs(5), changes.next())
        .await
        .expect("a direct chat change notification")
        .expect("stream open");
    let after = read_contact_security(&alice_session, bob_id.as_str())
        .await
        .expect("store read");
    assert_eq!(
        after.verification,
        ContactVerificationOffer::Offered {
            direct_chat: ContactVerificationDirectChat::ExistingUnencrypted
        }
    );
}

// ── Crafted /keys/query responses ───────────────────────────────────────────

const CONTACT: &str = "@carol:example.test";

fn contact_id() -> &'static UserId {
    user_id!("@carol:example.test")
}

fn template() -> KeyQueryResponseTemplate {
    KeyQueryResponseTemplate::new(contact_id().to_owned()).with_cross_signing_keys(
        Ed25519SecretKey::from_slice(b"master12master12master12master12"),
        Ed25519SecretKey::from_slice(b"self1234self1234self1234self1234"),
        Ed25519SecretKey::from_slice(b"user1234user1234user1234user1234"),
    )
}

fn with_device(
    template: KeyQueryResponseTemplate,
    device_id: &DeviceId,
    seed: u8,
    options: KeyQueryResponseTemplateDeviceOptions,
) -> KeyQueryResponseTemplate {
    template.with_device(
        device_id,
        &Curve25519PublicKey::from([seed; 32]),
        &Ed25519SecretKey::from_slice(&[seed; 32]),
        options,
    )
}

fn signed() -> KeyQueryResponseTemplateDeviceOptions {
    KeyQueryResponseTemplateDeviceOptions::new().verified(true)
}

fn response_json(template: &KeyQueryResponseTemplate) -> Value {
    ruma_response_to_json(template.build_response())
}

/// Replace the device's own signature (`own_key`) or its owner cross-signature
/// with a well-formed signature that does not verify.
fn corrupt_device_signature(response: &mut Value, device_id: &str, own_key: bool) {
    let signatures = response["device_keys"][CONTACT][device_id]["signatures"][CONTACT]
        .as_object_mut()
        .expect("device signatures");
    let own_key_id = format!("ed25519:{device_id}");
    let forged = Ed25519SecretKey::from_slice(&[0x55; 32])
        .sign(b"not the device keys")
        .to_base64();
    for (key_id, value) in signatures.iter_mut() {
        if (key_id == &own_key_id) == own_key {
            *value = Value::String(forged.clone());
        }
    }
}

async fn serve_keys_query(server: &MatrixMockServer, body: Value) {
    Mock::given(method("POST"))
        .and(path_regex(r"^/_matrix/client/.*/keys/query"))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        // One retrieval issues a single SDK query.
        .up_to_n_times(1)
        .mount(server.server())
        .await;
}

async fn viewer(server: &MatrixMockServer) -> MatrixClientSession {
    let client = server.client_builder().build().await;
    session(&client, server.uri())
}

#[tokio::test]
async fn invalid_owner_signatures_are_distinct_and_never_confirmation() {
    let server = MatrixMockServer::new().await;
    let viewer = viewer(&server).await;
    let mut body = response_json(&with_device(
        with_device(template(), device_id!("GOOD"), 1, signed()),
        device_id!("TAMPERED"),
        2,
        signed(),
    ));
    corrupt_device_signature(&mut body, "TAMPERED", false);
    serve_keys_query(&server, body).await;

    let summary = load_contact_security(&viewer, CONTACT).await.expect("load");
    assert_eq!(summary.devices, ContactDevicesStatus::SomeNotOwnerSigned);
    assert_eq!(
        summary.device_signatures,
        vec![
            ContactDeviceSignature::OwnerSigned,
            ContactDeviceSignature::OwnerSignatureInvalid
        ]
    );
    assert_eq!(summary.device_counts.owner_signature_invalid, 1);
    assert_eq!(
        summary.identity,
        ContactIdentityVerification::NotVerifiedByYou
    );
    // This viewer has no cross-signing keys of its own, so it cannot sign
    // the contact's identity: explained instead of offered.
    assert_eq!(
        summary.verification,
        ContactVerificationOffer::RequiresYourCrossSigning
    );
}

#[tokio::test]
async fn devices_with_invalid_self_signatures_are_rejected_by_the_sdk() {
    let server = MatrixMockServer::new().await;
    let viewer = viewer(&server).await;
    let mut body = response_json(&with_device(
        with_device(template(), device_id!("GOOD"), 1, signed()),
        device_id!("FORGED"),
        2,
        signed(),
    ));
    corrupt_device_signature(&mut body, "FORGED", true);
    serve_keys_query(&server, body).await;

    let summary = load_contact_security(&viewer, CONTACT).await.expect("load");
    // The forged device never enters the store, so it is neither counted as
    // confirmed nor shown.
    assert_eq!(summary.device_counts.total, 1);
    assert_eq!(summary.devices, ContactDevicesStatus::AllOwnerSigned);
}

#[tokio::test]
async fn device_removal_and_empty_lists_are_reflected() {
    let server = MatrixMockServer::new().await;
    let viewer = viewer(&server).await;
    serve_keys_query(
        &server,
        response_json(&with_device(
            with_device(template(), device_id!("SIGNED"), 1, signed()),
            device_id!("UNSIGNED"),
            2,
            KeyQueryResponseTemplateDeviceOptions::new(),
        )),
    )
    .await;
    let summary = load_contact_security(&viewer, CONTACT).await.expect("load");
    assert_eq!(
        devices(&summary),
        (ContactDevicesStatus::SomeNotOwnerSigned, 1, 1)
    );

    // The unsigned device signs out.
    serve_keys_query(
        &server,
        response_json(&with_device(template(), device_id!("SIGNED"), 1, signed())),
    )
    .await;
    let summary = load_contact_security(&viewer, CONTACT).await.expect("load");
    assert_eq!(
        devices(&summary),
        (ContactDevicesStatus::AllOwnerSigned, 1, 0)
    );

    // Every device signs out: the homeserver answers with an empty device
    // map for the user, and an empty list is not a confirmation.
    let mut empty = response_json(&template());
    empty["device_keys"] = json!({ CONTACT: {} });
    serve_keys_query(&server, empty).await;
    let summary = load_contact_security(&viewer, CONTACT).await.expect("load");
    assert_eq!(summary.devices, ContactDevicesStatus::NoDevices);
    assert_eq!(summary.device_counts.total, 0);
    assert!(summary.device_signatures.is_empty());
}

#[tokio::test]
async fn dehydrated_devices_are_excluded_from_the_aggregate() {
    let server = MatrixMockServer::new().await;
    let viewer = viewer(&server).await;
    serve_keys_query(
        &server,
        response_json(&with_device(
            with_device(template(), device_id!("SIGNED"), 1, signed()),
            device_id!("DEHYDRATED"),
            2,
            KeyQueryResponseTemplateDeviceOptions::new().dehydrated(true),
        )),
    )
    .await;
    let summary = load_contact_security(&viewer, CONTACT).await.expect("load");
    assert_eq!(
        devices(&summary),
        (ContactDevicesStatus::AllOwnerSigned, 1, 0)
    );
    assert_eq!(summary.device_counts.excluded_dehydrated, 1);
}

#[tokio::test]
async fn retrieval_failure_is_an_error_not_a_summary() {
    let server = MatrixMockServer::new().await;
    let viewer = viewer(&server).await;
    Mock::given(method("POST"))
        .and(path_regex(r"^/_matrix/client/.*/keys/query"))
        .respond_with(ResponseTemplate::new(500).set_body_json(json!({
            "errcode": "M_UNKNOWN",
            "error": "synthetic failure",
        })))
        .mount(server.server())
        .await;
    assert_eq!(
        load_contact_security(&viewer, CONTACT).await,
        Err(ContactSecurityFailureKind::Network)
    );
    assert_eq!(
        load_contact_security(&viewer, "not a user id").await,
        Err(ContactSecurityFailureKind::Sdk)
    );
}

// ── Pure fold ───────────────────────────────────────────────────────────────

#[test]
fn fold_never_reports_confirmation_without_an_owner_identity() {
    let unsigned = ContactDeviceFacts {
        cross_signed_by_owner: false,
        has_owner_cross_signature: false,
        dehydrated: false,
    };
    let can_sign = ContactVerificationFacts {
        can_sign_identities: true,
        direct_chat: ContactVerificationDirectChat::ExistingEncrypted,
    };
    let summary = classify_contact_security(None, [unsigned, unsigned], can_sign);
    assert_eq!(summary.devices, ContactDevicesStatus::OwnerIdentityMissing);
    assert_eq!(summary.device_counts.owner_signed, 0);
    assert_eq!(summary.identity, ContactIdentityVerification::Unknown);
    assert_eq!(summary.verification, ContactVerificationOffer::NotOffered);

    // A violation wins over a stale verified flag.
    let summary = classify_contact_security(
        Some(ContactIdentityFacts {
            verified: true,
            verification_violation: true,
        }),
        [],
        can_sign,
    );
    assert_eq!(summary.devices, ContactDevicesStatus::NoDevices);
    assert_eq!(
        summary.identity,
        ContactIdentityVerification::ChangedAfterVerification
    );
    assert_eq!(
        summary.verification,
        ContactVerificationOffer::Offered {
            direct_chat: ContactVerificationDirectChat::ExistingEncrypted
        }
    );
}
