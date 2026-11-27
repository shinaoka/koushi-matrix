//! The tables the profile isolation tests are generated from.

use std::collections::BTreeSet;

/// The dispatcher serves exactly the tools the MCP launcher advertises, so a
/// tool added to the schema without a dispatcher table row (or the reverse)
/// fails here, and with it the foreign-account table that is generated from it.
#[test]
fn the_dispatcher_table_is_the_advertised_tool_list() {
    let advertised: BTreeSet<String> = koushi_mcp::tools::definitions()
        .into_iter()
        .map(|tool| tool.name.to_owned())
        .collect();
    let served: BTreeSet<String> = crate::handlers::TOOLS
        .iter()
        .map(|tool| (*tool).to_owned())
        .collect();
    assert_eq!(served, advertised);
    assert_eq!(
        crate::handlers::TOOLS.len(),
        served.len(),
        "a tool is listed twice"
    );
}

#[test]
fn every_method_in_the_table_is_distinct() {
    let methods: BTreeSet<&&str> = crate::server::METHODS.iter().collect();
    assert_eq!(methods.len(), crate::server::METHODS.len());
}

/// No IPC path can write a profile or a grant. The broker's owner methods are
/// `pub`, so this is the structural check that the request path (`handlers.rs`)
/// and the connection loop (`server.rs`) never name them. Comment lines are
/// ignored; a legitimate new use belongs in the owner path, not here.
#[test]
fn the_request_path_never_calls_a_profile_or_grant_writer() {
    const FORBIDDEN: &[&str] = &[
        "create_profile(",
        "revoke_profile(",
        "set_permission(",
        "remove_subject(",
        "profiles::create(",
        "profiles::revoke(",
        "profiles::set_grant(",
        "set_grant(",
    ];
    // Read at run time: a compile-time path would record the checkout.
    let root = std::path::PathBuf::from(
        std::env::var_os("CARGO_MANIFEST_DIR").expect("cargo sets the manifest dir"),
    );
    for file in ["handlers.rs", "server.rs"] {
        let source = std::fs::read_to_string(root.join("src").join(file)).expect("source");
        for (number, line) in source.lines().enumerate() {
            let code = line.trim_start();
            if code.starts_with("//") {
                continue;
            }
            for token in FORBIDDEN {
                assert!(
                    !code.contains(token),
                    "{file}:{} calls {token}; no IPC method may write a profile or grant",
                    number + 1
                );
            }
        }
    }
}
