use matrix_sdk::{
    encryption::backups::BackupState,
    ruma::{CanonicalJsonValue, owned_user_id},
    test_utils::mocks::MatrixMockServer,
};
use matrix_sdk_test::{
    ruma_response_to_json, test_json::keys_query_sets::KeyQueryResponseTemplate,
};
use serde_json::json;
use vodozemac::Ed25519SecretKey;
use wiremock::{
    Mock, ResponseTemplate,
    matchers::{body_json, method, path},
};

use super::{
    CurrentDeviceTrustState, CurrentSessionBackupState, MatrixClientSession,
    MatrixCurrentSessionInspectionError, MatrixDeviceNameOutcome, OwnIdentityVerification,
    SessionInfo, classify_current_session_backup, classify_own_identity_verification,
    ensure_device_display_name,
};

async fn session(server: &MatrixMockServer) -> MatrixClientSession {
    let client = server.client_builder().build().await;
    let info = SessionInfo {
        homeserver: server.server().uri(),
        user_id: client.user_id().expect("mock user id").to_string(),
        device_id: client.device_id().expect("mock device id").to_string(),
        authentication_method: koushi_state::SessionAuthenticationMethod::OAuth,
    };
    MatrixClientSession::from_client_for_testing(client, info)
}

async fn mount_device(
    server: &MatrixMockServer,
    display_name: Option<&str>,
) -> wiremock::MockGuard {
    server
        .mock_devices()
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "devices": [{
                    "device_id": "DEVICEID",
                    "display_name": display_name,
                    "last_seen_ip": "private.invalid",
                    "last_seen_ts": 1_u64,
                    "user_id": "@example:localhost"
            }]
        })))
        .expect(1)
        .mount_as_scoped()
        .await
}

fn sign_json_for_test(
    value: &mut serde_json::Value,
    signing_key: &Ed25519SecretKey,
    user_id: &str,
    key_identifier: &str,
) {
    let mut unsigned = value.clone();
    let object = unsigned.as_object_mut().expect("device JSON object");
    object.remove("signatures");
    object.remove("unsigned");
    let canonical: CanonicalJsonValue = unsigned.try_into().expect("canonical device JSON");
    let signature = signing_key.sign(canonical.to_string().as_bytes());
    value["signatures"][user_id][format!("ed25519:{key_identifier}")] =
        signature.to_base64().into();
}

#[tokio::test]
async fn current_session_status_finds_current_device_display_name() {
    let server = MatrixMockServer::new().await;
    let session = session(&server).await;
    let _devices = mount_device(&server, Some("Koushi Workstation")).await;
    let _identity = server
        .mock_query_keys()
        .ok()
        .expect(1)
        .mount_as_scoped()
        .await;
    let _backup = server
        .mock_room_keys_version()
        .none()
        .expect(1)
        .mount_as_scoped()
        .await;

    let status = session
        .inspect_current_session()
        .await
        .expect("authoritative inspection");

    assert_eq!(
        status.device_display_name.as_deref(),
        Some("Koushi Workstation")
    );
    assert!(!status.is_cross_signed_by_owner);
    assert_eq!(
        status.own_identity_verification,
        OwnIdentityVerification::Missing
    );
    assert_eq!(status.key_backup, CurrentSessionBackupState::Disabled);
    assert!(!format!("{status:?}").contains("Koushi Workstation"));
    let serialized = serde_json::to_string(&status).expect("serialize coarse status");
    assert!(!serialized.contains("1234"));
    assert!(!serialized.contains("private.invalid"));
}

#[tokio::test]
async fn current_session_status_rejects_an_absent_current_device() {
    let server = MatrixMockServer::new().await;
    let session = session(&server).await;
    let _devices = server
        .mock_devices()
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "devices": [] })))
        .expect(1)
        .mount_as_scoped()
        .await;

    assert_eq!(
        session.inspect_current_session().await,
        Err(MatrixCurrentSessionInspectionError::CurrentDeviceMissing)
    );
}

#[tokio::test]
async fn current_session_status_classifies_server_and_crypto_failures_coarsely() {
    let device_server = MatrixMockServer::new().await;
    let device_session = session(&device_server).await;
    let _devices = device_server
        .mock_devices()
        .error500()
        .expect(1)
        .mount_as_scoped()
        .await;
    assert_eq!(
        device_session.inspect_current_session().await,
        Err(MatrixCurrentSessionInspectionError::Server)
    );

    let identity_server = MatrixMockServer::new().await;
    let identity_session = session(&identity_server).await;
    let _devices = mount_device(&identity_server, None).await;
    let _identity = identity_server
        .mock_query_keys()
        .error500()
        .expect(1)
        .mount_as_scoped()
        .await;
    assert_eq!(
        identity_session.inspect_current_session().await,
        Err(MatrixCurrentSessionInspectionError::IdentityRequest)
    );
}

#[tokio::test]
async fn current_session_status_reads_owner_cross_signing_and_unverified_own_identity() {
    let server = MatrixMockServer::new().await;
    let session = session(&server).await;
    let _devices = mount_device(&server, Some("Signed device")).await;
    let client = session.client();
    let user_id = client.user_id().expect("mock user id");
    let device_id = client.device_id().expect("mock device id");
    let current_device = client
        .encryption()
        .get_device(user_id, device_id)
        .await
        .expect("read current device")
        .expect("mock client stores its own device");
    let self_signing_key = Ed25519SecretKey::from_slice(b"self1234self1234self1234self1234");
    let response = KeyQueryResponseTemplate::new(owned_user_id!("@example:localhost"))
        .with_cross_signing_keys(
            Ed25519SecretKey::from_slice(b"master12master12master12master12"),
            Ed25519SecretKey::from_slice(b"self1234self1234self1234self1234"),
            Ed25519SecretKey::from_slice(b"user1234user1234user1234user1234"),
        )
        .build_response();
    let mut response_json = ruma_response_to_json(response);
    let device_keys = current_device
        .keys()
        .iter()
        .map(|(key_id, key)| (key_id.to_string(), key.to_base64()))
        .collect::<std::collections::BTreeMap<_, _>>();
    let mut current_device_json = json!({
            "user_id": user_id,
            "device_id": device_id,
            "algorithms": current_device.algorithms(),
            "keys": device_keys,
            "signatures": current_device.signatures(),
    });
    sign_json_for_test(
        &mut current_device_json,
        &self_signing_key,
        user_id.as_str(),
        &self_signing_key.public_key().to_base64(),
    );
    response_json["device_keys"][user_id.as_str()][device_id.as_str()] = current_device_json;
    let _identity = server
        .mock_query_keys()
        .respond_with(ResponseTemplate::new(200).set_body_json(response_json))
        .expect(1)
        .mount_as_scoped()
        .await;
    let _backup = server
        .mock_room_keys_version()
        .error500()
        .expect(1)
        .mount_as_scoped()
        .await;

    let status = session
        .inspect_current_session()
        .await
        .expect("authoritative inspection");

    assert_eq!(
        session.current_device_trust(),
        CurrentDeviceTrustState::Verified,
        "the SDK current-device verdict is authoritative even while own-identity verification is supplemental"
    );
    assert!(status.is_cross_signed_by_owner);
    assert_eq!(
        status.own_identity_verification,
        OwnIdentityVerification::Unverified
    );
    assert_eq!(status.key_backup, CurrentSessionBackupState::Unknown);
}

#[tokio::test]
async fn oauth_device_name_renames_only_an_empty_authoritative_name() {
    let server = MatrixMockServer::new().await;
    let session = session(&server).await;
    let _devices = mount_device(&server, Some("   ")).await;
    let _rename = Mock::given(method("PUT"))
        .and(path("/_matrix/client/v3/devices/DEVICEID"))
        .and(body_json(json!({ "display_name": "Koushi on Linux" })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
        .expect(1)
        .mount_as_scoped(server.server())
        .await;

    assert_eq!(
        ensure_device_display_name(&session, "Koushi on Linux").await,
        MatrixDeviceNameOutcome::Renamed
    );
}

#[tokio::test]
async fn oauth_device_name_preserves_existing_name_and_maps_failures_coarsely() {
    let named_server = MatrixMockServer::new().await;
    let named_session = session(&named_server).await;
    let _devices = mount_device(&named_server, Some("Custom device")).await;
    assert_eq!(
        ensure_device_display_name(&named_session, "Koushi on Linux").await,
        MatrixDeviceNameOutcome::Present
    );
    assert!(
        named_server
            .received_requests()
            .await
            .expect("request history")
            .iter()
            .all(|request| request.method.as_str() != "PUT")
    );

    let failed_server = MatrixMockServer::new().await;
    let failed_session = session(&failed_server).await;
    let _devices = mount_device(&failed_server, None).await;
    let _rename = Mock::given(method("PUT"))
        .and(path("/_matrix/client/v3/devices/DEVICEID"))
        .respond_with(ResponseTemplate::new(500).set_body_string("private raw failure"))
        .expect(1)
        .mount_as_scoped(failed_server.server())
        .await;
    let outcome = ensure_device_display_name(&failed_session, "Koushi on Linux").await;
    assert_eq!(outcome, MatrixDeviceNameOutcome::RenameFailed);
    assert!(!format!("{outcome:?}").contains("private raw failure"));
}

#[test]
fn current_session_status_classifies_identity_and_backup_without_secrets() {
    assert_eq!(
        classify_own_identity_verification(false, true),
        OwnIdentityVerification::Missing
    );
    assert_eq!(
        classify_own_identity_verification(true, false),
        OwnIdentityVerification::Unverified
    );
    assert_eq!(
        classify_own_identity_verification(true, true),
        OwnIdentityVerification::Verified
    );
    assert_eq!(
        classify_current_session_backup(BackupState::Enabled, Ok(true)),
        CurrentSessionBackupState::Ready
    );
    assert_eq!(
        classify_current_session_backup(BackupState::Unknown, Ok(false)),
        CurrentSessionBackupState::Disabled
    );
    assert_eq!(
        classify_current_session_backup(BackupState::Unknown, Err(())),
        CurrentSessionBackupState::Unknown
    );

    let error = MatrixCurrentSessionInspectionError::IdentityRequest;
    assert_eq!(
        serde_json::to_string(&error).expect("serialize coarse error"),
        "\"identity_request\""
    );
    assert!(!format!("{error:?}").contains("private"));
}
