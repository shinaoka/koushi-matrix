use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::profile::{
    AvatarImage, ProfileState, original_user_display_name, resolve_user_display_name,
};

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct LiveSignalsState {
    pub rooms: BTreeMap<String, RoomLiveSignals>,
    pub presence: BTreeMap<String, PresenceKind>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct RoomLiveSignals {
    pub receipts_by_event: BTreeMap<String, LiveEventReceiptSummary>,
    pub fully_read_event_id: Option<String>,
    pub typing_user_ids: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct LiveReadReceipt {
    pub user_id: String,
    pub display_name: Option<String>,
    #[serde(default)]
    pub original_display_label: String,
    pub avatar: Option<AvatarImage>,
    pub timestamp_ms: Option<u64>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct LiveEventReceiptSummary {
    pub readers: Vec<LiveReadReceipt>,
    pub total_count: u64,
    pub overflow_count: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct LiveEventReceipts {
    pub event_id: String,
    pub receipts: Vec<LiveReadReceipt>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct LiveRoomSignalUpdate {
    pub receipts_by_event: Vec<LiveEventReceipts>,
    pub fully_read_event_id: Option<String>,
    pub typing_user_ids: Vec<String>,
}

impl LiveRoomSignalUpdate {
    pub fn into_room_signals(self) -> RoomLiveSignals {
        self.into_room_signals_with_profiles(&ProfileState::default(), None)
    }

    pub fn into_room_signals_with_profiles(
        self,
        profiles: &ProfileState,
        own_user_id: Option<&str>,
    ) -> RoomLiveSignals {
        let receipts_by_event = self
            .receipts_by_event
            .into_iter()
            .map(|entry| {
                let receipts = normalize_receipts(entry.receipts, profiles, own_user_id);
                (entry.event_id, receipts)
            })
            .collect();

        RoomLiveSignals {
            receipts_by_event,
            fully_read_event_id: self.fully_read_event_id,
            typing_user_ids: sorted_unique(self.typing_user_ids),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PresenceKind {
    Online,
    Away,
    Offline,
}

fn normalize_receipts(
    receipts: Vec<LiveReadReceipt>,
    profiles: &ProfileState,
    own_user_id: Option<&str>,
) -> LiveEventReceiptSummary {
    let mut by_user = BTreeMap::new();
    for receipt in receipts {
        // Exclude the current user's own receipts before building the readers
        // list — own reads (including reads on other devices) must not appear
        // in the displayed readers or affect the counts.
        if own_user_id.is_some_and(|own| own == receipt.user_id) {
            continue;
        }
        let receipt = enrich_receipt(receipt, profiles, own_user_id);
        by_user
            .entry(receipt.user_id.clone())
            .and_modify(|existing: &mut LiveReadReceipt| {
                if receipt_is_newer(&receipt, existing) {
                    *existing = receipt.clone();
                }
            })
            .or_insert(receipt);
    }
    let mut readers = by_user.into_values().collect::<Vec<_>>();
    readers.sort_by(|left, right| {
        right
            .timestamp_ms
            .unwrap_or_default()
            .cmp(&left.timestamp_ms.unwrap_or_default())
            .then_with(|| left.user_id.cmp(&right.user_id))
    });

    let total_count = readers.len() as u64;

    LiveEventReceiptSummary {
        readers,
        total_count,
        overflow_count: 0,
    }
}

fn receipt_is_newer(candidate: &LiveReadReceipt, existing: &LiveReadReceipt) -> bool {
    candidate.timestamp_ms.unwrap_or_default() >= existing.timestamp_ms.unwrap_or_default()
}

fn enrich_receipt(
    mut receipt: LiveReadReceipt,
    profiles: &ProfileState,
    own_user_id: Option<&str>,
) -> LiveReadReceipt {
    let own_profile = own_user_id
        .filter(|user_id| *user_id == receipt.user_id)
        .map(|_| &profiles.own);
    let user_profile = profiles.users.get(&receipt.user_id);

    let receipt_display_name = receipt.display_name.clone();
    let receipt_original_display_label = receipt.original_display_label.clone();
    let original_source = if receipt_original_display_label.trim().is_empty() {
        receipt_display_name.as_deref()
    } else {
        Some(receipt_original_display_label.as_str())
    };
    let display_label = resolve_user_display_name(
        profiles,
        &receipt.user_id,
        receipt_display_name.as_deref(),
        own_user_id,
    );
    let original_display_label =
        original_user_display_name(profiles, &receipt.user_id, original_source, own_user_id);
    receipt.display_name = Some(display_label);
    receipt.original_display_label = original_display_label;
    if receipt.avatar.is_none() {
        receipt.avatar = own_profile
            .and_then(|profile| profile.avatar.clone())
            .or_else(|| user_profile.and_then(|profile| profile.avatar.clone()));
    }
    receipt
}

fn sorted_unique(mut values: Vec<String>) -> Vec<String> {
    values.sort();
    values.dedup();
    values
}
