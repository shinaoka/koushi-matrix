//! Contract for the main window's Ctrl/Cmd +/- webview zoom.
//!
//! The app captures zoom keys before focused widgets can stop propagation.
//! Disable Tauri's bubbling listener to avoid two independent zoom scales,
//! and admit the webview command used by the app's window/dialog port.

use std::fs;
use std::path::Path;

fn read_json(file: &str) -> serde_json::Value {
    // Run-time path: `env!` would bake the absolute checkout path into dep-info (#1937).
    let manifest_dir =
        std::env::var_os("CARGO_MANIFEST_DIR").expect("cargo test sets CARGO_MANIFEST_DIR");
    let path = Path::new(&manifest_dir).join(file);
    let contents = fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("{file} must be readable: {error}"));
    serde_json::from_str(&contents)
        .unwrap_or_else(|error| panic!("{file} must be valid JSON: {error}"))
}

#[test]
fn main_window_leaves_zoom_hotkeys_to_the_app() {
    let config = read_json("tauri.conf.json");
    let main_window = config["app"]["windows"]
        .as_array()
        .and_then(|windows| windows.first())
        .expect("main window config must be present");

    assert_eq!(
        main_window["zoomHotkeysEnabled"],
        serde_json::json!(false),
        "Tauri's injected zoom handler must not compete with the app's capture handler"
    );
}

#[test]
fn main_capability_admits_webview_zoom_command() {
    let capability = read_json("capabilities/default.json");
    let permissions = capability["permissions"]
        .as_array()
        .expect("main capability permissions must be an array");

    assert!(
        permissions
            .iter()
            .any(|permission| permission == "core:webview:allow-set-webview-zoom"),
        "main window must explicitly admit the webview zoom command"
    );
}
