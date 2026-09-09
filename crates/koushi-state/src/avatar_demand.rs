//! Portable resolved avatar demand. Core authorizes scope ownership and source
//! revisions before updating this state; AccountActor retains scheduling/I/O.

use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
};

/// Shared with Core's existing admission budget; not a separate avatar pool.
pub const VIEW_SCOPE_CAPACITY: usize = 64;
pub const AVATAR_VISIBLE_CAPACITY: usize = 256;
pub const AVATAR_PREFETCH_CAPACITY: usize = 8;

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AvatarDemandContext {
    pub account_id: String,
    pub session_generation: u64,
}

impl fmt::Debug for AvatarDemandContext {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AvatarDemandContext")
            .field("session_generation", &self.session_generation)
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AvatarDemandError {
    SessionChanged,
    Closed,
    StaleObservation,
    Capacity,
}

#[derive(Clone, Default, Serialize, Deserialize)]
struct ScopeDemand {
    revision: u64,
    // A missing avatar remains a visible identity, but creates no fetch demand.
    visible: Vec<Option<String>>,
    prefetch: Vec<Option<String>>,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct AvatarDemandState {
    context: AvatarDemandContext,
    scopes: BTreeMap<u64, ScopeDemand>,
}

impl fmt::Debug for AvatarDemandState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AvatarDemandState")
            .field("context", &self.context)
            .field("scope_count", &self.scopes.len())
            .finish_non_exhaustive()
    }
}

impl AvatarDemandState {
    pub fn context(&self) -> &AvatarDemandContext {
        &self.context
    }

    pub fn contains_resource(&self, resource: &str) -> bool {
        self.scopes.values().any(|scope| {
            scope
                .visible
                .iter()
                .chain(&scope.prefetch)
                .any(|candidate| candidate.as_deref() == Some(resource))
        })
    }

    pub fn new(context: AvatarDemandContext) -> Self {
        Self {
            context,
            scopes: BTreeMap::new(),
        }
    }

    /// Only Core may register an already-authorized, globally minted scope ID.
    /// Repeating admission does not discard its installed observation revision.
    pub fn open(&mut self, scope: u64) -> Result<(), AvatarDemandError> {
        if self.scopes.contains_key(&scope) {
            return Ok(());
        }
        if self.scopes.len() >= VIEW_SCOPE_CAPACITY {
            return Err(AvatarDemandError::Capacity);
        }
        self.scopes.insert(scope, ScopeDemand::default());
        Ok(())
    }

    /// Apply one complete observation atomically. A rejected observation neither
    /// changes demand nor consumes its revision. Resolved resources are Core
    /// inputs, not renderer-selected MXCs.
    pub fn replace(
        &mut self,
        context: &AvatarDemandContext,
        scope: u64,
        revision: u64,
        visible: Vec<Option<String>>,
        prefetch: Vec<Option<String>>,
    ) -> Result<(), AvatarDemandError> {
        if context != &self.context {
            return Err(AvatarDemandError::SessionChanged);
        }
        let current = self
            .scopes
            .get_mut(&scope)
            .ok_or(AvatarDemandError::Closed)?;
        if revision <= current.revision {
            return Err(AvatarDemandError::StaleObservation);
        }
        if visible.len() > AVATAR_VISIBLE_CAPACITY || prefetch.len() > AVATAR_PREFETCH_CAPACITY {
            return Err(AvatarDemandError::Capacity);
        }
        *current = ScopeDemand {
            revision,
            visible,
            prefetch,
        };
        Ok(())
    }

    pub fn close(&mut self, scope: u64) -> bool {
        self.scopes.remove(&scope).is_some()
    }

    /// All visible resources precede any prefetch resource. Shared resources
    /// occur once, regardless of the number of scopes/rows that consume them.
    /// No cache, in-flight status or second scheduling queue is kept here.
    pub fn resources_by_priority(&self) -> Vec<&str> {
        let mut seen = BTreeSet::new();
        self.scopes
            .values()
            .flat_map(|scope| &scope.visible)
            .chain(self.scopes.values().flat_map(|scope| &scope.prefetch))
            .filter_map(Option::as_deref)
            .filter(|resource| seen.insert(*resource))
            .collect()
    }
}
