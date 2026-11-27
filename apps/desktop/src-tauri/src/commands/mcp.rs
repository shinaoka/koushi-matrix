//! Commands for the AI assistants settings page and approval dialog (#1536).
//! The consent broker owns every decision; these only forward the user's
//! answers and return closed outcome codes.

use std::sync::atomic::Ordering;

use koushi_mcp_host::consent::{AnswerOutcome, GrantSubject, McpAssistantsState, ProfileId};
use koushi_mcp_host::registration::{Platform, RegistrationClient};
use koushi_mcp_host::{Need, ProfileError, ProfileGrants};
use tauri::{AppHandle, Manager, State};
use tauri_plugin_opener::OpenerExt;

use crate::mcp::McpController;
use crate::mcp::extension::{self, ExtensionOutcome};
use crate::mcp::register::{self, RegisterOutcome};

/// The current state, after re-reading the grant store and the agent profiles
/// so a hand edit, a role change or a deleted secret file shows.
#[tauri::command]
pub async fn get_mcp_assistants_state(
    state: State<'_, McpController>,
) -> Result<McpAssistantsState, ()> {
    state.broker.refresh_profiles(&*state.accounts).await;
    Ok(state.broker.refresh())
}

/// The permissions a new profile starts with. The webview names them; the
/// host decides whether the account may be bound at all.
#[derive(serde::Deserialize)]
pub struct ProfileGrantsInput {
    read: bool,
    send: bool,
    attachments: bool,
    message_actions: bool,
    draft: bool,
}

/// The outcome of creating a profile. Neither the secret nor the path of its
/// `0600` file is here; the launch snippets in the state carry the path.
#[derive(serde::Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CreateProfileOutcome {
    Created { profile_id: String },
    Failed { reason: &'static str },
}

#[derive(serde::Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RevokeProfileOutcome {
    Revoked,
    NotFound,
    Failed,
}

/// Owner action: creates a profile bound to an agent account. Not reachable
/// from remote content (no capability grants it) and not an IPC method.
#[tauri::command]
pub async fn create_agent_profile(
    state: State<'_, McpController>,
    label: String,
    account_id: String,
    grants: ProfileGrantsInput,
) -> Result<CreateProfileOutcome, ()> {
    let grants = ProfileGrants {
        read: grants.read,
        send: grants.send,
        attachments: grants.attachments,
        message_actions: grants.message_actions,
        draft: grants.draft,
    };
    let outcome = match state
        .broker
        .create_profile(&*state.accounts, &label, &account_id, grants)
        .await
    {
        Ok(created) => CreateProfileOutcome::Created {
            profile_id: created.profile_id.as_str().to_owned(),
        },
        Err(error) => CreateProfileOutcome::Failed {
            reason: match error {
                ProfileError::InvalidLabel => "invalid_label",
                ProfileError::NotAgentAccount => "not_agent_account",
                ProfileError::AccountUnavailable => "account_unavailable",
                ProfileError::StoreUnusable(_) => "store_unusable",
                ProfileError::NotFound | ProfileError::AccountMismatch | ProfileError::Io => "io",
            },
        },
    };
    record(
        "mcp.profile.create",
        match outcome {
            CreateProfileOutcome::Created { .. } => "created",
            CreateProfileOutcome::Failed { reason } => reason,
        },
    );
    Ok(outcome)
}

/// Owner action: revokes a profile. The webview names only the id.
#[tauri::command]
pub async fn revoke_agent_profile(
    state: State<'_, McpController>,
    profile_id: String,
) -> Result<RevokeProfileOutcome, ()> {
    let Some(id) = ProfileId::parse(&profile_id) else {
        return Ok(RevokeProfileOutcome::NotFound);
    };
    let outcome = match state.broker.revoke_profile(&id).await {
        Ok(true) => RevokeProfileOutcome::Revoked,
        Ok(false) => RevokeProfileOutcome::NotFound,
        Err(_) => RevokeProfileOutcome::Failed,
    };
    // Also on `NotFound`: a second press must clear a row that lingers.
    if outcome != RevokeProfileOutcome::Failed {
        state.broker.refresh_profiles(&*state.accounts).await;
    }
    record(
        "mcp.profile.revoke",
        match outcome {
            RevokeProfileOutcome::Revoked => "revoked",
            RevokeProfileOutcome::NotFound => "not_found",
            RevokeProfileOutcome::Failed => "failed",
        },
    );
    Ok(outcome)
}

/// Answers one approval. Returns `applied`, `stale` (already answered or
/// expired), or `store_failed` (the grant could not be saved).
#[tauri::command]
pub async fn answer_mcp_approval(
    state: State<'_, McpController>,
    request_id: u64,
    allow: bool,
) -> Result<&'static str, ()> {
    Ok(match state.broker.answer(request_id, allow).await {
        AnswerOutcome::Applied => "applied",
        AnswerOutcome::Stale => "stale",
        AnswerOutcome::StoreFailed => "store_failed",
    })
}

/// Turns one permission on or off for one assistant and account. A failure
/// is reported through the state's `store` status.
#[tauri::command]
pub async fn set_mcp_permission(
    state: State<'_, McpController>,
    client_name: String,
    account_id: String,
    need: Need,
    allowed: bool,
) -> Result<(), ()> {
    state
        .broker
        .set_permission(
            &GrantSubject::client(&client_name),
            &account_id,
            need,
            allowed,
        )
        .await
        .map_err(|_| ())
}

/// Removes an assistant's grants, open approvals, and remembered denials.
#[tauri::command]
pub async fn remove_mcp_assistant(
    state: State<'_, McpController>,
    client_name: String,
) -> Result<(), ()> {
    state
        .broker
        .remove_subject(&GrantSubject::client(&client_name))
        .await
        .map_err(|_| ())
}

/// Runs `client`'s own registration CLI with the bundled launcher. The webview
/// names only the client; the command is rebuilt here. One at a time.
#[tauri::command]
pub async fn register_mcp_client(
    state: State<'_, McpController>,
    client: RegistrationClient,
) -> Result<RegisterOutcome, ()> {
    let Some(launcher) = state.launcher_path() else {
        return Ok(RegisterOutcome::LauncherUnavailable);
    };
    if state.registering.swap(true, Ordering::SeqCst) {
        return Ok(RegisterOutcome::InProgress);
    }
    // Cleared on every exit, including a panic or a dropped command future.
    struct Registering<'a>(&'a std::sync::atomic::AtomicBool);
    impl Drop for Registering<'_> {
        fn drop(&mut self) {
            self.0.store(false, Ordering::SeqCst);
        }
    }
    let _registering = Registering(&state.registering);
    let outcome = register::register(client, &launcher).await;
    record("mcp.register", outcome.kind());
    Ok(outcome)
}

/// Writes the Claude Desktop extension to the Downloads folder (or Koushi's
/// data folder) and opens it.
#[tauri::command]
pub async fn create_claude_desktop_extension(
    app: AppHandle,
    state: State<'_, McpController>,
) -> Result<ExtensionOutcome, ()> {
    let platform = Platform::current();
    if platform == Platform::Linux {
        return Ok(ExtensionOutcome::Unsupported);
    }
    let Some(launcher) = state.launcher_path() else {
        return Ok(ExtensionOutcome::LauncherUnavailable);
    };
    let directory = app
        .path()
        .download_dir()
        .ok()
        .filter(|dir| dir.is_dir())
        .or_else(|| crate::app_data_dir().ok().map(|dir| dir.join("mcp")));
    let Some(directory) = directory else {
        return Ok(ExtensionOutcome::WriteFailed);
    };
    let written = tauri::async_runtime::spawn_blocking(move || {
        let bytes = extension::build_bundle(
            std::path::Path::new(&launcher),
            env!("CARGO_PKG_VERSION"),
            platform,
        )?;
        extension::write_bundle(&directory, &bytes)
    })
    .await;
    let Ok(Ok(path)) = written else {
        record("mcp.extension", "write_failed");
        return Ok(ExtensionOutcome::WriteFailed);
    };
    let shown = path.to_string_lossy().into_owned();
    let outcome = match app.opener().open_path(&shown, None::<&str>) {
        Ok(()) => ExtensionOutcome::Opened { path: shown },
        Err(_) => ExtensionOutcome::Saved { path: shown },
    };
    record(
        "mcp.extension",
        match outcome {
            ExtensionOutcome::Opened { .. } => "opened",
            _ => "saved",
        },
    );
    Ok(outcome)
}

/// Outcome codes only; the client's output and paths stay out of diagnostics.
fn record(source: &'static str, outcome: &'static str) {
    koushi_diagnostics::record(
        koushi_diagnostics::DiagnosticEvent::new(
            koushi_diagnostics::DiagnosticLevel::Info,
            source,
            "outcome",
        )
        .field(koushi_diagnostics::DiagnosticField::token("kind", outcome)),
    );
}
