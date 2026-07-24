use std::collections::BTreeSet;
use std::fmt;

use crate::ComposerTarget;

pub const MAX_LIVE_COMPOSER_ROOM_TOMBSTONES: usize = 128;
pub const MAX_LIVE_COMPOSER_THREAD_TOMBSTONES: usize = 256;

#[derive(Clone, Default, Eq, PartialEq)]
pub struct ComposerDraftProtection {
    pub active: BTreeSet<ComposerTarget>,
    pub leased: BTreeSet<ComposerTarget>,
    pub store_pending: BTreeSet<ComposerTarget>,
}

impl fmt::Debug for ComposerDraftProtection {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ComposerDraftProtection")
            .field("active_count", &self.active.len())
            .field("leased_count", &self.leased.len())
            .field("store_pending_count", &self.store_pending.len())
            .finish()
    }
}
