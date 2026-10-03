use super::*;

#[tauri::command]
pub async fn probe_local_encryption_health(
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendCommandAdmission, String> {
    let request_id = next_request_id(state.inner()).await;
    let admission = submit_core_command_with_admission(
        state.inner(),
        build_probe_local_encryption_health_command(request_id),
    )
    .await?;
    update_qa_window_title_from_state(&app, state.inner()).await;
    Ok(admission)
}

#[tauri::command]
pub async fn reset_local_data(
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendCommandAdmission, String> {
    let request_id = next_request_id(state.inner()).await;
    koushi_diagnostics::record_and_stderr(
        DiagnosticEvent::new(
            DiagnosticLevel::Info,
            "desktop.local_data_reset",
            "submitted",
        )
        .field(DiagnosticField::request_id(
            "request_id",
            request_id.connection_id.0,
            request_id.sequence,
        )),
    );
    let admission = submit_core_command_with_admission(
        state.inner(),
        build_reset_local_data_command(request_id),
    )
    .await?;
    koushi_diagnostics::record_and_stderr(
        DiagnosticEvent::new(
            DiagnosticLevel::Info,
            "desktop.local_data_reset",
            "admitted",
        )
        .field(DiagnosticField::request_id(
            "request_id",
            request_id.connection_id.0,
            request_id.sequence,
        )),
    );
    update_qa_window_title_from_state(&app, state.inner()).await;
    Ok(admission)
}

pub(super) fn build_probe_local_encryption_health_command(
    request_id: koushi_protocol::RequestId,
) -> CoreCommand {
    CoreCommand::Account(AccountCommand::ProbeLocalEncryptionHealth { request_id })
}

pub(super) fn build_reset_local_data_command(
    request_id: koushi_protocol::RequestId,
) -> CoreCommand {
    CoreCommand::Account(AccountCommand::ResetLocalData { request_id })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::contracts::fake_request_id;

    #[test]
    fn credential_health_command_routes_to_account_state_machine() {
        match build_probe_local_encryption_health_command(fake_request_id(47)) {
            CoreCommand::Account(AccountCommand::ProbeLocalEncryptionHealth { request_id }) => {
                assert_eq!(request_id, fake_request_id(47));
            }
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn reset_local_data_command_routes_to_account_state_machine() {
        match build_reset_local_data_command(fake_request_id(48)) {
            CoreCommand::Account(AccountCommand::ResetLocalData { request_id }) => {
                assert_eq!(request_id, fake_request_id(48));
            }
            other => panic!("unexpected command: {other:?}"),
        }
    }
}
