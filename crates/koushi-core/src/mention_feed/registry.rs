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
    settings: SettingsStore,
    accounts: Mutex<BTreeMap<String, MentionFeed>>,
}

impl std::fmt::Debug for MentionFeedRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("MentionFeedRegistry")
    }
}

impl MentionFeedRegistry {
    pub fn new(settings: SettingsStore) -> Self {
        Self {
            settings,
            accounts: Mutex::new(BTreeMap::new()),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, BTreeMap<String, MentionFeed>> {
        self.accounts.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub(crate) fn account_started(&self, user_id: &str, feed: MentionFeed) {
        self.lock().insert(user_id.to_owned(), feed);
    }

    pub(crate) fn account_stopped(&self, user_id: &str) {
        self.lock().remove(user_id);
    }

    /// The feed of a signed-in account, by Matrix user id.
    pub fn feed_for_user(&self, user_id: &str) -> Option<MentionFeed> {
        self.lock().get(user_id).cloned()
    }

    /// The users whose mentions may enter a feed right now: the signed-in
    /// accounts, minus what the owner removed, plus what the owner added.
    /// Evaluated per event, so an account added later is covered at once. An
    /// unreadable settings file allows nobody.
    pub fn allowed_senders(&self) -> BTreeSet<String> {
        let Ok(values) = self.settings.load() else {
            return BTreeSet::new();
        };
        let signed_in: Vec<String> = self.lock().keys().cloned().collect();
        values
            .mcp
            .allowed_senders
            .effective(signed_in.iter().map(String::as_str))
    }

    pub fn sender_allowed(&self, sender: &str) -> bool {
        self.allowed_senders().contains(sender)
    }
}
