//! Production Core command proof. Installation bypasses credential UI only;
//! every request, response, retry and import uses production Core and real SSS.
use super::*;
use koushi_core::{CoreConnection, CoreRuntime, runtime::IsolatedStores};
use koushi_protocol::command::{CoreCommand, RoomCommand, SyncCommand};
use koushi_state::{
    AppAction, CurrentDeviceTrustState, HistoryKeyRecoveryStatus as Status, SyncState,
};
use matrix_sdk::ruma::RoomId;
use matrix_sdk::ruma::events::room::join_rules::{JoinRule, RoomJoinRulesEventContent};

#[path = "history_key_recovery_rate.rs"]
mod rate;
#[path = "history_key_recovery_security.rs"]
mod security;

struct CoreFixture {
    runtime: Option<CoreRuntime>,
    connection: Option<CoreConnection>,
    _stores: IsolatedStores,
}
impl CoreFixture {
    fn conn(&self) -> &CoreConnection {
        self.connection.as_ref().expect("live fixture connection")
    }
    async fn start(participant: &mut Participant) -> Result<Self, String> {
        participant.stop_sync().await?;
        let session = participant.session();
        let (runtime, stores) = CoreRuntime::start_isolated();
        let connection = runtime.attach();
        if !runtime
            .install_account_session_for_testing(Arc::new(session.clone()))
            .await
        {
            drop(connection);
            runtime.shutdown().await;
            return Err("history recovery: Core session installation failed".to_owned());
        }
        runtime
            .inject_actions(vec![
                AppAction::AppStarted,
                AppAction::RestoreSessionSucceeded(session.info.clone()),
                AppAction::CurrentDeviceTrustChanged(CurrentDeviceTrustState::Verified),
            ])
            .await;
        let mut fixture = Self {
            runtime: Some(runtime),
            connection: Some(connection),
            _stores: stores,
        };
        if let Err(error) = fixture.sync(true).await {
            let _ = fixture.stop().await;
            return Err(error);
        }
        Ok(fixture)
    }
    async fn sync(&self, start: bool) -> Result<(), String> {
        let request_id = self.conn().next_request_id();
        self.conn()
            .command(CoreCommand::Sync(if start {
                SyncCommand::Start { request_id }
            } else {
                SyncCommand::Stop { request_id }
            }))
            .await
            .map_err(|_| "history recovery: sync command rejected")?;
        wait_until("Core sync state", || async {
            Ok(if start {
                matches!(self.conn().snapshot().sync, SyncState::Running)
            } else {
                matches!(self.conn().snapshot().sync, SyncState::Stopped)
            })
        })
        .await
    }
    async fn stop(&mut self) -> Result<(), String> {
        self.connection.take();
        if let Some(runtime) = self.runtime.take() {
            tokio::time::timeout(Duration::from_secs(20), runtime.shutdown_checked())
                .await
                .map_err(|_| "history recovery: Core shutdown timed out")?
                .map_err(|_| "history recovery: Core shutdown failed")?;
        }
        Ok(())
    }
    async fn request(
        &self,
        history: &HistoricalRoom,
        force_all: bool,
        recipients: u16,
    ) -> Result<(), String> {
        let previous = self
            .conn()
            .snapshot()
            .room_management
            .history_keys
            .get(history.room_id.as_str())
            .map_or(0, |s| s.generation);
        let request_id = self.conn().next_request_id();
        self.conn()
            .command(CoreCommand::Room(RoomCommand::RequestHistoryKeys {
                request_id,
                room_id: history.room_id.to_string(),
                force_all,
            }))
            .await
            .map_err(|_| "history recovery: request command rejected")?;
        wait_until("Core request sent", || async {
            let snapshot = self.conn().snapshot();
            match snapshot
                .room_management
                .history_keys
                .get(history.room_id.as_str())
            {
                Some(s) if s.generation > previous => match s.status {
                    Status::Requested {
                        recipients: actual, ..
                    } if actual == recipients => Ok(true),
                    Status::Failed { .. } | Status::Partial { .. } => {
                        Err("history recovery: request failed".to_owned())
                    }
                    _ => Ok(false),
                },
                _ => Ok(false),
            }
        })
        .await
    }
}

async fn assert_unverified_peer(client: &Client, peer: &Client) -> Result<(), String> {
    let identity = client
        .encryption()
        .request_user_identity(peer.user_id().ok_or("history recovery: peer unavailable")?)
        .await
        .map_err(|_| "history recovery: identity refresh failed")?
        .ok_or("history recovery: peer identity missing")?;
    let device = client
        .encryption()
        .get_device(
            peer.user_id().ok_or("history recovery: peer unavailable")?,
            peer.device_id()
                .ok_or("history recovery: peer device missing")?,
        )
        .await
        .map_err(|_| "history recovery: peer device lookup failed")?
        .ok_or("history recovery: peer device unavailable")?;
    if identity.is_verified()
        || identity.has_verification_violation()
        || !device.is_cross_signed_by_owner()
        || device.is_verified_with_cross_signing()
    {
        return Err("history recovery: expected cross-signed unverified peer".to_owned());
    }
    Ok(())
}
async fn join(participant: &Participant, room: &OwnedRoomId) -> Result<(), String> {
    participant
        .session()
        .client()
        .join_room_by_id(room)
        .await
        .map_err(|err| join_failure(&err))?;
    Ok(())
}
/// Keep the reason the join failed in the stage line without rendering the
/// SDK's message: `matrix_sdk::Error` is `#[error(transparent)]` down to
/// server-supplied text, request URLs and local paths, none of which may reach
/// a QA artifact.
fn join_failure(err: &matrix_sdk::Error) -> String {
    format!(
        "history recovery: join without invite failed: {}",
        join_failure_reason(err)
    )
}
/// A closed classification of a `join_room_by_id` failure: a fixed token for the
/// homeserver's errcode, `transport` when the request never produced a
/// response, `http_<status>` or `http_no_errcode` for any other HTTP-layer
/// failure, and `sdk` for anything else. Only booleans, an enum variant of a
/// closed set and the numeric status are read; never the errcode string or the
/// response body.
fn join_failure_reason(err: &matrix_sdk::Error) -> String {
    match err {
        matrix_sdk::Error::Http(http) => http_failure_reason(http),
        _ => "sdk".to_owned(),
    }
}
fn http_failure_reason(http: &matrix_sdk::HttpError) -> String {
    if let Some(kind) = http.client_api_error_kind() {
        return errcode_token(kind).to_owned();
    }
    match http {
        // The request never got a response: connect, DNS, TLS or socket.
        matrix_sdk::HttpError::Reqwest(_) => "transport".to_owned(),
        matrix_sdk::HttpError::Cached(http) => http_failure_reason(http),
        _ => http_status(http).map_or_else(
            || "http_no_errcode".to_owned(),
            |status| format!("http_{status}"),
        ),
    }
}
/// A fixed token for the errcodes this scenario can explain. Ruma keeps an
/// unrecognised errcode as an arbitrary server-supplied string, so the errcode
/// itself is matched as a variant and never rendered: anything outside this set
/// collapses to `http_unknown_errcode`.
fn errcode_token(kind: &matrix_sdk::ruma::api::error::ErrorKind) -> &'static str {
    use matrix_sdk::ruma::api::error::ErrorKind as K;
    match kind {
        K::Forbidden => "forbidden",
        K::Unauthorized => "unauthorized",
        K::MissingToken => "missing_token",
        K::UnknownToken(_) => "unknown_token",
        K::GuestAccessForbidden => "guest_access_forbidden",
        K::UserDeactivated => "user_deactivated",
        K::UserSuspended => "user_suspended",
        K::ResourceLimitExceeded(_) => "resource_limit_exceeded",
        K::LimitExceeded(_) => "limit_exceeded",
        K::BadState => "bad_state",
        K::NotFound => "not_found",
        K::UnableToAuthorizeJoin => "unable_to_authorize_join",
        K::UnableToGrantJoin => "unable_to_grant_join",
        K::IncompatibleRoomVersion(_) => "incompatible_room_version",
        K::Unknown => "unknown",
        _ => "http_unknown_errcode",
    }
}
/// The response status of a Matrix error body, and nothing from the body.
fn http_status(http: &matrix_sdk::HttpError) -> Option<u16> {
    let matrix_sdk::HttpError::Api(error) = http else {
        return None;
    };
    match error.as_ref() {
        matrix_sdk::ruma::api::error::FromHttpResponseError::Server(
            matrix_sdk::ruma::api::client::uiaa::UiaaResponse::MatrixError(error),
        ) => Some(error.status_code.as_u16()),
        _ => None,
    }
}
async fn public_history(sender: &Participant) -> Result<HistoricalRoom, String> {
    let history = create_history(sender).await?;
    sender
        .session()
        .client()
        .get_room(&history.room_id)
        .ok_or("history recovery: sender room missing")?
        .send_state_event(RoomJoinRulesEventContent::new(JoinRule::Public))
        .await
        .map_err(|_| "history recovery: public history setting failed")?;
    Ok(history)
}
async fn ledger(client: &Client) -> Result<serde_json::Value, String> {
    let bytes = client
        .encryption()
        .application_crypto_data("chat.koushi.history_keys.v1")
        .await
        .map_err(|_| "history recovery: ledger read failed")?
        .ok_or("history recovery: ledger missing")?;
    serde_json::from_slice(&bytes).map_err(|_| "history recovery: ledger malformed".to_owned())
}
async fn startup_record(
    core: &CoreFixture,
    client: &Client,
    history: &HistoricalRoom,
) -> Result<(u64, serde_json::Value), String> {
    if core
        .runtime
        .as_ref()
        .ok_or("history recovery: runtime missing")?
        .history_recovery_barrier_for_testing()
        .await
        .is_none()
    {
        return Err("history recovery: startup barrier failed".to_owned());
    }
    wait_until("startup request projection", || async {
        Ok(core
            .conn()
            .snapshot()
            .room_management
            .history_keys
            .contains_key(history.room_id.as_str()))
    })
    .await?;
    let record = ledger(client).await?["rooms"][history.room_id.as_str()].clone();
    Ok((
        record["generation"]
            .as_u64()
            .ok_or("history recovery: generation missing")?,
        record,
    ))
}

async fn flow(
    config: &QaConfig,
    sender: &mut Participant,
    recipient: &mut Participant,
    regular: &mut Participant,
    expired: &HistoricalRoom,
) -> Result<(), String> {
    let small = public_history(sender).await?;
    join(regular, &small.room_id).await?;
    join(recipient, &small.room_id).await?;
    if historical_messages_decrypt(&recipient.session().client(), &small).await? {
        return Err("history recovery: history available before request".to_owned());
    }
    for (client, peer) in [
        (sender.session().client(), recipient.session().client()),
        (recipient.session().client(), sender.session().client()),
        (regular.session().client(), recipient.session().client()),
        (recipient.session().client(), regular.session().client()),
    ] {
        assert_unverified_peer(&client, &peer).await?;
    }
    let mut a = CoreFixture::start(sender).await?;
    a.sync(false).await?;
    let mut b = match CoreFixture::start(recipient).await {
        Ok(b) => b,
        Err(e) => {
            let _ = a.stop().await;
            return Err(e);
        }
    };
    let mut c = match CoreFixture::start(regular).await {
        Ok(c) => c,
        Err(e) => {
            let _ = b.stop().await;
            let _ = a.stop().await;
            return Err(e);
        }
    };
    let result = async {
        b.request(&small, false, 2).await?;
        if historical_messages_decrypt(&recipient.session().client(), &small).await? {
            return Err("history recovery: offline administrator unexpectedly replied".to_owned());
        }
        println!("history_recovery_offline_pending=ok");
        // Simulate elapsed transport-coalescing time only. Nonces, peer pins,
        // keys and all request/response contents remain production-generated.
        let mut saved = ledger(&recipient.session().client()).await?;
        let room = &mut saved["rooms"][small.room_id.as_str()];
        room["sent_at"] = serde_json::json!(0);
        room["auto_at"] = serde_json::json!(0);
        let before = room["attempts"].clone();
        recipient
            .session()
            .client()
            .encryption()
            .set_application_crypto_data(
                "chat.koushi.history_keys.v1",
                &serde_json::to_vec(&saved)
                    .map_err(|_| "history recovery: ledger encoding failed")?,
            )
            .await
            .map_err(|_| "history recovery: elapsed-time fixture failed")?;
        b.stop().await?;
        recipient.restart_with_sync(false).await?;
        b = CoreFixture::start(recipient).await?;
        let (generation, restarted) =
            startup_record(&b, &recipient.session().client(), &small).await?;
        let administrator = sender
            .session()
            .client()
            .user_id()
            .ok_or("history recovery: administrator missing")?
            .to_string();
        if restarted["dispatch"]
            .as_array()
            .is_none_or(|pins| pins.len() != 1 || pins[0]["user"] != administrator)
            || restarted["auto_at"].as_u64().unwrap_or(0) == 0
        {
            return Err("history recovery: startup did not target only administrators".to_owned());
        }
        let attempts = restarted["attempts"]
            .as_object()
            .ok_or("history recovery: attempts missing")?;
        let pending_admin = attempts
            .values()
            .find(|a| a["pin"]["user"] == administrator)
            .ok_or("history recovery: administrator attempt missing")?;
        let original_admin = before
            .as_object()
            .ok_or("history recovery: prior attempts missing")?
            .values()
            .find(|a| a["pin"]["user"] == administrator)
            .ok_or("history recovery: original administrator attempt missing")?;
        if pending_admin["nonce"] != original_admin["nonce"] {
            return Err("history recovery: queued administrator correlation lost".to_owned());
        }
        println!("history_recovery_startup_admin_retry=ok");
        b.stop().await?;
        recipient.restart_with_sync(false).await?;
        b = CoreFixture::start(recipient).await?;
        let (second_generation, second) =
            startup_record(&b, &recipient.session().client(), &small).await?;
        if second_generation != generation || second["auto_at"] != restarted["auto_at"] {
            return Err("history recovery: restart cooldown was not persisted".to_owned());
        }
        println!("history_recovery_restart_cooldown=ok");
        a.sync(true).await?;
        let client = recipient.session().client();
        wait_until("post-join historical decryption", || {
            historical_messages_decrypt(&client, &small)
        })
        .await?;
        println!("history_recovery_join_without_invite=ok");
        assert_unverified_peer(&sender.session().client(), &client).await?;
        assert_unverified_peer(&client, &sender.session().client()).await?;
        println!("history_recovery_without_manual_verification=ok");
        let large_id = RoomId::parse(
            config
                .history_large_room
                .as_deref()
                .ok_or("history recovery: large fixture missing")?,
        )
        .map_err(|_| "history recovery: large room invalid")?;
        let sender_client = sender.session().client();
        wait_until("large fixture room", || async {
            Ok(sender_client.get_room(&large_id).is_some())
        })
        .await?;
        let large = create_history_in_room(
            sender_client
                .get_room(&large_id)
                .ok_or("history recovery: large room unavailable")?,
        )
        .await?;
        join(recipient, &large.room_id).await?;
        b.request(&large, false, 1).await?;
        println!("history_recovery_31_members_admin=ok");
        b.request(&large, true, 2).await?;
        println!("history_recovery_force_all=ok");
        wait_until("large historical decryption", || {
            historical_messages_decrypt(&client, &large)
        })
        .await?;
        b.request(expired, false, 1).await?;
        wait_until("explicit recovery after invitation expiry", || {
            historical_messages_decrypt(&client, expired)
        })
        .await?;
        println!("history_recovery_after_window_expiry=ok");
        drop(client);
        drop(sender_client);
        rate::run(sender, recipient, &mut a, &b).await?;
        security::run(config, sender, recipient, &mut a, &mut b).await?;
        Ok(())
    }
    .await;
    let cleanup_b = b.stop().await;
    let cleanup_a = a.stop().await;
    let cleanup_c = c.stop().await;
    result?;
    cleanup_b?;
    cleanup_a?;
    cleanup_c
}

pub(super) async fn run(
    config: &QaConfig,
    sender: &mut Participant,
    recipient: &mut Participant,
    expired: &HistoricalRoom,
) -> Result<(), String> {
    let root = tempfile::tempdir().map_err(|_| "history recovery: fixture directory failed")?;
    let mut regular = login(
        config,
        root.path(),
        "regular",
        config
            .user_c
            .as_deref()
            .ok_or("history recovery: regular user missing")?,
        config
            .password_c
            .as_deref()
            .ok_or("history recovery: regular credential missing")?,
    )
    .await?;
    let result = async {
        bootstrap(
            &mut regular,
            config
                .password_c
                .as_deref()
                .ok_or("history recovery: regular credential missing")?,
        )
        .await?;
        flow(config, sender, recipient, &mut regular, expired).await
    }
    .await;
    let cleanup = regular.cleanup().await;
    root.close()
        .map_err(|_| "history recovery: fixture directory cleanup failed")?;
    result?;
    cleanup
}

#[cfg(test)]
mod tests {
    use super::{join_failure, join_failure_reason};
    use matrix_sdk::{
        Error, HttpError, RefreshTokenError,
        ruma::{
            api::{
                client::uiaa::UiaaResponse,
                error::{
                    Error as ApiError, ErrorBody, ErrorKind, FromHttpResponseError,
                    StandardErrorBody,
                },
            },
            exports::http::StatusCode,
        },
    };

    const PREFIX: &str = "history recovery: join without invite failed: ";

    fn api_error(status: StatusCode, body: ErrorBody) -> Error {
        Error::Http(Box::new(HttpError::Api(Box::new(
            FromHttpResponseError::Server(UiaaResponse::MatrixError(ApiError::new(status, body))),
        ))))
    }

    fn refused() -> Error {
        api_error(
            StatusCode::FORBIDDEN,
            ErrorBody::Standard(StandardErrorBody::new(
                ErrorKind::Forbidden,
                "the join was refused".to_owned(),
            )),
        )
    }

    fn unknown_errcode(errcode: &str) -> Error {
        let body = serde_json::from_value(serde_json::json!({
            "errcode": errcode,
            "error": "the server refused the join",
        }))
        .expect("a standard error body with an unrecognised errcode");
        api_error(StatusCode::FORBIDDEN, ErrorBody::Standard(body))
    }

    fn body_without_errcode() -> Error {
        api_error(
            StatusCode::BAD_REQUEST,
            ErrorBody::Json(serde_json::json!({})),
        )
    }

    fn no_errcode_and_no_status() -> HttpError {
        HttpError::RefreshToken(RefreshTokenError::RefreshTokenRequired)
    }

    fn http_error(http: HttpError) -> Error {
        Error::Http(Box::new(http))
    }

    #[test]
    fn a_refused_join_reports_the_homeserver_errcode() {
        assert_eq!(join_failure_reason(&refused()), "forbidden");
    }

    #[test]
    fn an_unrecognised_errcode_collapses_to_a_fixed_token() {
        // Ruma keeps an unknown errcode as the server sent it, so this is the
        // shape an identifier or a secret would arrive in.
        for errcode in [
            "@member:example.org",
            "X_SECRET_/Users/member/store",
            "M_VERY_LONG_CUSTOM_ERRCODE",
            &"A".repeat(400),
        ] {
            let err = unknown_errcode(errcode);
            // The errcode is reachable from the error and the SDK renders it,
            // so a classification that reported the errcode string would leak.
            assert!(err.to_string().contains(errcode), "{err}");
            let message = join_failure(&err);
            assert_eq!(
                message,
                format!("{PREFIX}http_unknown_errcode"),
                "{message}"
            );
            assert!(!message.contains(errcode), "{message}");
        }
    }

    #[test]
    fn an_error_body_without_an_errcode_reports_the_status_not_a_network_failure() {
        assert_eq!(join_failure_reason(&body_without_errcode()), "http_400");
    }

    #[test]
    fn an_http_failure_with_neither_errcode_nor_status_is_http_no_errcode() {
        assert_eq!(
            join_failure_reason(&http_error(no_errcode_and_no_status())),
            "http_no_errcode"
        );
        let cached = http_error(HttpError::Cached(std::sync::Arc::new(
            no_errcode_and_no_status(),
        )));
        assert_eq!(join_failure_reason(&cached), "http_no_errcode");
    }

    #[test]
    fn a_non_http_failure_is_an_unclassified_sdk_error() {
        assert_eq!(
            join_failure(&Error::AuthenticationRequired),
            format!("{PREFIX}sdk")
        );
    }

    #[test]
    fn every_reason_is_a_closed_token_after_the_stage_and_never_the_sdk_message() {
        for err in [
            refused(),
            unknown_errcode("@member:example.org"),
            body_without_errcode(),
            http_error(no_errcode_and_no_status()),
            Error::AuthenticationRequired,
            Error::InsufficientData,
            Error::ConcurrentRequestFailed,
        ] {
            let message = join_failure(&err);
            assert!(message.starts_with(PREFIX), "{message}");
            let reason = &message[PREFIX.len()..];
            assert!(
                reason
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_'),
                "{reason}"
            );
            // The stage keeps a reason, but never the SDK's own rendering of
            // it, which can carry a URL, a local path or server text.
            assert!(!message.contains(&err.to_string()), "{message}");
        }
    }
}
