use std::{collections::BTreeMap, sync::Arc};

use eyeball_im::{Vector, VectorDiff};
use matrix_sdk_ui::timeline::{ReadReceiptSnapshot, TimelineItem as SdkTimelineItem};

use super::receipt_index::{ReceiptEpoch, ReceiptReaderIndex};
use koushi_protocol::view::{ReaderWindowLimit, ReceiptSourceRef, TimelineViewSource};

#[derive(Clone)]
pub(crate) struct RawReceiptWindow {
    pub(crate) total_count: u64,
    pub(crate) start: u64,
    pub(crate) receipts: Vec<koushi_state::LiveReadReceipt>,
    pub(crate) profiles: Vec<koushi_sdk::MatrixUserProfile>,
    pub(crate) owner: Option<ReceiptWindowOwner>,
    pub(crate) epoch: std::sync::Weak<std::sync::Mutex<ReceiptEpoch>>,
}

/// Selected host rows plus private avatar bindings after AppActor enrichment.
/// Still private preparation: installed scope revisions and authorization are separate.
pub(crate) struct ResolvedReceiptWindow {
    pub(crate) total_count: u64,
    pub(crate) start: u64,
    pub(crate) rows: Vec<koushi_protocol::view::ReaderRow>,
    pub(crate) avatar_resources: Vec<ReaderAvatarResource>,
    pub(crate) owner: Option<ReceiptWindowOwner>,
    pub(crate) epoch: std::sync::Weak<std::sync::Mutex<ReceiptEpoch>>,
}

impl ResolvedReceiptWindow {
    pub(crate) fn source_revision(&self) -> Result<u64, crate::view_scope_lifecycle::ScopeError> {
        use crate::view_scope_lifecycle::ScopeError;
        let epoch = self.epoch.upgrade().ok_or(ScopeError::SourceUnavailable)?;
        let revision = epoch.lock().expect("receipt epoch poisoned").revision;
        revision.ok_or(ScopeError::CounterExhausted)
    }

    pub(crate) fn matches_model(&self, model: &koushi_protocol::view::ViewModel) -> bool {
        match model {
            koushi_protocol::view::ViewModel::ReaderReady(window) => {
                self.owner
                    .as_ref()
                    .is_some_and(|owner| owner.source == window.source)
                    && self.source_revision().ok() == Some(window.source_revision)
                    && window.total_count == self.total_count
                    && window.start == self.start
            }
            _ => false,
        }
    }

    /// Only a prepared, synchronous commit may run here; source lock precedes registry locks.
    pub(crate) fn commit_if_current<R>(&self, commit: impl FnOnce() -> R) -> Option<R> {
        self.owner.as_ref()?.commit_if_current(&self.epoch, commit)
    }

    #[cfg(test)]
    pub(crate) fn acquire_source(&self) -> Option<super::navigation::TimelineActorGenerationLease> {
        let owner = self.owner.as_ref()?;
        let lease = owner
            .gate
            .try_acquire(&owner.source.timeline.key, owner.generation)?;
        let epoch = self.epoch.upgrade()?;
        let current = epoch.lock().expect("receipt epoch poisoned").valid;
        current.then_some(lease)
    }
}

/// Private demand identity and byte ownership; never serialized into host rows.
pub(crate) struct ReaderAvatarResource {
    pub(crate) user_id: String,
    pub(crate) mxc_uri: String,
    pub(crate) lease: Result<
        crate::renderable_thumbnail::RenderableThumbnailLease,
        crate::renderable_thumbnail::ThumbnailLeaseError,
    >,
}

#[derive(Clone)]
pub(crate) struct ReceiptWindowOwner {
    gate: Arc<super::navigation::TimelineActorGenerationGate>,
    source: ReceiptSourceRef,
    generation: u64,
}

impl ReceiptWindowOwner {
    fn commit_if_current<R>(
        &self,
        epoch: &std::sync::Weak<std::sync::Mutex<ReceiptEpoch>>,
        commit: impl FnOnce() -> R,
    ) -> Option<R> {
        let _generation = self
            .gate
            .try_acquire(&self.source.timeline.key, self.generation)?;
        let epoch = epoch.upgrade()?;
        let current = epoch.lock().expect("receipt epoch poisoned");
        if !current.valid {
            return None;
        }
        Some(commit())
    }
}

impl RawReceiptWindow {
    /// Hold the existing actor-generation and receipt-epoch guards through a
    /// synchronous observation commit. Source locks precede registry locks.
    pub(crate) fn commit_if_current<R>(&self, commit: impl FnOnce() -> R) -> Option<R> {
        self.owner.as_ref()?.commit_if_current(&self.epoch, commit)
    }

    pub(crate) fn owner_generation(&self) -> Option<u64> {
        self.owner.as_ref().map(|owner| owner.generation)
    }

    pub(crate) fn source(&self) -> Option<&ReceiptSourceRef> {
        self.owner.as_ref().map(|owner| &owner.source)
    }

    pub(crate) fn source_revision(&self) -> Result<u64, crate::view_scope_lifecycle::ScopeError> {
        use crate::view_scope_lifecycle::ScopeError;
        let epoch = self.epoch.upgrade().ok_or(ScopeError::SourceUnavailable)?;
        let revision = epoch.lock().expect("receipt epoch poisoned").revision;
        revision.ok_or(ScopeError::CounterExhausted)
    }

    pub(crate) fn into_resolved(
        self,
        locale: koushi_state::CatalogLocale,
    ) -> ResolvedReceiptWindow {
        let mut avatar_resources = Vec::new();
        ResolvedReceiptWindow {
            total_count: self.total_count,
            start: self.start,
            owner: self.owner,
            epoch: self.epoch,
            rows: self
                .receipts
                .into_iter()
                .map(|receipt| {
                    let display_label = receipt.display_name.unwrap_or_default();
                    let initials = reader_initials(&display_label);
                    let avatar = receipt.avatar.map(|mut avatar| {
                        use crate::renderable_thumbnail::{
                            RenderableThumbnailKind, ThumbnailLeaseError,
                            lease_renderable_thumbnail, renderable_thumbnail_cache_key,
                        };
                        use koushi_state::AvatarThumbnailState;
                        let lease = match &avatar.thumbnail {
                            AvatarThumbnailState::Ready { source_ref, .. } => {
                                lease_renderable_thumbnail(source_ref)
                            }
                            AvatarThumbnailState::NotRequested => {
                                lease_renderable_thumbnail(&renderable_thumbnail_cache_key(
                                    RenderableThumbnailKind::Avatar,
                                    &avatar.mxc_uri,
                                ))
                            }
                            _ => Err(ThumbnailLeaseError::Unavailable),
                        };
                        if matches!(avatar.thumbnail, AvatarThumbnailState::NotRequested) {
                            if let Ok(lease) = &lease {
                                avatar.thumbnail = lease.thumbnail_state();
                            }
                        }
                        if lease.is_err()
                            && matches!(avatar.thumbnail, AvatarThumbnailState::Ready { .. })
                        {
                            // Rust retains the demand identity; a stale Ready ref is not displayable.
                            avatar.thumbnail = AvatarThumbnailState::NotRequested;
                        }
                        avatar_resources.push(ReaderAvatarResource {
                            user_id: receipt.user_id.clone(),
                            mxc_uri: avatar.mxc_uri,
                            lease,
                        });
                        avatar.thumbnail
                    });
                    koushi_protocol::view::ReaderRow {
                        user_id: receipt.user_id,
                        display_label,
                        original_display_label: receipt.original_display_label,
                        initials,
                        timestamp: koushi_protocol::view::ReceiptTimestamp::from_sdk(
                            receipt.timestamp_ms,
                            locale,
                        ),
                        avatar,
                    }
                })
                .collect(),
            avatar_resources,
        }
    }

    pub(super) fn bind_owner(
        &mut self,
        gate: &Arc<super::navigation::TimelineActorGenerationGate>,
        source: &ReceiptSourceRef,
        generation: u64,
    ) {
        self.owner = Some(ReceiptWindowOwner {
            gate: gate.clone(),
            source: source.clone(),
            generation,
        });
    }

    pub(crate) fn acquire_source(&self) -> Option<super::navigation::TimelineActorGenerationLease> {
        if !self.source_is_current() {
            return None;
        }
        let owner = self.owner.as_ref()?;
        owner
            .gate
            .try_acquire(&owner.source.timeline.key, owner.generation)
    }

    pub(crate) fn source_is_current(&self) -> bool {
        self.epoch
            .upgrade()
            .is_some_and(|epoch| epoch.lock().expect("receipt epoch poisoned").valid)
    }

    #[cfg(test)]
    pub(crate) async fn bind_test_owner(
        &mut self,
        source: &ReceiptSourceRef,
    ) -> Arc<std::sync::Mutex<ReceiptEpoch>> {
        let gate = Arc::new(super::navigation::TimelineActorGenerationGate::default());
        let generation = gate
            .activate_after_quiescence(&source.timeline.key)
            .await
            .generation;
        self.bind_owner(&gate, source, generation);
        let epoch = Arc::new(std::sync::Mutex::new(ReceiptEpoch {
            valid: true,
            revision: Some(1),
        }));
        self.epoch = Arc::downgrade(&epoch);
        epoch
    }

    pub(crate) fn resolve_profiles(
        &mut self,
        profiles: &koushi_state::ProfileState,
        room_id: &str,
        own: Option<&str>,
    ) {
        self.receipts = std::mem::take(&mut self.receipts)
            .into_iter()
            .map(|mut receipt| {
                // A known current profile, including an explicitly absent avatar/name,
                // wins over SDK hints captured before this AppActor turn.
                let known = profiles.users.contains_key(&receipt.user_id)
                    || profiles
                        .room_users
                        .get(room_id)
                        .is_some_and(|users| users.contains_key(&receipt.user_id))
                    || own == Some(receipt.user_id.as_str());
                if known {
                    receipt.original_display_label.clear();
                    receipt.avatar = None;
                }
                let hint = self
                    .profiles
                    .iter()
                    .find(|profile| profile.user_id == receipt.user_id)
                    .filter(|_| !known);
                receipt.display_name = hint.and_then(|profile| profile.display_name.clone());
                let mut receipt = koushi_state::enrich_live_receipt(
                    receipt,
                    profiles,
                    profiles.room_users.get(room_id),
                    own,
                );
                if receipt.avatar.is_none() {
                    receipt.avatar =
                        hint.and_then(|profile| profile.avatar_mxc_uri.clone())
                            .map(|mxc_uri| koushi_state::AvatarImage {
                                mxc_uri,
                                thumbnail: koushi_state::AvatarThumbnailState::NotRequested,
                            });
                }
                receipt
            })
            .collect();
    }
}

fn reader_initials(label: &str) -> String {
    let ascii: String = label
        .chars()
        .filter(char::is_ascii_alphabetic)
        .take(2)
        .collect();
    if ascii.is_empty() {
        // Preserve the two-character fallback without splitting a UTF-16 surrogate.
        label.chars().take(2).collect()
    } else {
        ascii.to_ascii_uppercase()
    }
}

#[derive(Clone)]
struct ReceiptEndpoint {
    event_id: String,
    receipts: ReadReceiptSnapshot,
}

pub(super) struct ReceiptEndpointChange {
    pub(super) before: Option<ReadReceiptSnapshot>,
    pub(super) after: Option<ReadReceiptSnapshot>,
}

/// SDK-position mirror of receipt endpoints only; never retains message bodies.
pub(super) struct ReceiptEndpointMirror {
    entries: Vector<Option<ReceiptEndpoint>>,
    readers: BTreeMap<String, ReceiptReaderIndex>,
}

impl ReceiptEndpointMirror {
    pub(super) fn new<'a>(items: impl IntoIterator<Item = &'a Arc<SdkTimelineItem>>) -> Self {
        Self::from_entries(
            items
                .into_iter()
                .map(|item| endpoint_from_sdk(item))
                .collect(),
        )
    }

    fn from_entries(entries: Vector<Option<ReceiptEndpoint>>) -> Self {
        let readers = entries
            .iter()
            .flatten()
            .map(|entry| {
                (
                    entry.event_id.clone(),
                    ReceiptReaderIndex::new(entry.receipts.clone()),
                )
            })
            .collect();
        Self { entries, readers }
    }

    pub(super) fn read_window(
        &self,
        source: &ReceiptSourceRef,
        current: &TimelineViewSource,
        start: u64,
        limit: ReaderWindowLimit,
        own: Option<&matrix_sdk::ruma::UserId>,
    ) -> Option<RawReceiptWindow> {
        if source.timeline != *current {
            return None;
        }
        let index = self.readers(&source.event_id)?;
        let total_count = index.total(own) as u64;
        let start = start.min(total_count);
        let receipts = index
            .window(start as usize, usize::from(limit.get()), own)
            .map(|(user, receipt)| koushi_state::LiveReadReceipt {
                user_id: user.to_string(),
                display_name: None,
                original_display_label: String::new(),
                avatar: None,
                timestamp_ms: receipt.ts.map(|timestamp| timestamp.0.into()),
            })
            .collect();
        Some(RawReceiptWindow {
            total_count,
            start,
            receipts,
            profiles: Vec::new(),
            owner: None,
            epoch: index.window_epoch(),
        })
    }

    pub(super) fn readers(&self, event_id: &str) -> Option<&ReceiptReaderIndex> {
        self.readers.get(event_id)
    }

    pub(super) fn apply_batch(
        &mut self,
        diffs: &[VectorDiff<Arc<SdkTimelineItem>>],
    ) -> BTreeMap<String, ReceiptEndpointChange> {
        self.apply_endpoint_batch(
            diffs
                .iter()
                .cloned()
                .map(|diff| diff.map(|item| endpoint_from_sdk(&item))),
        )
    }

    fn apply_endpoint_batch(
        &mut self,
        diffs: impl IntoIterator<Item = VectorDiff<Option<ReceiptEndpoint>>>,
    ) -> BTreeMap<String, ReceiptEndpointChange> {
        let mut changes = BTreeMap::new();
        for diff in diffs {
            match &diff {
                VectorDiff::Set { index, .. } | VectorDiff::Remove { index } => {
                    record_before(&mut changes, self.entries.get(*index));
                }
                VectorDiff::PopFront => record_before(&mut changes, self.entries.front()),
                VectorDiff::PopBack => record_before(&mut changes, self.entries.back()),
                VectorDiff::Clear | VectorDiff::Reset { .. } => {
                    for entry in &self.entries {
                        record_before(&mut changes, Some(entry));
                    }
                }
                VectorDiff::Truncate { length } => {
                    for index in *length..self.entries.len() {
                        record_before(&mut changes, self.entries.get(index));
                    }
                }
                _ => {}
            }
            match &diff {
                VectorDiff::Set { value, .. }
                | VectorDiff::Insert { value, .. }
                | VectorDiff::PushFront { value }
                | VectorDiff::PushBack { value } => record_after(&mut changes, value),
                VectorDiff::Append { values } | VectorDiff::Reset { values } => {
                    for entry in values {
                        record_after(&mut changes, entry);
                    }
                }
                _ => {}
            }
            diff.apply(&mut self.entries);
        }
        // Adopt even logically unchanged endpoints so subsequent diffs retain sharing.
        for (event_id, change) in &changes {
            match &change.after {
                Some(after) => {
                    if let Some(index) = self.readers.get_mut(event_id) {
                        index.update(after.clone());
                    } else {
                        self.readers
                            .insert(event_id.clone(), ReceiptReaderIndex::new(after.clone()));
                    }
                }
                None => {
                    self.readers.remove(event_id);
                }
            }
        }
        changes.retain(|_, change| match (&change.before, &change.after) {
            (None, None) => false,
            (Some(before), Some(after)) => after.changes_since(before).next().is_some(),
            _ => true,
        });
        changes
    }
}

fn endpoint_from_sdk(item: &SdkTimelineItem) -> Option<ReceiptEndpoint> {
    let event = item.as_event()?;
    Some(ReceiptEndpoint {
        event_id: event.event_id()?.to_string(),
        receipts: event.read_receipt_snapshot().clone(),
    })
}

fn record_before(
    changes: &mut BTreeMap<String, ReceiptEndpointChange>,
    entry: Option<&Option<ReceiptEndpoint>>,
) {
    let Some(Some(entry)) = entry else { return };
    changes
        .entry(entry.event_id.clone())
        .or_insert_with(|| ReceiptEndpointChange {
            before: Some(entry.receipts.clone()),
            after: None,
        })
        .after = None;
}

fn record_after(
    changes: &mut BTreeMap<String, ReceiptEndpointChange>,
    entry: &Option<ReceiptEndpoint>,
) {
    let Some(entry) = entry else { return };
    changes
        .entry(entry.event_id.clone())
        .or_insert_with(|| ReceiptEndpointChange {
            before: None,
            after: None,
        })
        .after = Some(entry.receipts.clone());
}

#[cfg(test)]
mod tests;
