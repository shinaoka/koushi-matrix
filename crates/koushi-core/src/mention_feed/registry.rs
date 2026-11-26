//! The per-process directory of mention feeds and the allow-list policy.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Mutex, PoisonError};

use crate::settings::SettingsStore;

use super::MentionFeed;

/// Owned by the account manager and shared with every account runtime. A
/// runtime registers its feed when its session starts, and the registry answers
/// both questions that need every account at once: who is signed in (the
/// derived allow-list) and which feed belongs to which account (the MCP host).
pub struct MentionFeedRegistry {
    settings: tokio::sync::watch::Receiver<koushi_state::AppSettingsValues>,
    /// Accounts whose feed is running right now. Used only to find a feed.
    accounts: Mutex<BTreeMap<String, MentionFeed>>,
    /// Accounts the owner has signed in, from the persisted session list. This
    /// (not the running set) is the derived allow-list, so a mention from an
    /// account that has not finished starting, is restarting, or failed to open
    /// its feed is still accepted.
    persisted: Mutex<BTreeSet<String>>,
}

impl std::fmt::Debug for MentionFeedRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("MentionFeedRegistry")
    }
}

impl MentionFeedRegistry {
    pub fn new(settings: SettingsStore) -> Self {
        // Prime the settings cache: the watch channel carries the values once
        // loaded. A read error keeps the derived default (no edits), which still
        // admits the signed-in accounts, and is reported rather than silent.
        if settings.load().is_err() {
            koushi_diagnostics::record(koushi_diagnostics::DiagnosticEvent::new(
                koushi_diagnostics::DiagnosticLevel::Warn,
                "core.mention_feed",
                "allow_list_settings_unreadable",
            ));
        }
        Self {
            settings: settings.subscribe(),
            accounts: Mutex::new(BTreeMap::new()),
            persisted: Mutex::new(BTreeSet::new()),
        }
    }

    /// Replaces the persisted account list (the saved session index), read
    /// before any account runtime starts.
    pub(crate) fn set_persisted_accounts<I, S>(&self, user_ids: I)
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        *self
            .persisted
            .lock()
            .unwrap_or_else(PoisonError::into_inner) =
            user_ids.into_iter().map(Into::into).collect();
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, BTreeMap<String, MentionFeed>> {
        self.accounts.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub(crate) fn account_started(&self, user_id: &str, feed: MentionFeed) {
        self.persisted
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(user_id.to_owned());
        self.lock().insert(user_id.to_owned(), feed);
    }

    pub(crate) fn account_stopped(&self, user_id: &str) {
        self.lock().remove(user_id);
    }

    /// The feed of a signed-in account, by Matrix user id.
    pub fn feed_for_user(&self, user_id: &str) -> Option<MentionFeed> {
        self.lock().get(user_id).cloned()
    }

    /// The users whose mentions may enter a feed: the persisted accounts, minus
    /// what the owner removed, plus what the owner added. Evaluated per event
    /// from the settings cache, so an edit applies at once and no file is read.
    /// A running-state change (an account stopping or still starting) does not
    /// change it.
    pub fn allowed_senders(&self) -> BTreeSet<String> {
        let persisted: Vec<String> = self
            .persisted
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .cloned()
            .collect();
        self.settings
            .borrow()
            .mcp
            .allowed_senders
            .effective(persisted.iter().map(String::as_str))
    }

    pub fn sender_allowed(&self, sender: &str) -> bool {
        self.allowed_senders().contains(sender)
    }
}
