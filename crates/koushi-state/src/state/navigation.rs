use std::{
    collections::{BTreeMap, HashMap},
    fmt,
};

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum EventNavigationSource {
    Activity,
    Search,
    Pinned,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MissingTargetPolicy {
    LiveFallback,
    Fail,
}

#[derive(Clone, Copy, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum EventNavigationState {
    Idle,
    Opening {
        generation: u64,
        source: EventNavigationSource,
    },
    Anchored {
        generation: u64,
        source: EventNavigationSource,
    },
    LiveFallback {
        generation: u64,
        source: EventNavigationSource,
    },
    Failed {
        generation: u64,
        source: EventNavigationSource,
        #[serde(rename = "failureKind")]
        failure_kind: EventNavigationFailureKind,
    },
}

impl Default for EventNavigationState {
    fn default() -> Self {
        Self::Idle
    }
}

impl EventNavigationState {
    pub fn generation(self) -> u64 {
        match self {
            Self::Idle => 0,
            Self::Opening { generation, .. }
            | Self::Anchored { generation, .. }
            | Self::LiveFallback { generation, .. }
            | Self::Failed { generation, .. } => generation,
        }
    }
}

impl fmt::Debug for EventNavigationState {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Idle => formatter.write_str("EventNavigationState::Idle"),
            Self::Opening { generation, source } => formatter
                .debug_struct("EventNavigationOpening")
                .field("generation", generation)
                .field("source", source)
                .finish(),
            Self::Anchored { generation, source } => formatter
                .debug_struct("EventNavigationAnchored")
                .field("generation", generation)
                .field("source", source)
                .finish(),
            Self::LiveFallback { generation, source } => formatter
                .debug_struct("EventNavigationLiveFallback")
                .field("generation", generation)
                .field("source", source)
                .finish(),
            Self::Failed {
                generation,
                source,
                failure_kind,
            } => formatter
                .debug_struct("EventNavigationFailed")
                .field("generation", generation)
                .field("source", source)
                .field("failure_kind", failure_kind)
                .finish(),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum EventNavigationFailureKind {
    TargetMissing,
    RoomUnavailable,
    SessionUnavailable,
    Timeline,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TimelineScrollAnchorEdge {
    #[default]
    Top,
    Bottom,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct TimelineScrollAnchor {
    pub event_id: String,
    #[serde(default)]
    pub edge: TimelineScrollAnchorEdge,
    pub offset_px: i32,
    pub updated_at_ms: u64,
}

/// Rust-owned main-pane timeline mode (issue #161).
///
/// `NavigationState.main_timeline_anchor == None` means the main pane renders
/// the room's live timeline (live edge). `Some` means jump-to-date resolved an
/// event outside the loaded live window and the main pane is anchored to that
/// event — it renders the event-focused timeline until the user returns to live.
/// This is a guarded state machine: `Some` is only ever set for the active,
/// known room and is cleared on any room change or return-to-live.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct MainTimelineAnchor {
    pub event_id: String,
}

/// Which conversation surface a Space was last showing.
///
/// Every Space in this product has a DMs surface and a Rooms surface, so a
/// remembered conversation is only meaningful together with the surface it was
/// selected on (#445).
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SpaceConversationSurface {
    #[default]
    Rooms,
    Dms,
}

/// The remembered navigation selection for one Space.
///
/// `room_id` is `None` for a Space that has been visited without ever settling
/// on a conversation; the surface is still remembered in that case.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct SpaceNavigationSelection {
    #[serde(default)]
    pub surface: SpaceConversationSurface,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub room_id: Option<String>,
}

#[derive(Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct NavigationState {
    pub active_space_id: Option<String>,
    pub active_room_id: Option<String>,
    #[serde(default)]
    pub home_selection: HomeSelection,
    #[serde(default)]
    pub space_local_presentations: SpaceLocalPresentations,
    #[serde(default)]
    pub legacy_frontend_preferences_imported: bool,
    #[serde(default)]
    pub space_order: Vec<String>,
    /// Legacy non-DM-only memory. Retained so an older build and already
    /// persisted `navigation.v1` payloads keep working; superseded by
    /// `last_selection_by_space_id`.
    #[serde(default)]
    pub last_room_by_space_id: BTreeMap<String, String>,
    #[serde(default)]
    pub last_selection_by_space_id: BTreeMap<String, SpaceNavigationSelection>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub room_scroll_anchors: BTreeMap<String, TimelineScrollAnchor>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub main_timeline_anchor: Option<MainTimelineAnchor>,
    #[serde(default)]
    pub event_navigation: EventNavigationState,
}

impl Default for NavigationState {
    fn default() -> Self {
        Self {
            active_space_id: None,
            active_room_id: None,
            home_selection: HomeSelection::default(),
            space_local_presentations: SpaceLocalPresentations::default(),
            legacy_frontend_preferences_imported: false,
            space_order: Vec::new(),
            last_room_by_space_id: BTreeMap::new(),
            last_selection_by_space_id: BTreeMap::new(),
            room_scroll_anchors: BTreeMap::new(),
            main_timeline_anchor: None,
            event_navigation: EventNavigationState::Idle,
        }
    }
}

impl fmt::Debug for NavigationState {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NavigationState")
            .field("active_space_present", &self.active_space_id.is_some())
            .field("active_room_present", &self.active_room_id.is_some())
            .field("home_selection", &self.home_selection)
            .field(
                "space_local_presentation_count",
                &self.space_local_presentations.0.len(),
            )
            .field(
                "legacy_frontend_preferences_imported",
                &self.legacy_frontend_preferences_imported,
            )
            .field("space_order_count", &self.space_order.len())
            .field("last_room_count", &self.last_room_by_space_id.len())
            .field(
                "last_selection_count",
                &self.last_selection_by_space_id.len(),
            )
            .field("room_scroll_anchor_count", &self.room_scroll_anchors.len())
            .field(
                "main_timeline_anchored",
                &self.main_timeline_anchor.is_some(),
            )
            .field("event_navigation", &self.event_navigation)
            .finish()
    }
}

#[derive(Clone, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum HomeSelection {
    Activity,
    Explore,
    Invites,
    DirectMessage { room_id: String },
}

impl Default for HomeSelection {
    fn default() -> Self {
        Self::Activity
    }
}

impl fmt::Debug for HomeSelection {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Activity => "Activity",
            Self::Explore => "Explore",
            Self::Invites => "Invites",
            Self::DirectMessage { .. } => "DirectMessage { room_id: RoomId(..) }",
        })
    }
}

#[derive(Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct SpaceLocalPresentation {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
}

impl fmt::Debug for SpaceLocalPresentation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SpaceLocalPresentation")
            .field("name_present", &self.name.is_some())
            .field("icon_present", &self.icon.is_some())
            .finish()
    }
}

pub const MAX_SPACE_LOCAL_PRESENTATIONS: usize = 256;

#[derive(Clone, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SpaceLocalPresentations(pub BTreeMap<String, SpaceLocalPresentation>);

impl fmt::Debug for SpaceLocalPresentations {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SpaceLocalPresentations")
            .field("count", &self.0.len())
            .finish()
    }
}

#[derive(Clone, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum NavigationPreferenceUpdate {
    SetHomeSelection {
        selection: HomeSelection,
    },
    SetSpacePresentation {
        space_id: String,
        presentation: Option<SpaceLocalPresentation>,
    },
    ImportLegacy {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        home_selection: Option<HomeSelection>,
        #[serde(default)]
        space_local_presentations: SpaceLocalPresentations,
    },
}

impl NavigationState {
    pub fn apply_preference_update(&mut self, update: NavigationPreferenceUpdate) -> bool {
        match update {
            NavigationPreferenceUpdate::SetHomeSelection { selection } => {
                if self.home_selection == selection {
                    return false;
                }
                self.home_selection = selection;
                true
            }
            NavigationPreferenceUpdate::SetSpacePresentation {
                space_id,
                presentation,
            } => match presentation {
                Some(presentation) => {
                    if self.space_local_presentations.0.get(&space_id) == Some(&presentation) {
                        false
                    } else if !self.space_local_presentations.0.contains_key(&space_id)
                        && self.space_local_presentations.0.len() >= MAX_SPACE_LOCAL_PRESENTATIONS
                    {
                        false
                    } else {
                        self.space_local_presentations
                            .0
                            .insert(space_id, presentation);
                        true
                    }
                }
                None => self.space_local_presentations.0.remove(&space_id).is_some(),
            },
            NavigationPreferenceUpdate::ImportLegacy {
                home_selection,
                space_local_presentations,
            } => {
                if self.legacy_frontend_preferences_imported {
                    return false;
                }
                if self.home_selection == HomeSelection::Activity
                    && let Some(home_selection) = home_selection
                {
                    self.home_selection = home_selection;
                }
                for (space_id, presentation) in space_local_presentations.0 {
                    self.space_local_presentations
                        .0
                        .entry(space_id)
                        .or_insert(presentation);
                }
                self.legacy_frontend_preferences_imported = true;
                true
            }
        }
    }
}

impl fmt::Debug for NavigationPreferenceUpdate {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SetHomeSelection { selection } => formatter
                .debug_struct("SetHomeSelection")
                .field("selection", selection)
                .finish(),
            Self::SetSpacePresentation { presentation, .. } => formatter
                .debug_struct("SetSpacePresentation")
                .field("space_id", &"SpaceId(..)")
                .field("presentation", presentation)
                .finish(),
            Self::ImportLegacy {
                home_selection,
                space_local_presentations,
            } => formatter
                .debug_struct("ImportLegacy")
                .field("home_selection", home_selection)
                .field(
                    "space_local_presentation_count",
                    &space_local_presentations.0.len(),
                )
                .finish(),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum RoomListFilter {
    #[default]
    Rooms,
    Unread,
    People,
    Favourites,
    Invites,
}

pub use crate::state::settings::RoomListSort;

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct RoomListProjection {
    #[serde(default)]
    pub readiness: RoomListReadiness,
    pub active_filter: RoomListFilter,
    pub sort: RoomListSort,
    pub items: Vec<RoomListProjectionItem>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RoomListSource {
    #[default]
    Cache,
    Live,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RoomListFailureKind {
    Connectivity,
    Service,
    Stopped,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum RoomListReadiness {
    #[default]
    Uninitialized,
    Loading {
        source: RoomListSource,
        generation: u64,
    },
    Ready {
        source: RoomListSource,
        generation: u64,
    },
    Failed {
        source: RoomListSource,
        generation: u64,
        #[serde(rename = "failureKind")]
        kind: RoomListFailureKind,
    },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RoomListProjectionItem {
    pub room_id: String,
    pub kind: RoomListEntryKind,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RoomListEntryKind {
    Room,
    Invite,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum FocusedContextState {
    Closed,
    Opening {
        room_id: String,
        event_id: String,
    },
    Open {
        room_id: String,
        event_id: String,
        is_subscribed: bool,
    },
}

/// Rust-owned room-list filter + activity-sort projection. React renders this
/// snapshot and must not recompute filter membership or sort order.
pub fn compute_room_list_projection(
    active_filter: RoomListFilter,
    sort: RoomListSort,
    active_space_id: Option<&str>,
    spaces: &[super::room::SpaceSummary],
    rooms: &[super::room::RoomSummary],
    room_notification_settings: &HashMap<String, super::RoomNotificationSettings>,
    invites: &[super::room::InvitePreview],
) -> RoomListProjection {
    let active_space_child_room_ids = active_space_id.and_then(|active_space_id| {
        spaces
            .iter()
            .find(|space| space.space_id == active_space_id)
            .map(|space| space.child_room_ids.as_slice())
    });
    let mut items: Vec<RoomListProjectionItem> = match active_filter {
        RoomListFilter::Invites => invites
            .iter()
            .map(|invite| RoomListProjectionItem {
                room_id: invite.room_id.clone(),
                kind: RoomListEntryKind::Invite,
            })
            .collect(),
        _ => rooms
            .iter()
            .filter(|room| {
                room_visible_in_active_space(room, active_space_id, active_space_child_room_ids)
            })
            .filter(|room| match active_filter {
                RoomListFilter::Unread => {
                    room.unread_count > 0
                        || room.notification_count > 0
                        || room.highlight_count > 0
                        || room.marked_unread
                }
                RoomListFilter::People => room.is_dm,
                RoomListFilter::Rooms => {
                    !room.is_dm && room.tags.favourite.is_none() && room.tags.low_priority.is_none()
                }
                RoomListFilter::Favourites => room.tags.favourite.is_some(),
                RoomListFilter::Invites => unreachable!(),
            })
            .map(|room| RoomListProjectionItem {
                room_id: room.room_id.clone(),
                kind: RoomListEntryKind::Room,
            })
            .collect(),
    };

    match sort {
        RoomListSort::Activity => {
            let room_by_id: HashMap<&str, &super::room::RoomSummary> = rooms
                .iter()
                .map(|room| (room.room_id.as_str(), room))
                .collect();
            items.sort_by(|left, right| {
                super::room::RoomSummary::compare_attention_activity(
                    room_by_id.get(left.room_id.as_str()).copied(),
                    room_notification_settings
                        .get(left.room_id.as_str())
                        .map(|settings| settings.mode),
                    room_by_id.get(right.room_id.as_str()).copied(),
                    room_notification_settings
                        .get(right.room_id.as_str())
                        .map(|settings| settings.mode),
                )
            });
        }
        RoomListSort::RecentFirst => {
            let room_by_id: HashMap<&str, &super::room::RoomSummary> = rooms
                .iter()
                .map(|room| (room.room_id.as_str(), room))
                .collect();
            items.sort_by(|left, right| {
                super::compare_conversation_activity(
                    room_by_id.get(left.room_id.as_str()).copied(),
                    room_by_id.get(right.room_id.as_str()).copied(),
                )
            });
        }
        RoomListSort::NormalLocale => {
            let label_by_id: HashMap<&str, &str> = rooms
                .iter()
                .map(|room| (room.room_id.as_str(), room.display_label.as_str()))
                .collect();
            items.sort_by(|left, right| {
                let left_label = label_by_id
                    .get(left.room_id.as_str())
                    .copied()
                    .unwrap_or(left.room_id.as_str())
                    .to_lowercase();
                let right_label = label_by_id
                    .get(right.room_id.as_str())
                    .copied()
                    .unwrap_or(right.room_id.as_str())
                    .to_lowercase();
                left_label
                    .cmp(&right_label)
                    .then_with(|| left.room_id.cmp(&right.room_id))
            });
        }
    }

    RoomListProjection {
        readiness: RoomListReadiness::default(),
        active_filter,
        sort,
        items,
    }
}

fn room_visible_in_active_space(
    room: &super::room::RoomSummary,
    active_space_id: Option<&str>,
    active_space_child_room_ids: Option<&[String]>,
) -> bool {
    let Some(active_space_id) = active_space_id else {
        return true;
    };
    if room.is_dm {
        return room
            .dm_space_ids
            .iter()
            .any(|space_id| space_id == active_space_id);
    }
    room.parent_space_ids
        .iter()
        .any(|space_id| space_id == active_space_id)
        || active_space_child_room_ids.is_some_and(|child_room_ids| {
            child_room_ids
                .iter()
                .any(|room_id| room_id == &room.room_id)
        })
}
