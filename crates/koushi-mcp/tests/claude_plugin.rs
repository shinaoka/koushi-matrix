//! The Claude Code plugin manifest stays in step with the launcher's options
//! and never stores the profile choice (#2126).

const MCP_JSON: &str = include_str!("../claude-plugin/.mcp.json");
const PLUGIN_JSON: &str = include_str!("../claude-plugin/.claude-plugin/plugin.json");

#[test]
fn the_plugin_opts_in_to_the_channel_but_stores_no_profile() {
    let config: serde_json::Value = serde_json::from_str(MCP_JSON).expect("json");
    let server = &config["mcpServers"]["koushi"];
    let args: Vec<_> = server["args"]
        .as_array()
        .expect("args")
        .iter()
        .map(|arg| arg.as_str().expect("string"))
        .collect();
    assert!(args.contains(&"--claude-channel"));
    // The profile and its secret path come from the launching environment, so
    // a plugin loaded into every session never carries one.
    assert!(!MCP_JSON.contains("profile"));
    assert!(!MCP_JSON.contains("secret"));
    let manifest: serde_json::Value = serde_json::from_str(PLUGIN_JSON).expect("json");
    assert_eq!(manifest["name"], "koushi");
}
