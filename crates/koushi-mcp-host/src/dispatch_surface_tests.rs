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
