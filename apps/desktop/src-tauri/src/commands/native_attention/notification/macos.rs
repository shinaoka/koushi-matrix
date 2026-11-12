//! Async macOS notification activation via UNUserNotificationCenter.

use std::{
    collections::{HashMap, HashSet, VecDeque},
    sync::{
        Mutex, OnceLock,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::sync::watch;

use super::*;

pub(super) const MAX_PENDING_MACOS_ACTIVATIONS: usize = 64;
const NOTIFICATION_PREFIX: &str = "koushi-attention-";
static NEXT_ID: AtomicU64 = AtomicU64::new(0);
static REGISTRY: OnceLock<Mutex<MacActivationRegistry>> = OnceLock::new();

fn registry() -> &'static Mutex<MacActivationRegistry> {
    REGISTRY.get_or_init(|| Mutex::new(MacActivationRegistry::default()))
}

pub(super) struct MacPendingActivation {
    pub(super) id: String,
    owner: String,
    has_preview: bool,
    cancel: watch::Sender<Option<bool>>,
}

#[derive(Default)]
pub(super) struct MacActivationRegistry {
    pending: VecDeque<MacPendingActivation>,
    invalidations: HashMap<String, u64>,
    preview_owners: HashSet<String>,
    generations: HashMap<String, (u64, bool)>,
}

impl MacActivationRegistry {
    pub(super) fn len(&self) -> usize {
        self.pending.len()
    }

    pub(super) fn insert(
        &mut self,
        id: String,
        owner: String,
        has_preview: bool,
        cancel: watch::Sender<Option<bool>>,
    ) -> Option<MacPendingActivation> {
        let evicted = if self.pending.len() >= MAX_PENDING_MACOS_ACTIVATIONS {
            self.pending.pop_front()
        } else {
            None
        };
        self.pending.push_back(MacPendingActivation {
            id,
            owner,
            has_preview,
            cancel,
        });
        evicted
    }

    fn remove(&mut self, id: &str) {
        self.pending.retain(|pending| pending.id != id);
    }

    fn generation(&self, owner: &str) -> (u64, bool) {
        self.generations.get(owner).copied().unwrap_or_default()
    }

    fn invalidation(&self, owner: &str) -> u64 {
        self.invalidations.get(owner).copied().unwrap_or_default()
    }

    pub(super) fn stale_clear_mode(
        &self,
        owner: &str,
        generation: u64,
        invalidation: u64,
    ) -> Option<bool> {
        if invalidation != self.invalidation(owner) {
            Some(true)
        } else {
            let current = self.generation(owner);
            (generation != current.0).then_some(current.1)
        }
    }

    pub(super) fn has_previews(&self, owner: &str) -> bool {
        self.preview_owners.contains(owner)
            || self
                .pending
                .iter()
                .any(|pending| pending.owner == owner && pending.has_preview)
    }

    pub(super) fn clear(
        &mut self,
        owner: &str,
        immediate: bool,
    ) -> Vec<watch::Sender<Option<bool>>> {
        if immediate {
            self.preview_owners.remove(owner);
            *self.invalidations.entry(owner.to_owned()).or_default() += 1;
        }
        let generation = self.generation(owner).0.wrapping_add(1);
        self.generations
            .insert(owner.to_owned(), (generation, immediate));
        self.pending
            .iter()
            .filter(|pending| pending.owner == owner)
            .map(|pending| pending.cancel.clone())
            .collect()
    }
}

pub(super) fn pending_count() -> usize {
    registry()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .len()
}

fn account_prefix(owner: &str) -> String {
    let encoded: String = owner.bytes().map(|byte| format!("{byte:02x}")).collect();
    format!("{NOTIFICATION_PREFIX}{encoded}-")
}

pub(super) fn clear_delay(age: Duration, immediate: bool) -> Duration {
    if immediate {
        Duration::ZERO
    } else {
        Duration::from_secs(5).saturating_sub(age)
    }
}

pub(super) fn dispatch(
    app: &AppHandle,
    payload: &NativeNotificationPayload,
    fence: ActivationFence,
    has_preview: bool,
) -> NativeNotificationOutcome {
    // UNUserNotificationCenter requires a real application bundle. The normal
    // desktop release has one; a `cargo test` process does not.
    if mac_usernotifications::check_bundle().is_err() {
        record_show_failed();
        return NativeNotificationOutcome::Failed;
    }

    let owner = fence.account_tab_id.as_str().to_owned();
    let prefix = account_prefix(&owner);
    let id = format!(
        "{prefix}{}-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos(),
        NEXT_ID.fetch_add(1, Ordering::Relaxed)
    );
    let title = payload.title.clone();
    let body = payload.body.clone();
    let target = payload.target.clone();
    let app = app.clone();
    let (generation, invalidation) = {
        let mut registry = registry().lock().unwrap_or_else(|p| p.into_inner());
        registry.generations.entry(owner.clone()).or_default();
        if has_preview {
            registry.preview_owners.insert(owner.clone());
        }
        (registry.generation(&owner).0, registry.invalidation(&owner))
    };
    tauri::async_runtime::spawn(async move {
        if generation
            != registry()
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .generation(&owner)
                .0
        {
            return;
        }
        let authorized = mac_usernotifications::get_notification_settings()
            .await
            .is_ok_and(|settings| {
                authorization_from_status(settings.authorization_status).allows_delivery()
            });
        if !authorized {
            record_show_failed();
            return;
        }
        if generation
            != registry()
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .generation(&owner)
                .0
        {
            return;
        }
        let still_allowed = app
            .state::<CoreRuntimeState>()
            .runtime
            .tab_connection(&fence.account_tab_id)
            .is_some_and(|connection| {
                let current = connection.versioned_snapshot();
                matches!(current.state.session, SessionState::Ready(_))
                    && current
                        .state
                        .settings
                        .values
                        .notifications
                        .desktop_notifications
                    && (!has_preview
                        || current.state.settings.values.notifications.message_previews)
            });
        if !still_allowed {
            return;
        }
        let notification = mac_usernotifications::Notification::new()
            .id(&id)
            .title(title)
            .message(body);
        let handle = match notification.send().await {
            Ok(handle) => handle,
            Err(_) => {
                record_show_failed();
                return;
            }
        };
        let submitted_at = tokio::time::Instant::now();
        let (stale, evicted, mut cancelled) = {
            let mut pending = registry().lock().unwrap_or_else(|p| p.into_inner());
            let stale = pending.stale_clear_mode(&owner, generation, invalidation);
            let (cancel, cancelled) = watch::channel(stale);
            (
                stale,
                pending.insert(id.clone(), owner.clone(), has_preview, cancel),
                cancelled,
            )
        };
        if let Some(evicted) = evicted {
            let _ = evicted.cancel.send(Some(true));
            mac_usernotifications::close_delivered(&evicted.id).await;
        }
        let response = handle.response();
        tokio::pin!(response);
        let cancellation = async {
            let mut mode = stale;
            loop {
                if let Some(immediate) = mode {
                    let delay = clear_delay(submitted_at.elapsed(), immediate);
                    tokio::select! {
                        _ = tokio::time::sleep(delay) => break,
                        changed = cancelled.changed() => {
                            if changed.is_err() { break; }
                            mode = *cancelled.borrow_and_update();
                        }
                    }
                } else {
                    if cancelled.changed().await.is_err() {
                        break;
                    }
                    mode = *cancelled.borrow_and_update();
                }
            }
        };
        let response = tokio::select! {
            response = &mut response => response.ok(),
            _ = cancellation => None,
            _ = tokio::time::sleep(Duration::from_secs(24 * 60 * 60)) => None,
        };
        if response.is_none() {
            mac_usernotifications::cancel_pending(&id).await;
            mac_usernotifications::close_delivered(&id).await;
        }
        registry()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(&id);
        let activation_valid = registry()
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .invalidation(&owner)
            == invalidation;
        if activation_valid && response.is_some_and(|response| response.is_default_action()) {
            activate_notification(&app, &fence, target);
        }
    });
    NativeNotificationOutcome::Delivered
}

pub(super) async fn clear_missing_accounts(known: &[AccountTabId]) {
    let removed: Vec<String> = registry()
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .generations
        .keys()
        .filter(|owner| !known.iter().any(|tab| tab.as_str() == owner.as_str()))
        .cloned()
        .collect();
    for owner in removed {
        clear(&owner, true).await;
    }
}

pub(super) fn has_previews(owner: &str) -> bool {
    registry()
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .has_previews(owner)
}

pub(super) fn send_clear_mode(cancel: &watch::Sender<Option<bool>>, immediate: bool) {
    cancel.send_if_modified(|current| {
        if *current == Some(true) || *current == Some(immediate) {
            return false;
        }
        *current = Some(immediate);
        true
    });
}

pub(super) async fn clear(owner: &str, immediate: bool) {
    let senders = registry()
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .clear(owner, immediate);
    for cancel in senders {
        send_clear_mode(&cancel, immediate);
    }
    if !immediate {
        return;
    }
    // Privacy/session invalidation also removes banners from earlier process lifetimes.
    let prefix = account_prefix(owner);
    for id in mac_usernotifications::get_delivered_notification_ids().await {
        if id.starts_with(&prefix) && !id.starts_with(&format!("{prefix}{}-", std::process::id())) {
            mac_usernotifications::close_delivered(&id).await;
        }
    }
    for id in mac_usernotifications::get_pending_notification_ids().await {
        if id.starts_with(&prefix) && !id.starts_with(&format!("{prefix}{}-", std::process::id())) {
            mac_usernotifications::cancel_pending(&id).await;
        }
    }
}
