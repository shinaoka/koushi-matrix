//! Desktop ownership of the MCP bridge. The bridge and its consent broker live
//! in `koushi-mcp-host`, so the headless QA harness can host the same code;
//! this module only starts and stops the bridge from the `mcp.enabled` setting,
//! forwards the broker's state to the WebView, and asks for attention when an
//! approval is waiting.

pub(crate) mod extension;
mod launcher;
pub(crate) mod register;

use std::path::PathBuf;
use std::sync::Arc;

use koushi_core::account_runtime_manager::AccountRuntimeManager;
use koushi_mcp_host::BridgeHandle;
use koushi_mcp_host::consent::{BridgeStatus, ConsentBroker, ConsentMode, McpAssistantsState};
use tauri::{AppHandle, Emitter, Manager};

pub const MCP_EVENT_NAME: &str = "koushi-desktop://mcp";

/// Tauri-managed owner of the broker and the running bridge.
pub(crate) struct McpController {
    pub(crate) broker: Arc<ConsentBroker>,
    /// The accounts the bridge serves, for the owner's profile actions.
    pub(crate) accounts: Arc<dyn koushi_mcp_host::BridgeAccounts>,
    /// The running bridge. Never held across an await, so the synchronous
    /// exit handler can always take it.
    bridge: std::sync::Mutex<Option<BridgeHandle>>,
    /// Serializes switch changes, which await the bridge's shutdown.
    switch: tokio::sync::Mutex<()>,
    /// Set while a client's registration CLI runs; one at a time.
    pub(crate) registering: std::sync::atomic::AtomicBool,
}

impl McpController {
    /// Stops the bridge on process exit: removes the endpoint file and socket
    /// and releases waiting calls.
    pub(crate) fn stop_now(&self) {
        if let Some(handle) = self.take_bridge() {
            handle.stop();
        }
    }

    /// The launcher path the app resolved, never one the webview supplied.
    pub(crate) fn launcher_path(&self) -> Option<String> {
        match &self.broker.subscribe().borrow().launcher {
            koushi_mcp_host::consent::LauncherStatus::Available { path } => Some(path.clone()),
            koushi_mcp_host::consent::LauncherStatus::Unavailable { .. } => None,
        }
    }

    fn take_bridge(&self) -> Option<BridgeHandle> {
        self.bridge
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()
    }

    fn has_bridge(&self) -> bool {
        self.bridge
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .is_some()
    }

    fn set_bridge(&self, handle: BridgeHandle) {
        *self
            .bridge
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(handle);
    }
}

/// Creates the broker and starts following the `mcp.enabled` setting.
pub(crate) fn install(app: &AppHandle, runtime: Arc<AccountRuntimeManager>, data_dir: PathBuf) {
    let broker = ConsentBroker::new(data_dir.clone(), ConsentMode::Interactive);
    broker.set_launcher(launcher::resolve(&data_dir));
    app.manage(McpController {
        broker: Arc::clone(&broker),
        accounts: Arc::clone(&runtime) as Arc<dyn koushi_mcp_host::BridgeAccounts>,
        bridge: std::sync::Mutex::new(None),
        switch: tokio::sync::Mutex::new(()),
        registering: std::sync::atomic::AtomicBool::new(false),
    });
    spawn_forwarder(app.clone(), Arc::clone(&broker));
    spawn_switch(app.clone(), runtime, data_dir);
}

/// Starts or stops the bridge whenever the setting changes. The first decision
/// waits for the settings file: an unreadable file keeps the bridge off rather
/// than falling back to the default.
fn spawn_switch(app: AppHandle, runtime: Arc<AccountRuntimeManager>, data_dir: PathBuf) {
    tauri::async_runtime::spawn(async move {
        // Subscribe first: a change saved while the file loads is then seen
        // by the loop below rather than lost.
        let mut updates = runtime.subscribe_app_settings_updates();
        updates.borrow_and_update();
        let loader = Arc::clone(&runtime);
        let initial = tauri::async_runtime::spawn_blocking(move || loader.load_app_settings())
            .await
            .ok()
            .and_then(Result::ok)
            .is_some_and(|values| values.mcp.enabled);
        let mut enabled = initial;
        loop {
            apply(&app, &runtime, &data_dir, enabled).await;
            if updates.changed().await.is_err() {
                return;
            }
            enabled = updates.borrow_and_update().mcp.enabled;
        }
    });
}

async fn apply(
    app: &AppHandle,
    runtime: &Arc<AccountRuntimeManager>,
    data_dir: &std::path::Path,
    enabled: bool,
) {
    let Some(controller) = app.try_state::<McpController>() else {
        return;
    };
    let _switch = controller.switch.lock().await;
    match (enabled, controller.has_bridge()) {
        (true, false) => {
            let started = koushi_mcp_host::start(
                Arc::clone(runtime) as Arc<dyn koushi_mcp_host::BridgeAccounts>,
                data_dir.to_path_buf(),
                Arc::clone(&controller.broker),
            );
            match started {
                Ok(handle) => {
                    controller.set_bridge(handle);
                    controller.broker.set_bridge_status(BridgeStatus::Running);
                }
                Err(reason) => {
                    koushi_diagnostics::record(
                        koushi_diagnostics::DiagnosticEvent::new(
                            koushi_diagnostics::DiagnosticLevel::Warn,
                            "mcp.bridge",
                            "start_failed",
                        )
                        .field(koushi_diagnostics::DiagnosticField::token("reason", reason)),
                    );
                    controller
                        .broker
                        .set_bridge_status(BridgeStatus::StartFailed);
                }
            }
        }
        (false, true) => {
            if let Some(handle) = controller.take_bridge() {
                handle.shutdown().await;
            }
            controller.broker.set_bridge_status(BridgeStatus::Disabled);
        }
        (false, false) => controller.broker.set_bridge_status(BridgeStatus::Disabled),
        (true, true) => {}
    }
}

/// Forwards every state change to the WebView and draws attention to a newly
/// waiting approval. Attention never forces focus: an assistant must not be
/// able to pull keyboard input into the dialog.
fn spawn_forwarder(app: AppHandle, broker: Arc<ConsentBroker>) {
    let mut state = broker.subscribe();
    tauri::async_runtime::spawn(async move {
        let mut newest_request = 0;
        loop {
            let current: McpAssistantsState = state.borrow_and_update().clone();
            let newest = current
                .pending
                .iter()
                .map(|pending| pending.request_id)
                .max()
                .unwrap_or(0);
            if newest > newest_request {
                newest_request = newest;
                request_attention(&app);
            }
            let _ = app.emit(MCP_EVENT_NAME, &current);
            if state.changed().await.is_err() {
                return;
            }
        }
    });
}

fn request_attention(app: &AppHandle) {
    let Some(window) = app.get_webview_window("main") else {
        return;
    };
    if !window.is_visible().unwrap_or(true) {
        let _ = window.show();
    }
    if window.is_minimized().unwrap_or(false) {
        let _ = window.unminimize();
    }
    let _ = window.request_user_attention(Some(tauri::UserAttentionType::Informational));
}

#[cfg(test)]
#[path = "capability_tests.rs"]
mod capability_tests;
