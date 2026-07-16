use futures_util::{Stream, StreamExt};
use koushi_diagnostics::{DiagnosticEvent, DiagnosticField, DiagnosticLevel, record};
pub use koushi_state::E2eeRecoveryState;
use koushi_state::{
    AuthSecret, CrossSigningStatus, CurrentDeviceTrustState, DelegatedAuthLinks,
    IdentityResetAuthRequest, IdentityResetAuthType, KeyBackupStatus, LoginFlow, LoginFlowKind,
    LoginRequest, RecoveryRequest, RoomAttentionSummary, SasEmoji, SessionInfo,
    VerificationAccountKind, VerificationGateState, VerificationMethodCapability,
    VerificationTarget, room_attention_summary,
};

pub type CurrentDeviceTrustStream = Pin<Box<dyn Stream<Item = CurrentDeviceTrustState> + Send>>;

pub struct CurrentDeviceTrustObservation {
    pub current: CurrentDeviceTrustState,
    pub updates: CurrentDeviceTrustStream,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum IdentityFact {
    Existing,
    Missing,
    Unknown,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RecoveryFact {
    Available,
    Unavailable,
    Unknown,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct VerificationMethodFacts {
    identity: IdentityFact,
    verified_other_device_count: u64,
    recovery: RecoveryFact,
}

fn map_sdk_verification_state(
    state: matrix_sdk::encryption::VerificationState,
) -> CurrentDeviceTrustState {
    match state {
        matrix_sdk::encryption::VerificationState::Unknown => CurrentDeviceTrustState::Unknown,
        matrix_sdk::encryption::VerificationState::Verified => CurrentDeviceTrustState::Verified,
        matrix_sdk::encryption::VerificationState::Unverified => {
            CurrentDeviceTrustState::Unverified
        }
    }
}

fn map_verification_method_facts(facts: VerificationMethodFacts) -> VerificationGateState {
    let (account_kind, methods) = match facts.identity {
        IdentityFact::Unknown => (VerificationAccountKind::Unknown, Vec::new()),
        IdentityFact::Missing => (
            VerificationAccountKind::NewIdentity,
            vec![VerificationMethodCapability::Bootstrap],
        ),
        IdentityFact::Existing
            if matches!(facts.recovery, RecoveryFact::Unknown)
                && facts.verified_other_device_count == 0 =>
        {
            (VerificationAccountKind::Unknown, Vec::new())
        }
        IdentityFact::Existing => {
            let mut methods = Vec::new();
            if facts.verified_other_device_count > 0 {
                methods.push(VerificationMethodCapability::ExistingDeviceSas);
            }
            if matches!(facts.recovery, RecoveryFact::Available) {
                methods.push(VerificationMethodCapability::RecoveryKey);
                methods.push(VerificationMethodCapability::SecurityPhrase);
            }
            (VerificationAccountKind::ExistingIdentity, methods)
        }
    };
    VerificationGateState {
        methods,
        account_kind,
        failure: None,
    }
}

fn is_eligible_own_user_proof_device(
    current_device_id: &str,
    candidate_device_id: &str,
    cross_signed_by_owner: bool,
    blocked: bool,
) -> bool {
    is_own_user_verification_recipient(
        current_device_id,
        candidate_device_id,
        cross_signed_by_owner,
    ) && !blocked
}

fn is_own_user_verification_recipient(
    current_device_id: &str,
    candidate_device_id: &str,
    cross_signed_by_owner: bool,
) -> bool {
    candidate_device_id != current_device_id && cross_signed_by_owner
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct OwnUserSasDeviceFact {
    is_current: bool,
    cross_signed_by_owner: bool,
    blocked: bool,
    dehydrated: bool,
    curve_key_present: bool,
    ed25519_key_present: bool,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct OwnUserSasRecipientDiagnostics {
    other_device_count: u64,
    recipient_count: u64,
    eligible_device_count: u64,
    sender_device_query_visible: bool,
    sender_curve_key_present: bool,
    sender_ed25519_key_present: bool,
    interactive_recipient_count: u64,
    dehydrated_recipient_count: u64,
}

fn own_user_sas_recipient_diagnostics(
    devices: impl IntoIterator<Item = OwnUserSasDeviceFact>,
) -> OwnUserSasRecipientDiagnostics {
    let mut diagnostics = OwnUserSasRecipientDiagnostics::default();
    for device in devices {
        if device.is_current {
            diagnostics.sender_device_query_visible = true;
            diagnostics.sender_curve_key_present |= device.curve_key_present;
            diagnostics.sender_ed25519_key_present |= device.ed25519_key_present;
            continue;
        }

        diagnostics.other_device_count += 1;
        if !device.cross_signed_by_owner {
            continue;
        }

        diagnostics.recipient_count += 1;
        if !device.blocked {
            diagnostics.eligible_device_count += 1;
        }
        if device.dehydrated {
            diagnostics.dehydrated_recipient_count += 1;
        } else if device.curve_key_present && device.ed25519_key_present {
            diagnostics.interactive_recipient_count += 1;
        }
    }
    diagnostics
}

fn sas_delivery_event(stage: &'static str, flow_id: u64) -> DiagnosticEvent {
    DiagnosticEvent::new(DiagnosticLevel::Info, "core.sas_verification", stage)
        .field(DiagnosticField::count("flow_id", flow_id))
}

fn sas_recipients_resolved_event(
    flow_id: u64,
    diagnostics: OwnUserSasRecipientDiagnostics,
) -> DiagnosticEvent {
    sas_delivery_event("recipients_resolved", flow_id)
        .field(DiagnosticField::count(
            "other_device_count",
            diagnostics.other_device_count,
        ))
        .field(DiagnosticField::count(
            "recipient_count",
            diagnostics.recipient_count,
        ))
        .field(DiagnosticField::count(
            "eligible_device_count",
            diagnostics.eligible_device_count,
        ))
        .field(DiagnosticField::boolean(
            "sender_device_query_visible",
            diagnostics.sender_device_query_visible,
        ))
        .field(DiagnosticField::boolean(
            "sender_curve_key_present",
            diagnostics.sender_curve_key_present,
        ))
        .field(DiagnosticField::boolean(
            "sender_ed25519_key_present",
            diagnostics.sender_ed25519_key_present,
        ))
        .field(DiagnosticField::count(
            "interactive_recipient_count",
            diagnostics.interactive_recipient_count,
        ))
        .field(DiagnosticField::count(
            "dehydrated_recipient_count",
            diagnostics.dehydrated_recipient_count,
        ))
}

fn record_sas_delivery_event(event: DiagnosticEvent) {
    koushi_diagnostics::record_and_stderr(event);
}
use matrix_sdk::{
    authentication::{
        matrix::MatrixSession,
        oauth::{
            ClientId, ClientRegistrationData, OAuthSession, UserSession,
            registration::{ApplicationType, ClientMetadata, Localized, OAuthGrantType},
        },
    },
    deserialized_responses::SyncOrStrippedState,
    encryption::{BackupDownloadStrategy, EncryptionSettings},
    message_search::SearchError,
    room::ParentSpace,
    ruma::{
        events::{
            AnyGlobalAccountDataEventContent, AnySyncTimelineEvent, GlobalAccountDataEventType,
            SyncStateEvent,
            direct::DirectEventContent,
            fully_read::FullyReadEventContent,
            receipt::{ReceiptThread, ReceiptType},
            space::child::SpaceChildEventContent,
        },
        serde::Raw,
    },
    utils::UrlOrQuery,
};
use matrix_sdk_base::crypto::CollectStrategy;
use matrix_sdk_search::error::IndexError;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
    future::Future,
    net::IpAddr,
    path::{Path, PathBuf},
    pin::Pin,
    sync::Arc,
    time::{Duration, SystemTime},
};
use thiserror::Error;
use url::Url;
use zeroize::Zeroizing;

const LOGIN_DISCOVERY_PATH: &str = "_matrix/client/v3/login";
const DISCOVERY_TIMEOUT: Duration = Duration::from_secs(10);
const RESTRICTED_VERIFICATION_SYNC_SERVER_TIMEOUT: Duration = Duration::from_secs(3);
const MATRIX_ROOM_LIST_SNAPSHOT_LIMIT: usize = 4096;
pub const LOCAL_USER_ALIASES_ACCOUNT_DATA_TYPE: &str = "app.koushi.local_aliases";

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LoginDiscovery {
    pub homeserver: String,
    pub flows: Vec<LoginFlow>,
    pub delegated: DelegatedAuthLinks,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MatrixLoginDiscovery {
    pub homeserver: String,
    pub flows: Vec<MatrixLoginFlow>,
    pub delegated: DelegatedAuthLinks,
}

#[derive(Clone, Eq, PartialEq)]
pub struct OidcAuthorization {
    pub authorization_url: String,
    pub state: String,
}

impl fmt::Debug for OidcAuthorization {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("OidcAuthorization")
            .field("authorization_url", &"AuthorizationUrl(..)")
            .field("state", &"CsrfState(..)")
            .finish()
    }
}

#[derive(Clone)]
pub enum PendingOidcLogin {
    OAuth {
        client: matrix_sdk::Client,
        homeserver: String,
    },
    Sso {
        client: matrix_sdk::Client,
        homeserver: String,
    },
}

impl PendingOidcLogin {
    pub fn homeserver(&self) -> &str {
        match self {
            Self::OAuth { homeserver, .. } | Self::Sso { homeserver, .. } => homeserver,
        }
    }
}

impl fmt::Debug for PendingOidcLogin {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut debug = formatter.debug_struct("PendingOidcLogin");
        match self {
            Self::OAuth { .. } => debug.field("kind", &"OAuth"),
            Self::Sso { .. } => debug.field("kind", &"Sso"),
        }
        .field("client", &"MatrixClient(..)")
        .field("homeserver", &"Homeserver(..)")
        .finish()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MatrixLoginFlow {
    pub kind: MatrixLoginFlowKind,
    pub delegated_oidc_compatibility: bool,
    pub display_name: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MatrixLoginFlowKind {
    Password,
    Sso,
    Oidc,
    Token,
    Unknown(String),
}

#[derive(Clone, Eq, PartialEq)]
pub struct MatrixDeviceSessionSummary {
    pub raw_device_id: String,
    pub display_name: Option<String>,
    pub current: bool,
    pub verified: bool,
    pub inactive: bool,
}

impl fmt::Debug for MatrixDeviceSessionSummary {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MatrixDeviceSessionSummary")
            .field("raw_device_id", &"DeviceId(..)")
            .field(
                "display_name",
                &self.display_name.as_ref().map(|_| "DeviceDisplayName(..)"),
            )
            .field("current", &self.current)
            .field("verified", &self.verified)
            .field("inactive", &self.inactive)
            .finish()
    }
}

pub type E2eeRecoveryStateStream = Pin<Box<dyn Stream<Item = E2eeRecoveryState> + Send>>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MatrixCrossSigningStatus {
    pub has_master: bool,
    pub has_self_signing: bool,
    pub has_user_signing: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MatrixIdentityResetAuthType {
    Uiaa,
    OAuth,
    Unknown,
}

pub struct MatrixIdentityResetHandle {
    inner: matrix_sdk::encryption::recovery::IdentityResetHandle,
    auth_type: MatrixIdentityResetAuthType,
}

impl fmt::Debug for MatrixIdentityResetHandle {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MatrixIdentityResetHandle")
            .field("auth_type", &self.auth_type)
            .finish_non_exhaustive()
    }
}

impl MatrixIdentityResetHandle {
    pub fn auth_type(&self) -> MatrixIdentityResetAuthType {
        self.auth_type
    }

    pub fn desktop_auth_type(&self) -> IdentityResetAuthType {
        map_identity_reset_auth_type_to_desktop(self.auth_type)
    }

    pub async fn cancel(&self) {
        self.inner.cancel().await;
    }

    pub async fn reset(
        &self,
        session: &MatrixClientSession,
        request: &IdentityResetAuthRequest,
    ) -> Result<(), E2eeTrustError> {
        let auth = match request {
            IdentityResetAuthRequest::OAuthApproved => None,
            IdentityResetAuthRequest::UiaaPassword { password } => {
                let matrix_sdk::encryption::CrossSigningResetAuthType::Uiaa(uiaa) =
                    self.inner.auth_type()
                else {
                    return Err(E2eeTrustError::Sdk(
                        "identity reset auth type mismatch".to_owned(),
                    ));
                };
                let identifier = matrix_sdk::ruma::api::client::uiaa::UserIdentifier::Matrix(
                    matrix_sdk::ruma::api::client::uiaa::MatrixUserIdentifier::new(
                        session.info.user_id.clone(),
                    ),
                );
                let mut password_auth = matrix_sdk::ruma::api::client::uiaa::Password::new(
                    identifier,
                    password.expose_secret().to_owned(),
                );
                password_auth.session.clone_from(&uiaa.session);
                Some(matrix_sdk::ruma::api::client::uiaa::AuthData::Password(
                    password_auth,
                ))
            }
        };

        self.inner.reset(auth).await?;
        Ok(())
    }
}

pub enum IdentityResetOutcome {
    Completed,
    AuthRequired(MatrixIdentityResetHandle),
}

impl fmt::Debug for IdentityResetOutcome {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Completed => formatter.write_str("Completed"),
            Self::AuthRequired(handle) => formatter
                .debug_tuple("AuthRequired")
                .field(&handle.auth_type())
                .finish(),
        }
    }
}

#[derive(Clone)]
pub struct MatrixVerificationRequestHandle {
    inner: matrix_sdk::encryption::verification::VerificationRequest,
}

impl fmt::Debug for MatrixVerificationRequestHandle {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MatrixVerificationRequestHandle")
            .field("flow_id", &"FlowId(..)")
            .finish_non_exhaustive()
    }
}

impl MatrixVerificationRequestHandle {
    pub fn flow_id(&self) -> &str {
        self.inner.flow_id()
    }

    pub fn state(&self) -> MatrixVerificationRequestState {
        map_sdk_verification_request_state(self.inner.state())
    }

    pub fn changes(&self) -> MatrixVerificationRequestStateStream {
        Box::pin(self.inner.changes().map(map_sdk_verification_request_state))
    }
}

#[derive(Clone)]
pub struct MatrixOwnUserVerificationHandle {
    request: MatrixVerificationRequestHandle,
    eligible_device_count: u64,
}

impl fmt::Debug for MatrixOwnUserVerificationHandle {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MatrixOwnUserVerificationHandle")
            .field("eligible_device_count", &self.eligible_device_count)
            .finish_non_exhaustive()
    }
}

impl MatrixOwnUserVerificationHandle {
    pub fn eligible_device_count(&self) -> u64 {
        self.eligible_device_count
    }

    pub fn state(&self) -> MatrixVerificationRequestState {
        self.request.state()
    }

    pub fn changes(&self) -> MatrixVerificationRequestStateStream {
        self.request.changes()
    }
}

#[derive(Clone)]
pub struct MatrixIncomingVerificationRequest {
    target: VerificationTarget,
    handle: MatrixVerificationRequestHandle,
}

impl MatrixIncomingVerificationRequest {
    pub fn target(&self) -> &VerificationTarget {
        &self.target
    }

    pub fn handle(&self) -> &MatrixVerificationRequestHandle {
        &self.handle
    }

    pub fn into_parts(self) -> (VerificationTarget, MatrixVerificationRequestHandle) {
        (self.target, self.handle)
    }
}

impl fmt::Debug for MatrixIncomingVerificationRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MatrixIncomingVerificationRequest")
            .field("target", &"VerificationTarget(..)")
            .field("handle", &self.handle)
            .finish()
    }
}

pub struct MatrixIncomingVerificationRequestObserver {
    client: matrix_sdk::Client,
    receiver: tokio::sync::mpsc::Receiver<MatrixIncomingVerificationRequest>,
    handlers: Vec<matrix_sdk::event_handler::EventHandlerHandle>,
}

impl MatrixIncomingVerificationRequestObserver {
    pub async fn recv(&mut self) -> Option<MatrixIncomingVerificationRequest> {
        self.receiver.recv().await
    }
}

impl fmt::Debug for MatrixIncomingVerificationRequestObserver {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MatrixIncomingVerificationRequestObserver")
            .field("pending", &"Receiver(..)")
            .finish_non_exhaustive()
    }
}

impl Drop for MatrixIncomingVerificationRequestObserver {
    fn drop(&mut self) {
        for handler in self.handlers.drain(..) {
            self.client.remove_event_handler(handler);
        }
    }
}

#[derive(Clone)]
pub struct MatrixSasVerificationHandle {
    inner: matrix_sdk::encryption::verification::SasVerification,
}

impl fmt::Debug for MatrixSasVerificationHandle {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MatrixSasVerificationHandle")
            .field("flow_id", &"FlowId(..)")
            .finish_non_exhaustive()
    }
}

impl MatrixSasVerificationHandle {
    pub fn state(&self) -> MatrixSasState {
        map_sdk_sas_state(self.inner.state())
    }

    pub fn changes(&self) -> MatrixSasStateStream {
        Box::pin(self.inner.changes().map(map_sdk_sas_state))
    }

    pub fn emojis(&self) -> Option<Vec<SasEmoji>> {
        self.inner.emoji().map(map_sdk_sas_emojis_to_desktop)
    }
}

pub type MatrixVerificationRequestStateStream =
    Pin<Box<dyn Stream<Item = MatrixVerificationRequestState> + Send>>;
pub type MatrixSasStateStream = Pin<Box<dyn Stream<Item = MatrixSasState> + Send>>;

#[derive(Clone, Debug)]
pub enum MatrixVerificationRequestState {
    Created,
    Requested,
    Ready,
    SasStarted(MatrixSasVerificationHandle),
    Done,
    Cancelled {
        kind: MatrixVerificationCancelKind,
        cancelled_by_us: bool,
    },
    UnsupportedMethod,
}

fn verification_request_state_token(state: &MatrixVerificationRequestState) -> &'static str {
    match state {
        MatrixVerificationRequestState::Created => "created",
        MatrixVerificationRequestState::Requested => "requested",
        MatrixVerificationRequestState::Ready => "ready",
        MatrixVerificationRequestState::SasStarted(_) => "sas_started",
        MatrixVerificationRequestState::Done => "done",
        MatrixVerificationRequestState::Cancelled { .. } => "cancelled",
        MatrixVerificationRequestState::UnsupportedMethod => "unsupported_method",
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MatrixVerificationCancelKind {
    UnknownMethod,
    KeyMismatch,
    User,
    Timeout,
    AcceptedElsewhere,
    Other,
}

fn map_verification_cancel_kind(code: &str) -> MatrixVerificationCancelKind {
    match code {
        "m.unknown_method" => MatrixVerificationCancelKind::UnknownMethod,
        "m.key_mismatch" => MatrixVerificationCancelKind::KeyMismatch,
        "m.user" => MatrixVerificationCancelKind::User,
        "m.timeout" => MatrixVerificationCancelKind::Timeout,
        "m.accepted" => MatrixVerificationCancelKind::AcceptedElsewhere,
        _ => MatrixVerificationCancelKind::Other,
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MatrixSasState {
    Created,
    Started,
    Accepted,
    SasPresented { emojis: Vec<SasEmoji> },
    Confirmed,
    Done,
    Cancelled,
    UnsupportedShortAuth,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum KeyBackupRestoreScope {
    JoinedRooms,
}

#[derive(Clone, Eq, PartialEq)]
pub struct KeyBackupRestoreSummary {
    pub scope: KeyBackupRestoreScope,
    pub version: Option<String>,
    pub restored_rooms: u64,
    pub total_rooms: Option<u64>,
}

impl fmt::Debug for KeyBackupRestoreSummary {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("KeyBackupRestoreSummary")
            .field("scope", &self.scope)
            .field(
                "version",
                &self.version.as_ref().map(|_| "BackupVersion(..)"),
            )
            .field("restored_rooms", &self.restored_rooms)
            .field("total_rooms", &self.total_rooms)
            .finish()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RoomKeyExportSummary {
    pub exported_sessions: Option<u64>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RoomKeyImportSummary {
    pub imported_count: u64,
    pub total_count: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SecureBackupSetupSummary {
    pub recovery_key_written: bool,
}

#[derive(Clone, Eq, Error, PartialEq)]
pub enum E2eeTrustError {
    #[error("Matrix encryption is not initialized")]
    NoOlmMachine,
    #[error("Matrix SDK trust operation failed")]
    Sdk(String),
}

#[derive(thiserror::Error)]
pub enum DeleteDevicesError {
    #[error("interactive authentication required")]
    UiaaChallenge { session: Option<String> },
    #[error("Matrix SDK delete devices failed")]
    Sdk(String),
}

impl fmt::Debug for DeleteDevicesError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UiaaChallenge { session } => formatter
                .debug_struct("UiaaChallenge")
                .field("session", &session.as_ref().map(|_| "SessionId(..)"))
                .finish(),
            Self::Sdk(_) => formatter.write_str("Sdk(..)"),
        }
    }
}

impl fmt::Debug for E2eeTrustError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoOlmMachine => formatter.write_str("NoOlmMachine"),
            Self::Sdk(_) => formatter.write_str("Sdk(..)"),
        }
    }
}

impl From<matrix_sdk::Error> for E2eeTrustError {
    fn from(error: matrix_sdk::Error) -> Self {
        match error {
            matrix_sdk::Error::NoOlmMachine => Self::NoOlmMachine,
            other => Self::Sdk(other.to_string()),
        }
    }
}

impl From<matrix_sdk::encryption::recovery::RecoveryError> for E2eeTrustError {
    fn from(error: matrix_sdk::encryption::recovery::RecoveryError) -> Self {
        Self::Sdk(error.to_string())
    }
}

pub fn map_cross_signing_status_to_desktop(
    status: Option<MatrixCrossSigningStatus>,
) -> CrossSigningStatus {
    match status {
        None => CrossSigningStatus::Missing,
        Some(status) if status.has_master && status.has_self_signing && status.has_user_signing => {
            CrossSigningStatus::Trusted
        }
        Some(_) => CrossSigningStatus::NotTrusted,
    }
}

pub fn map_identity_reset_auth_type_to_desktop(
    auth_type: MatrixIdentityResetAuthType,
) -> IdentityResetAuthType {
    match auth_type {
        MatrixIdentityResetAuthType::Uiaa => IdentityResetAuthType::Uiaa,
        MatrixIdentityResetAuthType::OAuth => IdentityResetAuthType::OAuth,
        MatrixIdentityResetAuthType::Unknown => IdentityResetAuthType::Unknown,
    }
}

fn map_sdk_identity_reset_auth_type(
    auth_type: &matrix_sdk::encryption::CrossSigningResetAuthType,
) -> MatrixIdentityResetAuthType {
    match auth_type {
        matrix_sdk::encryption::CrossSigningResetAuthType::Uiaa(_) => {
            MatrixIdentityResetAuthType::Uiaa
        }
        matrix_sdk::encryption::CrossSigningResetAuthType::OAuth(_) => {
            MatrixIdentityResetAuthType::OAuth
        }
    }
}

fn map_sdk_verification_request_state(
    state: matrix_sdk::encryption::verification::VerificationRequestState,
) -> MatrixVerificationRequestState {
    use matrix_sdk::encryption::verification::{
        Verification, VerificationRequestState as SdkVerificationRequestState,
    };

    match state {
        SdkVerificationRequestState::Created { .. } => MatrixVerificationRequestState::Created,
        SdkVerificationRequestState::Requested { .. } => MatrixVerificationRequestState::Requested,
        SdkVerificationRequestState::Ready { .. } => MatrixVerificationRequestState::Ready,
        SdkVerificationRequestState::Transitioned { verification } => match verification {
            Verification::SasV1(inner) => {
                MatrixVerificationRequestState::SasStarted(MatrixSasVerificationHandle { inner })
            }
            #[allow(unreachable_patterns)]
            _ => MatrixVerificationRequestState::UnsupportedMethod,
        },
        SdkVerificationRequestState::Done => MatrixVerificationRequestState::Done,
        SdkVerificationRequestState::Cancelled(info) => MatrixVerificationRequestState::Cancelled {
            kind: map_verification_cancel_kind(info.cancel_code().as_str()),
            cancelled_by_us: info.cancelled_by_us(),
        },
    }
}

fn map_sdk_sas_state(state: matrix_sdk::encryption::verification::SasState) -> MatrixSasState {
    use matrix_sdk::encryption::verification::SasState as SdkSasState;

    match state {
        SdkSasState::Created { .. } => MatrixSasState::Created,
        SdkSasState::Started { .. } => MatrixSasState::Started,
        SdkSasState::Accepted { .. } => MatrixSasState::Accepted,
        SdkSasState::KeysExchanged { emojis, .. } => match emojis {
            Some(emojis) => MatrixSasState::SasPresented {
                emojis: map_sdk_sas_emojis_to_desktop(emojis.emojis),
            },
            None => MatrixSasState::UnsupportedShortAuth,
        },
        SdkSasState::Confirmed => MatrixSasState::Confirmed,
        SdkSasState::Done { .. } => MatrixSasState::Done,
        SdkSasState::Cancelled(_) => MatrixSasState::Cancelled,
    }
}

pub fn map_sdk_sas_emojis_to_desktop(
    emojis: [matrix_sdk::encryption::verification::Emoji; 7],
) -> Vec<SasEmoji> {
    emojis
        .into_iter()
        .map(|emoji| SasEmoji {
            symbol: emoji.symbol.to_owned(),
            description: emoji.description.to_owned(),
        })
        .collect()
}

pub async fn cross_signing_status(
    session: &MatrixClientSession,
) -> Result<CrossSigningStatus, E2eeTrustError> {
    let status = session
        .client()
        .encryption()
        .cross_signing_status()
        .await
        .map(|status| MatrixCrossSigningStatus {
            has_master: status.has_master,
            has_self_signing: status.has_self_signing,
            has_user_signing: status.has_user_signing,
        });
    Ok(map_cross_signing_status_to_desktop(status))
}

pub async fn bootstrap_cross_signing(
    session: &MatrixClientSession,
    auth_secret: Option<&AuthSecret>,
) -> Result<CrossSigningStatus, E2eeTrustError> {
    let encryption = session.client().encryption();
    match encryption.bootstrap_cross_signing(None).await {
        Ok(()) => {}
        Err(error) => {
            let Some(auth_secret) = auth_secret else {
                return Err(error.into());
            };
            let Some(uiaa_session) = error
                .as_uiaa_response()
                .and_then(|response| response.session.clone())
            else {
                return Err(error.into());
            };
            let identifier = matrix_sdk::ruma::api::client::uiaa::UserIdentifier::Matrix(
                matrix_sdk::ruma::api::client::uiaa::MatrixUserIdentifier::new(
                    session.info.user_id.clone(),
                ),
            );
            let mut password_auth = matrix_sdk::ruma::api::client::uiaa::Password::new(
                identifier,
                auth_secret.expose_secret().to_owned(),
            );
            password_auth.session = Some(uiaa_session);
            encryption
                .bootstrap_cross_signing(Some(
                    matrix_sdk::ruma::api::client::uiaa::AuthData::Password(password_auth),
                ))
                .await?;
        }
    }
    cross_signing_status(session).await
}

pub async fn enable_key_backup(
    session: &MatrixClientSession,
    passphrase: Option<&AuthSecret>,
) -> Result<KeyBackupStatus, E2eeTrustError> {
    let encryption = session.client().encryption();
    if let Some(passphrase) = passphrase {
        let _recovery_key = encryption
            .recovery()
            .enable()
            .wait_for_backups_to_upload()
            .with_passphrase(passphrase.expose_secret())
            .await?;
    } else {
        encryption.recovery().enable_backup().await?;
    }
    Ok(map_backup_state_to_desktop(encryption.backups().state()))
}

pub async fn restore_key_backup(
    session: &MatrixClientSession,
    request: &RecoveryRequest,
    version: Option<&str>,
) -> Result<KeyBackupRestoreSummary, E2eeTrustError> {
    let encryption = session.client().encryption();
    encryption
        .recovery()
        .recover(request.secret.expose_secret())
        .await?;

    download_joined_room_keys_from_backup(session, version).await
}

pub async fn download_joined_room_keys_from_backup(
    session: &MatrixClientSession,
    version: Option<&str>,
) -> Result<KeyBackupRestoreSummary, E2eeTrustError> {
    let encryption = session.client().encryption();
    let backup_state = encryption.backups().state();
    if !matches!(
        backup_state,
        matrix_sdk::encryption::backups::BackupState::Enabled
            | matrix_sdk::encryption::backups::BackupState::Downloading
    ) {
        return Err(E2eeTrustError::Sdk(
            "key backup unavailable after recovery".to_owned(),
        ));
    }

    let rooms = session.client().joined_rooms();
    let total_rooms = rooms.len() as u64;
    let mut restored_rooms = 0;
    for room in rooms {
        encryption
            .backups()
            .download_room_keys_for_room(room.room_id())
            .await?;
        restored_rooms += 1;
    }

    let backup_status = map_backup_state_to_desktop(encryption.backups().state());
    let backup_version = version.map(str::to_owned).or_else(|| match backup_status {
        KeyBackupStatus::Enabled { version } => Some(version),
        KeyBackupStatus::Restoring { version, .. } => version,
        _ => None,
    });

    Ok(KeyBackupRestoreSummary {
        scope: KeyBackupRestoreScope::JoinedRooms,
        version: backup_version,
        restored_rooms,
        total_rooms: Some(total_rooms),
    })
}

pub async fn download_room_key_from_backup(
    session: &MatrixClientSession,
    room_id: &str,
    session_id: &str,
) -> Result<bool, E2eeTrustError> {
    let room_id = matrix_sdk::ruma::RoomId::parse(room_id)
        .map_err(|_| E2eeTrustError::Sdk("invalid room id".to_owned()))?;
    session
        .client()
        .encryption()
        .backups()
        .download_room_key(room_id.as_ref(), session_id)
        .await
        .map_err(E2eeTrustError::from)
}

#[cfg(not(target_family = "wasm"))]
pub async fn export_room_keys_to_file(
    session: &MatrixClientSession,
    path: PathBuf,
    passphrase: &AuthSecret,
) -> Result<RoomKeyExportSummary, E2eeTrustError> {
    session
        .client()
        .encryption()
        .export_room_keys(path, passphrase.expose_secret(), |_| true)
        .await?;
    Ok(RoomKeyExportSummary {
        exported_sessions: None,
    })
}

#[cfg(not(target_family = "wasm"))]
pub async fn import_room_keys_from_file(
    session: &MatrixClientSession,
    path: PathBuf,
    passphrase: &AuthSecret,
) -> Result<RoomKeyImportSummary, E2eeTrustError> {
    let result = session
        .client()
        .encryption()
        .import_room_keys(path, passphrase.expose_secret())
        .await
        .map_err(|error| E2eeTrustError::Sdk(error.to_string()))?;
    Ok(RoomKeyImportSummary {
        imported_count: result.imported_count as u64,
        total_count: result.total_count as u64,
    })
}

pub async fn bootstrap_secure_backup(
    session: &MatrixClientSession,
    passphrase: Option<&AuthSecret>,
    recovery_key_destination_path: Option<PathBuf>,
) -> Result<SecureBackupSetupSummary, E2eeTrustError> {
    let recovery = session.client().encryption().recovery();
    let recovery_key = match passphrase {
        Some(passphrase) => {
            recovery
                .enable()
                .wait_for_backups_to_upload()
                .with_passphrase(passphrase.expose_secret())
                .await?
        }
        None => recovery.enable().wait_for_backups_to_upload().await?,
    };
    let recovery_key_written =
        write_recovery_key_if_requested(recovery_key, recovery_key_destination_path)?;
    Ok(SecureBackupSetupSummary {
        recovery_key_written,
    })
}

pub async fn change_secure_backup_passphrase(
    session: &MatrixClientSession,
    old_secret: &AuthSecret,
    new_passphrase: &AuthSecret,
    recovery_key_destination_path: Option<PathBuf>,
) -> Result<SecureBackupSetupSummary, E2eeTrustError> {
    let recovery_key = session
        .client()
        .encryption()
        .recovery()
        .recover_and_reset(old_secret.expose_secret())
        .with_passphrase(new_passphrase.expose_secret())
        .await?;
    let recovery_key_written =
        write_recovery_key_if_requested(recovery_key, recovery_key_destination_path)?;
    Ok(SecureBackupSetupSummary {
        recovery_key_written,
    })
}

fn write_recovery_key_if_requested(
    recovery_key: String,
    destination_path: Option<PathBuf>,
) -> Result<bool, E2eeTrustError> {
    let recovery_key = Zeroizing::new(recovery_key);
    let Some(destination_path) = destination_path else {
        return Ok(false);
    };
    std::fs::write(destination_path, recovery_key.as_bytes()).map_err(|_| {
        E2eeTrustError::Sdk("secure backup recovery key delivery failed".to_owned())
    })?;
    Ok(true)
}

pub async fn reset_identity(
    session: &MatrixClientSession,
) -> Result<IdentityResetOutcome, E2eeTrustError> {
    let outcome = session
        .client()
        .encryption()
        .recovery()
        .reset_identity()
        .await?;

    Ok(match outcome {
        Some(handle) => {
            let auth_type = map_sdk_identity_reset_auth_type(handle.auth_type());
            IdentityResetOutcome::AuthRequired(MatrixIdentityResetHandle {
                inner: handle,
                auth_type,
            })
        }
        None => IdentityResetOutcome::Completed,
    })
}

pub async fn complete_identity_reset(
    session: &MatrixClientSession,
    handle: &MatrixIdentityResetHandle,
    request: &IdentityResetAuthRequest,
) -> Result<(), E2eeTrustError> {
    handle.reset(session, request).await
}

/// Threshold after which a device is considered inactive (90 days).
const INACTIVE_DEVICE_THRESHOLD_DAYS: u64 = 90;

pub async fn list_devices(
    session: &MatrixClientSession,
) -> Result<Vec<MatrixDeviceSessionSummary>, E2eeTrustError> {
    let response = session
        .client()
        .devices()
        .await
        .map_err(|error| E2eeTrustError::Sdk(error.to_string()))?;
    let user_id = match matrix_sdk::ruma::OwnedUserId::try_from(session.info.user_id.as_str()) {
        Ok(user_id) => user_id,
        Err(_) => {
            return Err(E2eeTrustError::Sdk("invalid session user id".to_owned()));
        }
    };
    let now_millis = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or_default();
    let inactive_threshold_millis = INACTIVE_DEVICE_THRESHOLD_DAYS * 24 * 60 * 60 * 1000;

    let mut summaries = Vec::with_capacity(response.devices.len());
    for device in response.devices {
        let device_id = device.device_id.clone();
        let is_current = device_id.as_str() == session.info.device_id;
        let verified = match session
            .client()
            .encryption()
            .get_device(&user_id, &device_id)
            .await
        {
            Ok(Some(crypto_device)) => crypto_device.is_verified_with_cross_signing(),
            Ok(None) | Err(_) => false,
        };
        let inactive = device
            .last_seen_ts
            .map(|timestamp| i64::from(timestamp.0) as u64)
            .is_some_and(|timestamp_millis| {
                now_millis.saturating_sub(timestamp_millis) > inactive_threshold_millis
            });

        summaries.push(MatrixDeviceSessionSummary {
            current: is_current,
            raw_device_id: device_id.to_string(),
            display_name: device.display_name,
            verified,
            inactive,
        });
    }
    Ok(summaries)
}

pub async fn rename_device(
    session: &MatrixClientSession,
    raw_device_id: &str,
    display_name: &str,
) -> Result<(), E2eeTrustError> {
    let device_id = matrix_sdk::ruma::OwnedDeviceId::from(raw_device_id);
    session
        .client()
        .rename_device(&device_id, display_name)
        .await
        .map_err(|error| E2eeTrustError::Sdk(error.to_string()))?;
    Ok(())
}

pub async fn delete_devices(
    session: &MatrixClientSession,
    raw_device_ids: &[String],
    auth: Option<&IdentityResetAuthRequest>,
    uiaa_session: Option<&str>,
) -> Result<(), DeleteDevicesError> {
    let device_ids = raw_device_ids
        .iter()
        .map(|id| matrix_sdk::ruma::OwnedDeviceId::from(id.as_str()))
        .collect::<Vec<_>>();
    let auth_data = delete_devices_auth_data(session, auth, uiaa_session);
    match session
        .client()
        .delete_devices(&device_ids, auth_data)
        .await
    {
        Ok(_) => Ok(()),
        Err(error) => {
            if let Some(uiaa) = error.as_uiaa_response() {
                Err(DeleteDevicesError::UiaaChallenge {
                    session: uiaa.session.clone(),
                })
            } else {
                Err(DeleteDevicesError::Sdk(error.to_string()))
            }
        }
    }
}

fn delete_devices_auth_data(
    session: &MatrixClientSession,
    auth: Option<&IdentityResetAuthRequest>,
    uiaa_session: Option<&str>,
) -> Option<matrix_sdk::ruma::api::client::uiaa::AuthData> {
    let IdentityResetAuthRequest::UiaaPassword { password } = auth? else {
        return None;
    };
    let identifier = matrix_sdk::ruma::api::client::uiaa::UserIdentifier::Matrix(
        matrix_sdk::ruma::api::client::uiaa::MatrixUserIdentifier::new(
            session.info.user_id.clone(),
        ),
    );
    let mut password_auth = matrix_sdk::ruma::api::client::uiaa::Password::new(
        identifier,
        password.expose_secret().to_owned(),
    );
    password_auth.session = uiaa_session.map(str::to_owned);
    Some(matrix_sdk::ruma::api::client::uiaa::AuthData::Password(
        password_auth,
    ))
}

#[derive(thiserror::Error)]
pub enum AccountManagementError {
    #[error("interactive authentication required")]
    UiaaChallenge { session: Option<String> },
    #[error("Matrix SDK account management failed")]
    Sdk(String),
}

impl fmt::Debug for AccountManagementError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UiaaChallenge { session } => formatter
                .debug_struct("UiaaChallenge")
                .field("session", &session.as_ref().map(|_| "SessionId(..)"))
                .finish(),
            Self::Sdk(_) => formatter.write_str("Sdk(..)"),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AccountManagementCapabilities {
    pub change_password: bool,
}

pub async fn account_management_capabilities(
    session: &MatrixClientSession,
) -> AccountManagementCapabilities {
    let change_password = session
        .client()
        .homeserver_capabilities()
        .can_change_password()
        .await
        .ok()
        .unwrap_or(true);
    AccountManagementCapabilities { change_password }
}

pub async fn change_password(
    session: &MatrixClientSession,
    new_password: &AuthSecret,
    auth: Option<&IdentityResetAuthRequest>,
    uiaa_session: Option<&str>,
) -> Result<(), AccountManagementError> {
    let auth_data = account_management_auth_data(session, auth, uiaa_session);
    match session
        .client()
        .account()
        .change_password(new_password.expose_secret(), auth_data)
        .await
    {
        Ok(_) => Ok(()),
        Err(error) => {
            if let Some(uiaa) = error.as_uiaa_response() {
                Err(AccountManagementError::UiaaChallenge {
                    session: uiaa.session.clone(),
                })
            } else {
                Err(AccountManagementError::Sdk(error.to_string()))
            }
        }
    }
}

pub async fn deactivate_account(
    session: &MatrixClientSession,
    erase_data: bool,
    auth: Option<&IdentityResetAuthRequest>,
    uiaa_session: Option<&str>,
) -> Result<(), AccountManagementError> {
    let auth_data = account_management_auth_data(session, auth, uiaa_session);
    match session
        .client()
        .account()
        .deactivate(None, auth_data, erase_data)
        .await
    {
        Ok(_) => Ok(()),
        Err(error) => {
            if let Some(uiaa) = error.as_uiaa_response() {
                Err(AccountManagementError::UiaaChallenge {
                    session: uiaa.session.clone(),
                })
            } else {
                Err(AccountManagementError::Sdk(error.to_string()))
            }
        }
    }
}

fn account_management_auth_data(
    session: &MatrixClientSession,
    auth: Option<&IdentityResetAuthRequest>,
    uiaa_session: Option<&str>,
) -> Option<matrix_sdk::ruma::api::client::uiaa::AuthData> {
    let IdentityResetAuthRequest::UiaaPassword { password } = auth? else {
        return None;
    };
    let identifier = matrix_sdk::ruma::api::client::uiaa::UserIdentifier::Matrix(
        matrix_sdk::ruma::api::client::uiaa::MatrixUserIdentifier::new(
            session.info.user_id.clone(),
        ),
    );
    let mut password_auth = matrix_sdk::ruma::api::client::uiaa::Password::new(
        identifier,
        password.expose_secret().to_owned(),
    );
    password_auth.session = uiaa_session.map(str::to_owned);
    Some(matrix_sdk::ruma::api::client::uiaa::AuthData::Password(
        password_auth,
    ))
}

pub async fn request_device_verification(
    session: &MatrixClientSession,
    target: &VerificationTarget,
) -> Result<MatrixVerificationRequestHandle, E2eeTrustError> {
    let user_id = matrix_sdk::ruma::OwnedUserId::try_from(target.user_id.as_str())
        .map_err(|_| E2eeTrustError::Sdk("invalid verification user id".to_owned()))?;
    let device_id = matrix_sdk::ruma::OwnedDeviceId::from(target.device_id.as_str());
    let device = session
        .client()
        .encryption()
        .get_device(&user_id, &device_id)
        .await
        .map_err(|error| E2eeTrustError::Sdk(error.to_string()))?
        .ok_or_else(|| E2eeTrustError::Sdk("verification target device not found".to_owned()))?;

    let inner = device
        .request_verification_with_methods(vec![
            matrix_sdk::ruma::events::key::verification::VerificationMethod::SasV1,
        ])
        .await?;
    Ok(MatrixVerificationRequestHandle { inner })
}

pub async fn discover_current_session_verification_methods(
    session: &MatrixClientSession,
) -> VerificationGateState {
    let Ok(user_id) = matrix_sdk::ruma::OwnedUserId::try_from(session.info.user_id.as_str()) else {
        return map_verification_method_facts(VerificationMethodFacts {
            identity: IdentityFact::Unknown,
            verified_other_device_count: 0,
            recovery: RecoveryFact::Unknown,
        });
    };
    let encryption = session.client().encryption();
    let identity = match encryption.request_user_identity(&user_id).await {
        Ok(Some(_)) => IdentityFact::Existing,
        Ok(None) => IdentityFact::Missing,
        Err(_) => IdentityFact::Unknown,
    };
    if !matches!(identity, IdentityFact::Existing) {
        return map_verification_method_facts(VerificationMethodFacts {
            identity,
            verified_other_device_count: 0,
            recovery: if matches!(identity, IdentityFact::Unknown) {
                RecoveryFact::Unknown
            } else {
                RecoveryFact::Unavailable
            },
        });
    }
    let verified_other_device_count = match encryption.get_user_devices(&user_id).await {
        Ok(devices) => devices
            .devices()
            .filter(|device| {
                // A provisional device does not trust the owner identity yet,
                // so local cross-signing trust creates a chicken-and-egg
                // dependency. Proof eligibility is the authoritative owner
                // signature on a distinct, non-blocked device.
                is_eligible_own_user_proof_device(
                    &session.info.device_id,
                    device.device_id().as_str(),
                    device.is_cross_signed_by_owner(),
                    device.is_blacklisted(),
                )
            })
            .count() as u64,
        Err(_) => {
            return map_verification_method_facts(VerificationMethodFacts {
                identity: IdentityFact::Unknown,
                verified_other_device_count: 0,
                recovery: RecoveryFact::Unknown,
            });
        }
    };
    let recovery = match session.e2ee_recovery_state() {
        E2eeRecoveryState::Enabled | E2eeRecoveryState::Incomplete => RecoveryFact::Available,
        E2eeRecoveryState::Disabled => RecoveryFact::Unavailable,
        E2eeRecoveryState::Unknown => RecoveryFact::Unknown,
    };
    map_verification_method_facts(VerificationMethodFacts {
        identity,
        verified_other_device_count,
        recovery,
    })
}

pub async fn request_own_user_sas_verification(
    session: &MatrixClientSession,
    flow_id: u64,
) -> Result<MatrixOwnUserVerificationHandle, E2eeTrustError> {
    record_sas_delivery_event(sas_delivery_event("request_started", flow_id));
    let user_id = match matrix_sdk::ruma::OwnedUserId::try_from(session.info.user_id.as_str()) {
        Ok(user_id) => user_id,
        Err(_) => {
            record_sas_delivery_event(
                sas_delivery_event("request_send_finished", flow_id)
                    .field(DiagnosticField::token("outcome", "failed"))
                    .field(DiagnosticField::token("failure_stage", "invalid_user_id")),
            );
            return Err(E2eeTrustError::Sdk(
                "invalid verification user id".to_owned(),
            ));
        }
    };
    let encryption = session.client().encryption();
    let identity = match encryption.request_user_identity(&user_id).await {
        Ok(Some(identity)) => identity,
        Ok(None) => {
            record_sas_delivery_event(
                sas_delivery_event("request_send_finished", flow_id)
                    .field(DiagnosticField::token("outcome", "failed"))
                    .field(DiagnosticField::token("failure_stage", "identity_missing")),
            );
            return Err(E2eeTrustError::Sdk(
                "verification identity unavailable".to_owned(),
            ));
        }
        Err(_) => {
            record_sas_delivery_event(
                sas_delivery_event("request_send_finished", flow_id)
                    .field(DiagnosticField::token("outcome", "failed"))
                    .field(DiagnosticField::token("failure_stage", "identity_query")),
            );
            return Err(E2eeTrustError::Sdk(
                "verification identity unavailable".to_owned(),
            ));
        }
    };
    let devices = match encryption.get_user_devices(&user_id).await {
        Ok(devices) => devices,
        Err(_) => {
            record_sas_delivery_event(
                sas_delivery_event("request_send_finished", flow_id)
                    .field(DiagnosticField::token("outcome", "failed"))
                    .field(DiagnosticField::token("failure_stage", "device_query")),
            );
            return Err(E2eeTrustError::Sdk(
                "verification devices unavailable".to_owned(),
            ));
        }
    };
    let recipient_diagnostics =
        own_user_sas_recipient_diagnostics(devices.devices().map(|device| OwnUserSasDeviceFact {
            is_current: device.device_id().as_str() == session.info.device_id,
            cross_signed_by_owner: device.is_cross_signed_by_owner(),
            blocked: device.is_blacklisted(),
            dehydrated: device.is_dehydrated(),
            curve_key_present: device.curve25519_key().is_some(),
            ed25519_key_present: device.ed25519_key().is_some(),
        }));
    let eligible_device_count = recipient_diagnostics.eligible_device_count;
    record_sas_delivery_event(sas_recipients_resolved_event(
        flow_id,
        recipient_diagnostics,
    ));
    if eligible_device_count == 0 {
        record_sas_delivery_event(
            sas_delivery_event("request_send_finished", flow_id)
                .field(DiagnosticField::token("outcome", "failed"))
                .field(DiagnosticField::token(
                    "failure_stage",
                    "no_eligible_device",
                )),
        );
        return Err(E2eeTrustError::Sdk(
            "verification device unavailable".to_owned(),
        ));
    }
    let inner = match identity
        .request_verification_with_methods(vec![
            matrix_sdk::ruma::events::key::verification::VerificationMethod::SasV1,
        ])
        .await
    {
        Ok(inner) => inner,
        Err(_) => {
            record_sas_delivery_event(
                sas_delivery_event("request_send_finished", flow_id)
                    .field(DiagnosticField::token("outcome", "failed"))
                    .field(DiagnosticField::token("failure_stage", "send")),
            );
            return Err(E2eeTrustError::Sdk(
                "verification request failed".to_owned(),
            ));
        }
    };
    let request = MatrixVerificationRequestHandle { inner };
    record_sas_delivery_event(
        sas_delivery_event("request_send_finished", flow_id)
            .field(DiagnosticField::token("outcome", "success"))
            .field(DiagnosticField::token(
                "initial_state",
                verification_request_state_token(&request.state()),
            )),
    );
    Ok(MatrixOwnUserVerificationHandle {
        request,
        eligible_device_count,
    })
}

pub async fn start_own_user_sas_verification(
    handle: &MatrixOwnUserVerificationHandle,
) -> Result<Option<MatrixSasVerificationHandle>, E2eeTrustError> {
    start_sas_verification(&handle.request).await
}

pub async fn cancel_own_user_sas_verification(
    handle: &MatrixOwnUserVerificationHandle,
) -> Result<(), E2eeTrustError> {
    cancel_verification_request(&handle.request).await
}

pub fn observe_incoming_verification_requests(
    session: &MatrixClientSession,
) -> MatrixIncomingVerificationRequestObserver {
    let client = session.client();
    let (sender, receiver) = tokio::sync::mpsc::channel(32);

    let to_device_client = client.clone();
    let to_device_sender = sender.clone();
    let to_device_handler = client.add_event_handler(
        move |event: matrix_sdk::ruma::events::key::verification::request::ToDeviceKeyVerificationRequestEvent| {
            let client = to_device_client.clone();
            let sender = to_device_sender.clone();
            async move {
                if let Some(request) =
                    incoming_verification_request_for_flow(&client, &event.sender, event.content.transaction_id.as_str()).await
                {
                    let _ = sender.send(request).await;
                }
            }
        },
    );

    let room_client = client.clone();
    let room_sender = sender;
    let room_handler = client.add_event_handler(
        move |event: matrix_sdk::ruma::events::room::message::OriginalSyncRoomMessageEvent| {
            let client = room_client.clone();
            let sender = room_sender.clone();
            async move {
                if !matches!(
                    &event.content.msgtype,
                    matrix_sdk::ruma::events::room::message::MessageType::VerificationRequest(_)
                ) {
                    return;
                }
                if let Some(request) = incoming_verification_request_for_flow(
                    &client,
                    &event.sender,
                    event.event_id.as_str(),
                )
                .await
                {
                    let _ = sender.send(request).await;
                }
            }
        },
    );

    MatrixIncomingVerificationRequestObserver {
        client,
        receiver,
        handlers: vec![to_device_handler, room_handler],
    }
}

async fn incoming_verification_request_for_flow(
    client: &matrix_sdk::Client,
    sender: &matrix_sdk::ruma::UserId,
    flow_id: &str,
) -> Option<MatrixIncomingVerificationRequest> {
    let request = client
        .encryption()
        .get_verification_request(sender, flow_id)
        .await?;
    let matrix_sdk::encryption::verification::VerificationRequestState::Requested {
        other_device_data,
        ..
    } = request.state()
    else {
        return None;
    };

    Some(MatrixIncomingVerificationRequest {
        target: VerificationTarget {
            user_id: sender.to_string(),
            device_id: other_device_data.device_id().to_string(),
        },
        handle: MatrixVerificationRequestHandle { inner: request },
    })
}

pub async fn accept_verification_request(
    handle: &MatrixVerificationRequestHandle,
) -> Result<(), E2eeTrustError> {
    handle
        .inner
        .accept_with_methods(vec![
            matrix_sdk::ruma::events::key::verification::VerificationMethod::SasV1,
        ])
        .await?;
    Ok(())
}

pub async fn start_sas_verification(
    handle: &MatrixVerificationRequestHandle,
) -> Result<Option<MatrixSasVerificationHandle>, E2eeTrustError> {
    Ok(handle
        .inner
        .start_sas()
        .await?
        .map(|inner| MatrixSasVerificationHandle { inner }))
}

pub async fn accept_sas_verification(
    handle: &MatrixSasVerificationHandle,
) -> Result<(), E2eeTrustError> {
    handle.inner.accept().await?;
    Ok(())
}

pub async fn confirm_sas_verification(
    handle: &MatrixSasVerificationHandle,
) -> Result<(), E2eeTrustError> {
    handle.inner.confirm().await?;
    Ok(())
}

pub async fn mismatch_sas_verification(
    handle: &MatrixSasVerificationHandle,
) -> Result<(), E2eeTrustError> {
    handle.inner.mismatch().await?;
    Ok(())
}

pub async fn cancel_verification_request(
    handle: &MatrixVerificationRequestHandle,
) -> Result<(), E2eeTrustError> {
    handle.inner.cancel().await?;
    Ok(())
}

pub async fn cancel_sas_verification(
    handle: &MatrixSasVerificationHandle,
) -> Result<(), E2eeTrustError> {
    handle.inner.cancel().await?;
    Ok(())
}

pub fn map_backup_state_to_desktop(
    state: matrix_sdk::encryption::backups::BackupState,
) -> KeyBackupStatus {
    use matrix_sdk::encryption::backups::BackupState;

    match state {
        BackupState::Unknown => KeyBackupStatus::Unknown,
        BackupState::Creating | BackupState::Enabling | BackupState::Resuming => {
            KeyBackupStatus::Enabling { request_id: 0 }
        }
        BackupState::Enabled => KeyBackupStatus::Enabled {
            version: "available".to_owned(),
        },
        BackupState::Downloading => KeyBackupStatus::Restoring {
            request_id: 0,
            version: None,
            restored_rooms: 0,
            total_rooms: None,
        },
        BackupState::Disabling => KeyBackupStatus::Disabled,
    }
}

#[cfg(test)]
mod e2ee_trust_tests {
    use koushi_state::{
        AuthSecret, CrossSigningStatus, CurrentDeviceTrustState, IdentityResetAuthType,
        KeyBackupStatus, SasEmoji, VerificationAccountKind, VerificationMethodCapability,
    };
    use matrix_sdk::encryption::backups::BackupState;

    use super::{
        E2eeTrustError, IdentityFact, KeyBackupRestoreScope, KeyBackupRestoreSummary,
        MatrixCrossSigningStatus, MatrixDeviceSessionSummary, MatrixIdentityResetAuthType,
        MatrixIncomingVerificationRequest, MatrixIncomingVerificationRequestObserver,
        PersistableMatrixSession, RecoveryFact, RoomKeyExportSummary, RoomKeyImportSummary,
        SecureBackupSetupSummary, VerificationMethodFacts, accept_sas_verification,
        accept_verification_request, bootstrap_cross_signing, bootstrap_secure_backup,
        cancel_sas_verification, cancel_verification_request, change_secure_backup_passphrase,
        complete_identity_reset, confirm_sas_verification, cross_signing_status, delete_devices,
        enable_key_backup, export_room_keys_to_file, import_room_keys_from_file, list_devices,
        map_backup_state_to_desktop, map_cross_signing_status_to_desktop,
        map_identity_reset_auth_type_to_desktop, map_sdk_sas_emojis_to_desktop,
        map_sdk_verification_state, map_verification_method_facts, mismatch_sas_verification,
        observe_incoming_verification_requests, rename_device, request_device_verification,
        reset_identity, restore_key_backup, restore_session, restricted_verification_sync_filter,
        start_sas_verification, write_recovery_key_if_requested,
    };

    const MATRIX_KEY_EXPORT_HEADER: &str = "-----BEGIN MEGOLM SESSION DATA-----";
    const MATRIX_KEY_EXPORT_FOOTER: &str = "-----END MEGOLM SESSION DATA-----";
    const ELEMENT_COMPATIBLE_KEY_EXPORT: &str = "\
-----BEGIN MEGOLM SESSION DATA-----\n\
Af7mGhlzQ+eGvHu93u0YXd3D/+vYMs3E7gQqOhuCtkvGAAAAASH7pEdWvFyAP1JUisAcpEo\n\
Xke2Q7Kr9hVl/SCc6jXBNeJCZcrUbUV4D/tRQIl3E9L4fOk928YI1J+3z96qiH0uE7hpsCI\n\
CkHKwjPU+0XTzFdIk1X8H7sZ+MD/2Sg/q3y8rtUjz7uEj4GUTnb+9SCOTVmJsRfqgUpM1CU\n\
bDLytHf1JkohY4tWEgpsCc67xdzgodjr12qYrfg/zNm3LGpxlrffJknw4rk5QFTj4kMbqbD\n\
ZZgDTni+HxRTDGge2J620lMOiznvXX+H09Rwruqx5aJvvaaKd86jWRpiO2oSFqHn4u5ONl9\n\
41uzm62Sj0eIm6ZbA9NQs87jQw4LxsejhZVL+NdjIg80zVSBTWhTdo0DTnbFSNP4ReOiz0U\n\
XosOF8A5T8Vdx2nvA0GXltfcHKVKQYh/LJAkNQ7P9UYL4ae/5TtQZkhB1KxCLTRWqADCl53\n\
uBMGpG53EMgY6G6K2DEIOkcv7sdXQF5WpemiSWZqJRWj+cjfs9BpCTbkp/rszWFl2TniWpR\n\
RqIbT2jORlN4rTvdtF0F4z1pqP4qWyR3sLNTkXm9CFRzWADNG0RDZKxbCoo6RPvtaCTfaHo\n\
SwfvzBS6CjfAG+FOugpV48o7+XetaUUPZ6/tZSPhCdeV8eP9q5r0QwWeXFogzoNzWt4HYx9\n\
MdXxzD+f0mtg5gzehrrEEARwI2bCvPpHxlt/Na9oW/GBpkjwR1LSKgg4CtpRyWngPjdEKpZ\n\
GYW19pdjg0qdXNk/eqZsQTsNWVo6A\n\
-----END MEGOLM SESSION DATA-----";

    #[test]
    fn cross_signing_status_maps_to_private_data_free_desktop_status() {
        assert_eq!(
            map_cross_signing_status_to_desktop(None),
            CrossSigningStatus::Missing
        );
        assert_eq!(
            map_cross_signing_status_to_desktop(Some(MatrixCrossSigningStatus {
                has_master: true,
                has_self_signing: true,
                has_user_signing: true,
            })),
            CrossSigningStatus::Trusted
        );
        assert_eq!(
            map_cross_signing_status_to_desktop(Some(MatrixCrossSigningStatus {
                has_master: true,
                has_self_signing: false,
                has_user_signing: true,
            })),
            CrossSigningStatus::NotTrusted
        );
    }

    #[test]
    fn current_device_trust_maps_all_sdk_verification_states() {
        use matrix_sdk::encryption::VerificationState;

        assert_eq!(
            map_sdk_verification_state(VerificationState::Unknown),
            CurrentDeviceTrustState::Unknown
        );
        assert_eq!(
            map_sdk_verification_state(VerificationState::Verified),
            CurrentDeviceTrustState::Verified
        );
        assert_eq!(
            map_sdk_verification_state(VerificationState::Unverified),
            CurrentDeviceTrustState::Unverified
        );
    }

    #[test]
    fn verification_method_discovery_distinguishes_identity_facts() {
        let existing_with_sas = map_verification_method_facts(VerificationMethodFacts {
            identity: IdentityFact::Existing,
            verified_other_device_count: 2,
            recovery: RecoveryFact::Unavailable,
        });
        assert_eq!(
            existing_with_sas.account_kind,
            VerificationAccountKind::ExistingIdentity
        );
        assert_eq!(
            existing_with_sas.methods,
            vec![VerificationMethodCapability::ExistingDeviceSas]
        );

        let new_identity = map_verification_method_facts(VerificationMethodFacts {
            identity: IdentityFact::Missing,
            verified_other_device_count: 0,
            recovery: RecoveryFact::Unavailable,
        });
        assert_eq!(
            new_identity.account_kind,
            VerificationAccountKind::NewIdentity
        );
        assert_eq!(
            new_identity.methods,
            vec![VerificationMethodCapability::Bootstrap]
        );

        let unknown = map_verification_method_facts(VerificationMethodFacts {
            identity: IdentityFact::Unknown,
            verified_other_device_count: 0,
            recovery: RecoveryFact::Available,
        });
        assert_eq!(unknown.account_kind, VerificationAccountKind::Unknown);
        assert!(unknown.methods.is_empty());

        let existing_with_recovery = map_verification_method_facts(VerificationMethodFacts {
            identity: IdentityFact::Existing,
            verified_other_device_count: 0,
            recovery: RecoveryFact::Available,
        });
        assert_eq!(
            existing_with_recovery.methods,
            vec![
                VerificationMethodCapability::RecoveryKey,
                VerificationMethodCapability::SecurityPhrase,
            ]
        );

        let existing_without_proof = map_verification_method_facts(VerificationMethodFacts {
            identity: IdentityFact::Existing,
            verified_other_device_count: 0,
            recovery: RecoveryFact::Unavailable,
        });
        assert_eq!(
            existing_without_proof.account_kind,
            VerificationAccountKind::ExistingIdentity
        );
        assert!(existing_without_proof.methods.is_empty());

        let sas_survives_unknown_recovery =
            map_verification_method_facts(VerificationMethodFacts {
                identity: IdentityFact::Existing,
                verified_other_device_count: 1,
                recovery: RecoveryFact::Unknown,
            });
        assert_eq!(
            sas_survives_unknown_recovery.methods,
            vec![VerificationMethodCapability::ExistingDeviceSas]
        );

        let unknown_without_known_proof = map_verification_method_facts(VerificationMethodFacts {
            identity: IdentityFact::Existing,
            verified_other_device_count: 0,
            recovery: RecoveryFact::Unknown,
        });
        assert_eq!(
            unknown_without_known_proof.account_kind,
            VerificationAccountKind::Unknown
        );
    }

    #[test]
    fn own_user_proof_eligibility_requires_distinct_owner_signed_unblocked_device() {
        assert!(super::is_eligible_own_user_proof_device(
            "CURRENT", "OTHER", true, false
        ));
        assert!(!super::is_eligible_own_user_proof_device(
            "CURRENT", "CURRENT", true, false
        ));
        assert!(!super::is_eligible_own_user_proof_device(
            "CURRENT", "OTHER", false, false
        ));
        assert!(!super::is_eligible_own_user_proof_device(
            "CURRENT", "OTHER", true, true
        ));
    }

    #[test]
    fn own_user_request_recipient_requires_a_distinct_owner_signed_device() {
        assert!(super::is_own_user_verification_recipient(
            "CURRENT", "OTHER", true
        ));
        assert!(!super::is_own_user_verification_recipient(
            "CURRENT", "CURRENT", true
        ));
        assert!(!super::is_own_user_verification_recipient(
            "CURRENT", "OTHER", false
        ));
    }

    #[test]
    fn own_user_sas_recipient_diagnostics_distinguish_sender_and_interactive_targets() {
        use super::OwnUserSasDeviceFact as Fact;

        let diagnostics = super::own_user_sas_recipient_diagnostics([
            Fact {
                is_current: true,
                cross_signed_by_owner: false,
                blocked: false,
                dehydrated: false,
                curve_key_present: true,
                ed25519_key_present: true,
            },
            Fact {
                is_current: false,
                cross_signed_by_owner: true,
                blocked: false,
                dehydrated: false,
                curve_key_present: true,
                ed25519_key_present: true,
            },
            Fact {
                is_current: false,
                cross_signed_by_owner: true,
                blocked: false,
                dehydrated: true,
                curve_key_present: true,
                ed25519_key_present: true,
            },
            Fact {
                is_current: false,
                cross_signed_by_owner: true,
                blocked: true,
                dehydrated: false,
                curve_key_present: false,
                ed25519_key_present: true,
            },
            Fact {
                is_current: false,
                cross_signed_by_owner: false,
                blocked: false,
                dehydrated: false,
                curve_key_present: true,
                ed25519_key_present: true,
            },
        ]);

        assert!(diagnostics.sender_device_query_visible);
        assert!(diagnostics.sender_curve_key_present);
        assert!(diagnostics.sender_ed25519_key_present);
        assert_eq!(diagnostics.other_device_count, 4);
        assert_eq!(diagnostics.recipient_count, 3);
        assert_eq!(diagnostics.eligible_device_count, 2);
        assert_eq!(diagnostics.interactive_recipient_count, 1);
        assert_eq!(diagnostics.dehydrated_recipient_count, 1);
    }

    #[test]
    fn sas_delivery_event_contains_only_closed_private_safe_fields() {
        let event = super::sas_delivery_event("recipients_resolved", 41)
            .field(koushi_diagnostics::DiagnosticField::count(
                "other_device_count",
                3,
            ))
            .field(koushi_diagnostics::DiagnosticField::count(
                "recipient_count",
                1,
            ));
        assert_eq!(event.source, "core.sas_verification");
        assert_eq!(
            koushi_diagnostics::format_event(&event),
            "stage=recipients_resolved flow_id=41 other_device_count=3 recipient_count=1"
        );
    }

    #[test]
    fn sas_recipients_resolved_event_includes_sender_readiness_without_identifiers() {
        let event = super::sas_recipients_resolved_event(
            42,
            super::OwnUserSasRecipientDiagnostics {
                other_device_count: 9,
                recipient_count: 6,
                eligible_device_count: 6,
                sender_device_query_visible: true,
                sender_curve_key_present: true,
                sender_ed25519_key_present: true,
                interactive_recipient_count: 5,
                dehydrated_recipient_count: 1,
            },
        );

        assert_eq!(
            koushi_diagnostics::format_event(&event),
            "stage=recipients_resolved flow_id=42 other_device_count=9 recipient_count=6 eligible_device_count=6 sender_device_query_visible=true sender_curve_key_present=true sender_ed25519_key_present=true interactive_recipient_count=5 dehydrated_recipient_count=1"
        );
    }

    #[test]
    fn verification_cancel_codes_map_to_closed_private_safe_categories() {
        use super::MatrixVerificationCancelKind as Kind;

        assert_eq!(
            super::map_verification_cancel_kind("m.unknown_method"),
            Kind::UnknownMethod
        );
        assert_eq!(
            super::map_verification_cancel_kind("m.key_mismatch"),
            Kind::KeyMismatch
        );
        assert_eq!(super::map_verification_cancel_kind("m.user"), Kind::User);
        assert_eq!(
            super::map_verification_cancel_kind("m.timeout"),
            Kind::Timeout
        );
        assert_eq!(
            super::map_verification_cancel_kind("m.accepted"),
            Kind::AcceptedElsewhere
        );
        assert_eq!(
            super::map_verification_cancel_kind("m.future_code"),
            Kind::Other
        );
    }

    #[test]
    fn own_user_sas_api_returns_only_an_opaque_adapter_handle() {
        let _ = super::request_own_user_sas_verification;
        let _opaque: Option<super::MatrixOwnUserVerificationHandle> = None;
        assert!(!std::any::type_name::<super::MatrixOwnUserVerificationHandle>().contains('@'));
    }

    #[test]
    fn restricted_sync_filter_suppresses_rooms_and_presence_but_keeps_account_data() {
        let filter = restricted_verification_sync_filter();
        let json = serde_json::to_value(filter).expect("filter serializes");
        assert_eq!(json["presence"]["types"], serde_json::json!([]));
        assert_eq!(json["room"]["rooms"], serde_json::json!([]));
        assert!(json.get("account_data").is_none());
    }

    #[test]
    fn key_backup_state_maps_to_private_data_free_desktop_status() {
        assert_eq!(
            map_backup_state_to_desktop(BackupState::Unknown),
            KeyBackupStatus::Unknown
        );
        assert_eq!(
            map_backup_state_to_desktop(BackupState::Enabled),
            KeyBackupStatus::Enabled {
                version: "available".to_owned(),
            }
        );
        assert_eq!(
            map_backup_state_to_desktop(BackupState::Downloading),
            KeyBackupStatus::Restoring {
                request_id: 0,
                version: None,
                restored_rooms: 0,
                total_rooms: None,
            }
        );
    }

    #[test]
    fn e2ee_trust_error_debug_redacts_sdk_details() {
        let error = E2eeTrustError::Sdk("raw matrix sdk error with @alice:example.test".to_owned());
        let debug = format!("{error:?}");

        assert!(!debug.contains("@alice:example.test"));
        assert!(!debug.contains("raw matrix sdk error"));
        assert!(debug.contains("Sdk"));
    }

    #[test]
    fn key_backup_restore_summary_declares_joined_room_scope() {
        let summary = KeyBackupRestoreSummary {
            scope: KeyBackupRestoreScope::JoinedRooms,
            version: Some("available".to_owned()),
            restored_rooms: 2,
            total_rooms: Some(3),
        };

        let debug = format!("{summary:?}");
        assert!(debug.contains("JoinedRooms"));
        assert!(!debug.contains("BackupWide"));
        assert!(!debug.contains("AllRooms"));
    }

    #[test]
    fn device_session_summary_is_private_data_free() {
        let summary = MatrixDeviceSessionSummary {
            raw_device_id: "DEVICEID".to_owned(),
            display_name: Some("Alice private laptop".to_owned()),
            current: true,
            verified: false,
            inactive: false,
        };

        assert_eq!(summary.raw_device_id, "DEVICEID");
        let debug = format!("{summary:?}");
        assert!(!debug.contains("DEVICEID"), "{debug}");
        assert!(!debug.contains("Alice private laptop"), "{debug}");
        assert!(debug.contains("current"));
        assert!(debug.contains("verified"));
        assert!(debug.contains("inactive"));
    }

    #[test]
    fn room_key_file_transfer_summaries_are_private_data_free() {
        let export_summary = RoomKeyExportSummary {
            exported_sessions: None,
        };
        let import_summary = RoomKeyImportSummary {
            imported_count: 1,
            total_count: 1,
        };

        assert_eq!(export_summary.exported_sessions, None);
        assert_eq!(import_summary.imported_count, 1);
        assert_eq!(import_summary.total_count, 1);
        assert!(!format!("{export_summary:?}").contains("MEGOLM"));
        assert!(!format!("{import_summary:?}").contains("MEGOLM"));
    }

    #[test]
    fn secure_backup_setup_summary_is_private_data_free() {
        let summary = SecureBackupSetupSummary {
            recovery_key_written: true,
        };

        let debug = format!("{summary:?}");
        assert!(debug.contains("recovery_key_written"));
        assert!(!debug.contains("RecoveryKey("));
    }

    #[test]
    fn recovery_key_delivery_writes_native_artifact_without_debugging_material() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("recovery-artifact.txt");
        let artifact_payload = String::from("fixture-artifact-material");

        let written = write_recovery_key_if_requested(artifact_payload.clone(), Some(path.clone()))
            .expect("artifact write should succeed");

        assert!(written);
        assert_eq!(
            std::fs::read_to_string(path).expect("read artifact"),
            artifact_payload
        );
    }

    #[tokio::test]
    async fn room_key_import_accepts_element_compatible_key_export_envelope() {
        assert!(ELEMENT_COMPATIBLE_KEY_EXPORT.starts_with(MATRIX_KEY_EXPORT_HEADER));
        assert!(ELEMENT_COMPATIBLE_KEY_EXPORT.ends_with(MATRIX_KEY_EXPORT_FOOTER));

        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("element-compatible-room-keys.txt");
        std::fs::write(&path, ELEMENT_COMPATIBLE_KEY_EXPORT).expect("write fixture");
        let persistable = PersistableMatrixSession::from_json(
            r#"{"homeserver":"https://matrix.example.invalid","user_id":"@alice:example.invalid","device_id":"ALICEDEVICE","access_token":"synthetic-access"}"#,
        )
        .expect("synthetic session should deserialize");
        let session = restore_session(&persistable)
            .await
            .expect("synthetic session should restore");

        let summary = import_room_keys_from_file(&session, path, &AuthSecret::new("1234"))
            .await
            .expect("Matrix/Element key export envelope should import");

        assert_eq!(summary.total_count, 1);
    }

    #[test]
    fn e2ee_trust_public_async_api_is_exposed() {
        let _ = cross_signing_status;
        let _ = bootstrap_cross_signing;
        let _ = enable_key_backup;
        let _ = restore_key_backup;
        let _ = reset_identity;
        let _ = complete_identity_reset;
        let _ = request_device_verification;
        let _ = accept_verification_request;
        let _ = start_sas_verification;
        let _ = accept_sas_verification;
        let _ = confirm_sas_verification;
        let _ = mismatch_sas_verification;
        let _ = cancel_verification_request;
        let _ = cancel_sas_verification;
        let _ = observe_incoming_verification_requests;
        let _ = list_devices;
        let _ = rename_device;
        let _ = delete_devices;
        let _ = export_room_keys_to_file;
        let _ = import_room_keys_from_file;
        let _ = bootstrap_secure_backup;
        let _ = change_secure_backup_passphrase;
        let _: Option<MatrixIncomingVerificationRequest> = None;
        let _: Option<MatrixIncomingVerificationRequestObserver> = None;
    }

    #[test]
    fn sas_emojis_map_to_desktop_dto_without_sdk_types() {
        let emojis = [
            matrix_sdk::encryption::verification::Emoji {
                symbol: "🐶",
                description: "Dog",
            },
            matrix_sdk::encryption::verification::Emoji {
                symbol: "🐱",
                description: "Cat",
            },
            matrix_sdk::encryption::verification::Emoji {
                symbol: "🦁",
                description: "Lion",
            },
            matrix_sdk::encryption::verification::Emoji {
                symbol: "🐎",
                description: "Horse",
            },
            matrix_sdk::encryption::verification::Emoji {
                symbol: "🦄",
                description: "Unicorn",
            },
            matrix_sdk::encryption::verification::Emoji {
                symbol: "🐷",
                description: "Pig",
            },
            matrix_sdk::encryption::verification::Emoji {
                symbol: "🐘",
                description: "Elephant",
            },
        ];

        assert_eq!(
            map_sdk_sas_emojis_to_desktop(emojis),
            vec![
                SasEmoji {
                    symbol: "🐶".to_owned(),
                    description: "Dog".to_owned(),
                },
                SasEmoji {
                    symbol: "🐱".to_owned(),
                    description: "Cat".to_owned(),
                },
                SasEmoji {
                    symbol: "🦁".to_owned(),
                    description: "Lion".to_owned(),
                },
                SasEmoji {
                    symbol: "🐎".to_owned(),
                    description: "Horse".to_owned(),
                },
                SasEmoji {
                    symbol: "🦄".to_owned(),
                    description: "Unicorn".to_owned(),
                },
                SasEmoji {
                    symbol: "🐷".to_owned(),
                    description: "Pig".to_owned(),
                },
                SasEmoji {
                    symbol: "🐘".to_owned(),
                    description: "Elephant".to_owned(),
                },
            ]
        );
    }

    #[test]
    fn identity_reset_auth_type_maps_to_private_data_free_desktop_status() {
        assert_eq!(
            map_identity_reset_auth_type_to_desktop(MatrixIdentityResetAuthType::Uiaa),
            IdentityResetAuthType::Uiaa
        );
        assert_eq!(
            map_identity_reset_auth_type_to_desktop(MatrixIdentityResetAuthType::OAuth),
            IdentityResetAuthType::OAuth
        );
    }

    #[test]
    fn matrix_client_store_config_uses_the_required_key_for_sqlite_builder() {
        let source = include_str!("lib.rs");
        let impl_body = source
            .split("fn apply_to_builder(&self, builder: matrix_sdk::ClientBuilder)")
            .nth(1)
            .expect("apply_to_builder body");

        assert!(
            impl_body.contains(".key(Some(self.key.expose_key()))"),
            "apply_to_builder must pass the required MatrixClientStoreKey into sqlite_store"
        );

        let config = crate::MatrixClientStoreConfig::new(
            "/tmp/example-store",
            crate::MatrixClientStoreKey::new([7; 32]),
        );
        assert!(config.encrypted_at_rest_configured());
    }
}

#[derive(Clone)]
pub struct MatrixClientStoreConfig {
    path: PathBuf,
    cache_path: Option<PathBuf>,
    key: MatrixClientStoreKey,
    search_index_store: Option<MatrixSearchIndexStoreConfig>,
}

impl MatrixClientStoreConfig {
    pub fn new(path: impl Into<PathBuf>, key: MatrixClientStoreKey) -> Self {
        Self {
            path: path.into(),
            cache_path: None,
            key,
            search_index_store: None,
        }
    }

    pub fn with_cache_path(mut self, cache_path: impl Into<PathBuf>) -> Self {
        self.cache_path = Some(cache_path.into());
        self
    }

    pub fn with_search_index_store(
        mut self,
        search_index_store: MatrixSearchIndexStoreConfig,
    ) -> Self {
        self.search_index_store = Some(search_index_store);
        self
    }

    pub fn path(&self) -> &Path {
        self.path.as_path()
    }

    pub fn cache_path(&self) -> Option<&Path> {
        self.cache_path.as_deref()
    }

    /// The store is keyed by construction: `MatrixClientStoreConfig::new`
    /// requires a [`MatrixClientStoreKey`], and `apply_to_builder` always
    /// passes that key into the SQLite store config.
    pub fn encrypted_at_rest_configured(&self) -> bool {
        true
    }

    fn apply_to_builder(&self, builder: matrix_sdk::ClientBuilder) -> matrix_sdk::ClientBuilder {
        let sqlite_config =
            matrix_sdk::SqliteStoreConfig::new(&self.path).key(Some(self.key.expose_key()));
        let builder = builder
            .sqlite_store_with_config_and_cache_path(sqlite_config, self.cache_path.as_deref());
        match &self.search_index_store {
            Some(search_index_store) => {
                builder.search_index_store(search_index_store.as_sdk_store_kind())
            }
            None => builder,
        }
    }
}

impl fmt::Debug for MatrixClientStoreConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MatrixClientStoreConfig")
            .field("path", &self.path)
            .field("cache_path", &self.cache_path)
            .field("key", &self.key)
            .field("search_index_store", &self.search_index_store)
            .finish()
    }
}

#[derive(Clone, Eq, PartialEq)]
pub struct MatrixClientStoreKey {
    key: Zeroizing<[u8; 32]>,
}

impl MatrixClientStoreKey {
    pub fn new(key: [u8; 32]) -> Self {
        Self {
            key: Zeroizing::new(key),
        }
    }

    fn expose_key(&self) -> &[u8; 32] {
        &self.key
    }
}

impl fmt::Debug for MatrixClientStoreKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("MatrixClientStoreKey(..)")
    }
}

#[derive(Clone)]
pub struct MatrixSearchIndexStoreConfig {
    path: PathBuf,
    key: MatrixSearchIndexKey,
}

impl MatrixSearchIndexStoreConfig {
    pub fn new(path: impl Into<PathBuf>, key: MatrixSearchIndexKey) -> Self {
        Self {
            path: path.into(),
            key,
        }
    }

    pub fn path(&self) -> &Path {
        self.path.as_path()
    }

    fn as_sdk_store_kind(&self) -> matrix_sdk::search_index::SearchIndexStoreKind {
        matrix_sdk::search_index::SearchIndexStoreKind::encrypted_directory_ngram(
            self.path.clone(),
            self.key.expose_key().to_owned(),
            2,
            4,
        )
        .expect("desktop ngram search bounds should be valid")
    }
}

impl fmt::Debug for MatrixSearchIndexStoreConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MatrixSearchIndexStoreConfig")
            .field("path", &self.path)
            .field("key", &self.key)
            .finish()
    }
}

#[derive(Clone, Eq, PartialEq)]
pub struct MatrixSearchIndexKey {
    key: Zeroizing<String>,
}

impl MatrixSearchIndexKey {
    pub fn new(key: impl Into<String>) -> Self {
        Self {
            key: Zeroizing::new(key.into()),
        }
    }

    fn expose_key(&self) -> &str {
        self.key.as_str()
    }
}

impl fmt::Debug for MatrixSearchIndexKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("MatrixSearchIndexKey(..)")
    }
}

#[derive(Clone)]
pub struct MatrixClientSession {
    client: matrix_sdk::Client,
    pub info: SessionInfo,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MatrixTimelineContinuity {
    Unknown,
    Gapped,
    Complete,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MatrixTimelineGapError {
    InvalidRoom,
    RoomUnavailable,
    Sdk,
}

#[derive(Clone)]
pub struct MatrixTimelineGapHandle {
    room_id: matrix_sdk::ruma::OwnedRoomId,
    descriptor: matrix_sdk::event_cache::RoomTimelineGapDescriptor,
}

impl std::fmt::Debug for MatrixTimelineGapHandle {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("MatrixTimelineGapHandle")
            .finish_non_exhaustive()
    }
}

impl MatrixTimelineGapHandle {
    /// Coarse persisted-topology revision used by Core to detect an unchanged
    /// gap selection. The opaque descriptor, token, and boundary identities
    /// remain SDK-owned and actor-private.
    pub fn topology_revision(&self) -> u64 {
        self.descriptor.revision
    }

    pub fn older_boundary_event_id(&self) -> Option<&str> {
        self.descriptor
            .older_event_id
            .as_deref()
            .map(|event_id| event_id.as_str())
    }

    pub fn newer_boundary_event_id(&self) -> Option<&str> {
        self.descriptor
            .newer_event_id
            .as_deref()
            .map(|event_id| event_id.as_str())
    }
}

#[derive(Clone)]
pub struct MatrixTimelineGapInspection {
    pub continuity: MatrixTimelineContinuity,
    pub gaps: Vec<MatrixTimelineGapHandle>,
}

impl std::fmt::Debug for MatrixTimelineGapInspection {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("MatrixTimelineGapInspection")
            .field("continuity", &self.continuity)
            .field("gap_count", &self.gaps.len())
            .finish()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MatrixTimelineGapRepairBudget {
    pub event_limit: u16,
    pub cached_chunk_limit: u16,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MatrixTimelineGapRepairOutcome {
    Stale,
    Deferred { cached_chunks_loaded: usize },
    Failed,
    Progress { events: usize },
    BoundariesJoined { events: usize },
    StartReached { events: usize },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MatrixTimelineGapRepairResult {
    pub outcome: MatrixTimelineGapRepairOutcome,
    pub last_projection_batch: Option<u32>,
}

impl MatrixClientSession {
    #[cfg(feature = "test-hooks")]
    #[doc(hidden)]
    pub fn from_client_for_testing(client: matrix_sdk::Client, info: SessionInfo) -> Self {
        Self { client, info }
    }

    pub fn client(&self) -> matrix_sdk::Client {
        self.client.clone()
    }

    pub async fn inspect_room_timeline_gaps(
        &self,
        room_id: &str,
    ) -> Result<MatrixTimelineGapInspection, MatrixTimelineGapError> {
        use matrix_sdk::event_cache::RoomTimelineContinuity;

        let room_id = matrix_sdk::ruma::RoomId::parse(room_id)
            .map_err(|_| MatrixTimelineGapError::InvalidRoom)?;
        let room = self
            .client
            .get_room(&room_id)
            .ok_or(MatrixTimelineGapError::RoomUnavailable)?;
        let (cache, _drop_handles) = room
            .event_cache()
            .await
            .map_err(|_| MatrixTimelineGapError::Sdk)?;
        let inspection = cache
            .inspect_timeline_gaps()
            .await
            .map_err(|_| MatrixTimelineGapError::Sdk)?;
        let continuity = match inspection.continuity {
            RoomTimelineContinuity::Unknown => MatrixTimelineContinuity::Unknown,
            RoomTimelineContinuity::Gapped => MatrixTimelineContinuity::Gapped,
            RoomTimelineContinuity::Complete => MatrixTimelineContinuity::Complete,
        };
        let gaps = inspection
            .gaps
            .into_iter()
            .map(|descriptor| MatrixTimelineGapHandle {
                room_id: room_id.clone(),
                descriptor,
            })
            .collect();
        Ok(MatrixTimelineGapInspection { continuity, gaps })
    }

    pub async fn repair_room_timeline_gap(
        &self,
        gap: &MatrixTimelineGapHandle,
        budget: MatrixTimelineGapRepairBudget,
        actor_generation: u64,
        repair_generation: u64,
    ) -> Result<MatrixTimelineGapRepairResult, MatrixTimelineGapError> {
        use matrix_sdk::event_cache::{
            RoomTimelineGapProjectionId, RoomTimelineGapRepairBudget, RoomTimelineGapRepairOutcome,
        };

        let room = self
            .client
            .get_room(&gap.room_id)
            .ok_or(MatrixTimelineGapError::RoomUnavailable)?;
        let (cache, _drop_handles) = room
            .event_cache()
            .await
            .map_err(|_| MatrixTimelineGapError::Sdk)?;
        let result = cache
            .pagination()
            .repair_timeline_gap_with_projection(
                &gap.descriptor,
                RoomTimelineGapRepairBudget {
                    event_limit: budget.event_limit,
                    cached_chunk_limit: budget.cached_chunk_limit,
                },
                RoomTimelineGapProjectionId {
                    actor_generation,
                    repair_generation,
                },
            )
            .await
            .map_err(|_| MatrixTimelineGapError::Sdk)?;
        let outcome = match result.outcome {
            RoomTimelineGapRepairOutcome::Stale => MatrixTimelineGapRepairOutcome::Stale,
            RoomTimelineGapRepairOutcome::Deferred {
                cached_chunks_loaded,
            } => MatrixTimelineGapRepairOutcome::Deferred {
                cached_chunks_loaded,
            },
            RoomTimelineGapRepairOutcome::Failed => MatrixTimelineGapRepairOutcome::Failed,
            RoomTimelineGapRepairOutcome::Progress { events } => {
                MatrixTimelineGapRepairOutcome::Progress { events }
            }
            RoomTimelineGapRepairOutcome::BoundariesJoined { events } => {
                MatrixTimelineGapRepairOutcome::BoundariesJoined { events }
            }
            RoomTimelineGapRepairOutcome::StartReached { events } => {
                MatrixTimelineGapRepairOutcome::StartReached { events }
            }
        };
        Ok(MatrixTimelineGapRepairResult {
            outcome,
            last_projection_batch: result.last_projection_batch,
        })
    }

    pub fn persistable_session(&self) -> Result<PersistableMatrixSession, PasswordLoginError> {
        if let Some(oauth_session) = self.client.oauth().full_session() {
            return Ok(PersistableMatrixSession {
                info: self.info.clone(),
                session: PersistableSessionKind::OAuth {
                    user_session: oauth_session.user,
                    client_id: oauth_session.client_id,
                },
            });
        }

        let session = self
            .client
            .matrix_auth()
            .session()
            .ok_or(PasswordLoginError::MissingSession)?;
        Ok(PersistableMatrixSession {
            info: self.info.clone(),
            session: PersistableSessionKind::Matrix(session),
        })
    }

    pub fn e2ee_recovery_state(&self) -> E2eeRecoveryState {
        map_sdk_recovery_state(self.client().encryption().recovery().state())
    }

    pub fn e2ee_recovery_state_stream(&self) -> E2eeRecoveryStateStream {
        Box::pin(
            self.client()
                .encryption()
                .recovery()
                .state_stream()
                .map(map_sdk_recovery_state),
        )
    }

    pub fn current_device_trust(&self) -> CurrentDeviceTrustState {
        let subscriber = self.client().encryption().verification_state();
        map_sdk_verification_state(subscriber.get())
    }

    pub fn observe_current_device_trust(&self) -> CurrentDeviceTrustObservation {
        // Subscribe first, then read from the same subscriber so an update
        // cannot be lost between the current-value probe and stream creation.
        let subscriber = self.client().encryption().verification_state();
        let current = map_sdk_verification_state(subscriber.get());
        let updates = Box::pin(subscriber.map(map_sdk_verification_state));
        CurrentDeviceTrustObservation { current, updates }
    }
}

impl std::fmt::Debug for MatrixClientSession {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("MatrixClientSession")
            .field("info", &self.info)
            .field("client", &"MatrixClient(..)")
            .finish()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MatrixEventCacheStatus {
    AlreadyEnabled,
    Enabled,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
#[error("Matrix event cache subscription failed")]
pub enum MatrixEventCacheError {
    SubscribeFailed,
}

#[derive(Clone)]
pub struct PersistableMatrixSession {
    pub info: SessionInfo,
    session: PersistableSessionKind,
}

#[derive(Clone)]
enum PersistableSessionKind {
    Matrix(MatrixSession),
    OAuth {
        user_session: UserSession,
        client_id: ClientId,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PersistableAuthKind {
    Password,
    OAuth,
}

impl PersistableMatrixSession {
    pub fn to_json(&self) -> Result<String, PasswordLoginError> {
        match &self.session {
            PersistableSessionKind::Matrix(session) => {
                serde_json::to_string(&SerializedTaggedMatrixSession {
                    auth_kind: PersistableSessionJsonKind::Password,
                    homeserver: self.info.homeserver.clone(),
                    session: session.clone(),
                })
                .map_err(|error| PasswordLoginError::Serialization(error.to_string()))
            }
            PersistableSessionKind::OAuth {
                user_session,
                client_id,
            } => serde_json::to_string(&SerializedOauthPersistableMatrixSession {
                auth_kind: PersistableSessionJsonKind::OAuth,
                homeserver: self.info.homeserver.clone(),
                user_session: user_session.clone(),
                client_id: client_id.clone(),
            })
            .map_err(|error| PasswordLoginError::Serialization(error.to_string())),
        }
    }

    pub fn from_json(value: &str) -> Result<Self, PasswordLoginError> {
        let value_json = serde_json::from_str::<serde_json::Value>(value)
            .map_err(|error| PasswordLoginError::Serialization(error.to_string()))?;
        if value_json
            .get("auth_kind")
            .and_then(serde_json::Value::as_str)
            == Some("oauth")
        {
            let serialized =
                serde_json::from_value::<SerializedOauthPersistableMatrixSession>(value_json)
                    .map_err(|error| PasswordLoginError::Serialization(error.to_string()))?;
            let info = SessionInfo {
                homeserver: serialized.homeserver,
                user_id: serialized.user_session.meta.user_id.to_string(),
                device_id: serialized.user_session.meta.device_id.to_string(),
            };
            return Ok(Self {
                info,
                session: PersistableSessionKind::OAuth {
                    user_session: serialized.user_session,
                    client_id: serialized.client_id,
                },
            });
        }

        let serialized = serde_json::from_value::<SerializedPersistableMatrixSession>(value_json)
            .map_err(|error| PasswordLoginError::Serialization(error.to_string()))?;
        let session = serialized.session;
        let info = SessionInfo {
            homeserver: serialized.homeserver,
            user_id: session.meta.user_id.to_string(),
            device_id: session.meta.device_id.to_string(),
        };
        Ok(Self {
            info,
            session: PersistableSessionKind::Matrix(session),
        })
    }

    pub fn matrix_session(&self) -> Option<MatrixSession> {
        match &self.session {
            PersistableSessionKind::Matrix(session) => Some(session.clone()),
            PersistableSessionKind::OAuth { .. } => None,
        }
    }

    pub fn oauth_session(&self) -> Option<OAuthSession> {
        match &self.session {
            PersistableSessionKind::Matrix(_) => None,
            PersistableSessionKind::OAuth {
                user_session,
                client_id,
            } => Some(OAuthSession {
                user: user_session.clone(),
                client_id: client_id.clone(),
            }),
        }
    }

    pub fn auth_kind(&self) -> PersistableAuthKind {
        match &self.session {
            PersistableSessionKind::Matrix(_) => PersistableAuthKind::Password,
            PersistableSessionKind::OAuth { .. } => PersistableAuthKind::OAuth,
        }
    }
}

#[derive(Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
enum PersistableSessionJsonKind {
    Password,
    #[serde(rename = "oauth")]
    OAuth,
}

#[derive(Deserialize, Serialize)]
struct SerializedPersistableMatrixSession {
    homeserver: String,
    #[serde(flatten)]
    session: MatrixSession,
}

#[derive(Serialize)]
struct SerializedTaggedMatrixSession {
    auth_kind: PersistableSessionJsonKind,
    homeserver: String,
    #[serde(flatten)]
    session: MatrixSession,
}

#[derive(Deserialize, Serialize)]
struct SerializedOauthPersistableMatrixSession {
    auth_kind: PersistableSessionJsonKind,
    homeserver: String,
    user_session: UserSession,
    client_id: ClientId,
}

impl std::fmt::Debug for PersistableMatrixSession {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PersistableMatrixSession")
            .field("info", &self.info)
            .field("auth_kind", &self.auth_kind())
            .field("session", &"MatrixSession(..)")
            .finish()
    }
}

#[derive(Clone, Debug)]
pub struct Homeserver {
    base_url: Url,
}

impl Homeserver {
    pub fn parse(input: &str) -> Result<Self, LoginDiscoveryError> {
        let trimmed = input.trim();
        if trimmed.is_empty() {
            return Err(LoginDiscoveryError::InvalidHomeserver(
                "homeserver is empty".to_owned(),
            ));
        }

        let candidate = if trimmed.contains("://") {
            trimmed.to_owned()
        } else {
            format!("https://{trimmed}")
        };
        let mut base_url = Url::parse(&candidate)
            .map_err(|error| LoginDiscoveryError::InvalidHomeserver(error.to_string()))?;

        if !matches!(base_url.scheme(), "http" | "https") {
            return Err(LoginDiscoveryError::UnsupportedHomeserverScheme);
        }
        if base_url.host_str().is_none() {
            return Err(LoginDiscoveryError::InvalidHomeserver(
                "homeserver URL is missing a host".to_owned(),
            ));
        }
        if base_url.scheme() == "http" && !is_loopback_homeserver(&base_url) {
            return Err(LoginDiscoveryError::InsecureHomeserverScheme);
        }
        if base_url.query().is_some() || base_url.fragment().is_some() {
            return Err(LoginDiscoveryError::InvalidHomeserver(
                "homeserver URL must not include query or fragment".to_owned(),
            ));
        }

        if !base_url.path().ends_with('/') {
            let mut path = base_url.path().to_owned();
            path.push('/');
            base_url.set_path(&path);
        }

        Ok(Self { base_url })
    }

    pub fn normalized(&self) -> String {
        let mut normalized = self.base_url.to_string();
        if normalized.ends_with('/') {
            normalized.pop();
        }
        normalized
    }

    pub fn login_discovery_url(&self) -> Url {
        self.base_url
            .join(LOGIN_DISCOVERY_PATH)
            .expect("login discovery path should be relative")
    }
}

#[derive(Debug, Error, Eq, PartialEq)]
pub enum LoginDiscoveryError {
    #[error("homeserver URL is invalid: {0}")]
    InvalidHomeserver(String),
    #[error("homeserver URL scheme must be http or https")]
    UnsupportedHomeserverScheme,
    #[error("homeserver URL must use https unless it is localhost or loopback")]
    InsecureHomeserverScheme,
    #[error("login discovery request failed: {0}")]
    RequestFailed(String),
    #[error("login discovery failed with HTTP {status}: {message}")]
    HttpStatus { status: u16, message: String },
    #[error("login discovery response is missing flows")]
    MissingFlows,
    #[error("login discovery response is invalid: {0}")]
    InvalidResponse(String),
}

#[derive(Debug, Error)]
pub enum PasswordLoginError {
    #[error(transparent)]
    InvalidHomeserver(#[from] LoginDiscoveryError),
    #[error("password login runtime failed: {0}")]
    Runtime(String),
    #[error("password login failed: {0}")]
    Sdk(String),
    #[error("SDK session is not available")]
    MissingSession,
    #[error("session serialization failed: {0}")]
    Serialization(String),
}

#[derive(Debug, Error)]
pub enum E2eeRecoveryError {
    #[error("E2EE recovery runtime failed: {0}")]
    Runtime(String),
    #[error("E2EE recovery failed: {0}")]
    Sdk(String),
}

#[derive(Debug, Error)]
pub enum MatrixSyncError {
    #[error("Matrix sync failed")]
    Sdk,
}

#[derive(Debug, Error, Eq, PartialEq)]
pub enum MatrixRoomOperationError {
    #[error("Matrix room id is invalid")]
    InvalidRoomId,
    #[error("Matrix room alias is invalid")]
    InvalidRoomAlias,
    #[error("Matrix room setting is invalid")]
    InvalidRoomSetting,
    #[error("Matrix event id is invalid")]
    InvalidEventId,
    #[error("Matrix user id is invalid")]
    InvalidUserId,
    #[error("Matrix server name is invalid")]
    InvalidServerName,
    #[error("Matrix room is not available")]
    RoomUnavailable,
    #[error("Matrix room operation failed: {0}")]
    Sdk(MatrixRoomOperationFailureKind),
}

impl MatrixRoomOperationError {
    pub fn failure_kind(&self) -> Option<MatrixRoomOperationFailureKind> {
        match self {
            Self::Sdk(kind) => Some(*kind),
            Self::InvalidRoomId
            | Self::InvalidRoomAlias
            | Self::InvalidRoomSetting
            | Self::InvalidEventId
            | Self::InvalidUserId
            | Self::InvalidServerName
            | Self::RoomUnavailable => None,
        }
    }

    fn from_sdk_error(error: matrix_sdk::Error) -> Self {
        Self::Sdk(matrix_room_operation_failure_kind(&error))
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MatrixRoomOperationFailureKind {
    AuthenticationRequired,
    Encryption,
    Forbidden,
    Http,
    Store,
    WrongRoomState,
    Sdk,
}

impl fmt::Display for MatrixRoomOperationFailureKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let label = match self {
            Self::AuthenticationRequired => "authentication_required",
            Self::Encryption => "encryption",
            Self::Forbidden => "forbidden",
            Self::Http => "http",
            Self::Store => "store",
            Self::WrongRoomState => "wrong_room_state",
            Self::Sdk => "sdk",
        };
        formatter.write_str(label)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MatrixSearchCandidate {
    pub room_id: String,
    pub event_id: String,
    pub score_millis: u32,
}

#[derive(Debug, Error, Eq, PartialEq)]
pub enum MatrixSearchError {
    #[error("Matrix search index unavailable")]
    IndexUnavailable,
    #[error("Matrix search query failed")]
    Query,
    #[error("Matrix search internal failure")]
    Internal,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct MatrixRoomListSnapshot {
    pub spaces: Vec<MatrixRoomListSpace>,
    pub rooms: Vec<MatrixRoomListRoom>,
    pub invites: Vec<MatrixInvitePreview>,
    pub user_profiles: Vec<MatrixUserProfile>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct RoomListSmokeReport {
    pub rooms: usize,
    pub spaces: usize,
    pub dms: usize,
    pub unread_rooms: usize,
}

impl std::fmt::Display for RoomListSmokeReport {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "rooms={} spaces={} dms={} unread_rooms={}",
            self.rooms, self.spaces, self.dms, self.unread_rooms
        )
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct TimelineSmokeReport {
    pub selected_room_present: bool,
    pub timeline_items: usize,
}

impl std::fmt::Display for TimelineSmokeReport {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "selected_room_present={} timeline_items={}",
            self.selected_room_present, self.timeline_items
        )
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct SearchSmokeReport {
    pub invoked: bool,
    pub candidates: usize,
}

impl std::fmt::Display for SearchSmokeReport {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "search_invoked={} search_candidates={}",
            self.invoked, self.candidates
        )
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct RealAccountQaReport {
    pub room_list: RoomListSmokeReport,
    pub timeline: TimelineSmokeReport,
    pub session_restored: bool,
    pub search: SearchSmokeReport,
}

impl std::fmt::Display for RealAccountQaReport {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "{} {} session_restored={} {}",
            self.room_list, self.timeline, self.session_restored, self.search
        )
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MatrixRoomListSpace {
    pub space_id: String,
    pub display_name: String,
    pub avatar_mxc_uri: Option<String>,
    pub child_room_ids: Vec<String>,
    pub member_user_ids: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MatrixRoomListRoom {
    pub room_id: String,
    pub display_name: String,
    pub avatar_mxc_uri: Option<String>,
    pub is_dm: bool,
    pub dm_user_ids: Vec<String>,
    pub tags: MatrixRoomTags,
    pub unread_count: u64,
    pub notification_count: u64,
    pub highlight_count: u64,
    pub marked_unread: bool,
    pub recency_stamp: Option<u64>,
    pub conversation_activity: Option<MatrixConversationActivity>,
    pub latest_event: Option<MatrixRoomLatestEventSummary>,
    pub parent_space_ids: Vec<String>,
    pub is_encrypted: bool,
    pub joined_members: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MatrixConversationActivitySource {
    Message,
    EncryptedMessage,
    ThreadReply,
}

#[derive(Clone, Copy, Eq, PartialEq)]
pub struct MatrixConversationActivity {
    pub timestamp_ms: u64,
    pub source: MatrixConversationActivitySource,
}

impl fmt::Debug for MatrixConversationActivity {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MatrixConversationActivity")
            .field("source", &self.source)
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MatrixRoomLatestEventSummary {
    pub event_id: String,
    pub sender_id: Option<String>,
    pub sender_label: Option<String>,
    pub sender_avatar_mxc_uri: Option<String>,
    pub preview: Option<String>,
    pub timestamp_ms: u64,
    pub event_type: Option<String>,
    pub relation_type: Option<String>,
    pub relation_event_id: Option<String>,
    pub content_converted: bool,
    pub is_threaded: bool,
    pub is_reply: bool,
    pub has_thread_summary: bool,
    pub has_reactions: bool,
}

struct SdkUnreadTrace<'a> {
    unread_messages: u64,
    unread_count: u64,
    notification_count: u64,
    highlight_count: u64,
    marked_unread: bool,
    latest_event: &'a Option<MatrixRoomLatestEventSummary>,
    fully_read_event_id: Option<&'a str>,
    private_read_receipt_event_id: Option<&'a str>,
    recency_stamp_present: bool,
    conversation_activity: Option<MatrixConversationActivity>,
}

fn trace_sdk_unread_snapshot(trace: SdkUnreadTrace<'_>) {
    let latest_event = trace.latest_event.as_ref();
    record(
        DiagnosticEvent::new(DiagnosticLevel::Debug, "sdk.unread", "sdk_room_snapshot")
            .field(DiagnosticField::count(
                "unread_messages",
                trace.unread_messages,
            ))
            .field(DiagnosticField::count("unread_count", trace.unread_count))
            .field(DiagnosticField::count(
                "notification_count",
                trace.notification_count,
            ))
            .field(DiagnosticField::count(
                "highlight_count",
                trace.highlight_count,
            ))
            .field(DiagnosticField::boolean(
                "marked_unread",
                trace.marked_unread,
            ))
            .field(DiagnosticField::boolean(
                "latest_event_present",
                latest_event.is_some(),
            ))
            .field(DiagnosticField::boolean(
                "fully_read_present",
                trace.fully_read_event_id.is_some(),
            ))
            .field(DiagnosticField::boolean(
                "private_receipt_present",
                trace.private_read_receipt_event_id.is_some(),
            ))
            .field(DiagnosticField::boolean(
                "latest_event_content_converted",
                latest_event.is_some_and(|event| event.content_converted),
            ))
            .field(DiagnosticField::boolean(
                "latest_event_threaded",
                latest_event.is_some_and(|event| event.is_threaded),
            ))
            .field(DiagnosticField::boolean(
                "latest_event_reply",
                latest_event.is_some_and(|event| event.is_reply),
            ))
            .field(DiagnosticField::boolean(
                "latest_event_thread_summary",
                latest_event.is_some_and(|event| event.has_thread_summary),
            ))
            .field(DiagnosticField::boolean(
                "latest_event_reactions",
                latest_event.is_some_and(|event| event.has_reactions),
            ))
            .field(DiagnosticField::boolean(
                "recency_stamp_present",
                trace.recency_stamp_present,
            ))
            .field(DiagnosticField::boolean(
                "conversation_activity_present",
                trace.conversation_activity.is_some(),
            ))
            .field(DiagnosticField::token(
                "conversation_activity_source",
                conversation_activity_source_token(trace.conversation_activity),
            )),
    );
}

fn trace_sdk_conversation_activity(
    activity: Option<MatrixConversationActivity>,
    recency_stamp_present: bool,
) {
    record(
        DiagnosticEvent::new(
            DiagnosticLevel::Debug,
            "sdk.room_list",
            "conversation_activity_selected",
        )
        .field(DiagnosticField::boolean(
            "conversation_activity_present",
            activity.is_some(),
        ))
        .field(DiagnosticField::token(
            "conversation_activity_source",
            conversation_activity_source_token(activity),
        ))
        .field(DiagnosticField::token(
            "activity_sort_bucket",
            if activity.is_some() {
                "conversation"
            } else {
                "fallback"
            },
        ))
        .field(DiagnosticField::boolean(
            "recency_stamp_present",
            recency_stamp_present,
        )),
    );
}

fn conversation_activity_source_token(
    activity: Option<MatrixConversationActivity>,
) -> &'static str {
    match activity.map(|activity| activity.source) {
        Some(MatrixConversationActivitySource::Message) => "message",
        Some(MatrixConversationActivitySource::EncryptedMessage) => "encrypted_message",
        Some(MatrixConversationActivitySource::ThreadReply) => "thread_reply",
        None => "none",
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct MatrixRoomTags {
    pub favourite: Option<MatrixRoomTagInfo>,
    pub low_priority: Option<MatrixRoomTagInfo>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MatrixRoomTagInfo {
    pub order: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MatrixRoomTagKind {
    Favourite,
    LowPriority,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MatrixInvitePreview {
    pub room_id: String,
    pub display_name: String,
    pub avatar_mxc_uri: Option<String>,
    pub topic: Option<String>,
    pub inviter_display_name: Option<String>,
    pub inviter_user_id: Option<String>,
    pub is_dm: bool,
}

#[derive(Debug, Error, Eq, PartialEq)]
pub enum MatrixRoomListError {
    #[error("Matrix room list failed")]
    Sdk,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MatrixOwnProfile {
    pub display_name: Option<String>,
    pub avatar_mxc_uri: Option<String>,
}

#[derive(Clone, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct MatrixLocalUserAliases {
    #[serde(default)]
    pub aliases: BTreeMap<String, String>,
}

impl fmt::Debug for MatrixLocalUserAliases {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MatrixLocalUserAliases")
            .field("alias_count", &self.aliases.len())
            .finish()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MatrixUserProfile {
    pub user_id: String,
    pub display_name: Option<String>,
    pub avatar_mxc_uri: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MatrixPublicRoomDirectoryQuery {
    pub term: Option<String>,
    pub server_name: Option<String>,
    pub limit: Option<u32>,
    pub since: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MatrixPublicRoomDirectoryResult {
    pub rooms: Vec<MatrixPublicRoomDirectoryRoom>,
    pub next_batch: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MatrixPublicRoomDirectoryRoom {
    pub room_id: String,
    pub canonical_alias: Option<String>,
    pub name: String,
    pub topic: Option<String>,
    pub avatar_url: Option<String>,
    pub joined_members: u64,
    pub world_readable: bool,
    pub guest_can_join: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MatrixCreateRoomOptions {
    pub name: String,
    pub topic: Option<String>,
    pub alias_localpart: Option<String>,
    pub encrypted: bool,
    pub visibility: MatrixCreateRoomVisibility,
    pub parent_space: Option<MatrixCreateRoomParentSpace>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MatrixCreateRoomVisibility {
    Private,
    Public,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MatrixCreateRoomParentSpace {
    pub space_id: String,
    pub via_server: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MatrixRoomSettingsSnapshot {
    pub room_id: String,
    pub name: Option<String>,
    pub topic: Option<String>,
    pub avatar_url: Option<String>,
    pub canonical_alias: Option<String>,
    pub alternate_aliases: Vec<String>,
    pub join_rule: MatrixRoomJoinRule,
    pub history_visibility: MatrixRoomHistoryVisibility,
    pub permissions: MatrixRoomPermissionFacts,
    pub members: Vec<MatrixRoomMemberSummary>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MatrixRoomMemberSummary {
    pub user_id: String,
    pub display_name: Option<String>,
    pub avatar_url: Option<String>,
    pub power_level: Option<i64>,
    pub role: MatrixRoomMemberRole,
    pub user_trust: Option<MatrixUserTrustState>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MatrixRoomMemberRole {
    Creator,
    Administrator,
    Moderator,
    User,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MatrixUserTrustState {
    Unverified,
    Verified,
    IdentityReset,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MatrixRoomJoinRule {
    Public,
    Invite,
    Knock,
    Restricted,
    Private,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MatrixRoomHistoryVisibility {
    WorldReadable,
    Shared,
    Invited,
    Joined,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct MatrixRoomPermissionFacts {
    pub can_edit_settings: bool,
    pub can_edit_roles: bool,
    pub can_kick: bool,
    pub can_ban: bool,
    pub can_unban: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MatrixRoomSettingChange {
    Name(Option<String>),
    Topic(Option<String>),
    AvatarUrl(Option<String>),
    JoinRule(MatrixRoomJoinRule),
    HistoryVisibility(MatrixRoomHistoryVisibility),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MatrixRoomModerationAction {
    Kick,
    Ban,
    Unban,
}

#[derive(Debug, Error, Eq, PartialEq)]
pub enum MatrixProfileError {
    #[error("Matrix profile mime type is invalid")]
    InvalidMimeType,
    #[error("Matrix profile operation failed")]
    Sdk(MatrixProfileFailureKind),
}

impl MatrixProfileError {
    pub fn failure_kind(&self) -> MatrixProfileFailureKind {
        match self {
            Self::InvalidMimeType => MatrixProfileFailureKind::InvalidMimeType,
            Self::Sdk(kind) => *kind,
        }
    }

    fn from_sdk_error(error: matrix_sdk::Error) -> Self {
        Self::Sdk(matrix_profile_failure_kind(&error))
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MatrixProfileFailureKind {
    Forbidden,
    Network,
    InvalidMimeType,
    Sdk,
}

#[derive(Debug, Error, Eq, PartialEq)]
pub enum MatrixIgnoredUserListError {
    #[error("Matrix user id is invalid")]
    InvalidUserId,
    #[error("Matrix ignored user list operation failed")]
    Sdk(MatrixIgnoredUserListFailureKind),
}

impl MatrixIgnoredUserListError {
    pub fn failure_kind(&self) -> MatrixIgnoredUserListFailureKind {
        match self {
            Self::InvalidUserId => MatrixIgnoredUserListFailureKind::InvalidUserId,
            Self::Sdk(kind) => *kind,
        }
    }

    fn from_sdk_error(error: matrix_sdk::Error) -> Self {
        Self::Sdk(matrix_ignored_user_list_failure_kind(&error))
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MatrixIgnoredUserListFailureKind {
    Forbidden,
    Network,
    InvalidUserId,
    Sdk,
}

#[derive(Debug, Error, Eq, PartialEq)]
pub enum MatrixReportError {
    #[error("Matrix user id is invalid")]
    InvalidUserId,
    #[error("Matrix room id is invalid")]
    InvalidRoomId,
    #[error("Matrix event id is invalid")]
    InvalidEventId,
    #[error("Matrix report operation failed")]
    Sdk(MatrixReportFailureKind),
}

impl MatrixReportError {
    pub fn failure_kind(&self) -> MatrixReportFailureKind {
        match self {
            Self::InvalidUserId => MatrixReportFailureKind::InvalidUserId,
            Self::InvalidRoomId => MatrixReportFailureKind::InvalidRoomId,
            Self::InvalidEventId => MatrixReportFailureKind::InvalidEventId,
            Self::Sdk(kind) => *kind,
        }
    }

    fn from_sdk_error(error: matrix_sdk::Error) -> Self {
        Self::Sdk(matrix_report_failure_kind(&error))
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MatrixReportFailureKind {
    Forbidden,
    Network,
    InvalidUserId,
    InvalidRoomId,
    InvalidEventId,
    Sdk,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MatrixTimelineItem {
    pub room_id: String,
    pub event_id: String,
    pub sender: String,
    pub timestamp_ms: u64,
    pub body: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MatrixTimelineUpdate {
    Upsert(MatrixTimelineItem),
    Remove { room_id: String, event_id: String },
}

pub type MatrixTimelineUpdateStream = Pin<Box<dyn Stream<Item = Vec<MatrixTimelineUpdate>> + Send>>;

pub struct MatrixTimelineSubscription {
    room_id: String,
    timeline: Arc<matrix_sdk_ui::Timeline>,
    initial_items: Vec<MatrixTimelineItem>,
    updates: MatrixTimelineUpdateStream,
}

#[derive(Clone)]
pub struct MatrixTimelinePaginationHandle {
    timeline: Arc<matrix_sdk_ui::Timeline>,
}

impl MatrixTimelinePaginationHandle {
    pub async fn paginate_backwards(&self, event_count: u16) -> Result<bool, MatrixTimelineError> {
        self.timeline
            .paginate_backwards(event_count)
            .await
            .map_err(|_| MatrixTimelineError::Sdk)
    }
}

impl fmt::Debug for MatrixTimelinePaginationHandle {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MatrixTimelinePaginationHandle")
            .field("timeline", &"Timeline(..)")
            .finish()
    }
}

impl MatrixTimelineSubscription {
    pub fn initial_items(&self) -> &[MatrixTimelineItem] {
        &self.initial_items
    }

    pub async fn next_update(&mut self) -> Option<Vec<MatrixTimelineUpdate>> {
        self.updates.next().await
    }

    pub fn pagination_handle(&self) -> MatrixTimelinePaginationHandle {
        MatrixTimelinePaginationHandle {
            timeline: self.timeline.clone(),
        }
    }

    pub async fn current_items(&mut self) -> Vec<MatrixTimelineItem> {
        self.timeline
            .items()
            .await
            .iter()
            .filter_map(|item| matrix_timeline_item_from_ui(&self.room_id, item))
            .collect()
    }

    pub async fn paginate_backwards(&self, event_count: u16) -> Result<bool, MatrixTimelineError> {
        self.timeline
            .paginate_backwards(event_count)
            .await
            .map_err(|_| MatrixTimelineError::Sdk)
    }
}

impl fmt::Debug for MatrixTimelineSubscription {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MatrixTimelineSubscription")
            .field("initial_item_count", &self.initial_items.len())
            .field("timeline", &"Timeline(..)")
            .field("updates", &"TimelineUpdateStream(..)")
            .finish()
    }
}

#[derive(Debug, Error, Eq, PartialEq)]
pub enum MatrixTimelineError {
    #[error("Matrix room id is invalid")]
    InvalidRoomId,
    #[error("Matrix room is not available")]
    RoomUnavailable,
    #[error("Matrix timeline operation failed")]
    Sdk,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MatrixSyncLoopControl {
    Continue,
    Stop,
}

#[derive(Deserialize)]
struct LoginDiscoveryResponse {
    flows: Vec<RawLoginFlow>,
}

#[derive(Deserialize)]
struct RawLoginFlow {
    #[serde(rename = "type")]
    flow_type: String,
    #[serde(default, rename = "org.matrix.msc3824.delegated_oidc_compatibility")]
    delegated_oidc_compatibility: bool,
    #[serde(default)]
    display_name: Option<String>,
}

#[derive(Deserialize)]
struct MatrixErrorResponse {
    error: Option<String>,
}

pub fn discover_login_flows(homeserver: &str) -> Result<LoginDiscovery, LoginDiscoveryError> {
    let homeserver = Homeserver::parse(homeserver)?;
    let response = reqwest::blocking::Client::builder()
        .timeout(DISCOVERY_TIMEOUT)
        .user_agent("matrix-desktop-prelogin/0.1")
        .build()
        .map_err(|error| LoginDiscoveryError::RequestFailed(error.to_string()))?
        .get(homeserver.login_discovery_url())
        .send()
        .map_err(|error| LoginDiscoveryError::RequestFailed(error.to_string()))?;

    let status = response.status().as_u16();
    let body = response
        .text()
        .map_err(|error| LoginDiscoveryError::RequestFailed(error.to_string()))?;
    let flows = parse_login_discovery_http_response(status, &body)?;

    Ok(LoginDiscovery {
        homeserver: homeserver.normalized(),
        flows,
        delegated: DelegatedAuthLinks::default(),
    })
}

pub fn login_with_password_blocking(
    request: &LoginRequest,
) -> Result<MatrixClientSession, PasswordLoginError> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|error| PasswordLoginError::Runtime(error.to_string()))?;

    runtime.block_on(login_with_password(request))
}

pub fn logout_blocking(session: &MatrixClientSession) -> Result<(), PasswordLoginError> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|error| PasswordLoginError::Runtime(error.to_string()))?;

    runtime.block_on(logout(session))
}

pub fn sync_once_blocking(session: &MatrixClientSession) -> Result<(), MatrixSyncError> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|_| MatrixSyncError::Sdk)?;

    runtime.block_on(sync_once(session))
}

pub fn room_list_snapshot_blocking(
    session: &MatrixClientSession,
) -> Result<MatrixRoomListSnapshot, MatrixRoomListError> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|_| MatrixRoomListError::Sdk)?;

    runtime.block_on(room_list_snapshot(session))
}

pub fn subscribe_room_timeline_blocking(
    session: &MatrixClientSession,
    room_id: &str,
) -> Result<MatrixTimelineSubscription, MatrixTimelineError> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|_| MatrixTimelineError::Sdk)?;

    runtime.block_on(subscribe_room_timeline(session, room_id))
}

pub fn room_timeline_visible_items_blocking(
    session: &MatrixClientSession,
    room_id: &str,
    backfill_event_count: u16,
) -> Result<Vec<MatrixTimelineItem>, MatrixTimelineError> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|_| MatrixTimelineError::Sdk)?;

    runtime.block_on(room_timeline_visible_items(
        session,
        room_id,
        backfill_event_count,
    ))
}

pub fn search_message_candidates_blocking(
    session: &MatrixClientSession,
    query: &str,
    limit: usize,
) -> Result<Vec<MatrixSearchCandidate>, MatrixSearchError> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|_| MatrixSearchError::Internal)?;

    runtime.block_on(search_message_candidates(session, query, limit))
}

pub fn restore_session_blocking(
    session: &PersistableMatrixSession,
) -> Result<MatrixClientSession, PasswordLoginError> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|error| PasswordLoginError::Runtime(error.to_string()))?;

    runtime.block_on(restore_session(session))
}

pub fn recover_e2ee_blocking(
    session: &MatrixClientSession,
    request: &RecoveryRequest,
) -> Result<(), E2eeRecoveryError> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|error| E2eeRecoveryError::Runtime(error.to_string()))?;

    runtime.block_on(recover_e2ee(session, request))
}

pub async fn login_with_password(
    request: &LoginRequest,
) -> Result<MatrixClientSession, PasswordLoginError> {
    login_with_password_with_store(request, None).await
}

pub async fn login_with_password_with_store(
    request: &LoginRequest,
    store_config: Option<&MatrixClientStoreConfig>,
) -> Result<MatrixClientSession, PasswordLoginError> {
    let homeserver = Homeserver::parse(&request.homeserver)?;
    let client = build_client(&homeserver, store_config).await?;

    let mut login = client
        .matrix_auth()
        .login_username(&request.username, request.password.expose_secret());
    if let Some(device_display_name) = request.device_display_name.as_deref() {
        login = login.initial_device_display_name(device_display_name);
    }

    let response = login
        .send()
        .await
        .map_err(|error| PasswordLoginError::Sdk(error.to_string()))?;
    let user_id = response.user_id.to_string();
    let device_id = response.device_id.to_string();

    Ok(MatrixClientSession {
        client,
        info: SessionInfo {
            homeserver: homeserver.normalized(),
            user_id,
            device_id,
        },
    })
}

/// Re-authenticate an existing soft-logged-out session with the same device id.
/// The returned storeless session must be persisted and restored into the
/// existing per-account store by the caller so crypto/cached data is preserved.
pub async fn login_with_existing_device(
    homeserver: &str,
    user_id: &str,
    device_id: &str,
    password: &AuthSecret,
) -> Result<MatrixClientSession, PasswordLoginError> {
    let homeserver = Homeserver::parse(homeserver)?;
    let client = build_client(&homeserver, None).await?;

    let response = client
        .matrix_auth()
        .login_username(user_id, password.expose_secret())
        .device_id(device_id)
        .send()
        .await
        .map_err(|error| PasswordLoginError::Sdk(error.to_string()))?;

    Ok(MatrixClientSession {
        client,
        info: SessionInfo {
            homeserver: homeserver.normalized(),
            user_id: response.user_id.to_string(),
            device_id: response.device_id.to_string(),
        },
    })
}

pub async fn start_oidc_login(
    homeserver: &str,
    redirect_uri: &str,
) -> Result<(PendingOidcLogin, OidcAuthorization), PasswordLoginError> {
    let homeserver = Homeserver::parse(homeserver)?;
    let redirect_uri =
        Url::parse(redirect_uri).map_err(|error| PasswordLoginError::Sdk(error.to_string()))?;
    let client = build_client(&homeserver, None).await?;

    match client
        .oauth()
        .login(
            redirect_uri.clone(),
            None,
            Some(oidc_client_registration_data(redirect_uri.clone())),
            None,
        )
        .build()
        .await
    {
        Ok(authorization) => Ok((
            PendingOidcLogin::OAuth {
                client,
                homeserver: homeserver.normalized(),
            },
            OidcAuthorization {
                authorization_url: authorization.url.to_string(),
                state: authorization.state.secret().to_owned(),
            },
        )),
        Err(_) => {
            let authorization_url = client
                .matrix_auth()
                .get_sso_login_url(redirect_uri.as_str(), None)
                .await
                .map_err(|error| PasswordLoginError::Sdk(error.to_string()))?;
            Ok((
                PendingOidcLogin::Sso {
                    client,
                    homeserver: homeserver.normalized(),
                },
                OidcAuthorization {
                    authorization_url,
                    state: String::new(),
                },
            ))
        }
    }
}

pub async fn finish_oidc_login(
    pending: PendingOidcLogin,
    callback_url: &str,
) -> Result<MatrixClientSession, PasswordLoginError> {
    let callback_url =
        Url::parse(callback_url).map_err(|error| PasswordLoginError::Sdk(error.to_string()))?;
    let (client, homeserver) = match pending {
        PendingOidcLogin::OAuth { client, homeserver } => {
            client
                .oauth()
                .finish_login(callback_url.into())
                .await
                .map_err(|error| PasswordLoginError::Sdk(error.to_string()))?;
            (client, homeserver)
        }
        PendingOidcLogin::Sso { client, homeserver } => {
            client
                .matrix_auth()
                .login_with_sso_callback(UrlOrQuery::Url(callback_url))
                .map_err(|error| PasswordLoginError::Sdk(error.to_string()))?
                .initial_device_display_name("Koushi")
                .request_refresh_token()
                .send()
                .await
                .map_err(|error| PasswordLoginError::Sdk(error.to_string()))?;
            (client, homeserver)
        }
    };

    let user_id = client
        .user_id()
        .ok_or(PasswordLoginError::MissingSession)?
        .to_string();
    let device_id = client
        .device_id()
        .ok_or(PasswordLoginError::MissingSession)?
        .to_string();

    Ok(MatrixClientSession {
        client,
        info: SessionInfo {
            homeserver,
            user_id,
            device_id,
        },
    })
}

pub async fn restore_session(
    session: &PersistableMatrixSession,
) -> Result<MatrixClientSession, PasswordLoginError> {
    restore_session_with_store(session, None).await
}

fn oidc_client_registration_data(redirect_uri: Url) -> ClientRegistrationData {
    let client_uri = Localized::new(
        Url::parse("https://github.com/shinaoka/koushi-matrix")
            .expect("static client URI should parse"),
        [],
    );
    let metadata = ClientMetadata {
        client_name: Some(Localized::new("Koushi".to_owned(), [])),
        policy_uri: Some(client_uri.clone()),
        tos_uri: Some(client_uri.clone()),
        ..ClientMetadata::new(
            ApplicationType::Native,
            vec![OAuthGrantType::AuthorizationCode {
                redirect_uris: vec![redirect_uri],
            }],
            client_uri,
        )
    };

    ClientRegistrationData::new(Raw::new(&metadata).expect("OIDC client metadata should serialize"))
}

pub async fn restore_session_with_store(
    session: &PersistableMatrixSession,
    store_config: Option<&MatrixClientStoreConfig>,
) -> Result<MatrixClientSession, PasswordLoginError> {
    let homeserver = Homeserver::parse(&session.info.homeserver)?;
    let client = build_client(&homeserver, store_config).await?;

    if let Some(oauth_session) = session.oauth_session() {
        client
            .restore_session(oauth_session)
            .await
            .map_err(|error| PasswordLoginError::Sdk(error.to_string()))?;
    } else if let Some(matrix_session) = session.matrix_session() {
        client
            .restore_session(matrix_session)
            .await
            .map_err(|error| PasswordLoginError::Sdk(error.to_string()))?;
    } else {
        return Err(PasswordLoginError::MissingSession);
    }

    Ok(MatrixClientSession {
        client,
        info: session.info.clone(),
    })
}

pub async fn enable_event_cache(
    session: &MatrixClientSession,
) -> Result<MatrixEventCacheStatus, MatrixEventCacheError> {
    let client = session.client();
    let event_cache = client.event_cache();
    if event_cache.has_subscribed() {
        return Ok(MatrixEventCacheStatus::AlreadyEnabled);
    }

    event_cache
        .subscribe()
        .map_err(|_| MatrixEventCacheError::SubscribeFailed)?;
    Ok(MatrixEventCacheStatus::Enabled)
}

async fn build_client(
    homeserver: &Homeserver,
    store_config: Option<&MatrixClientStoreConfig>,
) -> Result<matrix_sdk::Client, PasswordLoginError> {
    let builder = desktop_client_builder_defaults(matrix_sdk::Client::builder())
        .homeserver_url(homeserver.normalized());
    let builder = match store_config {
        Some(store_config) => store_config.apply_to_builder(builder),
        None => builder,
    };
    builder
        .build()
        .await
        .map_err(|error| PasswordLoginError::Sdk(error.to_string()))
}

fn desktop_client_builder_defaults(
    builder: matrix_sdk::ClientBuilder,
) -> matrix_sdk::ClientBuilder {
    builder
        .handle_refresh_tokens()
        .with_room_key_recipient_strategy(desktop_room_key_recipient_strategy())
        .with_encryption_settings(EncryptionSettings {
            backup_download_strategy: BackupDownloadStrategy::AfterDecryptionFailure,
            ..Default::default()
        })
        .with_threading_support(matrix_sdk::ThreadingSupport::Enabled {
            with_subscriptions: true,
        })
}

fn desktop_room_key_recipient_strategy() -> CollectStrategy {
    CollectStrategy::AllDevices
}

pub async fn recover_e2ee(
    session: &MatrixClientSession,
    request: &RecoveryRequest,
) -> Result<(), E2eeRecoveryError> {
    session
        .client()
        .encryption()
        .recovery()
        .recover(request.secret.expose_secret())
        .await
        .map_err(|error| E2eeRecoveryError::Sdk(error.to_string()))
}

pub async fn logout(session: &MatrixClientSession) -> Result<(), PasswordLoginError> {
    let client = session.client();
    if client.oauth().full_session().is_some() {
        return client
            .oauth()
            .logout()
            .await
            .map_err(|error| PasswordLoginError::Sdk(error.to_string()));
    }

    client
        .logout()
        .await
        .map_err(|error| PasswordLoginError::Sdk(error.to_string()))
}

pub async fn get_own_profile(
    session: &MatrixClientSession,
) -> Result<MatrixOwnProfile, MatrixProfileError> {
    matrix_own_profile_from_session(session).await
}

pub async fn set_display_name(
    session: &MatrixClientSession,
    display_name: Option<&str>,
) -> Result<MatrixOwnProfile, MatrixProfileError> {
    session
        .client()
        .account()
        .set_display_name(display_name)
        .await
        .map_err(MatrixProfileError::from_sdk_error)?;
    matrix_own_profile_from_session(session).await
}

pub async fn set_avatar(
    session: &MatrixClientSession,
    mime_type: &str,
    bytes: Vec<u8>,
) -> Result<MatrixOwnProfile, MatrixProfileError> {
    let mime = mime_type
        .parse::<mime::Mime>()
        .map_err(|_| MatrixProfileError::InvalidMimeType)?;
    session
        .client()
        .account()
        .upload_avatar(&mime, bytes)
        .await
        .map_err(MatrixProfileError::from_sdk_error)?;
    matrix_own_profile_from_session(session).await
}

pub async fn get_local_user_aliases(
    session: &MatrixClientSession,
) -> Result<MatrixLocalUserAliases, MatrixProfileError> {
    let raw = fetch_local_user_aliases_raw(session).await?;
    let Some(raw) = raw else {
        return Ok(MatrixLocalUserAliases::default());
    };
    let content = raw
        .deserialize_as_unchecked::<MatrixLocalUserAliases>()
        .map_err(|_| matrix_profile_serialization_error())?;

    Ok(MatrixLocalUserAliases {
        aliases: normalized_local_user_aliases(content.aliases),
    })
}

pub async fn set_local_user_aliases(
    session: &MatrixClientSession,
    aliases: BTreeMap<String, String>,
) -> Result<MatrixLocalUserAliases, MatrixProfileError> {
    let content = MatrixLocalUserAliases {
        aliases: normalized_local_user_aliases(aliases),
    };
    let raw: Raw<AnyGlobalAccountDataEventContent> = Raw::new(&content)
        .map_err(|_| matrix_profile_serialization_error())?
        .cast_unchecked();
    session
        .client()
        .account()
        .set_account_data_raw(local_user_aliases_event_type(), raw)
        .await
        .map_err(MatrixProfileError::from_sdk_error)?;

    Ok(content)
}

pub async fn update_local_user_alias(
    session: &MatrixClientSession,
    user_id: &str,
    alias: Option<&str>,
) -> Result<MatrixLocalUserAliases, MatrixProfileError> {
    let mut aliases = get_local_user_aliases(session).await?.aliases;
    if let Some(alias) = normalize_local_user_alias(alias) {
        aliases.insert(user_id.to_owned(), alias);
    } else {
        aliases.remove(user_id);
    }
    set_local_user_aliases(session, aliases).await
}

pub async fn get_ignored_user_list(
    session: &MatrixClientSession,
) -> Result<BTreeSet<String>, MatrixIgnoredUserListError> {
    let account = session.client().account();
    let raw = account
        .account_data::<matrix_sdk::ruma::events::ignored_user_list::IgnoredUserListEventContent>()
        .await
        .map_err(MatrixIgnoredUserListError::from_sdk_error)?;
    let Some(raw) = raw else {
        return Ok(BTreeSet::new());
    };
    let content = raw
        .deserialize()
        .map_err(|_| MatrixIgnoredUserListError::Sdk(MatrixIgnoredUserListFailureKind::Sdk))?;

    Ok(content
        .ignored_users
        .into_keys()
        .map(|user_id| user_id.to_string())
        .collect())
}

pub async fn ignore_user(
    session: &MatrixClientSession,
    user_id: &str,
) -> Result<BTreeSet<String>, MatrixIgnoredUserListError> {
    let user_id = matrix_sdk::ruma::UserId::parse(user_id)
        .map_err(|_| MatrixIgnoredUserListError::InvalidUserId)?;
    session
        .client()
        .account()
        .ignore_user(&user_id)
        .await
        .map_err(MatrixIgnoredUserListError::from_sdk_error)?;
    get_ignored_user_list(session).await
}

pub async fn unignore_user(
    session: &MatrixClientSession,
    user_id: &str,
) -> Result<BTreeSet<String>, MatrixIgnoredUserListError> {
    let user_id = matrix_sdk::ruma::UserId::parse(user_id)
        .map_err(|_| MatrixIgnoredUserListError::InvalidUserId)?;
    session
        .client()
        .account()
        .unignore_user(&user_id)
        .await
        .map_err(MatrixIgnoredUserListError::from_sdk_error)?;
    get_ignored_user_list(session).await
}

pub async fn report_content(
    session: &MatrixClientSession,
    room_id: &str,
    event_id: &str,
    reason: Option<String>,
) -> Result<(), MatrixReportError> {
    let room = matrix_room(session, room_id).map_err(|error| match error {
        MatrixRoomOperationError::InvalidRoomId => MatrixReportError::InvalidRoomId,
        _ => MatrixReportError::Sdk(MatrixReportFailureKind::Sdk),
    })?;
    let event_id = matrix_sdk::ruma::EventId::parse(event_id)
        .map_err(|_| MatrixReportError::InvalidEventId)?;
    room.report_content(event_id, reason)
        .await
        .map_err(MatrixReportError::from_sdk_error)?;
    Ok(())
}

pub async fn report_room(
    session: &MatrixClientSession,
    room_id: &str,
    reason: String,
) -> Result<(), MatrixReportError> {
    let room = matrix_room(session, room_id).map_err(|error| match error {
        MatrixRoomOperationError::InvalidRoomId => MatrixReportError::InvalidRoomId,
        _ => MatrixReportError::Sdk(MatrixReportFailureKind::Sdk),
    })?;
    room.report_room(reason)
        .await
        .map_err(MatrixReportError::from_sdk_error)?;
    Ok(())
}

pub async fn report_user(
    session: &MatrixClientSession,
    user_id: &str,
    reason: String,
) -> Result<(), MatrixReportError> {
    let user_id =
        matrix_sdk::ruma::UserId::parse(user_id).map_err(|_| MatrixReportError::InvalidUserId)?;
    let request =
        matrix_sdk::ruma::api::client::reporting::report_user::v3::Request::new(user_id, reason);
    session.client().send(request).await.map_err(|error| {
        MatrixReportError::from_sdk_error(matrix_sdk::Error::Http(Box::new(error)))
    })?;
    Ok(())
}

pub async fn send_text_message(
    session: &MatrixClientSession,
    room_id: &str,
    body: &str,
    transaction_id: &str,
) -> Result<(), MatrixRoomOperationError> {
    let room = matrix_room(session, room_id)?;
    let txn_id = matrix_sdk::ruma::OwnedTransactionId::from(transaction_id);
    let content =
        matrix_sdk::ruma::events::room::message::RoomMessageEventContent::text_plain(body);

    let result = room
        .send(content)
        .with_transaction_id(txn_id)
        .await
        .map(|_| ());
    map_room_send_result(result)
}

fn map_room_send_result(
    result: Result<(), matrix_sdk::Error>,
) -> Result<(), MatrixRoomOperationError> {
    result.map_err(MatrixRoomOperationError::from_sdk_error)
}

pub async fn room_can_send_text_message(
    session: &MatrixClientSession,
    room_id: &str,
) -> Result<bool, MatrixRoomOperationError> {
    let room = matrix_room(session, room_id)?;
    if room.state() != matrix_sdk::RoomState::Joined {
        return Ok(false);
    }

    let power_levels = room.power_levels_or_default().await;
    Ok(power_levels.user_can_send_message(
        room.own_user_id(),
        matrix_sdk::ruma::events::MessageLikeEventType::RoomMessage,
    ))
}

pub async fn get_room_settings_snapshot(
    session: &MatrixClientSession,
    room_id: &str,
) -> Result<MatrixRoomSettingsSnapshot, MatrixRoomOperationError> {
    let room = matrix_room(session, room_id)?;
    Ok(matrix_room_settings_snapshot(&room).await)
}

pub async fn reshare_room_key(
    session: &MatrixClientSession,
    room_id: &str,
) -> Result<(), MatrixRoomOperationError> {
    let room = matrix_room(session, room_id)?;
    room.reshare_room_key()
        .await
        .map_err(MatrixRoomOperationError::from_sdk_error)
}

pub async fn request_room_key_for_event(
    session: &MatrixClientSession,
    room_id: &str,
    event: &Raw<AnySyncTimelineEvent>,
) -> Result<(), MatrixRoomOperationError> {
    let room_id = matrix_sdk::ruma::RoomId::parse(room_id)
        .map_err(|_| MatrixRoomOperationError::InvalidRoomId)?;
    session
        .client()
        .request_room_key_for_event(event, room_id.as_ref())
        .await
        .map_err(MatrixRoomOperationError::from_sdk_error)
}

pub async fn update_room_setting(
    session: &MatrixClientSession,
    room_id: &str,
    change: MatrixRoomSettingChange,
) -> Result<MatrixRoomSettingsSnapshot, MatrixRoomOperationError> {
    let room = matrix_room(session, room_id)?;
    let snapshot = matrix_room_settings_snapshot(&room).await;
    match &change {
        MatrixRoomSettingChange::Name(name) => {
            room.set_name(name.clone().unwrap_or_default())
                .await
                .map_err(MatrixRoomOperationError::from_sdk_error)?;
        }
        MatrixRoomSettingChange::Topic(topic) => {
            room.set_room_topic(topic.as_deref().unwrap_or_default())
                .await
                .map_err(MatrixRoomOperationError::from_sdk_error)?;
        }
        MatrixRoomSettingChange::AvatarUrl(Some(avatar_url)) => {
            let avatar_url = matrix_sdk::ruma::OwnedMxcUri::from(avatar_url.clone());
            room.set_avatar_url(avatar_url.as_ref(), None)
                .await
                .map_err(MatrixRoomOperationError::from_sdk_error)?;
        }
        MatrixRoomSettingChange::AvatarUrl(None) => {
            room.remove_avatar()
                .await
                .map_err(MatrixRoomOperationError::from_sdk_error)?;
        }
        MatrixRoomSettingChange::JoinRule(join_rule) => {
            let join_rule = sdk_join_rule_for_update(*join_rule)?;
            room.privacy_settings()
                .update_join_rule(join_rule)
                .await
                .map_err(MatrixRoomOperationError::from_sdk_error)?;
        }
        MatrixRoomSettingChange::HistoryVisibility(history_visibility) => {
            room.privacy_settings()
                .update_room_history_visibility(sdk_history_visibility(*history_visibility))
                .await
                .map_err(MatrixRoomOperationError::from_sdk_error)?;
        }
    }

    Ok(room_settings_snapshot_with_change(snapshot, &change))
}

pub async fn moderate_room_member(
    session: &MatrixClientSession,
    room_id: &str,
    target_user_id: &str,
    action: MatrixRoomModerationAction,
    reason: Option<&str>,
) -> Result<(), MatrixRoomOperationError> {
    let room = matrix_room(session, room_id)?;
    let target_user_id = matrix_sdk::ruma::UserId::parse(target_user_id)
        .map_err(|_| MatrixRoomOperationError::InvalidUserId)?;

    match action {
        MatrixRoomModerationAction::Kick => room.kick_user(&target_user_id, reason).await,
        MatrixRoomModerationAction::Ban => room.ban_user(&target_user_id, reason).await,
        MatrixRoomModerationAction::Unban => room.unban_user(&target_user_id, reason).await,
    }
    .map_err(MatrixRoomOperationError::from_sdk_error)
}

pub async fn update_room_member_power_level(
    session: &MatrixClientSession,
    room_id: &str,
    target_user_id: &str,
    power_level: i64,
) -> Result<MatrixRoomSettingsSnapshot, MatrixRoomOperationError> {
    let room = matrix_room(session, room_id)?;
    let target_user_id = matrix_sdk::ruma::UserId::parse(target_user_id)
        .map_err(|_| MatrixRoomOperationError::InvalidUserId)?;
    let power_level = matrix_sdk::ruma::Int::try_from(power_level)
        .map_err(|_| MatrixRoomOperationError::InvalidRoomSetting)?;

    room.update_power_levels(vec![(target_user_id.as_ref(), power_level)])
        .await
        .map_err(MatrixRoomOperationError::from_sdk_error)?;

    let target_user_id_ref: &matrix_sdk::ruma::UserId = target_user_id.as_ref();
    Ok(room_settings_snapshot_with_member_power_level(
        matrix_room_settings_snapshot(&room).await,
        target_user_id_ref.as_str(),
        power_level.into(),
    ))
}

pub async fn create_room(
    session: &MatrixClientSession,
    options: MatrixCreateRoomOptions,
) -> Result<String, MatrixRoomOperationError> {
    let request = create_room_request(options)?;
    let room = session
        .client()
        .create_room(request)
        .await
        .map_err(MatrixRoomOperationError::from_sdk_error)?;
    Ok(room.room_id().to_string())
}

pub async fn create_public_directory_room(
    session: &MatrixClientSession,
    name: &str,
    alias_localpart: &str,
) -> Result<String, MatrixRoomOperationError> {
    create_room(
        session,
        MatrixCreateRoomOptions {
            name: name.to_owned(),
            topic: None,
            alias_localpart: Some(alias_localpart.to_owned()),
            encrypted: false,
            visibility: MatrixCreateRoomVisibility::Public,
            parent_space: None,
        },
    )
    .await
}

fn create_room_request(
    options: MatrixCreateRoomOptions,
) -> Result<matrix_sdk::ruma::api::client::room::create_room::v3::Request, MatrixRoomOperationError>
{
    let mut request = matrix_sdk::ruma::api::client::room::create_room::v3::Request::new();
    request.name = non_empty_name(&options.name);
    request.topic = options
        .topic
        .as_deref()
        .map(str::trim)
        .filter(|topic| !topic.is_empty())
        .map(ToOwned::to_owned);

    let is_public = matches!(options.visibility, MatrixCreateRoomVisibility::Public);
    if is_public {
        let alias_localpart = options
            .alias_localpart
            .as_deref()
            .map(str::trim)
            .filter(|alias| !alias.is_empty())
            .ok_or(MatrixRoomOperationError::InvalidRoomAlias)?;
        validate_alias_localpart(alias_localpart)?;
        request.room_alias_name = Some(alias_localpart.to_owned());
        request.visibility = matrix_sdk::ruma::api::client::room::Visibility::Public;
        request.preset =
            Some(matrix_sdk::ruma::api::client::room::create_room::v3::RoomPreset::PublicChat);
    }

    if options.encrypted && !is_public {
        request.initial_state.push(
            matrix_sdk::ruma::events::InitialStateEvent::with_empty_state_key(
                matrix_sdk::ruma::events::room::encryption::RoomEncryptionEventContent::with_recommended_defaults(),
            )
            .to_raw_any(),
        );
    }

    if let Some(parent_space) = options.parent_space {
        let parent_space_id = matrix_sdk::ruma::OwnedRoomId::try_from(parent_space.space_id)
            .map_err(|_| MatrixRoomOperationError::InvalidRoomId)?;
        let via_server = matrix_sdk::ruma::OwnedServerName::try_from(parent_space.via_server)
            .map_err(|_| MatrixRoomOperationError::InvalidServerName)?;
        let mut parent_content =
            matrix_sdk::ruma::events::space::parent::SpaceParentEventContent::new(vec![via_server]);
        parent_content.canonical = true;
        request.initial_state.push(
            matrix_sdk::ruma::events::InitialStateEvent::new(
                parent_space_id.clone(),
                parent_content,
            )
            .to_raw_any(),
        );

        if !is_public {
            request.room_version = Some(matrix_sdk::ruma::RoomVersionId::V9);
            request.initial_state.push(
                matrix_sdk::ruma::events::InitialStateEvent::with_empty_state_key(
                    matrix_sdk::ruma::events::room::join_rules::RoomJoinRulesEventContent::restricted(
                        vec![
                            matrix_sdk::ruma::events::room::join_rules::AllowRule::room_membership(
                                parent_space_id,
                            ),
                        ],
                    ),
                )
                .to_raw_any(),
            );
            request.initial_state.push(
                matrix_sdk::ruma::events::InitialStateEvent::with_empty_state_key(
                    matrix_sdk::ruma::events::room::history_visibility::RoomHistoryVisibilityEventContent::new(
                        matrix_sdk::ruma::events::room::history_visibility::HistoryVisibility::Invited,
                    ),
                )
                .to_raw_any(),
            );
        }
    }

    Ok(request)
}

fn validate_alias_localpart(alias_localpart: &str) -> Result<(), MatrixRoomOperationError> {
    if alias_localpart.starts_with('#') || alias_localpart.contains(':') {
        return Err(MatrixRoomOperationError::InvalidRoomAlias);
    }
    Ok(())
}

pub async fn create_space(
    session: &MatrixClientSession,
    name: &str,
) -> Result<String, MatrixRoomOperationError> {
    let mut creation_content =
        matrix_sdk::ruma::api::client::room::create_room::v3::CreationContent::new();
    creation_content.room_type = Some(matrix_sdk::ruma::room::RoomType::Space);

    let mut request = matrix_sdk::ruma::api::client::room::create_room::v3::Request::new();
    request.name = non_empty_name(name);
    request.creation_content = Some(
        matrix_sdk::ruma::serde::Raw::new(&creation_content)
            .map_err(|_| MatrixRoomOperationError::Sdk(MatrixRoomOperationFailureKind::Sdk))?,
    );

    let room = session
        .client()
        .create_room(request)
        .await
        .map_err(MatrixRoomOperationError::from_sdk_error)?;
    Ok(room.room_id().to_string())
}

pub async fn invite_user_to_room(
    session: &MatrixClientSession,
    room_id: &str,
    user_id: &str,
) -> Result<(), MatrixRoomOperationError> {
    let room = matrix_room(session, room_id)?;
    let user_id = matrix_sdk::ruma::UserId::parse(user_id)
        .map_err(|_| MatrixRoomOperationError::InvalidUserId)?;
    room.invite_user_by_id(&user_id)
        .await
        .map_err(MatrixRoomOperationError::from_sdk_error)
}

pub async fn room_has_active_member_no_sync(
    session: &MatrixClientSession,
    room_id: &str,
    user_id: &str,
) -> Result<bool, MatrixRoomOperationError> {
    let room = matrix_room(session, room_id)?;
    let user_id = matrix_sdk::ruma::UserId::parse(user_id)
        .map_err(|_| MatrixRoomOperationError::InvalidUserId)?;
    let members = room
        .members_no_sync(matrix_sdk::RoomMemberships::ACTIVE)
        .await
        .map_err(MatrixRoomOperationError::from_sdk_error)?;
    Ok(members
        .iter()
        .any(|member| member.user_id().as_str() == user_id.as_str()))
}

pub async fn start_direct_message(
    session: &MatrixClientSession,
    user_id: &str,
) -> Result<String, MatrixRoomOperationError> {
    let user_id = matrix_sdk::ruma::UserId::parse(user_id)
        .map_err(|_| MatrixRoomOperationError::InvalidUserId)?;
    let room = session
        .client()
        .create_dm(&user_id)
        .await
        .map_err(MatrixRoomOperationError::from_sdk_error)?;
    Ok(room.room_id().to_string())
}

pub async fn join_room_by_id(
    session: &MatrixClientSession,
    room_id: &str,
) -> Result<String, MatrixRoomOperationError> {
    let room_id = matrix_sdk::ruma::RoomId::parse(room_id)
        .map_err(|_| MatrixRoomOperationError::InvalidRoomId)?;
    let room = session
        .client()
        .join_room_by_id(&room_id)
        .await
        .map_err(MatrixRoomOperationError::from_sdk_error)?;
    Ok(room.room_id().to_string())
}

pub async fn query_public_room_directory(
    session: &MatrixClientSession,
    query: MatrixPublicRoomDirectoryQuery,
) -> Result<MatrixPublicRoomDirectoryResult, MatrixRoomOperationError> {
    let mut filter = matrix_sdk::ruma::directory::Filter::new();
    filter.generic_search_term = query.term;

    let mut request =
        matrix_sdk::ruma::api::client::directory::get_public_rooms_filtered::v3::Request::new();
    request.filter = filter;
    request.limit = query.limit.map(Into::into);
    request.since = query.since;
    request.server = query
        .server_name
        .map(matrix_sdk::ruma::OwnedServerName::try_from)
        .transpose()
        .map_err(|_| MatrixRoomOperationError::InvalidServerName)?;

    let response = session
        .client()
        .public_rooms_filtered(request)
        .await
        .map_err(|error| MatrixRoomOperationError::from_sdk_error(error.into()))?;

    Ok(MatrixPublicRoomDirectoryResult {
        rooms: response
            .chunk
            .into_iter()
            .map(matrix_public_room_from_chunk)
            .collect(),
        next_batch: response.next_batch,
    })
}

pub async fn join_room_by_alias(
    session: &MatrixClientSession,
    alias: &str,
    via_server: Option<&str>,
) -> Result<String, MatrixRoomOperationError> {
    let alias = matrix_sdk::ruma::RoomAliasId::parse(alias)
        .map_err(|_| MatrixRoomOperationError::InvalidRoomAlias)?;
    let room_or_alias = matrix_sdk::ruma::OwnedRoomOrAliasId::from(alias);
    let via = via_server
        .map(matrix_sdk::ruma::OwnedServerName::try_from)
        .transpose()
        .map_err(|_| MatrixRoomOperationError::InvalidServerName)?
        .into_iter()
        .collect::<Vec<_>>();
    let room = session
        .client()
        .join_room_by_id_or_alias(room_or_alias.as_ref(), &via)
        .await
        .map_err(MatrixRoomOperationError::from_sdk_error)?;
    Ok(room.room_id().to_string())
}

pub async fn leave_room(
    session: &MatrixClientSession,
    room_id: &str,
) -> Result<String, MatrixRoomOperationError> {
    let room = matrix_room(session, room_id)?;
    let room_id = room.room_id().to_string();
    room.leave()
        .await
        .map_err(MatrixRoomOperationError::from_sdk_error)?;
    Ok(room_id)
}

pub async fn forget_room(
    session: &MatrixClientSession,
    room_id: &str,
) -> Result<String, MatrixRoomOperationError> {
    let room = matrix_room(session, room_id)?;
    let room_id = room.room_id().to_string();
    room.forget()
        .await
        .map_err(MatrixRoomOperationError::from_sdk_error)?;
    Ok(room_id)
}

pub async fn set_space_child(
    session: &MatrixClientSession,
    space_id: &str,
    child_room_id: &str,
    via_server: &str,
) -> Result<(), MatrixRoomOperationError> {
    let space = matrix_room(session, space_id)?;
    let child_room_id = matrix_sdk::ruma::OwnedRoomId::try_from(child_room_id)
        .map_err(|_| MatrixRoomOperationError::InvalidRoomId)?;
    let via_server = matrix_sdk::ruma::OwnedServerName::try_from(via_server)
        .map_err(|_| MatrixRoomOperationError::InvalidServerName)?;
    let content =
        matrix_sdk::ruma::events::space::child::SpaceChildEventContent::new(vec![via_server]);

    space
        .send_state_event_for_key(&child_room_id, content)
        .await
        .map(|_| ())
        .map_err(MatrixRoomOperationError::from_sdk_error)
}

pub fn room_id_server_name(room_id: &str) -> Result<String, MatrixRoomOperationError> {
    let room_id = matrix_sdk::ruma::RoomId::parse(room_id)
        .map_err(|_| MatrixRoomOperationError::InvalidRoomId)?;
    room_id
        .server_name()
        .map(ToString::to_string)
        .ok_or(MatrixRoomOperationError::InvalidRoomId)
}

pub async fn set_room_tag(
    session: &MatrixClientSession,
    room_id: &str,
    tag: MatrixRoomTagKind,
    order: Option<f64>,
) -> Result<(), MatrixRoomOperationError> {
    let room = matrix_room(session, room_id)?;
    match tag {
        MatrixRoomTagKind::Favourite => room.set_is_favourite(true, order).await,
        MatrixRoomTagKind::LowPriority => room.set_is_low_priority(true, order).await,
    }
    .map_err(MatrixRoomOperationError::from_sdk_error)
}

pub async fn remove_room_tag(
    session: &MatrixClientSession,
    room_id: &str,
    tag: MatrixRoomTagKind,
) -> Result<(), MatrixRoomOperationError> {
    let room = matrix_room(session, room_id)?;
    match tag {
        MatrixRoomTagKind::Favourite => room.set_is_favourite(false, None).await,
        MatrixRoomTagKind::LowPriority => room.set_is_low_priority(false, None).await,
    }
    .map_err(MatrixRoomOperationError::from_sdk_error)
}

pub async fn pin_event(
    session: &MatrixClientSession,
    room_id: &str,
    event_id: &str,
) -> Result<(), MatrixRoomOperationError> {
    let room = matrix_room(session, room_id)?;
    let event_id = matrix_sdk::ruma::EventId::parse(event_id)
        .map_err(|_| MatrixRoomOperationError::InvalidEventId)?;
    room.pin_event(&event_id)
        .await
        .map(|_| ())
        .map_err(MatrixRoomOperationError::from_sdk_error)
}

pub async fn unpin_event(
    session: &MatrixClientSession,
    room_id: &str,
    event_id: &str,
) -> Result<(), MatrixRoomOperationError> {
    let room = matrix_room(session, room_id)?;
    let event_id = matrix_sdk::ruma::EventId::parse(event_id)
        .map_err(|_| MatrixRoomOperationError::InvalidEventId)?;
    room.unpin_event(&event_id)
        .await
        .map(|_| ())
        .map_err(MatrixRoomOperationError::from_sdk_error)
}

pub async fn mark_room_as_read(
    session: &MatrixClientSession,
    room_id: &str,
    event_id: &str,
) -> Result<(), MatrixRoomOperationError> {
    let room = matrix_room(session, room_id)?;
    let event_id = matrix_sdk::ruma::EventId::parse(event_id)
        .map_err(|_| MatrixRoomOperationError::InvalidEventId)?;
    let receipts = matrix_sdk::room::Receipts::new()
        .fully_read_marker(event_id.clone())
        .private_read_receipt(event_id);
    room.send_multiple_receipts(receipts)
        .await
        .map_err(MatrixRoomOperationError::from_sdk_error)
}

pub async fn mark_room_as_unread(
    session: &MatrixClientSession,
    room_id: &str,
    unread: bool,
) -> Result<(), MatrixRoomOperationError> {
    let room = matrix_room(session, room_id)?;
    room.set_unread_flag(unread)
        .await
        .map_err(MatrixRoomOperationError::from_sdk_error)
}

const ROOM_NOTIFICATION_RULE_ID_PREFIX: &str = "org.matrix.desktop.notify.room.";

fn room_notification_rule_id(room_id: &matrix_sdk::ruma::RoomId) -> String {
    format!("{ROOM_NOTIFICATION_RULE_ID_PREFIX}{room_id}")
}

/// Sets the per-room notification mode by manipulating app-owned push rules.
///
/// - `All`: removes any app-owned override/underride rule for the room.
/// - `Mentions`: adds an underride rule with empty actions so generic message
///   rules are suppressed but mention/highlight rules still fire.
/// - `Mute`: adds an override rule with empty actions so all notifications for
///   the room are suppressed.
pub async fn set_room_notification_mode(
    session: &MatrixClientSession,
    room_id: &str,
    mode: koushi_state::RoomNotificationMode,
) -> Result<(), MatrixRoomOperationError> {
    use matrix_sdk::ruma::{
        RoomId,
        api::client::push::{delete_pushrule, set_pushrule},
        push::{
            EventMatchConditionData, NewConditionalPushRule, NewPushRule, PushCondition, RuleKind,
        },
    };

    let room_id = RoomId::parse(room_id).map_err(|_| MatrixRoomOperationError::InvalidRoomId)?;
    let rule_id = room_notification_rule_id(&room_id);
    let client = session.client();

    // Remove any previous app-owned rule for this room. Missing-rule errors are
    // ignored so the operation is idempotent.
    for kind in [RuleKind::Override, RuleKind::Underride] {
        let delete_request = delete_pushrule::v3::Request::new(kind, rule_id.clone());
        let _ = client.send(delete_request).await;
    }

    if mode != koushi_state::RoomNotificationMode::All {
        let actions = Vec::new();
        let conditions = vec![PushCondition::EventMatch(EventMatchConditionData::new(
            "room_id".to_owned(),
            room_id.to_string(),
        ))];
        let new_rule = NewConditionalPushRule::new(rule_id, conditions, actions);
        let new_push_rule = match mode {
            koushi_state::RoomNotificationMode::Mentions => NewPushRule::Underride(new_rule),
            koushi_state::RoomNotificationMode::Mute => NewPushRule::Override(new_rule),
            koushi_state::RoomNotificationMode::All => unreachable!(),
        };
        let set_request = set_pushrule::v3::Request::new(new_push_rule);
        client.send(set_request).await.map_err(|error| {
            MatrixRoomOperationError::from_sdk_error(matrix_sdk::Error::Http(Box::new(error)))
        })?;
    }

    Ok(())
}

pub async fn load_pinned_event_ids(
    session: &MatrixClientSession,
    room_id: &str,
) -> Result<Vec<String>, MatrixRoomOperationError> {
    let room = matrix_room(session, room_id)?;
    let pinned = room
        .load_pinned_events()
        .await
        .map_err(MatrixRoomOperationError::from_sdk_error)?
        .unwrap_or_default();
    Ok(pinned
        .into_iter()
        .map(|event_id| event_id.to_string())
        .collect())
}

pub async fn edit_text_message(
    session: &MatrixClientSession,
    room_id: &str,
    event_id: &str,
    body: &str,
) -> Result<(), MatrixRoomOperationError> {
    let room = matrix_room(session, room_id)?;
    let event_id = matrix_sdk::ruma::EventId::parse(event_id)
        .map_err(|_| MatrixRoomOperationError::InvalidEventId)?;
    let content =
        matrix_sdk::ruma::events::room::message::RoomMessageEventContentWithoutRelation::text_plain(
            body,
        );
    let edit_content = matrix_sdk::room::edit::EditedContent::RoomMessage(content);
    let edit_event = room
        .make_edit_event(&event_id, edit_content)
        .await
        .map_err(|_| MatrixRoomOperationError::Sdk(MatrixRoomOperationFailureKind::Sdk))?;

    room.send(edit_event)
        .await
        .map(|_| ())
        .map_err(MatrixRoomOperationError::from_sdk_error)
}

pub async fn redact_message(
    session: &MatrixClientSession,
    room_id: &str,
    event_id: &str,
) -> Result<(), MatrixRoomOperationError> {
    let room = matrix_room(session, room_id)?;
    let event_id = matrix_sdk::ruma::EventId::parse(event_id)
        .map_err(|_| MatrixRoomOperationError::InvalidEventId)?;

    room.redact(&event_id, None, None)
        .await
        .map(|_| ())
        .map_err(|_| MatrixRoomOperationError::Sdk(MatrixRoomOperationFailureKind::Sdk))
}

pub async fn search_message_candidates(
    session: &MatrixClientSession,
    query: &str,
    limit: usize,
) -> Result<Vec<MatrixSearchCandidate>, MatrixSearchError> {
    if query.trim().is_empty() || limit == 0 {
        return Ok(Vec::new());
    }

    let mut iterator = session
        .client()
        .search_messages(query.to_owned(), limit)
        .build();
    let Some(candidates) = iterator
        .next()
        .await
        .map_err(matrix_search_error_from_sdk)?
    else {
        return Ok(Vec::new());
    };

    Ok(candidates
        .into_iter()
        .take(limit)
        .enumerate()
        .map(|(index, (room_id, event_id))| MatrixSearchCandidate {
            room_id: room_id.to_string(),
            event_id: event_id.to_string(),
            score_millis: 1_000_u32.saturating_sub(index as u32),
        })
        .collect())
}

fn matrix_search_error_from_sdk(error: SearchError) -> MatrixSearchError {
    match error {
        SearchError::IndexError(error) => matrix_search_error_from_index(&error),
        SearchError::EventLoadError(_) => MatrixSearchError::Internal,
    }
}

fn matrix_search_error_from_index(error: &IndexError) -> MatrixSearchError {
    match error {
        IndexError::OpenDirectoryError(_) | IndexError::IO(_) => {
            MatrixSearchError::IndexUnavailable
        }
        IndexError::QueryParserError(_) => MatrixSearchError::Query,
        IndexError::TantivyError(_)
        | IndexError::IndexSchemaError(_)
        | IndexError::IndexWriteError(_)
        | IndexError::MessageTypeNotSupported
        | IndexError::CannotIndexRedactedMessage
        | IndexError::EmptyMessage => MatrixSearchError::Internal,
    }
}

/// Normalize a room list snapshot from caller-provided SDK rooms.
///
/// This is the normalization entry point for callers that already hold the
/// room list source of truth: entries from the ONE live `RoomListService`
/// owned by the running `SyncService` (converted with
/// `RoomListItem::into_inner()`), or `client.joined_rooms()` on the
/// `LegacySync` backend. Unlike [`room_list_snapshot`], it never constructs
/// a `RoomListService` of its own.
pub async fn room_list_snapshot_from_sdk_rooms(
    rooms: impl IntoIterator<Item = matrix_sdk::Room>,
) -> MatrixRoomListSnapshot {
    matrix_room_list_snapshot_from_rooms(&BTreeMap::new(), rooms).await
}

/// Normalize joined rooms from the caller's source of truth plus invited rooms
/// from the base client. The live `RoomListService` path remains the owner of
/// joined-room entries; invites are projected from `client.invited_rooms()`
/// because the live entries adapter is intentionally joined-filtered.
pub async fn room_list_snapshot_from_sdk_rooms_with_invites(
    session: &MatrixClientSession,
    rooms: impl IntoIterator<Item = matrix_sdk::Room>,
) -> MatrixRoomListSnapshot {
    let client = session.client();
    let direct_targets_by_room = matrix_direct_account_data_targets_by_room(&client).await;
    let mut snapshot = matrix_room_list_snapshot_from_rooms(&direct_targets_by_room, rooms).await;
    snapshot.invites = matrix_invite_previews_from_rooms(client.invited_rooms()).await;
    snapshot
}

/// One-shot room list snapshot that constructs a DISPOSABLE
/// `RoomListService` internally.
///
/// DEPRECATED FOR CORE USE (canon, overview.md RoomActor): a disposable
/// `RoomListService` is not driven by the sync loop, races the running
/// `SyncService`, and returns entries without the live service's
/// `required_state` (e.g. `m.room.create`), so space classification is
/// unreliable (deterministically broken on Conduit). The core runtime must
/// use [`room_list_snapshot_from_sdk_rooms`] with rooms taken from the live
/// service's entries or from `client.joined_rooms()`. This function remains
/// only for the legacy auth-crate QA flow, which runs without a
/// `SyncService`.
pub async fn room_list_snapshot(
    session: &MatrixClientSession,
) -> Result<MatrixRoomListSnapshot, MatrixRoomListError> {
    let client = session.client();
    let service = match matrix_sdk_ui::room_list_service::RoomListService::new_with(
        client.clone(),
        false,
        "matrix-desktop-room-list-snapshot",
        1,
    )
    .await
    {
        Ok(service) => service,
        Err(_) => {
            let direct_targets_by_room = matrix_direct_account_data_targets_by_room(&client).await;
            return Ok(matrix_room_list_snapshot_from_rooms(
                &direct_targets_by_room,
                client.joined_rooms(),
            )
            .await);
        }
    };
    let all_rooms = service
        .all_rooms()
        .await
        .map_err(|_| MatrixRoomListError::Sdk)?;
    let (entries, entries_controller) =
        all_rooms.entries_with_dynamic_adapters(MATRIX_ROOM_LIST_SNAPSHOT_LIMIT);
    entries_controller.set_filter(Box::new(
        matrix_sdk_ui::room_list_service::filters::new_filter_joined(),
    ));

    let mut entries = Box::pin(entries);
    let Some(diffs) = entries.next().await else {
        return Ok(MatrixRoomListSnapshot::default());
    };

    let direct_targets_by_room = matrix_direct_account_data_targets_by_room(&client).await;
    let snapshot = matrix_room_list_snapshot_from_diffs(&direct_targets_by_room, diffs).await;
    if snapshot.rooms.is_empty() && snapshot.spaces.is_empty() {
        return Ok(matrix_room_list_snapshot_from_rooms(
            &direct_targets_by_room,
            client.joined_rooms(),
        )
        .await);
    }

    Ok(snapshot)
}

pub fn room_list_smoke_report(snapshot: &MatrixRoomListSnapshot) -> RoomListSmokeReport {
    RoomListSmokeReport {
        rooms: snapshot.rooms.len(),
        spaces: snapshot.spaces.len(),
        dms: snapshot.rooms.iter().filter(|room| room.is_dm).count(),
        unread_rooms: snapshot
            .rooms
            .iter()
            .filter(|room| room.unread_count > 0)
            .count(),
    }
}

pub fn room_attention_summary_from_counts(
    room_display_name: Option<String>,
    is_dm: bool,
    notification_count: u64,
    highlight_count: u64,
    unread_messages: u64,
    is_marked_unread: bool,
) -> Option<RoomAttentionSummary> {
    let unread_count =
        room_attention_unread_count(notification_count, unread_messages, is_marked_unread);
    let room_display_name = room_display_name.unwrap_or_else(|| "Room".to_owned());

    room_attention_summary(
        room_display_name,
        is_dm,
        notification_count,
        highlight_count,
        unread_count,
    )
}

pub fn timeline_smoke_report(
    selected_room_present: bool,
    initial_items: &[MatrixTimelineItem],
) -> TimelineSmokeReport {
    TimelineSmokeReport {
        selected_room_present,
        timeline_items: initial_items.len(),
    }
}

pub fn real_account_qa_report(
    snapshot: &MatrixRoomListSnapshot,
    selected_room_present: bool,
    initial_items: &[MatrixTimelineItem],
) -> RealAccountQaReport {
    real_account_qa_report_with_restore_state(snapshot, selected_room_present, initial_items, false)
}

pub fn restored_real_account_qa_report(
    snapshot: &MatrixRoomListSnapshot,
    selected_room_present: bool,
    initial_items: &[MatrixTimelineItem],
) -> RealAccountQaReport {
    real_account_qa_report_with_restore_state(snapshot, selected_room_present, initial_items, true)
}

pub fn real_account_qa_report_with_search(
    snapshot: &MatrixRoomListSnapshot,
    selected_room_present: bool,
    initial_items: &[MatrixTimelineItem],
    session_restored: bool,
    search_candidates: &[MatrixSearchCandidate],
) -> RealAccountQaReport {
    let mut report = real_account_qa_report_with_restore_state(
        snapshot,
        selected_room_present,
        initial_items,
        session_restored,
    );
    report.search = search_smoke_report(true, search_candidates);
    report
}

pub fn search_smoke_report(
    invoked: bool,
    candidates: &[MatrixSearchCandidate],
) -> SearchSmokeReport {
    SearchSmokeReport {
        invoked,
        candidates: candidates.len(),
    }
}

fn real_account_qa_report_with_restore_state(
    snapshot: &MatrixRoomListSnapshot,
    selected_room_present: bool,
    initial_items: &[MatrixTimelineItem],
    session_restored: bool,
) -> RealAccountQaReport {
    RealAccountQaReport {
        room_list: room_list_smoke_report(snapshot),
        timeline: timeline_smoke_report(selected_room_present, initial_items),
        session_restored,
        search: SearchSmokeReport::default(),
    }
}

pub async fn subscribe_room_timeline(
    session: &MatrixClientSession,
    room_id: &str,
) -> Result<MatrixTimelineSubscription, MatrixTimelineError> {
    let room = timeline_room(session, room_id)?;
    let timeline = matrix_sdk_ui::timeline::TimelineBuilder::new(&room)
        .build()
        .await
        .map_err(|_| MatrixTimelineError::Sdk)?;
    let timeline = Arc::new(timeline);
    let (items, updates) = timeline.subscribe().await;
    let initial_items = items
        .iter()
        .filter_map(|item| matrix_timeline_item_from_ui(room_id, item))
        .collect();
    let update_room_id = room_id.to_owned();
    let updates = updates
        .map(move |diffs| matrix_timeline_updates_from_diffs(&update_room_id, diffs))
        .boxed();

    Ok(MatrixTimelineSubscription {
        room_id: room_id.to_owned(),
        timeline,
        initial_items,
        updates,
    })
}

pub async fn room_timeline_visible_items(
    session: &MatrixClientSession,
    room_id: &str,
    backfill_event_count: u16,
) -> Result<Vec<MatrixTimelineItem>, MatrixTimelineError> {
    let room = timeline_room(session, room_id)?;
    let timeline = matrix_sdk_ui::timeline::TimelineBuilder::new(&room)
        .build()
        .await
        .map_err(|_| MatrixTimelineError::Sdk)?;
    let (items, updates) = timeline.subscribe().await;
    let mut items = items
        .iter()
        .filter_map(|item| matrix_timeline_item_from_ui(room_id, item))
        .collect::<Vec<_>>();
    if !items.is_empty() || backfill_event_count == 0 {
        return Ok(items);
    }

    let mut updates = Box::pin(updates);
    timeline
        .paginate_backwards(backfill_event_count)
        .await
        .map_err(|_| MatrixTimelineError::Sdk)?;

    for _ in 0..3 {
        let Some(diffs) = tokio::time::timeout(Duration::from_secs(10), updates.next())
            .await
            .map_err(|_| MatrixTimelineError::Sdk)?
        else {
            break;
        };
        items.extend(
            matrix_timeline_updates_from_diffs(room_id, diffs)
                .into_iter()
                .filter_map(|update| match update {
                    MatrixTimelineUpdate::Upsert(item) => Some(item),
                    MatrixTimelineUpdate::Remove { .. } => None,
                }),
        );
        if !items.is_empty() {
            break;
        }
    }

    Ok(items)
}

pub async fn sync_once(session: &MatrixClientSession) -> Result<(), MatrixSyncError> {
    session
        .client()
        .sync_once(matrix_sdk::config::SyncSettings::default())
        .await
        .map(|_| ())
        .map_err(|_| MatrixSyncError::Sdk)
}

/// Close every SDK store connection for a session before deleting its keyed
/// on-disk store. Completion is a barrier: all in-flight store operations and
/// SQLite pools have drained when this returns.
pub async fn close_session_stores(session: &MatrixClientSession) -> Result<(), MatrixSyncError> {
    session
        .client()
        .pause()
        .await
        .map_err(|_| MatrixSyncError::Sdk)
}

fn restricted_verification_sync_filter() -> matrix_sdk::ruma::api::client::filter::FilterDefinition
{
    let mut filter = matrix_sdk::ruma::api::client::filter::FilterDefinition::default();
    filter.presence = matrix_sdk::ruma::api::client::filter::Filter::ignore_all();
    filter.room = matrix_sdk::ruma::api::client::filter::RoomFilter::ignore_all();
    filter
}

fn restricted_verification_sync_settings() -> matrix_sdk::config::SyncSettings {
    matrix_sdk::config::SyncSettings::new()
        .timeout(RESTRICTED_VERIFICATION_SYNC_SERVER_TIMEOUT)
        .filter(
            matrix_sdk::ruma::api::client::sync::sync_events::v3::Filter::FilterDefinition(
                restricted_verification_sync_filter(),
            ),
        )
}

pub async fn restricted_verification_sync_once(
    session: &MatrixClientSession,
) -> Result<(), MatrixSyncError> {
    restricted_verification_sync_once_with_token(session, None)
        .await
        .map(|_| ())
}

pub async fn restricted_verification_sync_once_with_token(
    session: &MatrixClientSession,
    token: Option<String>,
) -> Result<String, MatrixSyncError> {
    let mut settings = restricted_verification_sync_settings();
    if let Some(token) = token {
        settings = settings.token(token);
    }
    session
        .client()
        .sync_once(settings)
        .await
        .map(|response| response.next_batch)
        .map_err(|_| MatrixSyncError::Sdk)
}

pub async fn sync_loop<F, C>(
    session: &MatrixClientSession,
    on_successful_sync: F,
) -> Result<(), MatrixSyncError>
where
    F: Fn() -> C,
    C: Future<Output = MatrixSyncLoopControl>,
{
    session
        .client()
        .sync_with_callback(matrix_sdk::config::SyncSettings::default(), move |_| {
            let callback = on_successful_sync();
            async move {
                match callback.await {
                    MatrixSyncLoopControl::Continue => matrix_sdk::LoopCtrl::Continue,
                    MatrixSyncLoopControl::Stop => matrix_sdk::LoopCtrl::Break,
                }
            }
        })
        .await
        .map_err(|_| MatrixSyncError::Sdk)
}

fn matrix_room(
    session: &MatrixClientSession,
    room_id: &str,
) -> Result<matrix_sdk::Room, MatrixRoomOperationError> {
    let room_id = matrix_sdk::ruma::RoomId::parse(room_id)
        .map_err(|_| MatrixRoomOperationError::InvalidRoomId)?;
    session
        .client()
        .get_room(&room_id)
        .ok_or(MatrixRoomOperationError::RoomUnavailable)
}

fn matrix_public_room_from_chunk(
    chunk: matrix_sdk::ruma::directory::PublicRoomsChunk,
) -> MatrixPublicRoomDirectoryRoom {
    MatrixPublicRoomDirectoryRoom {
        room_id: chunk.room_id.to_string(),
        canonical_alias: chunk.canonical_alias.map(|alias| alias.to_string()),
        name: chunk.name.unwrap_or_else(|| "Public room".to_owned()),
        topic: chunk.topic,
        avatar_url: chunk.avatar_url.map(|avatar_url| avatar_url.to_string()),
        joined_members: chunk.num_joined_members.into(),
        world_readable: chunk.world_readable,
        guest_can_join: chunk.guest_can_join,
    }
}

async fn matrix_room_settings_snapshot(room: &matrix_sdk::Room) -> MatrixRoomSettingsSnapshot {
    let power_levels = room.power_levels_or_default().await;
    let own_user_id = room.own_user_id();
    let members = matrix_room_member_summaries(room).await;
    let can_edit_settings = power_levels.user_can_send_state(
        own_user_id,
        matrix_sdk::ruma::events::StateEventType::RoomName,
    ) && power_levels.user_can_send_state(
        own_user_id,
        matrix_sdk::ruma::events::StateEventType::RoomTopic,
    ) && power_levels.user_can_send_state(
        own_user_id,
        matrix_sdk::ruma::events::StateEventType::RoomAvatar,
    ) && power_levels.user_can_send_state(
        own_user_id,
        matrix_sdk::ruma::events::StateEventType::RoomJoinRules,
    ) && power_levels.user_can_send_state(
        own_user_id,
        matrix_sdk::ruma::events::StateEventType::RoomHistoryVisibility,
    );

    MatrixRoomSettingsSnapshot {
        room_id: room.room_id().to_string(),
        name: room.name(),
        topic: room.topic(),
        avatar_url: room.avatar_url().map(|url| url.to_string()),
        canonical_alias: room.canonical_alias().map(|alias| alias.to_string()),
        alternate_aliases: room
            .alt_aliases()
            .into_iter()
            .map(|alias| alias.to_string())
            .collect(),
        join_rule: room
            .join_rule()
            .as_ref()
            .map(matrix_room_join_rule)
            .unwrap_or(MatrixRoomJoinRule::Invite),
        history_visibility: matrix_room_history_visibility(&room.history_visibility_or_default()),
        permissions: MatrixRoomPermissionFacts {
            can_edit_settings,
            can_edit_roles: power_levels.user_can_send_state(
                own_user_id,
                matrix_sdk::ruma::events::StateEventType::RoomPowerLevels,
            ),
            can_kick: power_levels.user_can_kick(own_user_id),
            can_ban: power_levels.user_can_ban(own_user_id),
            can_unban: power_levels.user_can_ban(own_user_id),
        },
        members,
    }
}

async fn matrix_room_member_summaries(room: &matrix_sdk::Room) -> Vec<MatrixRoomMemberSummary> {
    let Ok(members) = room.members(matrix_sdk::RoomMemberships::ACTIVE).await else {
        return Vec::new();
    };
    let encryption = room.client().encryption();
    let mut summaries: Vec<MatrixRoomMemberSummary> = Vec::with_capacity(members.len());
    for member in members {
        let power_level = matrix_room_member_power_level(member.power_level());
        let user_trust = encryption
            .get_user_identity(member.user_id())
            .await
            .ok()
            .flatten()
            .map(matrix_user_trust_state_from_sdk_identity);
        summaries.push(MatrixRoomMemberSummary {
            user_id: member.user_id().to_string(),
            display_name: member.display_name().map(ToOwned::to_owned),
            avatar_url: member.avatar_url().map(ToString::to_string),
            power_level,
            role: matrix_room_member_role(power_level),
            user_trust,
        });
    }
    summaries.sort_by(|left, right| left.user_id.cmp(&right.user_id));
    summaries
}

fn matrix_user_trust_state_from_sdk_identity(
    identity: matrix_sdk::encryption::identities::UserIdentity,
) -> MatrixUserTrustState {
    if identity.has_verification_violation() {
        MatrixUserTrustState::IdentityReset
    } else if identity.is_verified() {
        MatrixUserTrustState::Verified
    } else {
        MatrixUserTrustState::Unverified
    }
}

fn room_settings_snapshot_with_member_power_level(
    mut snapshot: MatrixRoomSettingsSnapshot,
    target_user_id: &str,
    power_level: i64,
) -> MatrixRoomSettingsSnapshot {
    if let Some(member) = snapshot
        .members
        .iter_mut()
        .find(|member| member.user_id == target_user_id)
    {
        member.power_level = Some(power_level);
        member.role = matrix_room_member_role(Some(power_level));
    }
    snapshot
}

fn matrix_room_member_power_level(
    power_level: matrix_sdk::ruma::events::room::power_levels::UserPowerLevel,
) -> Option<i64> {
    match power_level {
        matrix_sdk::ruma::events::room::power_levels::UserPowerLevel::Infinite => None,
        matrix_sdk::ruma::events::room::power_levels::UserPowerLevel::Int(value) => {
            Some(value.into())
        }
        _ => None,
    }
}

fn matrix_room_member_role(power_level: Option<i64>) -> MatrixRoomMemberRole {
    match power_level {
        None => MatrixRoomMemberRole::Creator,
        Some(level) if level >= 100 => MatrixRoomMemberRole::Administrator,
        Some(level) if level >= 50 => MatrixRoomMemberRole::Moderator,
        Some(_) => MatrixRoomMemberRole::User,
    }
}

fn room_settings_snapshot_with_change(
    mut snapshot: MatrixRoomSettingsSnapshot,
    change: &MatrixRoomSettingChange,
) -> MatrixRoomSettingsSnapshot {
    match change {
        MatrixRoomSettingChange::Name(name) => {
            snapshot.name = name.clone();
        }
        MatrixRoomSettingChange::Topic(topic) => {
            snapshot.topic = topic.clone();
        }
        MatrixRoomSettingChange::AvatarUrl(avatar_url) => {
            snapshot.avatar_url = avatar_url.clone();
        }
        MatrixRoomSettingChange::JoinRule(join_rule) => {
            snapshot.join_rule = *join_rule;
        }
        MatrixRoomSettingChange::HistoryVisibility(history_visibility) => {
            snapshot.history_visibility = *history_visibility;
        }
    }
    snapshot
}

fn matrix_room_join_rule(
    join_rule: &matrix_sdk::ruma::events::room::join_rules::JoinRule,
) -> MatrixRoomJoinRule {
    use matrix_sdk::ruma::events::room::join_rules::JoinRule;
    match join_rule {
        JoinRule::Public => MatrixRoomJoinRule::Public,
        JoinRule::Invite => MatrixRoomJoinRule::Invite,
        JoinRule::Knock => MatrixRoomJoinRule::Knock,
        JoinRule::Restricted(_) | JoinRule::KnockRestricted(_) => MatrixRoomJoinRule::Restricted,
        JoinRule::Private => MatrixRoomJoinRule::Private,
        _ => MatrixRoomJoinRule::Invite,
    }
}

fn sdk_join_rule_for_update(
    join_rule: MatrixRoomJoinRule,
) -> Result<matrix_sdk::ruma::events::room::join_rules::JoinRule, MatrixRoomOperationError> {
    use matrix_sdk::ruma::events::room::join_rules::JoinRule;
    match join_rule {
        MatrixRoomJoinRule::Public => Ok(JoinRule::Public),
        MatrixRoomJoinRule::Invite => Ok(JoinRule::Invite),
        MatrixRoomJoinRule::Knock => Ok(JoinRule::Knock),
        MatrixRoomJoinRule::Private => Ok(JoinRule::Private),
        MatrixRoomJoinRule::Restricted => Err(MatrixRoomOperationError::InvalidRoomSetting),
    }
}

fn matrix_room_history_visibility(
    history_visibility: &matrix_sdk::ruma::events::room::history_visibility::HistoryVisibility,
) -> MatrixRoomHistoryVisibility {
    use matrix_sdk::ruma::events::room::history_visibility::HistoryVisibility;
    match history_visibility {
        HistoryVisibility::WorldReadable => MatrixRoomHistoryVisibility::WorldReadable,
        HistoryVisibility::Shared => MatrixRoomHistoryVisibility::Shared,
        HistoryVisibility::Invited => MatrixRoomHistoryVisibility::Invited,
        HistoryVisibility::Joined => MatrixRoomHistoryVisibility::Joined,
        _ => MatrixRoomHistoryVisibility::Shared,
    }
}

fn sdk_history_visibility(
    history_visibility: MatrixRoomHistoryVisibility,
) -> matrix_sdk::ruma::events::room::history_visibility::HistoryVisibility {
    use matrix_sdk::ruma::events::room::history_visibility::HistoryVisibility;
    match history_visibility {
        MatrixRoomHistoryVisibility::WorldReadable => HistoryVisibility::WorldReadable,
        MatrixRoomHistoryVisibility::Shared => HistoryVisibility::Shared,
        MatrixRoomHistoryVisibility::Invited => HistoryVisibility::Invited,
        MatrixRoomHistoryVisibility::Joined => HistoryVisibility::Joined,
    }
}

fn non_empty_name(name: &str) -> Option<String> {
    let name = name.trim();
    if name.is_empty() {
        None
    } else {
        Some(name.to_owned())
    }
}

fn matrix_room_operation_failure_kind(error: &matrix_sdk::Error) -> MatrixRoomOperationFailureKind {
    match error {
        matrix_sdk::Error::AuthenticationRequired => {
            MatrixRoomOperationFailureKind::AuthenticationRequired
        }
        matrix_sdk::Error::WrongRoomState(_) => MatrixRoomOperationFailureKind::WrongRoomState,
        matrix_sdk::Error::Http(error) => {
            if error
                .as_client_api_error()
                .is_some_and(|error| error.status_code.as_u16() == 403)
                || matches!(
                    error.client_api_error_kind(),
                    Some(matrix_sdk::ruma::api::error::ErrorKind::Forbidden)
                )
            {
                MatrixRoomOperationFailureKind::Forbidden
            } else {
                MatrixRoomOperationFailureKind::Http
            }
        }
        matrix_sdk::Error::BadCryptoStoreState
        | matrix_sdk::Error::NoOlmMachine
        | matrix_sdk::Error::CryptoStoreError(_)
        | matrix_sdk::Error::OlmError(_)
        | matrix_sdk::Error::MegolmError(_)
        | matrix_sdk::Error::DecryptorError(_) => MatrixRoomOperationFailureKind::Encryption,
        matrix_sdk::Error::StateStore(_)
        | matrix_sdk::Error::EventCacheStore(_)
        | matrix_sdk::Error::MediaStore(_) => MatrixRoomOperationFailureKind::Store,
        matrix_sdk::Error::SerdeJson(_)
        | matrix_sdk::Error::Io(_)
        | matrix_sdk::Error::CrossProcessLockError(_)
        | matrix_sdk::Error::Identifier(_)
        | matrix_sdk::Error::Url(_)
        | matrix_sdk::Error::SlidingSync(_)
        | matrix_sdk::Error::MultipleSessionCallbacks
        | matrix_sdk::Error::OAuth(_)
        | matrix_sdk::Error::ConcurrentRequestFailed
        | matrix_sdk::Error::UnknownError(_)
        | matrix_sdk::Error::EventCache(_)
        | matrix_sdk::Error::SendQueueWedgeError(_)
        | matrix_sdk::Error::BackupNotEnabled
        | matrix_sdk::Error::CantIgnoreLoggedInUser
        | matrix_sdk::Error::Media(_)
        | matrix_sdk::Error::ReplyError(_)
        | matrix_sdk::Error::PowerLevels(_)
        | matrix_sdk::Error::Timeout
        | matrix_sdk::Error::InsufficientData => MatrixRoomOperationFailureKind::Sdk,
        _ => MatrixRoomOperationFailureKind::Sdk,
    }
}

fn timeline_room(
    session: &MatrixClientSession,
    room_id: &str,
) -> Result<matrix_sdk::Room, MatrixTimelineError> {
    let room_id =
        matrix_sdk::ruma::RoomId::parse(room_id).map_err(|_| MatrixTimelineError::InvalidRoomId)?;
    session
        .client()
        .get_room(&room_id)
        .ok_or(MatrixTimelineError::RoomUnavailable)
}

fn matrix_timeline_updates_from_diffs(
    room_id: &str,
    diffs: Vec<eyeball_im::VectorDiff<Arc<matrix_sdk_ui::timeline::TimelineItem>>>,
) -> Vec<MatrixTimelineUpdate> {
    let mut updates = Vec::new();
    for diff in diffs {
        match diff {
            eyeball_im::VectorDiff::Append { values }
            | eyeball_im::VectorDiff::Reset { values } => {
                updates.extend(
                    values
                        .iter()
                        .filter_map(|item| matrix_timeline_update_from_ui(room_id, item)),
                );
            }
            eyeball_im::VectorDiff::PushFront { value }
            | eyeball_im::VectorDiff::PushBack { value }
            | eyeball_im::VectorDiff::Insert { value, .. }
            | eyeball_im::VectorDiff::Set { value, .. } => {
                if let Some(update) = matrix_timeline_update_from_ui(room_id, &value) {
                    updates.push(update);
                }
            }
            eyeball_im::VectorDiff::Clear
            | eyeball_im::VectorDiff::PopFront
            | eyeball_im::VectorDiff::PopBack
            | eyeball_im::VectorDiff::Remove { .. }
            | eyeball_im::VectorDiff::Truncate { .. } => {}
        }
    }
    updates
}

fn matrix_timeline_update_from_ui(
    room_id: &str,
    item: &matrix_sdk_ui::timeline::TimelineItem,
) -> Option<MatrixTimelineUpdate> {
    let event = item.as_event()?;
    let event_id = event.event_id()?.to_string();
    match event.content().as_message() {
        Some(content) => Some(MatrixTimelineUpdate::Upsert(MatrixTimelineItem {
            room_id: room_id.to_owned(),
            event_id,
            sender: event.sender().to_string(),
            timestamp_ms: event.timestamp().0.into(),
            body: content.body().to_owned(),
        })),
        None => Some(MatrixTimelineUpdate::Remove {
            room_id: room_id.to_owned(),
            event_id,
        }),
    }
}

async fn matrix_room_list_snapshot_from_diffs(
    direct_targets_by_room: &BTreeMap<String, Vec<String>>,
    diffs: Vec<eyeball_im::VectorDiff<matrix_sdk_ui::room_list_service::RoomListItem>>,
) -> MatrixRoomListSnapshot {
    let mut items = Vec::new();
    for diff in diffs {
        match diff {
            eyeball_im::VectorDiff::Append { values }
            | eyeball_im::VectorDiff::Reset { values } => {
                items.extend(values.into_iter());
            }
            eyeball_im::VectorDiff::PushFront { value }
            | eyeball_im::VectorDiff::PushBack { value }
            | eyeball_im::VectorDiff::Insert { value, .. }
            | eyeball_im::VectorDiff::Set { value, .. } => {
                items.push(value);
            }
            eyeball_im::VectorDiff::Clear
            | eyeball_im::VectorDiff::PopFront
            | eyeball_im::VectorDiff::PopBack
            | eyeball_im::VectorDiff::Remove { .. }
            | eyeball_im::VectorDiff::Truncate { .. } => {}
        }
    }

    matrix_room_list_snapshot_from_items(direct_targets_by_room, items).await
}

async fn matrix_room_list_snapshot_from_items(
    direct_targets_by_room: &BTreeMap<String, Vec<String>>,
    items: Vec<matrix_sdk_ui::room_list_service::RoomListItem>,
) -> MatrixRoomListSnapshot {
    matrix_room_list_snapshot_from_rooms(
        direct_targets_by_room,
        items.into_iter().map(|item| item.into_inner()),
    )
    .await
}

async fn matrix_room_list_snapshot_from_rooms(
    direct_targets_by_room: &BTreeMap<String, Vec<String>>,
    rooms: impl IntoIterator<Item = matrix_sdk::Room>,
) -> MatrixRoomListSnapshot {
    let mut snapshot = MatrixRoomListSnapshot::default();
    let mut user_profiles = BTreeMap::new();
    for room in rooms {
        if room.state() != matrix_sdk::RoomState::Joined {
            continue;
        }

        let room_id = room.room_id().to_string();
        let display_name = room
            .cached_display_name()
            .map(|name| name.to_string())
            .unwrap_or_else(|| room_id.clone());

        if room.is_space() {
            let child_room_ids = matrix_space_child_room_ids(&room).await;
            let member_user_ids = matrix_space_member_user_ids_no_sync(&room).await;
            snapshot.spaces.push(MatrixRoomListSpace {
                space_id: room_id,
                display_name,
                avatar_mxc_uri: room.avatar_url().map(|uri| uri.to_string()),
                child_room_ids,
                member_user_ids,
            });
            continue;
        }

        let unread_notifications = room.unread_notification_counts();
        let notification_count = unread_notifications.notification_count.into();
        let highlight_count = unread_notifications.highlight_count.into();
        let is_marked_unread = room.is_marked_unread();
        let unread_messages = room.num_unread_messages();
        let unread_count =
            room_attention_unread_count(notification_count, unread_messages, is_marked_unread);

        let parent_space_ids = matrix_parent_space_ids(&room).await;
        let tags = matrix_room_tags(&room).await;

        let is_dm = if direct_targets_by_room.contains_key(&room_id) {
            true
        } else if !room.direct_targets().is_empty() {
            true
        } else {
            room.is_direct().await.unwrap_or_else(|_| room.is_dm())
        };
        let dm_user_ids =
            matrix_room_list_dm_user_ids(&room, direct_targets_by_room, is_dm, &mut user_profiles)
                .await;
        let joined_members = room.joined_members_count();

        let is_encrypted = room
            .latest_encryption_state()
            .await
            .map(|state| state.is_encrypted())
            .unwrap_or(false);
        let (latest_event, conversation_activity) =
            matrix_room_latest_event_projection(&room).await;
        let fully_read_event_id = matrix_room_fully_read_event_id(&room).await;
        let private_read_receipt_event_id = matrix_room_private_read_receipt_event_id(&room).await;
        let recency_stamp = room.recency_stamp().map(Into::into);
        trace_sdk_conversation_activity(conversation_activity, recency_stamp.is_some());

        if unread_count > 0 || notification_count > 0 || highlight_count > 0 || is_marked_unread {
            trace_sdk_unread_snapshot(SdkUnreadTrace {
                unread_messages,
                unread_count,
                notification_count,
                highlight_count,
                marked_unread: is_marked_unread,
                latest_event: &latest_event,
                fully_read_event_id: fully_read_event_id.as_deref(),
                private_read_receipt_event_id: private_read_receipt_event_id.as_deref(),
                recency_stamp_present: recency_stamp.is_some(),
                conversation_activity,
            });
        }

        snapshot.rooms.push(matrix_room_list_room_from_counts(
            room_id,
            display_name,
            room.avatar_url().map(|uri| uri.to_string()),
            is_dm,
            dm_user_ids,
            tags,
            notification_count,
            highlight_count,
            unread_count,
            is_marked_unread,
            recency_stamp,
            conversation_activity,
            latest_event,
            fully_read_event_id,
            private_read_receipt_event_id,
            parent_space_ids,
            is_encrypted,
            joined_members,
        ));
    }
    snapshot.user_profiles = user_profiles.into_values().collect();
    snapshot
}

async fn matrix_direct_account_data_targets_by_room(
    client: &matrix_sdk::Client,
) -> BTreeMap<String, Vec<String>> {
    let Some(targets_by_room) = client
        .account()
        .account_data::<DirectEventContent>()
        .await
        .ok()
        .flatten()
        .and_then(|raw_content| raw_content.deserialize().ok())
        .map(|content| direct_account_data_targets_by_room(&content))
    else {
        return client
            .account()
            .fetch_account_data_static::<DirectEventContent>()
            .await
            .ok()
            .flatten()
            .and_then(|raw_content| raw_content.deserialize().ok())
            .map(|content| direct_account_data_targets_by_room(&content))
            .unwrap_or_default();
    };
    targets_by_room
}

fn direct_account_data_targets_by_room(
    content: &DirectEventContent,
) -> BTreeMap<String, Vec<String>> {
    let mut targets_by_room: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for (user_id, room_ids) in content.iter() {
        for room_id in room_ids {
            targets_by_room
                .entry(room_id.to_string())
                .or_default()
                .insert(user_id.to_string());
        }
    }
    targets_by_room
        .into_iter()
        .map(|(room_id, targets)| (room_id, targets.into_iter().collect()))
        .collect()
}

async fn matrix_room_list_dm_user_ids(
    room: &matrix_sdk::Room,
    direct_targets_by_room: &BTreeMap<String, Vec<String>>,
    is_dm: bool,
    user_profiles: &mut BTreeMap<String, MatrixUserProfile>,
) -> Vec<String> {
    let room_id = room.room_id().to_string();
    let own_user_id = room.own_user_id().to_string();
    let mut candidate_user_ids = if let Some(targets) = direct_targets_by_room.get(&room_id) {
        targets.clone()
    } else {
        let cached_direct_targets: Vec<String> = room
            .direct_targets()
            .into_iter()
            .map(|user_id| user_id.to_string())
            .collect();
        if !cached_direct_targets.is_empty() {
            cached_direct_targets
        } else if is_dm {
            room.heroes()
                .into_iter()
                .map(|hero| hero.user_id.to_string())
                .filter(|user_id| user_id != &own_user_id)
                .collect()
        } else {
            Vec::new()
        }
    };

    candidate_user_ids.sort();
    candidate_user_ids.dedup();

    let mut dm_user_ids = Vec::new();
    for candidate_user_id in candidate_user_ids {
        if candidate_user_id == own_user_id {
            continue;
        }

        let Ok(candidate_user_id) =
            matrix_sdk::ruma::OwnedUserId::try_from(candidate_user_id.as_str())
        else {
            continue;
        };

        let candidate_user_id_string = candidate_user_id.to_string();
        dm_user_ids.push(candidate_user_id_string.clone());

        if let Some(member) = room
            .get_member_no_sync(&candidate_user_id)
            .await
            .ok()
            .flatten()
        {
            user_profiles
                .entry(candidate_user_id_string.clone())
                .or_insert_with(|| MatrixUserProfile {
                    user_id: candidate_user_id_string,
                    display_name: member.display_name().map(ToOwned::to_owned),
                    avatar_mxc_uri: member.avatar_url().map(ToString::to_string),
                });
        }
    }

    dm_user_ids.sort();
    dm_user_ids.dedup();
    dm_user_ids
}

async fn matrix_space_member_user_ids_no_sync(room: &matrix_sdk::Room) -> Vec<String> {
    let Ok(members) = room
        .members_no_sync(matrix_sdk::RoomMemberships::ACTIVE)
        .await
    else {
        return Vec::new();
    };
    let mut user_ids: Vec<String> = members
        .into_iter()
        .map(|member| member.user_id().to_string())
        .collect();
    user_ids.sort();
    user_ids.dedup();
    user_ids
}

async fn matrix_invite_previews_from_rooms(
    rooms: impl IntoIterator<Item = matrix_sdk::Room>,
) -> Vec<MatrixInvitePreview> {
    let mut invites = Vec::new();
    for room in rooms {
        if room.state() != matrix_sdk::RoomState::Invited {
            continue;
        }

        let display_name = room
            .display_name()
            .await
            .ok()
            .map(|name| name.to_string())
            .or_else(|| room.name())
            .unwrap_or_else(|| "Invite".to_owned());
        let inviter = room
            .invite_details()
            .await
            .ok()
            .and_then(|details| details.inviter);
        let inviter_display_name = inviter
            .as_ref()
            .and_then(|inviter| inviter.display_name().map(ToOwned::to_owned));
        let inviter_user_id = inviter.map(|inviter| inviter.user_id().to_string());
        let is_dm = room.is_direct().await.unwrap_or(false);

        invites.push(MatrixInvitePreview {
            room_id: room.room_id().to_string(),
            display_name,
            avatar_mxc_uri: room.avatar_url().map(|uri| uri.to_string()),
            topic: room.topic(),
            inviter_display_name,
            inviter_user_id,
            is_dm,
        });
    }
    invites
}

fn room_attention_unread_count(
    notification_count: u64,
    unread_messages: u64,
    is_marked_unread: bool,
) -> u64 {
    let unread_count = notification_count.max(unread_messages);
    if unread_count == 0 && is_marked_unread {
        1
    } else {
        unread_count
    }
}

fn matrix_room_list_room_from_counts(
    room_id: String,
    display_name: String,
    avatar_mxc_uri: Option<String>,
    is_dm: bool,
    dm_user_ids: Vec<String>,
    tags: MatrixRoomTags,
    notification_count: u64,
    highlight_count: u64,
    unread_count: u64,
    marked_unread: bool,
    recency_stamp: Option<u64>,
    conversation_activity: Option<MatrixConversationActivity>,
    latest_event: Option<MatrixRoomLatestEventSummary>,
    fully_read_event_id: Option<String>,
    private_read_receipt_event_id: Option<String>,
    parent_space_ids: Vec<String>,
    is_encrypted: bool,
    joined_members: u64,
) -> MatrixRoomListRoom {
    let marker_covers_latest =
        !marked_unread && read_marker_matches_latest(&latest_event, &fully_read_event_id);
    let marker_covers_latest = marker_covers_latest
        || (!marked_unread
            && read_marker_matches_latest(&latest_event, &private_read_receipt_event_id));
    let (unread_count, notification_count, highlight_count) = if marker_covers_latest {
        (0, 0, 0)
    } else {
        (unread_count, notification_count, highlight_count)
    };
    MatrixRoomListRoom {
        room_id,
        display_name,
        avatar_mxc_uri,
        is_dm,
        dm_user_ids,
        tags,
        unread_count,
        notification_count,
        highlight_count,
        marked_unread,
        recency_stamp,
        conversation_activity,
        latest_event,
        parent_space_ids,
        is_encrypted,
        joined_members,
    }
}

fn read_marker_matches_latest(
    latest_event: &Option<MatrixRoomLatestEventSummary>,
    read_marker_event_id: &Option<String>,
) -> bool {
    latest_event
        .as_ref()
        .map(|event| event.event_id.as_str())
        .is_some_and(|latest_event_id| read_marker_event_id.as_deref() == Some(latest_event_id))
}

async fn matrix_room_fully_read_event_id(room: &matrix_sdk::Room) -> Option<String> {
    room.account_data_static::<FullyReadEventContent>()
        .await
        .ok()
        .flatten()?
        .deserialize()
        .ok()
        .map(|event| event.content.event_id.to_string())
}

async fn matrix_room_private_read_receipt_event_id(room: &matrix_sdk::Room) -> Option<String> {
    let user_id = room.client().user_id()?.to_owned();
    room.load_user_receipt(
        ReceiptType::ReadPrivate,
        ReceiptThread::Unthreaded,
        &user_id,
    )
    .await
    .ok()
    .flatten()
    .map(|(event_id, _)| event_id.to_string())
}

fn matrix_timeline_event_type(
    timeline_event: &matrix_sdk::deserialized_responses::TimelineEvent,
) -> Option<String> {
    timeline_event
        .raw()
        .get_field::<String>("type")
        .ok()
        .flatten()
}

fn matrix_timeline_event_relation(
    timeline_event: &matrix_sdk::deserialized_responses::TimelineEvent,
) -> (Option<String>, Option<String>) {
    let Ok(Some(content)) = timeline_event
        .raw()
        .get_field::<serde_json::Value>("content")
    else {
        return (None, None);
    };
    let Some(relates_to) = content.get("m.relates_to") else {
        return (None, None);
    };
    let relation_event_id = relates_to
        .get("event_id")
        .and_then(serde_json::Value::as_str)
        .or_else(|| {
            relates_to
                .get("m.in_reply_to")
                .and_then(|reply| reply.get("event_id"))
                .and_then(serde_json::Value::as_str)
        })
        .map(ToOwned::to_owned);
    let relation_type = relates_to
        .get("rel_type")
        .and_then(serde_json::Value::as_str)
        .map(ToOwned::to_owned)
        .or_else(|| {
            relation_event_id
                .as_ref()
                .map(|_| "m.in_reply_to".to_owned())
        });
    (relation_type, relation_event_id)
}

async fn matrix_room_latest_event_projection(
    room: &matrix_sdk::Room,
) -> (
    Option<MatrixRoomLatestEventSummary>,
    Option<MatrixConversationActivity>,
) {
    let client = room.client();
    if client.event_cache().has_subscribed() {
        let latest_events = client.latest_events().await;
        let _ = latest_events.listen_to_room(room.room_id()).await;
    }
    let cached_conversation_activity = matrix_room_cached_conversation_activity(room).await;
    let latest_event = room.latest_event();
    match latest_event {
        matrix_sdk::latest_events::LatestEventValue::Remote(timeline_event) => {
            let Some(event_id) = timeline_event
                .event_id()
                .map(|event_id| event_id.to_string())
            else {
                return (None, cached_conversation_activity);
            };
            let sender = timeline_event.sender();
            let timestamp_ms = timeline_event
                .timestamp()
                .map(|timestamp| u64::from(timestamp.get()))
                .unwrap_or(0);
            let event_type = matrix_timeline_event_type(&timeline_event);
            let (relation_type, relation_event_id) =
                matrix_timeline_event_relation(&timeline_event);
            let content =
                matrix_sdk_ui::timeline::TimelineItemContent::from_event(room, timeline_event)
                    .await;
            let content_converted = content.is_some();
            let is_threaded = content
                .as_ref()
                .is_some_and(|content| content.thread_root().is_some());
            let is_reply = content
                .as_ref()
                .is_some_and(|content| content.in_reply_to().is_some());
            let has_thread_summary = content
                .as_ref()
                .is_some_and(|content| content.thread_summary().is_some());
            let has_reactions = content
                .as_ref()
                .and_then(|content| content.reactions())
                .is_some_and(|reactions| !reactions.is_empty());
            let (sender_label, sender_avatar_mxc_uri) = match sender.as_ref() {
                Some(sender) => matrix_room_member_display(room, sender).await,
                None => (None, None),
            };
            let conversation_activity = matrix_conversation_activity_source(
                event_type.as_deref().unwrap_or_default(),
                relation_type.as_deref(),
            )
            .map(|source| MatrixConversationActivity {
                timestamp_ms,
                source,
            });
            let latest_event = MatrixRoomLatestEventSummary {
                event_id,
                sender_id: sender.map(|sender| sender.to_string()),
                sender_label,
                sender_avatar_mxc_uri,
                preview: content.as_ref().and_then(matrix_latest_event_preview),
                timestamp_ms,
                event_type,
                relation_type,
                relation_event_id,
                content_converted,
                is_threaded,
                is_reply,
                has_thread_summary,
                has_reactions,
            };
            (
                Some(latest_event),
                newest_conversation_activity(cached_conversation_activity, conversation_activity),
            )
        }
        matrix_sdk::latest_events::LatestEventValue::LocalHasBeenSent { event_id, value } => {
            let sender_id = room.client().user_id().map(|user_id| user_id.to_string());
            let (raw_content, event_type) = value.content.raw();
            let relation_type = raw_content
                .get_field::<serde_json::Value>("m.relates_to")
                .ok()
                .flatten()
                .and_then(|relates_to| {
                    relates_to
                        .get("rel_type")
                        .and_then(serde_json::Value::as_str)
                        .map(ToOwned::to_owned)
                        .or_else(|| {
                            relates_to
                                .get("m.in_reply_to")
                                .and_then(|reply| reply.get("event_id"))
                                .is_some()
                                .then(|| "m.in_reply_to".to_owned())
                        })
                });
            let timestamp_ms = u64::from(value.timestamp.get());
            let conversation_activity =
                matrix_conversation_activity_source(event_type, relation_type.as_deref()).map(
                    |source| MatrixConversationActivity {
                        timestamp_ms,
                        source,
                    },
                );
            let latest_event = MatrixRoomLatestEventSummary {
                event_id: event_id.to_string(),
                sender_id: sender_id.clone(),
                sender_label: sender_id,
                sender_avatar_mxc_uri: None,
                preview: matrix_local_latest_event_preview(&value.content),
                timestamp_ms,
                event_type: Some(event_type.to_owned()),
                relation_type,
                relation_event_id: None,
                content_converted: false,
                is_threaded: false,
                is_reply: false,
                has_thread_summary: false,
                has_reactions: false,
            };
            (
                Some(latest_event),
                newest_conversation_activity(cached_conversation_activity, conversation_activity),
            )
        }
        matrix_sdk::latest_events::LatestEventValue::LocalIsSending(value) => {
            let (raw_content, event_type) = value.content.raw();
            let relation_type = raw_content
                .get_field::<serde_json::Value>("m.relates_to")
                .ok()
                .flatten()
                .and_then(|relates_to| {
                    relates_to
                        .get("rel_type")
                        .and_then(serde_json::Value::as_str)
                        .map(ToOwned::to_owned)
                });
            let local_activity =
                matrix_conversation_activity_source(event_type, relation_type.as_deref()).map(
                    |source| MatrixConversationActivity {
                        timestamp_ms: u64::from(value.timestamp.get()),
                        source,
                    },
                );
            (
                None,
                newest_conversation_activity(cached_conversation_activity, local_activity),
            )
        }
        matrix_sdk::latest_events::LatestEventValue::None
        | matrix_sdk::latest_events::LatestEventValue::RemoteInvite { .. }
        | matrix_sdk::latest_events::LatestEventValue::LocalCannotBeSent(_) => {
            (None, cached_conversation_activity)
        }
    }
}

async fn matrix_room_cached_conversation_activity(
    room: &matrix_sdk::Room,
) -> Option<MatrixConversationActivity> {
    let (event_cache, _drop_handles) = room.event_cache().await.ok()?;
    event_cache
        .rfind_map_event_in_memory_by(matrix_conversation_activity_from_timeline_event)
        .await
        .ok()
        .flatten()
}

fn matrix_conversation_activity_from_timeline_event(
    timeline_event: &matrix_sdk::deserialized_responses::TimelineEvent,
) -> Option<MatrixConversationActivity> {
    let event_type = matrix_timeline_event_type(timeline_event)?;
    let (relation_type, _) = matrix_timeline_event_relation(timeline_event);
    let source = matrix_conversation_activity_source(&event_type, relation_type.as_deref())?;
    let timestamp_ms = timeline_event
        .timestamp()
        .map(|timestamp| u64::from(timestamp.get()))?;
    Some(MatrixConversationActivity {
        timestamp_ms,
        source,
    })
}

fn newest_conversation_activity(
    left: Option<MatrixConversationActivity>,
    right: Option<MatrixConversationActivity>,
) -> Option<MatrixConversationActivity> {
    match (left, right) {
        (Some(left), Some(right)) if right.timestamp_ms > left.timestamp_ms => Some(right),
        (Some(left), _) => Some(left),
        (None, right) => right,
    }
}

fn matrix_conversation_activity_source(
    event_type: &str,
    relation_type: Option<&str>,
) -> Option<MatrixConversationActivitySource> {
    if matches!(relation_type, Some("m.replace" | "m.annotation")) {
        return None;
    }
    if relation_type == Some("m.thread") {
        return matches!(event_type, "m.room.message" | "m.room.encrypted")
            .then_some(MatrixConversationActivitySource::ThreadReply);
    }
    match event_type {
        "m.room.message" => Some(MatrixConversationActivitySource::Message),
        "m.room.encrypted" => Some(MatrixConversationActivitySource::EncryptedMessage),
        _ => None,
    }
}

async fn matrix_room_member_display(
    room: &matrix_sdk::Room,
    user_id: &matrix_sdk::ruma::UserId,
) -> (Option<String>, Option<String>) {
    match room.get_member_no_sync(user_id).await {
        Ok(Some(member)) => (
            member.display_name().map(ToOwned::to_owned),
            member.avatar_url().map(ToString::to_string),
        ),
        Ok(None) | Err(_) => (None, None),
    }
}

fn matrix_latest_event_preview(
    content: &matrix_sdk_ui::timeline::TimelineItemContent,
) -> Option<String> {
    match content {
        matrix_sdk_ui::timeline::TimelineItemContent::MsgLike(msglike) => match &msglike.kind {
            matrix_sdk_ui::timeline::MsgLikeKind::Message(message) => {
                Some(message.body().to_owned())
            }
            matrix_sdk_ui::timeline::MsgLikeKind::UnableToDecrypt(_) => {
                Some("Unable to decrypt message".to_owned())
            }
            matrix_sdk_ui::timeline::MsgLikeKind::Redacted => Some("Message deleted".to_owned()),
            matrix_sdk_ui::timeline::MsgLikeKind::Sticker(_)
            | matrix_sdk_ui::timeline::MsgLikeKind::Poll(_)
            | matrix_sdk_ui::timeline::MsgLikeKind::Other(_)
            | matrix_sdk_ui::timeline::MsgLikeKind::LiveLocation(_) => content.event_type_str(),
        },
        matrix_sdk_ui::timeline::TimelineItemContent::MembershipChange(_)
        | matrix_sdk_ui::timeline::TimelineItemContent::ProfileChange(_)
        | matrix_sdk_ui::timeline::TimelineItemContent::OtherState(_)
        | matrix_sdk_ui::timeline::TimelineItemContent::FailedToParseMessageLike { .. }
        | matrix_sdk_ui::timeline::TimelineItemContent::FailedToParseState { .. }
        | matrix_sdk_ui::timeline::TimelineItemContent::CallInvite
        | matrix_sdk_ui::timeline::TimelineItemContent::RtcNotification { .. } => {
            content.event_type_str()
        }
    }
}

fn matrix_local_latest_event_preview(
    content: &matrix_sdk::store::SerializableEventContent,
) -> Option<String> {
    let content: matrix_sdk::ruma::events::AnyMessageLikeEventContent =
        content.deserialize().ok()?;
    match content {
        matrix_sdk::ruma::events::AnyMessageLikeEventContent::RoomMessage(message) => {
            Some(message.body().to_owned())
        }
        _ => None,
    }
}

async fn matrix_room_tags(room: &matrix_sdk::Room) -> MatrixRoomTags {
    let tags = room.tags().await.ok().flatten();
    let favourite = tags
        .as_ref()
        .and_then(|tags| tags.get(&matrix_sdk::ruma::events::tag::TagName::Favorite))
        .map(matrix_room_tag_info_from_sdk)
        .or_else(|| {
            room.is_favourite()
                .then_some(MatrixRoomTagInfo { order: None })
        });
    let low_priority = tags
        .as_ref()
        .and_then(|tags| tags.get(&matrix_sdk::ruma::events::tag::TagName::LowPriority))
        .map(matrix_room_tag_info_from_sdk)
        .or_else(|| {
            room.is_low_priority()
                .then_some(MatrixRoomTagInfo { order: None })
        });

    MatrixRoomTags {
        favourite,
        low_priority,
    }
}

fn matrix_room_tag_info_from_sdk(
    info: &matrix_sdk::ruma::events::tag::TagInfo,
) -> MatrixRoomTagInfo {
    MatrixRoomTagInfo {
        order: info.order.map(|order| order.to_string()),
    }
}

pub fn room_attention_summary_from_room(room: &matrix_sdk::Room) -> Option<RoomAttentionSummary> {
    let room_display_name = room.cached_display_name().map(|name| name.to_string())?;
    let unread_notifications = room.unread_notification_counts();

    room_attention_summary_from_counts(
        Some(room_display_name),
        room.is_dm(),
        unread_notifications.notification_count.into(),
        unread_notifications.highlight_count.into(),
        room.num_unread_messages(),
        room.is_marked_unread(),
    )
}

async fn matrix_own_profile_from_session(
    session: &MatrixClientSession,
) -> Result<MatrixOwnProfile, MatrixProfileError> {
    let account = session.client().account();
    let display_name = account
        .get_display_name()
        .await
        .map_err(MatrixProfileError::from_sdk_error)?;
    let avatar_mxc_uri = account
        .get_avatar_url()
        .await
        .map_err(MatrixProfileError::from_sdk_error)?
        .map(|uri| uri.to_string());
    Ok(MatrixOwnProfile {
        display_name,
        avatar_mxc_uri,
    })
}

fn local_user_aliases_event_type() -> GlobalAccountDataEventType {
    GlobalAccountDataEventType::from(LOCAL_USER_ALIASES_ACCOUNT_DATA_TYPE.to_owned())
}

async fn fetch_local_user_aliases_raw(
    session: &MatrixClientSession,
) -> Result<Option<Raw<AnyGlobalAccountDataEventContent>>, MatrixProfileError> {
    let account = session.client().account();
    account
        .fetch_account_data(local_user_aliases_event_type())
        .await
        .map_err(MatrixProfileError::from_sdk_error)
}

fn normalized_local_user_aliases(aliases: BTreeMap<String, String>) -> BTreeMap<String, String> {
    aliases
        .into_iter()
        .filter_map(|(user_id, alias)| {
            if user_id.trim().is_empty() {
                return None;
            }
            normalize_local_user_alias(Some(&alias)).map(|alias| (user_id, alias))
        })
        .collect()
}

fn normalize_local_user_alias(alias: Option<&str>) -> Option<String> {
    alias
        .map(str::trim)
        .filter(|alias| !alias.is_empty())
        .map(ToOwned::to_owned)
}

fn matrix_profile_serialization_error() -> MatrixProfileError {
    MatrixProfileError::Sdk(MatrixProfileFailureKind::Sdk)
}

fn matrix_profile_failure_kind(error: &matrix_sdk::Error) -> MatrixProfileFailureKind {
    match error {
        matrix_sdk::Error::Http(error) => {
            if error
                .as_client_api_error()
                .is_some_and(|error| error.status_code.as_u16() == 403)
                || matches!(
                    error.client_api_error_kind(),
                    Some(matrix_sdk::ruma::api::error::ErrorKind::Forbidden)
                )
            {
                MatrixProfileFailureKind::Forbidden
            } else {
                MatrixProfileFailureKind::Sdk
            }
        }
        matrix_sdk::Error::Timeout => MatrixProfileFailureKind::Network,
        _ => MatrixProfileFailureKind::Sdk,
    }
}

fn matrix_ignored_user_list_failure_kind(
    error: &matrix_sdk::Error,
) -> MatrixIgnoredUserListFailureKind {
    match error {
        matrix_sdk::Error::Http(error) => {
            if error
                .as_client_api_error()
                .is_some_and(|error| error.status_code.as_u16() == 403)
                || matches!(
                    error.client_api_error_kind(),
                    Some(matrix_sdk::ruma::api::error::ErrorKind::Forbidden)
                )
            {
                MatrixIgnoredUserListFailureKind::Forbidden
            } else {
                MatrixIgnoredUserListFailureKind::Sdk
            }
        }
        matrix_sdk::Error::Timeout => MatrixIgnoredUserListFailureKind::Network,
        _ => MatrixIgnoredUserListFailureKind::Sdk,
    }
}

fn matrix_report_failure_kind(error: &matrix_sdk::Error) -> MatrixReportFailureKind {
    match error {
        matrix_sdk::Error::Http(error) => {
            if error
                .as_client_api_error()
                .is_some_and(|error| error.status_code.as_u16() == 403)
                || matches!(
                    error.client_api_error_kind(),
                    Some(matrix_sdk::ruma::api::error::ErrorKind::Forbidden)
                )
            {
                MatrixReportFailureKind::Forbidden
            } else {
                MatrixReportFailureKind::Sdk
            }
        }
        matrix_sdk::Error::Timeout => MatrixReportFailureKind::Network,
        _ => MatrixReportFailureKind::Sdk,
    }
}

async fn matrix_parent_space_ids(room: &matrix_sdk::Room) -> Vec<String> {
    let Ok(parent_spaces) = room.parent_spaces().await else {
        return Vec::new();
    };

    parent_spaces
        .filter_map(|parent_space| async move {
            match parent_space.ok()? {
                ParentSpace::Reciprocal(space) | ParentSpace::WithPowerlevel(space) => {
                    Some(space.room_id().to_string())
                }
                ParentSpace::Illegitimate(_) | ParentSpace::Unverifiable(_) => None,
            }
        })
        .collect()
        .await
}

async fn matrix_space_child_room_ids(room: &matrix_sdk::Room) -> Vec<String> {
    let Ok(child_events) = room
        .get_state_events_static::<SpaceChildEventContent>()
        .await
    else {
        return Vec::new();
    };

    let mut child_room_ids: Vec<String> = child_events
        .into_iter()
        .filter_map(|child_event| match child_event.deserialize() {
            Ok(SyncOrStrippedState::Sync(SyncStateEvent::Original(event))) => {
                Some(event.state_key.to_string())
            }
            Ok(SyncOrStrippedState::Sync(SyncStateEvent::Redacted(_))) => None,
            Ok(SyncOrStrippedState::Stripped(event)) => Some(event.state_key.to_string()),
            Err(_) => None,
        })
        .collect();
    child_room_ids.sort();
    child_room_ids.dedup();
    child_room_ids
}

fn matrix_timeline_item_from_ui(
    room_id: &str,
    item: &matrix_sdk_ui::timeline::TimelineItem,
) -> Option<MatrixTimelineItem> {
    match matrix_timeline_update_from_ui(room_id, item)? {
        MatrixTimelineUpdate::Upsert(item) => Some(item),
        MatrixTimelineUpdate::Remove { .. } => None,
    }
}

fn map_sdk_recovery_state(
    state: matrix_sdk::encryption::recovery::RecoveryState,
) -> E2eeRecoveryState {
    match state {
        matrix_sdk::encryption::recovery::RecoveryState::Unknown => E2eeRecoveryState::Unknown,
        matrix_sdk::encryption::recovery::RecoveryState::Enabled => E2eeRecoveryState::Enabled,
        matrix_sdk::encryption::recovery::RecoveryState::Disabled => E2eeRecoveryState::Disabled,
        matrix_sdk::encryption::recovery::RecoveryState::Incomplete => {
            E2eeRecoveryState::Incomplete
        }
    }
}

pub fn parse_login_discovery_http_response(
    status: u16,
    body: &str,
) -> Result<Vec<LoginFlow>, LoginDiscoveryError> {
    if status != 200 {
        return Err(LoginDiscoveryError::HttpStatus {
            status,
            message: matrix_error_message(body),
        });
    }

    let value = serde_json::from_str::<serde_json::Value>(body)
        .map_err(|error| LoginDiscoveryError::InvalidResponse(error.to_string()))?;
    parse_login_discovery(&value)
}

pub fn parse_login_discovery(
    value: &serde_json::Value,
) -> Result<Vec<LoginFlow>, LoginDiscoveryError> {
    Ok(map_login_flows_to_desktop(parse_matrix_login_flows(value)?))
}

pub fn parse_matrix_login_flows(
    value: &serde_json::Value,
) -> Result<Vec<MatrixLoginFlow>, LoginDiscoveryError> {
    if !value.get("flows").is_some_and(serde_json::Value::is_array) {
        return Err(LoginDiscoveryError::MissingFlows);
    }

    let response = serde_json::from_value::<LoginDiscoveryResponse>(value.clone())
        .map_err(|error| LoginDiscoveryError::InvalidResponse(error.to_string()))?;

    Ok(response
        .flows
        .into_iter()
        .map(|flow| MatrixLoginFlow {
            kind: parse_flow_kind(flow.flow_type),
            delegated_oidc_compatibility: flow.delegated_oidc_compatibility,
            display_name: flow.display_name,
        })
        .collect())
}

pub fn map_login_flows_to_desktop(flows: Vec<MatrixLoginFlow>) -> Vec<LoginFlow> {
    flows
        .into_iter()
        .map(|flow| LoginFlow {
            kind: match flow.kind {
                MatrixLoginFlowKind::Password => LoginFlowKind::Password,
                MatrixLoginFlowKind::Sso => LoginFlowKind::Sso,
                MatrixLoginFlowKind::Oidc => LoginFlowKind::Oidc,
                MatrixLoginFlowKind::Token => LoginFlowKind::Token,
                MatrixLoginFlowKind::Unknown(value) => LoginFlowKind::Unknown(value),
            },
            delegated_oidc_compatibility: flow.delegated_oidc_compatibility,
            display_name: flow.display_name,
        })
        .collect()
}

fn parse_flow_kind(flow_type: String) -> MatrixLoginFlowKind {
    match flow_type.as_str() {
        "m.login.password" => MatrixLoginFlowKind::Password,
        "m.login.sso" => MatrixLoginFlowKind::Sso,
        "m.login.oidc" | "m.login.oauth2" => MatrixLoginFlowKind::Oidc,
        "m.login.token" => MatrixLoginFlowKind::Token,
        _ => MatrixLoginFlowKind::Unknown(flow_type),
    }
}

fn is_loopback_homeserver(url: &Url) -> bool {
    let Some(host) = url.host_str() else {
        return false;
    };

    host.eq_ignore_ascii_case("localhost")
        || host
            .parse::<IpAddr>()
            .is_ok_and(|address| address.is_loopback())
}

fn matrix_error_message(body: &str) -> String {
    serde_json::from_str::<MatrixErrorResponse>(body)
        .ok()
        .and_then(|response| response.error)
        .filter(|message| !message.trim().is_empty())
        .unwrap_or_else(|| "homeserver did not return login flows".to_owned())
}

#[cfg(test)]
mod tests {
    use std::{collections::BTreeMap, path::PathBuf};

    use super::{
        LOCAL_USER_ALIASES_ACCOUNT_DATA_TYPE, MatrixConversationActivity,
        MatrixConversationActivitySource, MatrixCreateRoomOptions, MatrixCreateRoomParentSpace,
        MatrixCreateRoomVisibility, MatrixEventCacheError, MatrixLocalUserAliases,
        MatrixPublicRoomDirectoryQuery, MatrixPublicRoomDirectoryRoom, MatrixRoomHistoryVisibility,
        MatrixRoomJoinRule, MatrixRoomMemberRole, MatrixRoomModerationAction,
        MatrixRoomPermissionFacts, MatrixRoomSettingChange, MatrixRoomSettingsSnapshot,
        MatrixRoomTagInfo, MatrixRoomTags, MatrixSearchIndexKey, MatrixSearchIndexStoreConfig,
        SdkUnreadTrace, create_public_directory_room, create_room_request,
        get_room_settings_snapshot, join_room_by_alias, matrix_conversation_activity_source,
        matrix_room_list_room_from_counts, matrix_room_member_role, moderate_room_member,
        newest_conversation_activity, normalized_local_user_aliases, query_public_room_directory,
        room_settings_snapshot_with_change, room_settings_snapshot_with_member_power_level,
        trace_sdk_conversation_activity, trace_sdk_unread_snapshot, update_room_member_power_level,
        update_room_setting,
    };

    #[test]
    fn conversation_activity_classifies_messages_encryption_and_threads_only() {
        use MatrixConversationActivitySource::{EncryptedMessage, Message, ThreadReply};

        let cases = [
            ("m.room.message", None, Some(Message)),
            ("m.room.message", Some("m.in_reply_to"), Some(Message)),
            ("m.room.message", Some("m.thread"), Some(ThreadReply)),
            ("m.room.encrypted", None, Some(EncryptedMessage)),
            ("m.room.encrypted", Some("m.thread"), Some(ThreadReply)),
            ("m.room.message", Some("m.replace"), None),
            ("m.room.message", Some("m.annotation"), None),
            ("m.room.redaction", None, None),
            ("m.room.member", None, None),
            ("m.room.name", None, None),
            ("m.reaction", Some("m.annotation"), None),
            ("m.receipt", None, None),
            ("m.typing", None, None),
            ("m.presence", None, None),
        ];

        for (event_type, relation_type, expected) in cases {
            assert_eq!(
                matrix_conversation_activity_source(event_type, relation_type),
                expected,
                "unexpected classification for {event_type} / {relation_type:?}"
            );
        }
    }

    #[test]
    fn conversation_activity_keeps_the_newest_cache_or_local_candidate() {
        let cached = super::MatrixConversationActivity {
            timestamp_ms: 41,
            source: MatrixConversationActivitySource::EncryptedMessage,
        };
        let local = super::MatrixConversationActivity {
            timestamp_ms: 42,
            source: MatrixConversationActivitySource::Message,
        };

        assert_eq!(
            newest_conversation_activity(Some(cached), Some(local)),
            Some(local)
        );
        assert_eq!(
            newest_conversation_activity(Some(cached), None),
            Some(cached)
        );
        assert_eq!(newest_conversation_activity(None, None), None);
    }

    #[test]
    fn conversation_activity_debug_hides_raw_timestamp() {
        let activity = MatrixConversationActivity {
            timestamp_ms: 42,
            source: MatrixConversationActivitySource::ThreadReply,
        };

        let debug = format!("{activity:?}");

        assert!(debug.contains("ThreadReply"), "{debug}");
        assert!(!debug.contains("42"), "{debug}");
    }

    #[test]
    fn client_builder_defaults_enable_thread_subscriptions() {
        let source = include_str!("lib.rs");
        let defaults_body = source
            .split("fn desktop_client_builder_defaults")
            .nth(1)
            .expect("desktop client builder defaults helper should exist")
            .split("pub async fn recover_e2ee")
            .next()
            .expect("recover_e2ee should follow desktop client builder defaults");

        assert!(defaults_body.contains("with_threading_support"));
        assert!(defaults_body.contains("ThreadingSupport::Enabled"));
        assert!(defaults_body.contains("with_subscriptions: true"));
    }

    #[test]
    fn client_builder_defaults_download_backup_keys_after_decryption_failures() {
        let source = include_str!("lib.rs");
        let defaults_body = source
            .split("fn desktop_client_builder_defaults")
            .nth(1)
            .expect("desktop client builder defaults helper should exist")
            .split("pub async fn recover_e2ee")
            .next()
            .expect("recover_e2ee should follow desktop client builder defaults");

        assert!(defaults_body.contains("with_encryption_settings"));
        assert!(defaults_body.contains("BackupDownloadStrategy::AfterDecryptionFailure"));
    }

    #[test]
    fn typed_peer_policy_is_all_devices_not_only_trusted() {
        assert!(matches!(
            super::desktop_room_key_recipient_strategy(),
            matrix_sdk_base::crypto::CollectStrategy::AllDevices
        ));
    }

    #[test]
    fn send_wrapper_propagates_recipient_collection_failure() {
        assert_eq!(
            super::map_room_send_result(Err(matrix_sdk::Error::NoOlmMachine)),
            Err(super::MatrixRoomOperationError::Sdk(
                super::MatrixRoomOperationFailureKind::Encryption
            ))
        );
    }

    #[test]
    fn mark_room_as_read_sends_read_marker_with_private_receipt() {
        let source = include_str!("lib.rs");
        let body = source
            .split("pub async fn mark_room_as_read")
            .nth(1)
            .expect("mark_room_as_read should exist")
            .split("pub async fn mark_room_as_unread")
            .next()
            .expect("mark_room_as_unread should follow mark_room_as_read");

        assert!(
            body.contains("send_multiple_receipts"),
            "mark_room_as_read must persist the read marker and unread-count receipt through one SDK request"
        );
        assert!(
            body.contains("fully_read_marker"),
            "mark_room_as_read must update the user's fully-read marker"
        );
        assert!(
            body.contains("private_read_receipt"),
            "mark_room_as_read must reset unread counts without publishing a public read receipt"
        );
        assert!(
            !body.contains("send_single_receipt(ReceiptType::FullyRead"),
            "fully-read alone must not be treated as sufficient to clear persistent unread counts"
        );
    }

    #[test]
    fn joined_room_list_prefers_async_direct_dm_detection() {
        let source = include_str!("lib.rs");
        let projection_body = source
            .split("async fn matrix_room_list_snapshot_from_rooms")
            .nth(1)
            .expect("joined room projection should exist")
            .split("async fn matrix_direct_account_data_targets_by_room")
            .next()
            .expect("direct account data helper should follow joined room projection");

        assert!(
            projection_body.contains("room.is_direct().await"),
            "joined room projection should read m.direct via async Room::is_direct"
        );
        assert!(
            projection_body.contains("unwrap_or_else(|_| room.is_dm())"),
            "joined room projection should fall back to cached is_dm when direct lookup fails"
        );
    }

    #[test]
    fn joined_room_list_snapshot_avoids_full_member_scans() {
        let source = include_str!("lib.rs");
        let projection_body = source
            .split("async fn matrix_room_list_snapshot_from_rooms")
            .nth(1)
            .expect("joined room projection should exist")
            .split("async fn matrix_direct_account_data_targets_by_room")
            .next()
            .expect("direct account data helper should follow joined room projection");

        assert!(projection_body.contains("room.joined_members_count()"));
        assert!(projection_body.contains("matrix_space_member_user_ids_no_sync(&room).await"));
        assert!(!projection_body.contains("collect_active_member_profiles"));
        assert!(
            !projection_body.contains("room.members(matrix_sdk::RoomMemberships::ACTIVE)"),
            "room-list projection must not load the full active member list"
        );
        assert!(
            !projection_body.contains("joined_user_ids"),
            "room-list projection should not derive joined members from joined_user_ids"
        );
    }

    #[test]
    fn joined_room_list_dm_resolution_uses_account_data_cached_and_heroes_candidates() {
        let source = include_str!("lib.rs");
        let helper_body = source
            .split("async fn matrix_room_list_dm_user_ids")
            .nth(1)
            .expect("DM resolution helper should exist")
            .split("async fn matrix_space_member_user_ids_no_sync")
            .next()
            .expect("space member helper should follow DM resolution helper");

        assert!(
            helper_body.contains("direct_targets_by_room.get(&room_id)"),
            "DM resolution should prefer direct account-data targets first"
        );
        assert!(
            helper_body.contains(".direct_targets()"),
            "DM resolution should fall back to cached SDK direct targets"
        );
        assert!(
            helper_body.contains("room.heroes()"),
            "DM resolution should use heroes when the room is already considered a DM"
        );
        assert!(
            helper_body.contains("get_member_no_sync"),
            "DM resolution should only probe candidate members without syncing the full list"
        );
        assert!(
            helper_body.contains("dm_user_ids.push(candidate_user_id_string.clone())"),
            "DM resolution should preserve valid direct/cached/hero IDs even when a local member profile is absent"
        );
        assert!(
            !helper_body.contains("room.members(matrix_sdk::RoomMemberships::ACTIVE)"),
            "DM resolution helper must not hide a full active-member scan behind the hot path"
        );
        assert!(
            !helper_body.contains("room.members_no_sync(matrix_sdk::RoomMemberships::ACTIVE)"),
            "DM resolution helper must not enumerate every active member even without syncing"
        );
    }

    #[test]
    fn space_member_ids_are_no_sync_and_space_only() {
        let source = include_str!("lib.rs");
        let helper_body = source
            .split("async fn matrix_space_member_user_ids_no_sync")
            .nth(1)
            .expect("space member helper should exist")
            .split("async fn matrix_invite_previews_from_rooms")
            .next()
            .expect("invite previews should follow space member helper");

        assert!(
            helper_body.contains("members_no_sync(matrix_sdk::RoomMemberships::ACTIVE)"),
            "space membership may read only already-synced local state"
        );
        assert!(
            !helper_body.contains("room.members(matrix_sdk::RoomMemberships::ACTIVE)"),
            "space membership helper must not sync/fetch the full member list"
        );
    }

    #[test]
    fn matrix_room_member_summaries_still_scans_full_members() {
        let source = include_str!("lib.rs");
        let helper_body = source
            .split("async fn matrix_room_member_summaries")
            .nth(1)
            .expect("member summary helper should exist")
            .split("fn matrix_user_trust_state_from_sdk_identity")
            .next()
            .expect("trust-state helper should follow member summary helper");

        assert!(
            helper_body.contains("room.members(matrix_sdk::RoomMemberships::ACTIVE)"),
            "member summaries should still be allowed to load the full active member list"
        );
    }

    #[test]
    fn direct_account_data_targets_are_indexed_by_room() {
        use matrix_sdk::ruma::{
            OwnedRoomId, OwnedUserId,
            events::direct::{DirectEventContent, OwnedDirectUserIdentifier},
        };

        let alice: OwnedUserId = "@alice:example.invalid".try_into().unwrap();
        let bob: OwnedUserId = "@bob:example.invalid".try_into().unwrap();
        let dm_room: OwnedRoomId = "!dm:example.invalid".try_into().unwrap();
        let other_room: OwnedRoomId = "!other:example.invalid".try_into().unwrap();
        let mut content = DirectEventContent::default();
        content.insert(
            OwnedDirectUserIdentifier::from(alice),
            vec![dm_room.clone(), other_room.clone()],
        );
        content.insert(OwnedDirectUserIdentifier::from(bob), vec![dm_room.clone()]);

        let by_room = super::direct_account_data_targets_by_room(&content);

        assert_eq!(
            by_room.get(dm_room.as_str()),
            Some(&vec![
                "@alice:example.invalid".to_owned(),
                "@bob:example.invalid".to_owned()
            ])
        );
        assert_eq!(
            by_room.get(other_room.as_str()),
            Some(&vec!["@alice:example.invalid".to_owned()])
        );
    }

    #[test]
    fn event_cache_error_is_private_data_free() {
        let error = MatrixEventCacheError::SubscribeFailed;

        assert_eq!(error.to_string(), "Matrix event cache subscription failed");
        assert_eq!(format!("{error:?}"), "SubscribeFailed");
    }

    #[test]
    fn direct_account_data_dm_detection_fetches_server_when_store_misses() {
        let source = include_str!("lib.rs");
        let helper_body = source
            .split("async fn matrix_direct_account_data_targets_by_room")
            .nth(1)
            .expect("direct account data helper should exist")
            .split("fn direct_account_data_targets_by_room")
            .next()
            .expect("pure direct account data helper should follow async helper");

        assert!(helper_body.contains("account_data::<DirectEventContent>()"));
        assert!(helper_body.contains("fetch_account_data_static::<DirectEventContent>()"));
    }

    #[test]
    fn search_index_store_config_uses_encrypted_ngram_index() {
        let config = MatrixSearchIndexStoreConfig::new(
            PathBuf::from("search-index"),
            MatrixSearchIndexKey::new("synthetic-search-key"),
        );

        let kind = config.as_sdk_store_kind();

        assert!(matches!(
            kind,
            matrix_sdk::search_index::SearchIndexStoreKind::EncryptedDirectoryWithConfig(_, _, _)
        ));
    }

    #[test]
    fn local_user_aliases_account_data_serde_uses_private_flat_map() {
        let mut aliases = BTreeMap::new();
        aliases.insert(
            "@alice:example.invalid".to_owned(),
            "Local Alice".to_owned(),
        );
        let content = MatrixLocalUserAliases {
            aliases: aliases.clone(),
        };

        let value = serde_json::to_value(&content).expect("serialize local aliases");
        assert_eq!(
            LOCAL_USER_ALIASES_ACCOUNT_DATA_TYPE,
            "app.koushi.local_aliases"
        );
        assert_eq!(
            value["aliases"]["@alice:example.invalid"],
            serde_json::json!("Local Alice")
        );

        let parsed: MatrixLocalUserAliases =
            serde_json::from_value(value).expect("deserialize local aliases");
        assert_eq!(parsed.aliases, aliases);
    }

    #[test]
    fn local_user_aliases_debug_is_artifact_safe() {
        let content = MatrixLocalUserAliases {
            aliases: BTreeMap::from([(
                "@alice:example.invalid".to_owned(),
                "Local Alice".to_owned(),
            )]),
        };

        let debug = format!("{content:?}");

        assert!(debug.contains("MatrixLocalUserAliases"));
        assert!(debug.contains("alias_count"));
        assert!(!debug.contains("@alice:example.invalid"));
        assert!(!debug.contains("Local Alice"));
    }

    #[test]
    fn local_user_aliases_debug_redacts_user_ids_and_aliases() {
        let content = MatrixLocalUserAliases {
            aliases: BTreeMap::from([(
                "@alice:example.invalid".to_owned(),
                "Local Alice".to_owned(),
            )]),
        };

        let debug = format!("{content:?}");

        assert!(debug.contains("MatrixLocalUserAliases"));
        assert!(debug.contains("alias_count"));
        assert!(!debug.contains("@alice:example.invalid"));
        assert!(!debug.contains("Local Alice"));
    }

    #[test]
    fn normalized_local_user_aliases_trims_and_drops_empty_entries() {
        let aliases = BTreeMap::from([
            (
                "@alice:example.invalid".to_owned(),
                "  Local Alice  ".to_owned(),
            ),
            ("@bob:example.invalid".to_owned(), "   ".to_owned()),
        ]);

        let normalized = normalized_local_user_aliases(aliases);

        assert_eq!(
            normalized,
            BTreeMap::from([(
                "@alice:example.invalid".to_owned(),
                "Local Alice".to_owned()
            )])
        );
    }

    #[test]
    fn room_list_room_from_counts_carries_notification_metadata() {
        let room = matrix_room_list_room_from_counts(
            "!room:example.invalid".to_owned(),
            "Room".to_owned(),
            None,
            true,
            vec!["@alice:example.invalid".to_owned()],
            MatrixRoomTags::default(),
            4,
            2,
            4,
            false,
            None,
            None,
            None,
            None,
            None,
            vec!["!space:example.invalid".to_owned()],
            false,
            2,
        );

        assert_eq!(room.notification_count, 4);
        assert_eq!(room.highlight_count, 2);
        assert_eq!(room.unread_count, 4);
        assert_eq!(room.joined_members, 2);
        assert!(room.is_dm);
    }

    #[test]
    fn room_list_room_from_counts_carries_room_tags() {
        let tags = MatrixRoomTags {
            favourite: Some(MatrixRoomTagInfo {
                order: Some("0.25".to_owned()),
            }),
            low_priority: None,
        };

        let room = matrix_room_list_room_from_counts(
            "!room:example.invalid".to_owned(),
            "Room".to_owned(),
            None,
            false,
            Vec::new(),
            tags.clone(),
            0,
            0,
            0,
            false,
            None,
            None,
            None,
            None,
            None,
            vec![],
            false,
            0,
        );

        assert_eq!(room.tags, tags);
    }

    #[test]
    fn unread_diagnostic_snapshot_rejects_private_synthetic_inputs() {
        let latest_event = Some(crate::MatrixRoomLatestEventSummary {
            event_id: "$event:example.invalid".to_owned(),
            sender_id: Some("@user:example.invalid".to_owned()),
            sender_label: None,
            sender_avatar_mxc_uri: None,
            preview: Some("secret message".to_owned()),
            timestamp_ms: 42,
            event_type: Some("m.room.message".to_owned()),
            relation_type: None,
            relation_event_id: None,
            content_converted: true,
            is_threaded: false,
            is_reply: false,
            has_thread_summary: false,
            has_reactions: false,
        });
        trace_sdk_unread_snapshot(SdkUnreadTrace {
            unread_messages: 2,
            unread_count: 2,
            notification_count: 1,
            highlight_count: 1,
            marked_unread: true,
            latest_event: &latest_event,
            fully_read_event_id: Some("$event:example.invalid"),
            private_read_receipt_event_id: None,
            recency_stamp_present: true,
            conversation_activity: Some(MatrixConversationActivity {
                timestamp_ms: 3,
                source: MatrixConversationActivitySource::Message,
            }),
        });
        trace_sdk_conversation_activity(
            Some(MatrixConversationActivity {
                timestamp_ms: 3,
                source: MatrixConversationActivitySource::Message,
            }),
            true,
        );

        let serialized = serde_json::to_string(&koushi_diagnostics::snapshot()).unwrap();
        assert!(serialized.contains("conversation_activity_source"));
        assert!(serialized.contains("activity_sort_bucket"));
        for forbidden in [
            "!room:example.invalid",
            "@user:example.invalid",
            "$event:example.invalid",
            "/Users/alice/private",
            "secret message",
        ] {
            assert!(
                !serialized.contains(forbidden),
                "serialized diagnostics leaked {forbidden}"
            );
        }
        assert!(serialized.contains("unread_messages"));
        assert!(serialized.contains("latest_event_present"));
    }

    fn test_latest_event(event_id: &str) -> crate::MatrixRoomLatestEventSummary {
        crate::MatrixRoomLatestEventSummary {
            event_id: event_id.to_owned(),
            sender_id: None,
            sender_label: None,
            sender_avatar_mxc_uri: None,
            preview: None,
            timestamp_ms: 42,
            event_type: Some("m.room.message".to_owned()),
            relation_type: None,
            relation_event_id: None,
            content_converted: true,
            is_threaded: false,
            is_reply: false,
            has_thread_summary: false,
            has_reactions: false,
        }
    }

    #[test]
    fn room_list_room_from_counts_suppresses_stale_unread_when_fully_read_matches_latest_event() {
        let room = matrix_room_list_room_from_counts(
            "!room:example.invalid".to_owned(),
            "Room".to_owned(),
            None,
            false,
            Vec::new(),
            MatrixRoomTags::default(),
            2,
            1,
            2,
            false,
            Some(42),
            None,
            Some(test_latest_event("$latest:example.invalid")),
            Some("$latest:example.invalid".to_owned()),
            None,
            vec![],
            false,
            2,
        );

        assert_eq!(room.unread_count, 0);
        assert_eq!(room.notification_count, 0);
        assert_eq!(room.highlight_count, 0);
        assert!(!room.marked_unread);
    }

    #[test]
    fn room_list_room_from_counts_preserves_unread_when_read_marker_differs_from_latest_event() {
        let room = matrix_room_list_room_from_counts(
            "!room:example.invalid".to_owned(),
            "Room".to_owned(),
            None,
            false,
            Vec::new(),
            MatrixRoomTags::default(),
            2,
            1,
            2,
            false,
            Some(42),
            None,
            Some(test_latest_event("$latest:example.invalid")),
            Some("$older:example.invalid".to_owned()),
            None,
            vec![],
            false,
            2,
        );

        assert_eq!(room.unread_count, 2);
        assert_eq!(room.notification_count, 2);
        assert_eq!(room.highlight_count, 1);
    }

    #[test]
    fn create_room_request_projects_space_room_options() {
        let request = create_room_request(MatrixCreateRoomOptions {
            name: "Synthetic Ops".to_owned(),
            topic: Some("Deployment notes".to_owned()),
            alias_localpart: None,
            encrypted: true,
            visibility: MatrixCreateRoomVisibility::Private,
            parent_space: Some(MatrixCreateRoomParentSpace {
                space_id: "!space:example.invalid".to_owned(),
                via_server: "example.invalid".to_owned(),
            }),
        })
        .expect("request should build");

        assert_eq!(request.name.as_deref(), Some("Synthetic Ops"));
        assert_eq!(request.topic.as_deref(), Some("Deployment notes"));
        assert_eq!(
            request
                .room_version
                .as_ref()
                .map(|version| version.as_str()),
            Some("9")
        );
        let initial_state = initial_state_json(&request);
        assert!(initial_state.iter().any(|event| {
            event.get("type").and_then(serde_json::Value::as_str) == Some("m.room.encryption")
        }));
        assert!(initial_state.iter().any(|event| {
            event.get("type").and_then(serde_json::Value::as_str) == Some("m.space.parent")
                && event.get("state_key").and_then(serde_json::Value::as_str)
                    == Some("!space:example.invalid")
        }));
        let join_rules = initial_state
            .iter()
            .find(|event| {
                event.get("type").and_then(serde_json::Value::as_str) == Some("m.room.join_rules")
            })
            .expect("join rules");
        assert_eq!(
            join_rules
                .get("content")
                .and_then(|content| content.get("join_rule"))
                .and_then(serde_json::Value::as_str),
            Some("restricted")
        );
        let history_visibility = initial_state
            .iter()
            .find(|event| {
                event.get("type").and_then(serde_json::Value::as_str)
                    == Some("m.room.history_visibility")
            })
            .expect("history visibility");
        assert_eq!(
            history_visibility
                .get("content")
                .and_then(|content| content.get("history_visibility"))
                .and_then(serde_json::Value::as_str),
            Some("invited")
        );
    }

    #[test]
    fn create_room_request_projects_public_alias_without_encryption() {
        let request = create_room_request(MatrixCreateRoomOptions {
            name: "Synthetic Public".to_owned(),
            topic: None,
            alias_localpart: Some("synthetic-public".to_owned()),
            encrypted: true,
            visibility: MatrixCreateRoomVisibility::Public,
            parent_space: None,
        })
        .expect("request should build");

        assert_eq!(request.room_alias_name.as_deref(), Some("synthetic-public"));
        assert_eq!(
            request.visibility,
            matrix_sdk::ruma::api::client::room::Visibility::Public
        );
        let initial_state = initial_state_json(&request);
        assert!(!initial_state.iter().any(|event| {
            event.get("type").and_then(serde_json::Value::as_str) == Some("m.room.encryption")
        }));
    }

    fn initial_state_json(
        request: &matrix_sdk::ruma::api::client::room::create_room::v3::Request,
    ) -> Vec<serde_json::Value> {
        request
            .initial_state
            .iter()
            .map(|event| {
                serde_json::from_str::<serde_json::Value>(event.json().get())
                    .expect("initial state event JSON")
            })
            .collect()
    }

    #[test]
    fn room_tag_operations_use_sdk_tag_methods() {
        let source = include_str!("lib.rs");

        assert!(source.contains("set_is_favourite(true"));
        assert!(source.contains("set_is_favourite(false"));
        assert!(source.contains("set_is_low_priority(true"));
        assert!(source.contains("set_is_low_priority(false"));
    }

    #[test]
    fn pin_operations_use_sdk_pinned_event_methods() {
        let source = include_str!("lib.rs");
        let pin_body = source
            .split("pub async fn pin_event")
            .nth(1)
            .expect("pin_event wrapper")
            .split("pub async fn unpin_event")
            .next()
            .expect("pin_event body");
        let unpin_body = source
            .split("pub async fn unpin_event")
            .nth(1)
            .expect("unpin_event wrapper")
            .split("pub async fn edit_text_message")
            .next()
            .expect("unpin_event body");

        assert!(pin_body.contains(".pin_event(&event_id)"));
        assert!(unpin_body.contains(".unpin_event(&event_id)"));
    }

    #[test]
    fn directory_operations_use_public_room_and_alias_join_apis() {
        let _query = MatrixPublicRoomDirectoryQuery {
            term: Some("synthetic".to_owned()),
            server_name: Some("example.invalid".to_owned()),
            limit: Some(10),
            since: None,
        };
        let _room = MatrixPublicRoomDirectoryRoom {
            room_id: "!room:example.invalid".to_owned(),
            canonical_alias: Some("#room:example.invalid".to_owned()),
            name: "Synthetic Room".to_owned(),
            topic: None,
            avatar_url: None,
            joined_members: 1,
            world_readable: true,
            guest_can_join: false,
        };
        let _query_fn = query_public_room_directory;
        let _join_fn = join_room_by_alias;
        let _create_public_fn = create_public_directory_room;
    }

    #[test]
    fn room_management_wrappers_use_settings_privacy_and_moderation_apis() {
        let _snapshot = MatrixRoomSettingsSnapshot {
            room_id: "!room:example.invalid".to_owned(),
            name: Some("Synthetic Room".to_owned()),
            topic: Some("Synthetic topic".to_owned()),
            avatar_url: None,
            canonical_alias: None,
            alternate_aliases: Vec::new(),
            join_rule: MatrixRoomJoinRule::Invite,
            history_visibility: MatrixRoomHistoryVisibility::Shared,
            permissions: MatrixRoomPermissionFacts {
                can_edit_settings: true,
                can_edit_roles: true,
                can_kick: true,
                can_ban: true,
                can_unban: false,
            },
            members: vec![super::MatrixRoomMemberSummary {
                user_id: "@member:example.invalid".to_owned(),
                display_name: Some("Synthetic Member".to_owned()),
                avatar_url: None,
                power_level: Some(50),
                role: MatrixRoomMemberRole::Moderator,
                user_trust: None,
            }],
        };
        let _change = MatrixRoomSettingChange::JoinRule(MatrixRoomJoinRule::Public);
        let _moderation = MatrixRoomModerationAction::Kick;
        let _snapshot_fn = get_room_settings_snapshot;
        let _update_fn = update_room_setting;
        let _moderate_fn = moderate_room_member;
        let _role_fn = update_room_member_power_level;

        let source = include_str!("lib.rs");
        assert!(source.contains(".set_name("));
        assert!(source.contains(".set_room_topic("));
        assert!(source.contains(".set_avatar_url("));
        assert!(source.contains(".remove_avatar("));
        assert!(source.contains(".privacy_settings()"));
        assert!(source.contains(".update_join_rule("));
        assert!(source.contains(".update_room_history_visibility("));
        assert!(source.contains(".kick_user("));
        assert!(source.contains(".ban_user("));
        assert!(source.contains(".unban_user("));
        assert!(source.contains(".update_power_levels("));
    }

    #[test]
    fn room_setting_update_projects_the_sent_change_into_the_success_snapshot() {
        let original = MatrixRoomSettingsSnapshot {
            room_id: "!room:example.invalid".to_owned(),
            name: Some("Original Room".to_owned()),
            topic: Some("Original topic".to_owned()),
            avatar_url: Some("mxc://example.invalid/original".to_owned()),
            canonical_alias: None,
            alternate_aliases: Vec::new(),
            join_rule: MatrixRoomJoinRule::Invite,
            history_visibility: MatrixRoomHistoryVisibility::Shared,
            permissions: MatrixRoomPermissionFacts {
                can_edit_settings: true,
                can_edit_roles: true,
                can_kick: true,
                can_ban: true,
                can_unban: true,
            },
            members: vec![],
        };

        assert_eq!(
            room_settings_snapshot_with_change(
                original.clone(),
                &MatrixRoomSettingChange::Topic(Some("Updated topic".to_owned())),
            )
            .topic
            .as_deref(),
            Some("Updated topic")
        );
        assert_eq!(
            room_settings_snapshot_with_change(
                original.clone(),
                &MatrixRoomSettingChange::Name(None),
            )
            .name,
            None
        );
        assert_eq!(
            room_settings_snapshot_with_change(
                original.clone(),
                &MatrixRoomSettingChange::AvatarUrl(None),
            )
            .avatar_url,
            None
        );
        assert_eq!(
            room_settings_snapshot_with_change(
                original.clone(),
                &MatrixRoomSettingChange::JoinRule(MatrixRoomJoinRule::Public),
            )
            .join_rule,
            MatrixRoomJoinRule::Public
        );
        assert_eq!(
            room_settings_snapshot_with_change(
                original,
                &MatrixRoomSettingChange::HistoryVisibility(MatrixRoomHistoryVisibility::Joined,),
            )
            .history_visibility,
            MatrixRoomHistoryVisibility::Joined
        );
    }

    #[test]
    fn room_member_power_level_projection_updates_role_in_success_snapshot() {
        let original = MatrixRoomSettingsSnapshot {
            room_id: "!room:example.invalid".to_owned(),
            name: Some("Original Room".to_owned()),
            topic: Some("Original topic".to_owned()),
            avatar_url: None,
            canonical_alias: None,
            alternate_aliases: Vec::new(),
            join_rule: MatrixRoomJoinRule::Invite,
            history_visibility: MatrixRoomHistoryVisibility::Shared,
            permissions: MatrixRoomPermissionFacts {
                can_edit_settings: true,
                can_edit_roles: true,
                can_kick: true,
                can_ban: true,
                can_unban: true,
            },
            members: vec![super::MatrixRoomMemberSummary {
                user_id: "@member:example.invalid".to_owned(),
                display_name: Some("Synthetic Member".to_owned()),
                avatar_url: None,
                power_level: Some(0),
                role: MatrixRoomMemberRole::User,
                user_trust: None,
            }],
        };

        let updated =
            room_settings_snapshot_with_member_power_level(original, "@member:example.invalid", 50);
        let member = updated.members.first().expect("member summary");
        assert_eq!(member.power_level, Some(50));
        assert_eq!(member.role, MatrixRoomMemberRole::Moderator);
        assert_eq!(
            matrix_room_member_role(Some(100)),
            MatrixRoomMemberRole::Administrator
        );
        assert_eq!(matrix_room_member_role(None), MatrixRoomMemberRole::Creator);
    }
}
