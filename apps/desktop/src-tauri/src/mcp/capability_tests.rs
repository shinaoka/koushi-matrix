//! The owner's profile commands must not be reachable by remote content (#2132).
//!
//! Tauri refuses an app command from a remote origin unless a capability with a
//! `remote` scope grants it, and the app has no `AppManifest`, so no capability
//! can name an app command at all. Adding a manifest would switch ACL checking
//! on for every local command too, and a remote capability naming a command
//! would hand it to that origin. The tests pin both halves of that arrangement.

use std::path::PathBuf;

const OWNER_COMMANDS: [&str; 2] = ["create_agent_profile", "revoke_agent_profile"];
/// The plugins whose permissions a capability may name.
const PLUGIN_PREFIXES: [&str; 4] = ["core:", "dialog:", "notification:", "opener:"];

fn manifest_dir() -> PathBuf {
    PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").expect("cargo sets the manifest dir"))
}

fn capabilities() -> Vec<(String, serde_json::Value)> {
    let directory = manifest_dir().join("capabilities");
    let mut found = Vec::new();
    for entry in std::fs::read_dir(directory).expect("capabilities directory") {
        let path = entry.expect("entry").path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
            continue;
        }
        let text = std::fs::read_to_string(&path).expect("capability file");
        let value = serde_json::from_str(&text).expect("capability JSON");
        found.push((path.display().to_string(), value));
    }
    assert!(!found.is_empty(), "no capability files found");
    found
}

fn permission_names(capability: &serde_json::Value) -> Vec<String> {
    capability["permissions"]
        .as_array()
        .expect("permissions array")
        .iter()
        .map(|permission| match permission {
            serde_json::Value::String(name) => name.clone(),
            other => other["identifier"]
                .as_str()
                .expect("a permission names an identifier")
                .to_owned(),
        })
        .collect()
}

#[test]
fn no_capability_grants_an_app_command_so_remote_content_is_refused() {
    for (file, capability) in capabilities() {
        for permission in permission_names(&capability) {
            assert!(
                PLUGIN_PREFIXES
                    .iter()
                    .any(|prefix| permission.starts_with(prefix)),
                "{file} grants `{permission}`, which is not a plugin permission; a capability \
                 (above all one with a `remote` scope) must never name an app command"
            );
        }
    }
}

#[test]
fn the_owner_commands_are_in_no_capability_and_are_registered() {
    let capability_text: String = capabilities()
        .into_iter()
        .map(|(_, value)| value.to_string())
        .collect();
    let lib = std::fs::read_to_string(manifest_dir().join("src/lib.rs")).expect("lib.rs");
    for command in OWNER_COMMANDS {
        assert!(
            !capability_text.contains(command)
                && !capability_text.contains(&command.replace('_', "-")),
            "{command} appears in a capability file"
        );
        assert!(
            lib.contains(&format!("commands::mcp::{command},")),
            "{command} is not registered in the invoke handler"
        );
    }
}

#[test]
fn the_app_has_no_manifest_that_would_move_commands_into_the_acl() {
    let build = std::fs::read_to_string(manifest_dir().join("build.rs")).expect("build.rs");
    assert!(
        !build.contains("app_manifest") && !build.contains("AppManifest"),
        "an AppManifest turns ACL checking on for local commands; if one is added, \
         the owner commands must be listed in a capability scoped to the local window \
         only and this test updated"
    );
}
