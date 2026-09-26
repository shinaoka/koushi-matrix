use super::*;
#[cfg(test)]
use crate::commands::contracts::fake_request_id;

fn admit_frontend_session_status_trigger(
    trigger: koushi_state::SessionStatusRefreshTrigger,
) -> Result<koushi_state::SessionStatusRefreshTrigger, String> {
    if trigger == koushi_state::SessionStatusRefreshTrigger::Recovery {
        Err("recovery refresh is core-owned".to_owned())
    } else {
        Ok(trigger)
    }
}

#[tauri::command]
pub async fn refresh_current_session_status(
    trigger: koushi_state::SessionStatusRefreshTrigger,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendCommandAdmission, String> {
    let trigger = admit_frontend_session_status_trigger(trigger)?;
    let request_id = next_request_id(state.inner()).await;
    let admission = submit_core_command_with_admission(
        state.inner(),
        CoreCommand::Account(AccountCommand::RefreshCurrentSessionStatus {
            request_id,
            trigger,
        }),
    )
    .await?;
    Ok(admission)
}

#[tauri::command]
pub async fn load_account_management_capabilities(
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendCommandAdmission, String> {
    let request_id = next_request_id(state.inner()).await;
    let admission = submit_core_command_with_admission(
        state.inner(),
        CoreCommand::Account(AccountCommand::LoadAccountManagementCapabilities { request_id }),
    )
    .await?;
    Ok(admission)
}

#[tauri::command]
pub async fn change_password(
    new_password: String,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendCommandAdmission, String> {
    let request_id = next_request_id(state.inner()).await;
    let admission = submit_core_command_with_admission(
        state.inner(),
        CoreCommand::Account(AccountCommand::ChangePassword {
            request_id,
            new_password: AuthSecret::new(new_password),
        }),
    )
    .await?;
    Ok(admission)
}

#[tauri::command]
pub async fn deactivate_account(
    erase_data: bool,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendCommandAdmission, String> {
    let request_id = next_request_id(state.inner()).await;
    let admission = submit_core_command_with_admission(
        state.inner(),
        CoreCommand::Account(AccountCommand::DeactivateAccount {
            request_id,
            erase_data,
        }),
    )
    .await?;
    Ok(admission)
}

#[tauri::command]
pub async fn submit_account_management_uia(
    flow_id: u64,
    password: String,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendCommandAdmission, String> {
    let request_id = next_request_id(state.inner()).await;
    let admission = submit_core_command_with_admission(
        state.inner(),
        build_submit_account_management_uia_command(request_id, flow_id, AuthSecret::new(password)),
    )
    .await?;
    update_qa_window_title_from_state(&app, state.inner()).await;
    Ok(admission)
}

// ── Account notification settings (#981) ────────────────────────────────────

async fn submit_account_notifications(
    state: &CoreRuntimeState,
    request: koushi_protocol::command::AccountNotificationsRequest,
) -> Result<FrontendCommandAdmission, String> {
    let request_id = next_request_id(state).await;
    submit_core_command_with_admission(
        state,
        build_account_notifications_command(request_id, request),
    )
    .await
}

pub(super) fn build_account_notifications_command(
    request_id: koushi_protocol::RequestId,
    request: koushi_protocol::command::AccountNotificationsRequest,
) -> CoreCommand {
    CoreCommand::Account(AccountCommand::AccountNotifications {
        request_id,
        request,
    })
}

/// Read-only load of the account's notification rules, emails and pushers.
#[tauri::command]
pub async fn load_account_notifications(
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendCommandAdmission, String> {
    submit_account_notifications(
        state.inner(),
        koushi_protocol::command::AccountNotificationsRequest::Load,
    )
    .await
}

#[tauri::command]
pub async fn set_notification_category(
    category: koushi_state::NotificationCategory,
    enabled: bool,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendCommandAdmission, String> {
    submit_account_notifications(
        state.inner(),
        koushi_protocol::command::AccountNotificationsRequest::SetCategory { category, enabled },
    )
    .await
}

#[tauri::command]
pub async fn set_account_push_enabled(
    enabled: bool,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendCommandAdmission, String> {
    submit_account_notifications(
        state.inner(),
        koushi_protocol::command::AccountNotificationsRequest::SetAccountPush { enabled },
    )
    .await
}

#[tauri::command]
pub async fn request_notification_email_token(
    address: String,
    lang: String,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendCommandAdmission, String> {
    submit_account_notifications(
        state.inner(),
        koushi_protocol::command::AccountNotificationsRequest::RequestEmailToken {
            address,
            lang: admit_notification_lang(lang),
        },
    )
    .await
}

#[tauri::command]
pub async fn resend_notification_email_token(
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendCommandAdmission, String> {
    submit_account_notifications(
        state.inner(),
        koushi_protocol::command::AccountNotificationsRequest::ResendEmailToken,
    )
    .await
}

#[tauri::command]
pub async fn confirm_notification_email(
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendCommandAdmission, String> {
    submit_account_notifications(
        state.inner(),
        koushi_protocol::command::AccountNotificationsRequest::ConfirmEmail,
    )
    .await
}

#[tauri::command]
pub async fn submit_notification_email_uia(
    flow_id: u64,
    password: String,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendCommandAdmission, String> {
    submit_account_notifications(
        state.inner(),
        koushi_protocol::command::AccountNotificationsRequest::SubmitUia {
            flow_id,
            auth: IdentityResetAuthRequest::UiaaPassword {
                password: AuthSecret::new(password),
            },
        },
    )
    .await
}

#[tauri::command]
pub async fn cancel_notification_email(
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendCommandAdmission, String> {
    submit_account_notifications(
        state.inner(),
        koushi_protocol::command::AccountNotificationsRequest::CancelPendingEmail,
    )
    .await
}

#[tauri::command]
pub async fn enable_email_notifications(
    address: String,
    lang: String,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendCommandAdmission, String> {
    submit_account_notifications(
        state.inner(),
        koushi_protocol::command::AccountNotificationsRequest::EnableEmailNotifications {
            address,
            lang: admit_notification_lang(lang),
        },
    )
    .await
}

#[tauri::command]
pub async fn disable_email_notifications(
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendCommandAdmission, String> {
    submit_account_notifications(
        state.inner(),
        koushi_protocol::command::AccountNotificationsRequest::DisableEmailNotifications,
    )
    .await
}

/// Digest language hint for the email pusher: a short BCP 47-ish tag only.
fn admit_notification_lang(lang: String) -> String {
    let valid = !lang.is_empty()
        && lang.len() <= 16
        && lang
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || character == '-');
    if valid { lang } else { "en".to_owned() }
}

pub(super) fn build_start_device_cleanup_command(
    request_id: koushi_protocol::RequestId,
) -> CoreCommand {
    CoreCommand::Account(AccountCommand::StartDeviceCleanup { request_id })
}

pub(super) fn build_submit_device_cleanup_uia_command(
    request_id: koushi_protocol::RequestId,
    flow_id: u64,
    password: AuthSecret,
) -> CoreCommand {
    CoreCommand::Account(AccountCommand::SubmitDeviceCleanupUia {
        request_id,
        flow_id,
        password,
    })
}

pub(super) fn build_erase_device_cleanup_local_data_anyway_command(
    request_id: koushi_protocol::RequestId,
) -> CoreCommand {
    CoreCommand::Account(AccountCommand::EraseDeviceCleanupLocalDataAnyway { request_id })
}

pub(super) fn build_submit_account_management_uia_command(
    request_id: koushi_protocol::RequestId,
    flow_id: u64,
    password: AuthSecret,
) -> CoreCommand {
    CoreCommand::Account(AccountCommand::SubmitAccountManagementUia {
        request_id,
        flow_id,
        auth: IdentityResetAuthRequest::UiaaPassword { password },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frontend_cannot_forge_the_core_owned_recovery_trigger() {
        assert!(
            admit_frontend_session_status_trigger(koushi_state::SessionStatusRefreshTrigger::Open)
                .is_ok()
        );
        assert!(
            admit_frontend_session_status_trigger(
                koushi_state::SessionStatusRefreshTrigger::Manual
            )
            .is_ok()
        );
        assert!(
            admit_frontend_session_status_trigger(
                koushi_state::SessionStatusRefreshTrigger::Recovery
            )
            .is_err()
        );
    }

    #[test]
    fn device_cleanup_commands_route_to_the_provisional_account_state_machine() {
        let request_id = fake_request_id(49);
        assert!(matches!(
            build_start_device_cleanup_command(request_id),
            CoreCommand::Account(AccountCommand::StartDeviceCleanup {
                request_id: actual
            }) if actual == request_id
        ));
        match build_submit_device_cleanup_uia_command(
            request_id,
            41,
            AuthSecret::new("private-password"),
        ) {
            CoreCommand::Account(AccountCommand::SubmitDeviceCleanupUia {
                request_id: actual,
                flow_id,
                password,
            }) => {
                assert_eq!(actual, request_id);
                assert_eq!(flow_id, 41);
                assert_eq!(password.expose_secret(), "private-password");
            }
            other => panic!("unexpected command: {other:?}"),
        }
        assert!(matches!(
            build_erase_device_cleanup_local_data_anyway_command(request_id),
            CoreCommand::Account(AccountCommand::EraseDeviceCleanupLocalDataAnyway {
                request_id: actual
            }) if actual == request_id
        ));
    }
}
