//! Room-key export/import file choice: the adapter owns the native dialogs.
//!
//! React asks for a destination or a source and receives the chosen local
//! path (or `None` when the dialog is dismissed), then submits it with the
//! export/import command. Unattended GUI QA cannot drive a native dialog, so a
//! debug-build-only override answers with a fixed file instead.

use super::*;
use tauri_plugin_dialog::DialogExt;

#[cfg(any(debug_assertions, test))]
const QA_ROOM_KEY_FILE_ENV: &str = "KOUSHI_QA_ROOM_KEY_FILE";

const ROOM_KEY_FILE_NAME: &str = "koushi-room-keys.txt";
const ROOM_KEY_FILE_EXTENSIONS: &[&str] = &["txt", "json"];

#[tauri::command]
pub async fn choose_room_key_export_destination(
    dialog_title: String,
    app: AppHandle,
    window: tauri::WebviewWindow,
) -> Result<Option<String>, String> {
    #[cfg(any(debug_assertions, test))]
    if let Some(path) = qa_room_key_file() {
        return Ok(Some(path));
    }
    let (sender, receiver) = tokio::sync::oneshot::channel();
    app.dialog()
        .file()
        .set_parent(&window)
        .set_title(dialog_title.clone())
        .set_file_name(ROOM_KEY_FILE_NAME)
        .add_filter(dialog_title, ROOM_KEY_FILE_EXTENSIONS)
        .save_file(move |selected| {
            let _ = sender.send(selected);
        });
    selected_local_path(receiver.await.ok().flatten())
}

#[tauri::command]
pub async fn choose_room_key_import_source(
    dialog_title: String,
    app: AppHandle,
    window: tauri::WebviewWindow,
) -> Result<Option<String>, String> {
    #[cfg(any(debug_assertions, test))]
    if let Some(path) = qa_room_key_file() {
        return Ok(Some(path));
    }
    let (sender, receiver) = tokio::sync::oneshot::channel();
    app.dialog()
        .file()
        .set_parent(&window)
        .set_title(dialog_title.clone())
        .add_filter(dialog_title, ROOM_KEY_FILE_EXTENSIONS)
        .pick_file(move |selected| {
            let _ = sender.send(selected);
        });
    selected_local_path(receiver.await.ok().flatten())
}

fn selected_local_path(
    selected: Option<tauri_plugin_dialog::FilePath>,
) -> Result<Option<String>, String> {
    let Some(selected) = selected else {
        return Ok(None);
    };
    let path = selected
        .into_path()
        .map_err(|_| "room-key file is not a local path".to_owned())?;
    path.into_os_string()
        .into_string()
        .map(Some)
        .map_err(|_| "room-key file path is not valid UTF-8".to_owned())
}

#[cfg(any(debug_assertions, test))]
fn qa_room_key_file() -> Option<String> {
    std::env::var(QA_ROOM_KEY_FILE_ENV)
        .ok()
        .filter(|value| !value.is_empty())
}

#[cfg(test)]
mod tests {
    #[test]
    fn dismissed_dialog_chooses_nothing() {
        assert_eq!(super::selected_local_path(None), Ok(None));
    }

    #[test]
    fn a_local_selection_is_returned_as_its_path() {
        let path = std::env::temp_dir().join("synthetic-room-keys.txt");
        assert_eq!(
            super::selected_local_path(Some(tauri_plugin_dialog::FilePath::Path(path.clone()))),
            Ok(Some(path.to_string_lossy().into_owned()))
        );
    }
}
