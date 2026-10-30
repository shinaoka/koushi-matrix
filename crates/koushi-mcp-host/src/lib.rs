//! Local MCP bridge: a stdio launcher (`koushi-mcp`) talks to this crate over a
//! private local socket.
//!
//! The crate is a transport adapter over an `AccountRuntimeManager`. It
//! publishes `endpoint.json`, verifies the per-launch token, and dispatches tool
//! calls to the account runtimes it was handed. It never opens a crypto store
//! and never reads Matrix credentials, so the desktop app and the headless QA
//! harness host the same bridge. Tool calls are authorized by a
//! [`consent::ConsentBroker`], which asks the user in the app when a host has a
//! consent surface and fails closed when it does not.

pub mod consent;
mod endpoint;
mod events;
mod grants;
mod handlers;
mod recent_activity;
pub mod registration;
mod server;

pub use grants::{Need, StoreStatus};
pub use server::{BridgeHandle, start};

use std::path::PathBuf;

use koushi_core::CoreConnection;
use koushi_core::account_runtime_manager::AccountRuntimeManager;

/// One account the bridge may talk to. The desktop app builds these from its
/// runtime manager; the headless QA harness builds a single one.
pub struct BridgeAccount {
    pub id: String,
    pub homeserver: Option<String>,
    pub connection: CoreConnection,
    pub media_cache_dir: PathBuf,
}

/// The accounts a host exposes to MCP clients.
pub trait BridgeAccounts: Send + Sync + 'static {
    fn accounts(&self) -> Vec<BridgeAccount>;
}

impl BridgeAccounts for AccountRuntimeManager {
    /// Every tab bound to an account, named by its stable account id. A tab
    /// that signed in during this run still has its sign-in tab id, which
    /// would hide the account from assistants until the next launch and
    /// change the id its grants are keyed by.
    fn accounts(&self) -> Vec<BridgeAccount> {
        self.tab_descriptors()
            .into_iter()
            .filter_map(|tab| {
                let stable_id = tab.stable_account_id()?;
                let connection = self.tab_connection(&tab.id)?;
                let media_cache_dir = self.media_cache_dir_for_tab(&tab.id)?;
                Some(BridgeAccount {
                    id: stable_id.as_str().to_owned(),
                    homeserver: tab.homeserver.clone(),
                    connection,
                    media_cache_dir,
                })
            })
            .collect()
    }
}
