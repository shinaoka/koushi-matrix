//! Local-app decryption key import (#1871): detection and staging commands.
//!
//! Locating another application's local data and extracting its keys is
//! platform work and lives in [`crate::migration`]. The import itself is a Core
//! command, so what Koushi keeps from the staged keys stays in Core.

use super::*;
use crate::{
    commands::e2ee::build_import_migrated_room_keys_command,
    migration::{self, MigrationSources},
};

/// Detected local sources with counts. Read-only: nothing is imported. Element X
/// reports only how much it found and why it cannot be imported.
#[tauri::command]
pub async fn migration_detect_sources() -> Result<MigrationSources, String> {
    tokio::task::spawn_blocking(migration::detect_sources)
        .await
        .map_err(|_| "migration detection failed".to_owned())
}

#[tauri::command]
pub async fn migration_import_element_desktop(
    account_tab_id: Option<String>,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<MigrationImportAdmission, String> {
    // Ask for the request id first: a signed-out window must not relaunch the
    // user's Element just to discover it cannot import.
    let request_id = next_request_id_for(state.inner(), account_tab_id.as_deref()).await?;
    let staged = migration::stage_element_desktop()
        .await
        .map_err(|error| error.to_string())?;
    submit_staged_room_keys(request_id, staged, &app, state.inner()).await
}

/// Keys staged for Core, plus the admission the import command returned.
/// Imported counts arrive with the room-key import events.
#[derive(Debug, Serialize)]
pub struct MigrationImportAdmission {
    pub(crate) admission: FrontendCommandAdmission,
    pub(crate) staged_keys: u64,
    pub(crate) staged_rooms: u64,
}

async fn submit_staged_room_keys(
    request_id: koushi_protocol::RequestId,
    staged: migration::StagedRoomKeys,
    app: &AppHandle,
    state: &CoreRuntimeState,
) -> Result<MigrationImportAdmission, String> {
    let admission = match submit_core_command_with_native_artifact_path(
        state,
        request_id,
        NativeArtifactKind::RoomKeyImportSource,
        staged.path.clone(),
        build_import_migrated_room_keys_command(request_id),
    )
    .await
    {
        Ok(admission) => admission,
        Err(error) => {
            discard_staging(&staged);
            return Err(error);
        }
    };
    update_qa_window_title_from_state(app, state).await;
    Ok(MigrationImportAdmission {
        admission,
        staged_keys: staged.keys,
        staged_rooms: staged.rooms,
    })
}

/// Core deletes the staging file after the import; this runs only when the
/// command never reached Core.
fn discard_staging(staged: &migration::StagedRoomKeys) {
    let _ = std::fs::remove_file(&staged.path);
}
