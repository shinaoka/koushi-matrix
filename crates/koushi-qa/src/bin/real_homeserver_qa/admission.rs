use super::config::SYNC_TIMEOUT;
use super::event_source::{QaEventDeadline, QaSnapshotEventSource};
use super::{
    AccountCommand, AccountEvent, AccountKey, AuthSecret, CoreCommand, CoreConnection, CoreEvent,
    RecoveryRequest, RequestId, SessionState,
};
use std::{future::Future, pin::Pin};

pub(super) trait AdmissionSource: QaSnapshotEventSource {
    fn next_request_id(&mut self) -> RequestId;
    fn command(
        &mut self,
        command: CoreCommand,
    ) -> Pin<Box<dyn Future<Output = Result<(), String>> + Send + '_>>;
}
impl AdmissionSource for CoreConnection {
    fn next_request_id(&mut self) -> RequestId {
        CoreConnection::next_request_id(self)
    }
    fn command(
        &mut self,
        command: CoreCommand,
    ) -> Pin<Box<dyn Future<Output = Result<(), String>> + Send + '_>> {
        Box::pin(async move {
            CoreConnection::command(self, command)
                .await
                .map_err(|_| "admission command rejected".to_owned())
        })
    }
}

#[derive(Debug, Eq, PartialEq)]
pub(super) struct AdmissionOutcome {
    pub account_key: AccountKey,
    pub recovered: bool,
}

pub(super) async fn wait_for_admission<S: AdmissionSource>(
    conn: &mut S,
    request_id: RequestId,
    expected: Option<&AccountKey>,
    secret: Option<&AuthSecret>,
    label: &str,
) -> Result<AdmissionOutcome, String> {
    let deadline = QaEventDeadline::after(SYNC_TIMEOUT);
    let mut terminal_key: Option<AccountKey> = None;
    let mut recovery_id = None;
    let mut recovered_key: Option<AccountKey> = None;
    loop {
        let snapshot = conn.snapshot();
        if let SessionState::AwaitingVerification { gate, .. } = &snapshot.session {
            use koushi_state::{VerificationAccountKind, VerificationMethodCapability};
            if recovery_id.is_none() {
                if gate.account_kind != VerificationAccountKind::ExistingIdentity
                    || !gate.methods.iter().any(|method| {
                        matches!(
                            method,
                            VerificationMethodCapability::RecoveryKey
                                | VerificationMethodCapability::SecurityPhrase
                        )
                    })
                {
                    return Err(format!("{label}: recovery method unavailable"));
                }
                let secret = secret.ok_or_else(|| format!("{label}: recovery secret required"))?;
                let id = conn.next_request_id();
                tokio::time::timeout_at(
                    deadline.instant,
                    conn.command(CoreCommand::Account(AccountCommand::SubmitRecovery {
                        request_id: id,
                        request: RecoveryRequest {
                            secret: secret.clone(),
                        },
                    })),
                )
                .await
                .map_err(|_| format!("{label}: recovery submit timed out"))??;
                recovery_id = Some(id);
            }
        }
        if let Some(key) = &terminal_key {
            if expected.is_some_and(|expected| expected != key)
                || recovered_key
                    .as_ref()
                    .is_some_and(|recovered| recovered != key)
            {
                return Err(format!("{label}: account mismatch"));
            }
            if let SessionState::Ready(info) = &snapshot.session {
                if info.user_id != key.0 {
                    return Err(format!("{label}: Ready account mismatch"));
                }
                if recovery_id.is_none() || recovered_key.is_some() {
                    if recovery_id.is_some() {
                        println!("recovery=completed");
                    }
                    return Ok(AdmissionOutcome {
                        account_key: key.clone(),
                        recovered: recovery_id.is_some(),
                    });
                }
            }
        }
        let event = deadline
            .recv(conn)
            .await
            .map_err(|_| format!("{label}: admission timed out"))?
            .map_err(|_| format!("{label}: event stream lagged"))?;
        match event {
            CoreEvent::Account(AccountEvent::LoggedIn {
                request_id: id,
                account_key,
            }) if expected.is_none() && id == request_id => terminal_key = Some(account_key),
            CoreEvent::Account(AccountEvent::SessionRestored {
                request_id: id,
                account_key,
            }) if expected.is_some() && id == request_id => terminal_key = Some(account_key),
            CoreEvent::Account(AccountEvent::RecoveryCompleted {
                request_id: id,
                account_key,
            }) if Some(id) == recovery_id => recovered_key = Some(account_key),
            CoreEvent::OperationFailed { request_id: id, .. }
                if id == request_id || Some(id) == recovery_id =>
            {
                return Err(format!("{label}: admission failed"));
            }
            _ => {}
        }
    }
}

/// A failed admission still owns a provisional server session. Revoke it using
/// this connection before dropping the runtime; it may not yet be persisted.
pub(super) async fn cleanup_failed_admission(conn: &mut CoreConnection) {
    let request_id = conn.next_request_id();
    let deadline = QaEventDeadline::after(SYNC_TIMEOUT);
    let result = async {
        tokio::time::timeout_at(
            deadline.instant,
            conn.command(CoreCommand::Account(AccountCommand::Logout { request_id })),
        )
        .await
        .map_err(|_| ())?
        .map_err(|_| ())?;
        let mut logged_out = false;
        loop {
            if logged_out && matches!(conn.snapshot().session, SessionState::SignedOut) {
                return Ok::<(), ()>(());
            }
            match deadline.recv(conn).await.map_err(|_| ())?.map_err(|_| ())? {
                CoreEvent::Account(AccountEvent::LoggedOut { request_id: id, .. })
                    if id == request_id =>
                {
                    logged_out = true
                }
                CoreEvent::OperationFailed { request_id: id, .. } if id == request_id => {
                    return Err(());
                }
                _ => {}
            }
        }
    }
    .await;
    if result.is_ok() {
        println!("admission_cleanup=ok");
    } else {
        eprintln!("cleanup_warning=admission_logout_failed");
    }
}

#[cfg(test)]
#[path = "admission_tests.rs"]
mod tests;
