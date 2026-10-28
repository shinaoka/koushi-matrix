//! MCP bridge host mode for agent scenarios (#1526).
//!
//! The QA binary normally runs one scenario and exits. With
//! `KOUSHI_QA_MCP_HOST=1` it instead logs in through the same guarded QA login
//! path, exposes that single account to [`koushi_mcp_host`], publishes
//! `mcp/endpoint.json`, grants the named MCP client, and serves until it is
//! asked to stop. A coding agent then drives Koushi through the real
//! `koushi-mcp` launcher, so an agent scenario exercises the transport, the
//! grant file, and the tool surface the desktop app hosts, not a test double.
//!
//! Everything here is disposable: the homeserver, the account, the data
//! directory, and the grant are created for one run and thrown away.

#[path = "mcp_host/cli_fixture.rs"]
mod cli_fixture;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use koushi_core::runtime::CoreRuntime;
use koushi_mcp_host::{BridgeAccount, BridgeAccounts};

use super::participants::{QaParticipantLoginGate, login_synced_participant_for_qa, qa_data_dir};
use super::registry::{ENV_HOMESERVER, ENV_PASSWORD_A, ENV_USER_A};

/// Set to `1` to serve MCP clients instead of running a QA scenario.
pub(super) const ENV_MCP_HOST: &str = "KOUSHI_QA_MCP_HOST";

/// Directory that receives `mcp/endpoint.json` and `mcp/connections.json`.
/// Defaults to a directory next to the QA data directory.
pub(super) const ENV_MCP_BRIDGE_DIR: &str = "KOUSHI_QA_MCP_BRIDGE_DIR";

/// MCP client name that receives the grant. Defaults to `pi`.
pub(super) const ENV_MCP_CLIENT: &str = "KOUSHI_QA_MCP_CLIENT";

const DEVICE_DISPLAY_NAME: &str = "Koushi MCP host";

/// The account directory holds the media cache the bridge returns paths from.
/// `CoreRuntime` owns that mapping and exposes no accessor, so this process
/// looks it up the only way it can: a disposable run has exactly one account.
fn account_media_cache_dir(data_dir: &Path) -> Result<PathBuf, String> {
    let accounts = data_dir.join("accounts");
    for root in [accounts.join("v2"), accounts.clone()] {
        let Ok(entries) = std::fs::read_dir(&root) else {
            continue;
        };
        let mut dirs: Vec<PathBuf> = entries
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| path.is_dir() && path.file_name().is_some_and(|name| name != "v2"))
            .collect();
        match dirs.len() {
            0 => continue,
            1 => return Ok(dirs.remove(0).join("media-downloads")),
            count => {
                return Err(format!(
                    "expected one account directory under {}, found {count}",
                    root.display()
                ));
            }
        }
    }
    Err(format!(
        "no account directory under {}; login did not create one",
        accounts.display()
    ))
}

/// Whether this process should serve MCP clients instead of running a scenario.
pub(super) fn requested() -> bool {
    matches!(std::env::var(ENV_MCP_HOST).as_deref(), Ok("1") | Ok("true"))
}

fn required(name: &str) -> Result<String, String> {
    std::env::var(name).map_err(|_| format!("{name} is required"))
}

fn bridge_dir() -> PathBuf {
    std::env::var_os(ENV_MCP_BRIDGE_DIR)
        .map(PathBuf::from)
        .unwrap_or_else(|| qa_data_dir("mcp-host-bridge"))
}

/// The bridge reads grants on every call and fails closed, so the QA host
/// writes the same file a granted desktop installation has.
fn write_grant(bridge_dir: &Path, client_name: &str, account_id: &str) -> Result<(), String> {
    let dir = bridge_dir.join("mcp");
    std::fs::create_dir_all(&dir)
        .map_err(|error| format!("create MCP directory {}: {error}", dir.display()))?;
    let grants = serde_json::json!({
        "schema": 1,
        "connections": [{
            "client_name": client_name,
            "accounts": [account_id],
            "allow_read": true,
            "allow_send": true,
            "allow_attachments": true,
        }],
    });
    let path = dir.join("connections.json");
    let bytes = serde_json::to_vec_pretty(&grants)
        .map_err(|error| format!("encode MCP grants: {error}"))?;
    std::fs::write(&path, bytes)
        .map_err(|error| format!("write MCP grants {}: {error}", path.display()))
}

async fn wait_for_shutdown() -> Result<(), String> {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};
        let mut terminate = signal(SignalKind::terminate())
            .map_err(|error| format!("install SIGTERM handler: {error}"))?;
        tokio::select! {
            _ = terminate.recv() => {}
            _ = tokio::signal::ctrl_c() => {}
        }
        Ok(())
    }

    #[cfg(not(unix))]
    {
        tokio::signal::ctrl_c()
            .await
            .map_err(|error| format!("wait for Ctrl-C: {error}"))
    }
}

/// The single account this process exposes. Every call hands the bridge a fresh
/// connection, exactly as the desktop runtime manager does per tab.
struct SingleAccount {
    id: String,
    homeserver: String,
    runtime: CoreRuntime,
    media_cache_dir: PathBuf,
}

impl BridgeAccounts for SingleAccount {
    fn accounts(&self) -> Vec<BridgeAccount> {
        vec![BridgeAccount {
            id: self.id.clone(),
            homeserver: Some(self.homeserver.clone()),
            connection: self.runtime.attach(),
            media_cache_dir: self.media_cache_dir.clone(),
        }]
    }
}

async fn host(
    homeserver: String,
    bridge_dir: PathBuf,
    client_name: String,
) -> Result<String, String> {
    let (username, password) = match cli_fixture::pipe_credentials()? {
        Some(credentials) => credentials,
        None => (required(ENV_USER_A)?, required(ENV_PASSWORD_A)?),
    };

    let data_dir = qa_data_dir("mcp-host");
    let mut participant = login_synced_participant_for_qa(
        &homeserver,
        data_dir.clone(),
        &username,
        &password,
        DEVICE_DISPLAY_NAME,
        "mcp host login",
        "mcp host",
        QaParticipantLoginGate::BootstrapNewIdentity,
    )
    .await?;

    if std::env::var("KOUSHI_QA_CLI_FIXTURE").as_deref() == Ok("1") {
        cli_fixture::seed(&mut participant).await?;
    }

    let account_id = format!("account:{}", participant.account_key.0);
    let media_cache_dir = account_media_cache_dir(&data_dir)?;
    let accounts: Arc<dyn BridgeAccounts> = Arc::new(SingleAccount {
        id: account_id.clone(),
        homeserver,
        runtime: participant.runtime,
        media_cache_dir,
    });

    write_grant(&bridge_dir, &client_name, &account_id)?;
    // No consent surface here: a missing permission must fail at once rather
    // than wait for a dialog nobody can answer. The schema 1 grant above also
    // keeps proving that older grant files still work.
    let broker = koushi_mcp_host::consent::ConsentBroker::new(
        bridge_dir.clone(),
        koushi_mcp_host::consent::ConsentMode::FailClosed,
    );
    let bridge = koushi_mcp_host::start(Arc::clone(&accounts), bridge_dir.clone(), broker)
        .map_err(|code| format!("MCP bridge failed to start: {code}"))?;

    let endpoint = bridge_dir.join("mcp").join("endpoint.json");
    println!(
        "MCP_HOST_READY {}",
        serde_json::json!({
            "endpoint": endpoint.to_string_lossy(),
            "client_name": client_name,
            "account_id": account_id,
        })
    );
    use std::io::Write;
    std::io::stdout()
        .flush()
        .map_err(|error| format!("flush MCP host ready line: {error}"))?;

    wait_for_shutdown().await?;
    bridge.stop();
    Ok("mcp host stopped".to_owned())
}

/// Runs the bridge host to completion. Called from the binary's `run` after the
/// file-credential guard, so this mode cannot reach the OS keychain either.
pub(super) fn run() -> Result<String, String> {
    let homeserver = required(ENV_HOMESERVER)?;
    let client_name = std::env::var(ENV_MCP_CLIENT).unwrap_or_else(|_| "pi".to_owned());
    let bridge_dir = bridge_dir();

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|error| format!("MCP host runtime creation failed: {error}"))?;
    // SDK handles must drop inside the runtime context (Async rule 11).
    runtime.block_on(host(homeserver, bridge_dir, client_name))
}
