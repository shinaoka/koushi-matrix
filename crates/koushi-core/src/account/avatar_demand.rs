//! Reconcile the latest Rust-owned demand into the existing avatar downloader.

use koushi_state::AppAction;

use super::{
    actor::AccountActor,
    profile::{AVATAR_DOWNLOAD_CONCURRENCY, AVATAR_DOWNLOAD_QUEUE_CAPACITY},
};

impl AccountActor {
    pub(super) async fn accept_avatar_demand(&mut self) {
        let next = self.avatar_demand_rx.borrow_and_update().clone();
        if let Some(demand) = &next {
            let context = demand.context();
            if context.session_generation
                != self
                    .avatar_session_generation
                    .load(std::sync::atomic::Ordering::Acquire)
                || !self
                    .session
                    .as_ref()
                    .is_some_and(|session| session.info.user_id == context.account_id)
            {
                return;
            }
        }
        self.avatar_demand = next;
        self.reconcile_avatar_demand(true).await;
    }

    pub(super) async fn reconcile_avatar_demand(&mut self, publish_cached: bool) {
        // The Arc is a bounded resolved-demand snapshot, not a clone of AppState.
        let demand = self.avatar_demand.clone();
        let resources = demand
            .as_ref()
            .map(|demand| demand.resources_by_priority())
            .unwrap_or_default();
        let obsolete: Vec<_> = self
            .avatar_inflight
            .keys()
            .filter(|uri| {
                !demand
                    .as_ref()
                    .is_some_and(|demand| demand.contains_resource(uri))
            })
            .cloned()
            .collect();
        for uri in obsolete {
            self.cancel_unwanted_avatar(&uri);
        }

        // Rebuild only queued scoped work in current priority order. Keep active
        // work and ordinary command waiters; neither gets restarted or lost.
        for uri in std::mem::take(&mut self.avatar_pending) {
            if self.avatar_inflight.get(&uri).is_some_and(Vec::is_empty) {
                self.avatar_inflight.remove(&uri);
            } else {
                self.avatar_pending.push_back(uri);
            }
        }
        for uri in resources {
            // Revalidate Ready bytes when an observation is published, not on
            // every completion: a working set larger than the renderable LRU
            // must not drive an endless eviction/refill loop.
            let cached = if publish_cached {
                self.cached_avatar_thumbnail(uri)
            } else {
                self.avatar_cache.get(uri).cloned()
            };
            if let Some(cached) = cached {
                if publish_cached {
                    self.send_actions(vec![AppAction::AvatarThumbnailUpdated {
                        mxc_uri: uri.to_owned(),
                        thumbnail: cached.clone(),
                    }])
                    .await;
                }
                continue;
            }
            if self.avatar_inflight.contains_key(uri) || self.session.is_none() {
                continue;
            }
            if self.avatar_active_fetches < AVATAR_DOWNLOAD_CONCURRENCY {
                self.avatar_inflight.insert(uri.to_owned(), Vec::new());
                self.spawn_avatar_fetch(uri.to_owned(), 0);
            } else if self.avatar_pending.len() < AVATAR_DOWNLOAD_QUEUE_CAPACITY {
                self.avatar_inflight.insert(uri.to_owned(), Vec::new());
                self.avatar_pending.push_back(uri.to_owned());
            }
            // Excess demand remains in the bounded state snapshot. Completion
            // reconciles it again; Capacity is not cached as a terminal failure.
        }
        self.start_pending_avatar_fetches();
    }
}
