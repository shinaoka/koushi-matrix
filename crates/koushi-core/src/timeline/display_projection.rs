use std::collections::{HashMap, HashSet};
use std::sync::{Arc, atomic::Ordering};

use koushi_diagnostics::{DiagnosticEvent, DiagnosticField, DiagnosticLevel};

use crate::event::{TimelineDiff, TimelineItem, TimelineItemId, TimelineViewportObservation};
use crate::ids::{TimelineKey, TimelineKind};

// BEGIN GENERATED SIBLING IMPORTS
use super::navigation::{
    DISPLAY_PROJECTION_RESET_FALLBACKS, ROOM_REPLAY_INITIAL_ITEMS_MAX, TimelineActorGenerationGate,
    TimelineActorGenerationLease,
};
// END GENERATED SIBLING IMPORTS

#[derive(Clone, Debug, Eq, PartialEq)]

struct DisplayProjectionSlot {
    canonical_index: usize,
    item: TimelineItem,
}

/// Pre-normalization ownership for the exact canonical slots represented by
/// the desktop's current display window. Duplicate render identities remain
/// separate slots here even though `display_items` contains only their first
/// rendered owner.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(super) struct DisplayProjectionState {
    slots: Vec<DisplayProjectionSlot>,
    display_items: Vec<TimelineItem>,
}

impl DisplayProjectionState {
    pub(super) fn from_canonical_window(
        canonical_items: &[TimelineItem],
        window: std::ops::Range<usize>,
    ) -> Self {
        let start = window.start.min(canonical_items.len());
        let end = window.end.min(canonical_items.len()).max(start);
        let slots = canonical_items[start..end]
            .iter()
            .cloned()
            .enumerate()
            .map(|(offset, item)| DisplayProjectionSlot {
                canonical_index: start + offset,
                item,
            })
            .collect::<Vec<_>>();
        let display_items = normalize_display_projection_slots(&slots);
        Self {
            slots,
            display_items,
        }
    }

    pub(super) fn display_items(&self) -> &[TimelineItem] {
        &self.display_items
    }

    fn refresh_display_items(&mut self) {
        self.display_items = normalize_display_projection_slots(&self.slots);
    }
}

fn flush_pending_canonical_push_fronts(
    canonical_items: &mut Vec<TimelineItem>,
    pending_push_fronts: &mut Vec<TimelineItem>,
) {
    if pending_push_fronts.is_empty() {
        return;
    }
    pending_push_fronts.reverse();
    let prefix = std::mem::take(pending_push_fronts);
    canonical_items.splice(0..0, prefix);
}

/// Sparse weighted sequence for display membership. `Gap` compresses any
/// canonical-only run while subtree weights translate the next SDK index
/// without rescanning the bounded display or traversing/cloning the N-item
/// canonical history. SplitMix priorities give expected logarithmic split /
/// merge depth. Projection overhead is therefore expected
/// `O(W + B log(W + B) + D)` time and `O(W + B + D)` temporary space, where W
/// is represented pre-normalization membership, B is the SDK batch, and D is
/// the emitted display diff (`D = O(W)` for the private builder below). Room
/// live-edge W is hard-capped at `ROOM_REPLAY_INITIAL_ITEMS_MAX` (120).
///
/// This bound is deliberately only for projection. The existing canonical
/// `Vec<TimelineItem>` still pays its normal Vec costs for prefix/interior
/// Insert/Remove and scans a Reset payload once; none of those costs is hidden
/// in the projection counter or repeated as a projection scan per diff.
enum DisplayMembershipCell {
    Gap(usize),
    Slot(TimelineItem),
}

impl DisplayMembershipCell {
    fn canonical_len(&self) -> usize {
        match self {
            Self::Gap(len) => *len,
            Self::Slot(_) => 1,
        }
    }

    fn visible_len(&self) -> usize {
        usize::from(matches!(self, Self::Slot(_)))
    }
}

type DisplayMembershipLink = Option<Box<DisplayMembershipNode>>;

struct DisplayMembershipNode {
    cell: DisplayMembershipCell,
    left: DisplayMembershipLink,
    right: DisplayMembershipLink,
    priority: u64,
    canonical_len: usize,
    visible_len: usize,
}

struct PendingDisplayMembershipNode {
    cell: Option<DisplayMembershipCell>,
    left: Option<usize>,
    right: Option<usize>,
    priority: u64,
}

impl DisplayMembershipNode {
    fn new(cell: DisplayMembershipCell, priority: u64) -> Box<Self> {
        let canonical_len = cell.canonical_len();
        let visible_len = cell.visible_len();
        Box::new(Self {
            cell,
            left: None,
            right: None,
            priority,
            canonical_len,
            visible_len,
        })
    }

    fn refresh(&mut self) {
        self.canonical_len = display_membership_canonical_len(&self.left)
            .saturating_add(self.cell.canonical_len())
            .saturating_add(display_membership_canonical_len(&self.right));
        self.visible_len = display_membership_visible_len(&self.left)
            .saturating_add(self.cell.visible_len())
            .saturating_add(display_membership_visible_len(&self.right));
    }
}

fn display_membership_canonical_len(link: &DisplayMembershipLink) -> usize {
    link.as_ref().map_or(0, |node| node.canonical_len)
}

fn display_membership_visible_len(link: &DisplayMembershipLink) -> usize {
    link.as_ref().map_or(0, |node| node.visible_len)
}

struct DisplayMembershipRope {
    root: DisplayMembershipLink,
    next_seed: u64,
    /// Test-only count of visible payloads inspected while binding and
    /// materializing membership. Structural tree-node visits are deliberately
    /// excluded: this counter proves that payload normalization never rescans
    /// all W display rows for every diff; it is not a wall-clock complexity
    /// counter for the expected-logarithmic implicit treap.
    #[cfg(test)]
    display_payload_visits: usize,
    /// Deterministic test-only count of implicit-treap nodes traversed or
    /// constructed. Unlike payload visits, this exposes expected-logarithmic
    /// structural work for indexed batches without measuring wall-clock time.
    #[cfg(test)]
    structural_node_visits: usize,
}

impl DisplayMembershipRope {
    fn empty(display_payload_visits: usize) -> Self {
        #[cfg(not(test))]
        let _ = display_payload_visits;
        Self {
            root: None,
            next_seed: 1,
            #[cfg(test)]
            display_payload_visits,
            #[cfg(test)]
            structural_node_visits: 0,
        }
    }

    fn record_structural_node_visit(&mut self) {
        #[cfg(test)]
        {
            self.structural_node_visits = self.structural_node_visits.saturating_add(1);
        }
    }

    fn merge(
        &mut self,
        left: DisplayMembershipLink,
        right: DisplayMembershipLink,
    ) -> DisplayMembershipLink {
        match (left, right) {
            (None, right) => right,
            (left, None) => left,
            (Some(mut left), Some(mut right)) => {
                self.record_structural_node_visit();
                if left.priority >= right.priority {
                    left.right = self.merge(left.right.take(), Some(right));
                    left.refresh();
                    Some(left)
                } else {
                    right.left = self.merge(Some(left), right.left.take());
                    right.refresh();
                    Some(right)
                }
            }
        }
    }

    fn from_projection_state(
        canonical_len: usize,
        display_state: &mut DisplayProjectionState,
    ) -> (Self, bool) {
        let slots = std::mem::take(&mut display_state.slots);
        let payload_visits = slots.len();
        let mut cells = Vec::with_capacity(slots.len().saturating_mul(2).saturating_add(1));
        let mut cursor = 0;
        let mut ambiguous = false;
        for slot in slots {
            if slot.canonical_index < cursor || slot.canonical_index >= canonical_len {
                ambiguous = true;
                continue;
            }
            cells.push(DisplayMembershipCell::Gap(slot.canonical_index - cursor));
            cells.push(DisplayMembershipCell::Slot(slot.item));
            cursor = slot.canonical_index + 1;
        }
        cells.push(DisplayMembershipCell::Gap(
            canonical_len.saturating_sub(cursor),
        ));
        (Self::from_cells(cells, payload_visits), ambiguous)
    }

    fn from_canonical_window(
        canonical_items: &[TimelineItem],
        window: std::ops::Range<usize>,
    ) -> Self {
        let start = window.start.min(canonical_items.len());
        let end = window.end.min(canonical_items.len()).max(start);
        let mut cells = Vec::with_capacity(end.saturating_sub(start).saturating_add(2));
        cells.push(DisplayMembershipCell::Gap(start));
        for item in canonical_items[start..end].iter().cloned() {
            cells.push(DisplayMembershipCell::Slot(item));
        }
        cells.push(DisplayMembershipCell::Gap(canonical_items.len() - end));
        Self::from_cells(cells, end - start)
    }

    fn canonical_len(&self) -> usize {
        display_membership_canonical_len(&self.root)
    }

    fn visible_len(&self) -> usize {
        display_membership_visible_len(&self.root)
    }

    fn next_priority(&mut self) -> u64 {
        let mut value = self.next_seed;
        self.next_seed = self.next_seed.wrapping_add(1);
        value = value.wrapping_add(0x9e37_79b9_7f4a_7c15);
        value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        value ^ (value >> 31)
    }

    fn node(&mut self, cell: DisplayMembershipCell) -> DisplayMembershipLink {
        self.record_structural_node_visit();
        Some(DisplayMembershipNode::new(cell, self.next_priority()))
    }

    /// Build the initial implicit treap in one Cartesian-tree pass. Repeated
    /// `merge` appends would add an avoidable `W log W` construction term.
    fn from_cells(cells: Vec<DisplayMembershipCell>, display_payload_visits: usize) -> Self {
        let mut rope = Self::empty(display_payload_visits);
        let mut pending = Vec::<PendingDisplayMembershipNode>::with_capacity(cells.len());
        let mut stack = Vec::<usize>::with_capacity(cells.len());
        for cell in cells {
            if matches!(cell, DisplayMembershipCell::Gap(0)) {
                continue;
            }
            rope.record_structural_node_visit();
            let priority = rope.next_priority();
            let mut left = None;
            while stack
                .last()
                .is_some_and(|index| pending[*index].priority < priority)
            {
                left = stack.pop();
            }
            let index = pending.len();
            pending.push(PendingDisplayMembershipNode {
                cell: Some(cell),
                left,
                right: None,
                priority,
            });
            if let Some(parent) = stack.last().copied() {
                pending[parent].right = Some(index);
            }
            stack.push(index);
        }

        fn build(
            index: usize,
            pending: &mut [PendingDisplayMembershipNode],
        ) -> Box<DisplayMembershipNode> {
            let left = pending[index].left;
            let right = pending[index].right;
            let priority = pending[index].priority;
            let cell = pending[index]
                .cell
                .take()
                .expect("Cartesian membership node is materialized once");
            let mut node = DisplayMembershipNode::new(cell, priority);
            node.left = left.map(|child| build(child, pending));
            node.right = right.map(|child| build(child, pending));
            node.refresh();
            node
        }

        if let Some(root) = stack.first().copied() {
            rope.root = Some(build(root, &mut pending));
        }
        rope
    }

    fn split(
        &mut self,
        link: DisplayMembershipLink,
        index: usize,
    ) -> (DisplayMembershipLink, DisplayMembershipLink) {
        let Some(mut node) = link else {
            return (None, None);
        };
        self.record_structural_node_visit();
        let left_len = display_membership_canonical_len(&node.left);
        let cell_len = node.cell.canonical_len();
        if index < left_len {
            let (left, middle) = self.split(node.left.take(), index);
            node.left = middle;
            node.refresh();
            return (left, Some(node));
        }
        if index > left_len.saturating_add(cell_len) {
            let (middle, right) = self.split(
                node.right.take(),
                index.saturating_sub(left_len).saturating_sub(cell_len),
            );
            node.right = middle;
            node.refresh();
            return (Some(node), right);
        }
        if index == left_len {
            let left = node.left.take();
            node.refresh();
            return (left, Some(node));
        }
        if index == left_len.saturating_add(cell_len) {
            let right = node.right.take();
            node.refresh();
            return (Some(node), right);
        }

        let DisplayMembershipCell::Gap(gap_len) = node.cell else {
            unreachable!("a visible slot cannot be split inside its unit length");
        };
        let offset = index - left_len;
        let left_gap = self.node(DisplayMembershipCell::Gap(offset));
        let right_gap = self.node(DisplayMembershipCell::Gap(gap_len - offset));
        let left = self.merge(node.left.take(), left_gap);
        let right = self.merge(right_gap, node.right.take());
        (left, right)
    }

    fn split_root(&mut self, index: usize) -> (DisplayMembershipLink, DisplayMembershipLink) {
        let root = self.root.take();
        self.split(root, index)
    }

    fn edge_is_visible(&mut self, link: &DisplayMembershipLink, first: bool) -> bool {
        let Some(mut node) = link.as_deref() else {
            return false;
        };
        loop {
            self.record_structural_node_visit();
            let next = if first {
                node.left.as_deref()
            } else {
                node.right.as_deref()
            };
            let Some(next) = next else {
                return matches!(node.cell, DisplayMembershipCell::Slot(_));
            };
            node = next;
        }
    }

    fn insert(&mut self, index: usize, item: TimelineItem, include: Option<bool>) -> bool {
        if index > self.canonical_len() {
            return false;
        }
        let was_empty = self.canonical_len() == 0;
        let (left, right) = self.split_root(index);
        let include = include.unwrap_or_else(|| {
            was_empty || self.edge_is_visible(&left, false) || self.edge_is_visible(&right, true)
        });
        let cell = if include {
            #[cfg(test)]
            {
                self.display_payload_visits = self.display_payload_visits.saturating_add(1);
            }
            DisplayMembershipCell::Slot(item)
        } else {
            DisplayMembershipCell::Gap(1)
        };
        let middle = self.node(cell);
        let left = self.merge(left, middle);
        self.root = self.merge(left, right);
        true
    }

    fn split_one(
        &mut self,
        index: usize,
    ) -> Option<(
        DisplayMembershipLink,
        Box<DisplayMembershipNode>,
        DisplayMembershipLink,
    )> {
        if index >= self.canonical_len() {
            return None;
        }
        let (left, tail) = self.split_root(index);
        let (middle, right) = self.split(tail, 1);
        let middle = middle?;
        (middle.canonical_len == 1).then_some((left, middle, right))
    }

    fn set(&mut self, index: usize, item: TimelineItem, expected_render_id: &str) -> bool {
        let Some((left, mut middle, right)) = self.split_one(index) else {
            return false;
        };
        let valid = match &mut middle.cell {
            DisplayMembershipCell::Gap(_) => true,
            DisplayMembershipCell::Slot(old_item) => {
                #[cfg(test)]
                {
                    self.display_payload_visits = self.display_payload_visits.saturating_add(1);
                }
                let valid = timeline_item_render_id(old_item) == expected_render_id;
                *old_item = item;
                valid
            }
        };
        middle.refresh();
        let left = self.merge(left, Some(middle));
        self.root = self.merge(left, right);
        valid
    }

    fn remove(&mut self, index: usize, expected_render_id: &str) -> bool {
        let Some((left, middle, right)) = self.split_one(index) else {
            return false;
        };
        let valid = match middle.cell {
            DisplayMembershipCell::Gap(_) => true,
            DisplayMembershipCell::Slot(old_item) => {
                #[cfg(test)]
                {
                    self.display_payload_visits = self.display_payload_visits.saturating_add(1);
                }
                timeline_item_render_id(&old_item) == expected_render_id
            }
        };
        self.root = self.merge(left, right);
        valid
    }

    fn truncate(&mut self, length: usize) {
        let (left, _) = self.split_root(length.min(self.canonical_len()));
        self.root = left;
    }

    fn clear(&mut self) {
        self.root = None;
    }

    fn hide_first_visible(&mut self, link: &mut DisplayMembershipLink, remaining: &mut usize) {
        if *remaining == 0 || display_membership_visible_len(link) == 0 {
            return;
        }
        let Some(node) = link.as_mut() else {
            return;
        };
        self.record_structural_node_visit();
        self.hide_first_visible(&mut node.left, remaining);
        if *remaining > 0 && matches!(node.cell, DisplayMembershipCell::Slot(_)) {
            node.cell = DisplayMembershipCell::Gap(1);
            *remaining -= 1;
        }
        self.hide_first_visible(&mut node.right, remaining);
        node.refresh();
    }

    fn trim_to_live_edge(&mut self, max_items: Option<usize>) {
        let Some(max_items) = max_items else {
            return;
        };
        let mut excess = self.visible_len().saturating_sub(max_items);
        let mut root = self.root.take();
        self.hide_first_visible(&mut root, &mut excess);
        self.root = root;
    }

    fn materialize(mut self, display_state: &mut DisplayProjectionState) -> (usize, usize) {
        let visible_len = self.visible_len();
        let mut slots = Vec::with_capacity(visible_len);
        let mut display_items = Vec::with_capacity(visible_len);
        let mut seen = HashSet::new();
        let mut canonical_index = 0_usize;
        let mut pending = Vec::new();
        let mut cursor = self.root.take();
        while cursor.is_some() || !pending.is_empty() {
            while let Some(mut node) = cursor {
                cursor = node.left.take();
                pending.push(node);
            }
            let mut node = pending.pop().expect("membership traversal has a node");
            self.record_structural_node_visit();
            match node.cell {
                DisplayMembershipCell::Gap(len) => {
                    canonical_index = canonical_index.saturating_add(len);
                }
                DisplayMembershipCell::Slot(item) => {
                    #[cfg(test)]
                    {
                        self.display_payload_visits = self.display_payload_visits.saturating_add(1);
                    }
                    if seen.insert(timeline_item_render_id(&item)) {
                        display_items.push(item.clone());
                    }
                    slots.push(DisplayProjectionSlot {
                        canonical_index,
                        item,
                    });
                    canonical_index = canonical_index.saturating_add(1);
                }
            }
            cursor = node.right.take();
        }
        display_state.slots = slots;
        display_state.display_items = display_items;
        #[cfg(test)]
        {
            (self.display_payload_visits, self.structural_node_visits)
        }
        #[cfg(not(test))]
        {
            (0, 0)
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct DisplayProjectionContext {
    max_live_edge_items: Option<usize>,
    include_prepend: bool,
    include_append: bool,
}

impl DisplayProjectionContext {
    pub(super) fn for_timeline(
        kind: &TimelineKind,
        observation: &TimelineViewportObservation,
        restoring_anchor: bool,
    ) -> Self {
        let bounded_live_edge =
            matches!(kind, TimelineKind::Room { .. }) && observation.at_bottom && !restoring_anchor;
        Self {
            max_live_edge_items: bounded_live_edge.then_some(ROOM_REPLAY_INITIAL_ITEMS_MAX),
            include_prepend: !bounded_live_edge,
            include_append: true,
        }
    }

    #[cfg(test)]
    fn bounded_live_edge() -> Self {
        Self {
            max_live_edge_items: Some(ROOM_REPLAY_INITIAL_ITEMS_MAX),
            include_prepend: false,
            include_append: true,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct DisplayProjectionBatch {
    display_after: Vec<TimelineItem>,
    pub(super) display_diffs: Vec<TimelineDiff>,
    used_reset_fallback: bool,
    #[cfg(test)]
    display_payload_visits: usize,
    #[cfg(test)]
    structural_node_visits: usize,
}

pub(super) fn commit_sdk_batch_for_generation<R>(
    timeline_actor_generations: &Arc<TimelineActorGenerationGate>,
    key: &TimelineKey,
    actor_generation: u64,
    canonical_items: &mut Vec<TimelineItem>,
    display_state: &mut DisplayProjectionState,
    canonical_diffs: &[TimelineDiff],
    context: &DisplayProjectionContext,
    publish: impl FnOnce(
        &TimelineActorGenerationLease,
        DisplayProjectionBatch,
        &[TimelineItem],
        &DisplayProjectionState,
    ) -> R,
) -> Option<R> {
    let lease = timeline_actor_generations.try_acquire(key, actor_generation)?;
    let projection = project_sdk_batch(canonical_items, display_state, canonical_diffs, context);
    Some(publish(&lease, projection, canonical_items, display_state))
}

fn project_sdk_batch(
    canonical_items: &mut Vec<TimelineItem>,
    display_state: &mut DisplayProjectionState,
    canonical_diffs: &[TimelineDiff],
    context: &DisplayProjectionContext,
) -> DisplayProjectionBatch {
    let display_before = display_state.display_items.clone();
    let (mut membership, mut translation_ambiguous) =
        DisplayMembershipRope::from_projection_state(canonical_items.len(), display_state);
    let mut pending_push_fronts = Vec::new();

    for diff in canonical_diffs {
        if let TimelineDiff::PushFront { item } = diff {
            pending_push_fronts.push(item.clone());
            if !membership.insert(0, item.clone(), Some(context.include_prepend)) {
                translation_ambiguous = true;
            }
            membership.trim_to_live_edge(context.max_live_edge_items);
            continue;
        }
        flush_pending_canonical_push_fronts(canonical_items, &mut pending_push_fronts);
        match diff {
            TimelineDiff::PushFront { .. } => unreachable!("PushFront is batched above"),
            TimelineDiff::PushBack { item } => {
                let canonical_index = canonical_items.len();
                canonical_items.push(item.clone());
                if !membership.insert(canonical_index, item.clone(), Some(context.include_append)) {
                    translation_ambiguous = true;
                }
            }
            TimelineDiff::Insert { index, item } => {
                let old_len = canonical_items.len();
                let canonical_index = (*index).min(old_len);
                if *index > old_len {
                    translation_ambiguous = true;
                }
                canonical_items.insert(canonical_index, item.clone());
                let include = (canonical_index == 0 && context.include_prepend).then_some(true);
                if !membership.insert(canonical_index, item.clone(), include) {
                    translation_ambiguous = true;
                }
            }
            TimelineDiff::Set { index, item } => {
                // Read the pre-batch owner before changing canonical state. A
                // Set may change render identity (notably Transaction -> Event).
                let Some(old_item) = canonical_items.get(*index).cloned() else {
                    translation_ambiguous = true;
                    continue;
                };
                let old_render_identity = timeline_item_render_id(&old_item);
                canonical_items[*index] = item.clone();
                if !membership.set(*index, item.clone(), &old_render_identity) {
                    translation_ambiguous = true;
                }
            }
            TimelineDiff::Remove { index } => {
                // Capture the removed canonical identity before the Vec shifts.
                let Some(old_item) = canonical_items.get(*index).cloned() else {
                    translation_ambiguous = true;
                    continue;
                };
                let old_render_identity = timeline_item_render_id(&old_item);
                if !membership.remove(*index, &old_render_identity) {
                    translation_ambiguous = true;
                }
                canonical_items.remove(*index);
            }
            TimelineDiff::Truncate { length } => {
                if *length > canonical_items.len() {
                    translation_ambiguous = true;
                }
                canonical_items.truncate(*length);
                membership.truncate(*length);
            }
            TimelineDiff::Clear => {
                canonical_items.clear();
                membership.clear();
            }
            TimelineDiff::Reset { items } => {
                *canonical_items = items.clone();
                let start = context
                    .max_live_edge_items
                    .map(|max_items| canonical_items.len().saturating_sub(max_items))
                    .unwrap_or(0);
                membership = DisplayMembershipRope::from_canonical_window(
                    canonical_items,
                    start..canonical_items.len(),
                );
            }
        }
        membership.trim_to_live_edge(context.max_live_edge_items);
    }

    flush_pending_canonical_push_fronts(canonical_items, &mut pending_push_fronts);
    if membership.canonical_len() != canonical_items.len() {
        translation_ambiguous = true;
    }
    let (display_payload_visits, structural_node_visits) = membership.materialize(display_state);
    #[cfg(not(test))]
    let _ = (display_payload_visits, structural_node_visits);
    let display_after = display_state.display_items.clone();
    let (display_diffs, used_reset_fallback) =
        finalize_display_projection_diffs(&display_before, &display_after, translation_ambiguous);

    DisplayProjectionBatch {
        display_after,
        display_diffs,
        used_reset_fallback,
        #[cfg(test)]
        display_payload_visits,
        #[cfg(test)]
        structural_node_visits,
    }
}

fn normalize_display_projection_slots(slots: &[DisplayProjectionSlot]) -> Vec<TimelineItem> {
    let mut seen = HashSet::new();
    slots
        .iter()
        .filter(|slot| seen.insert(timeline_item_render_id(&slot.item)))
        .map(|slot| slot.item.clone())
        .collect()
}

/// Diff program produced by the one projection builder. Keeping the wrapper's
/// field private prevents the validator from silently becoming a generic
/// arbitrary-diff interpreter: the builder emits only a constant number of
/// structural groups, so validation is `O(W + D)` rather than `O(W * D)`.
struct BuiltDisplayProjectionDiffs(Vec<TimelineDiff>);

fn finalize_display_projection_diffs(
    display_before: &[TimelineItem],
    display_after: &[TimelineItem],
    translation_ambiguous: bool,
) -> (Vec<TimelineDiff>, bool) {
    let mut display_diffs = build_display_projection_diffs(display_before, display_after);
    let incrementally_valid = display_diffs.as_ref().is_some_and(|diffs| {
        validate_display_projection_diffs(display_before, diffs, display_after)
    });
    let used_reset_fallback = translation_ambiguous || !incrementally_valid;
    let display_diffs = if used_reset_fallback {
        record_display_projection_reset_fallback();
        vec![TimelineDiff::Reset {
            items: display_after.to_vec(),
        }]
    } else {
        display_diffs
            .take()
            .map(|built| built.0)
            .unwrap_or_default()
    };
    (display_diffs, used_reset_fallback)
}

fn build_display_projection_diffs(
    display_before: &[TimelineItem],
    display_after: &[TimelineItem],
) -> Option<BuiltDisplayProjectionDiffs> {
    let unique_after = display_after
        .iter()
        .map(timeline_item_render_id)
        .collect::<HashSet<_>>();
    if unique_after.len() != display_after.len() {
        return None;
    }
    if display_after.is_empty() {
        return Some(BuiltDisplayProjectionDiffs(
            (!display_before.is_empty())
                .then_some(TimelineDiff::Clear)
                .into_iter()
                .collect(),
        ));
    }

    let common_limit = display_before.len().min(display_after.len());
    let prefix_len = display_before
        .iter()
        .zip(display_after)
        .take_while(|(before, after)| before == after)
        .count();
    let suffix_len = display_before[prefix_len..]
        .iter()
        .rev()
        .zip(display_after[prefix_len..].iter().rev())
        .take(common_limit.saturating_sub(prefix_len))
        .take_while(|(before, after)| before == after)
        .count();
    let before_middle_end = display_before.len() - suffix_len;
    let after_middle_end = display_after.len() - suffix_len;
    let before_middle = &display_before[prefix_len..before_middle_end];
    let after_middle = &display_after[prefix_len..after_middle_end];
    let mut diffs = Vec::new();

    if before_middle.len() != after_middle.len() {
        diffs.extend((0..before_middle.len()).map(|_| TimelineDiff::Remove { index: prefix_len }));
        diffs.extend(
            after_middle
                .iter()
                .cloned()
                .enumerate()
                .map(|(offset, item)| TimelineDiff::Insert {
                    index: prefix_len + offset,
                    item,
                }),
        );
        return Some(BuiltDisplayProjectionDiffs(diffs));
    }

    let before_ids = before_middle
        .iter()
        .map(timeline_item_render_id)
        .collect::<HashSet<_>>();
    let positional_sets_are_safe = before_middle
        .iter()
        .zip(after_middle)
        .all(|(before, after)| {
            timeline_item_render_id(before) == timeline_item_render_id(after)
                || !before_ids.contains(&timeline_item_render_id(after))
        });
    if positional_sets_are_safe {
        for (offset, (before, after)) in before_middle.iter().zip(after_middle).enumerate() {
            if before != after {
                diffs.push(TimelineDiff::Set {
                    index: prefix_len + offset,
                    item: after.clone(),
                });
            }
        }
        return Some(BuiltDisplayProjectionDiffs(diffs));
    }

    // Identity movement cannot use positional Set: the desktop's duplicate
    // defense would update the existing owner instead of moving it. Retain the
    // longest contiguous identity run, then remove/insert only around it.
    let before_index_by_id = before_middle
        .iter()
        .enumerate()
        .map(|(index, item)| (timeline_item_render_id(item), index))
        .collect::<HashMap<_, _>>();
    let mut best = (0, 0, 0);
    let mut current = (0, 0, 0);
    let mut previous_before_index = None;
    for (after_index, item) in after_middle.iter().enumerate() {
        let Some(&before_index) = before_index_by_id.get(&timeline_item_render_id(item)) else {
            previous_before_index = None;
            current = (0, 0, 0);
            continue;
        };
        if previous_before_index.is_some_and(|previous| previous + 1 == before_index) {
            current.2 += 1;
        } else {
            current = (before_index, after_index, 1);
        }
        previous_before_index = Some(before_index);
        if current.2 > best.2 {
            best = current;
        }
    }
    let (before_keep, after_keep, keep_len) = best;
    let kept_end = before_keep + keep_len;
    diffs.extend(
        (kept_end..before_middle.len()).map(|_| TimelineDiff::Remove {
            index: prefix_len + kept_end,
        }),
    );
    diffs.extend((0..before_keep).map(|_| TimelineDiff::Remove { index: prefix_len }));
    diffs.extend(
        after_middle[..after_keep]
            .iter()
            .cloned()
            .enumerate()
            .map(|(offset, item)| TimelineDiff::Insert {
                index: prefix_len + offset,
                item,
            }),
    );
    diffs.extend(
        after_middle[after_keep + keep_len..]
            .iter()
            .cloned()
            .enumerate()
            .map(|(offset, item)| TimelineDiff::Insert {
                index: prefix_len + after_keep + keep_len + offset,
                item,
            }),
    );
    for offset in 0..keep_len {
        let before = &before_middle[before_keep + offset];
        let after = &after_middle[after_keep + offset];
        if before != after {
            diffs.push(TimelineDiff::Set {
                index: prefix_len + after_keep + offset,
                item: after.clone(),
            });
        }
    }
    Some(BuiltDisplayProjectionDiffs(diffs))
}

fn validate_display_projection_diffs(
    display_before: &[TimelineItem],
    diffs: &BuiltDisplayProjectionDiffs,
    expected_after: &[TimelineItem],
) -> bool {
    let diffs = &diffs.0;
    let mut items = display_before.to_vec();
    let mut index_by_id = items
        .iter()
        .enumerate()
        .map(|(index, item)| (timeline_item_render_id(item), index))
        .collect::<HashMap<_, _>>();
    let mut cursor = 0;
    while cursor < diffs.len() {
        match &diffs[cursor] {
            TimelineDiff::PushFront { .. } => {
                let mut prefix = Vec::new();
                let mut prefix_ids = HashSet::new();
                while let Some(TimelineDiff::PushFront { item }) = diffs.get(cursor + prefix.len())
                {
                    let item_id = timeline_item_render_id(item);
                    if index_by_id.contains_key(&item_id) || !prefix_ids.insert(item_id) {
                        return false;
                    }
                    prefix.push(item.clone());
                }
                cursor += prefix.len();
                prefix.reverse();
                items.splice(0..0, prefix);
                rebuild_display_projection_index(&items, &mut index_by_id);
            }
            TimelineDiff::PushBack { .. } => {
                let start = items.len();
                while let Some(TimelineDiff::PushBack { item }) = diffs.get(cursor) {
                    let item_id = timeline_item_render_id(item);
                    if index_by_id.contains_key(&item_id) {
                        return false;
                    }
                    index_by_id.insert(item_id, items.len());
                    items.push(item.clone());
                    cursor += 1;
                }
                if items.len() == start {
                    return false;
                }
            }
            TimelineDiff::Insert { index, .. } => {
                let start = *index;
                if start > items.len() {
                    return false;
                }
                let mut inserted = Vec::new();
                let mut inserted_ids = HashSet::new();
                while let Some(TimelineDiff::Insert { index, item }) = diffs.get(cursor) {
                    if *index != start + inserted.len() {
                        break;
                    }
                    let item_id = timeline_item_render_id(item);
                    if index_by_id.contains_key(&item_id) || !inserted_ids.insert(item_id) {
                        return false;
                    }
                    inserted.push(item.clone());
                    cursor += 1;
                }
                if inserted.is_empty() {
                    return false;
                }
                items.splice(start..start, inserted);
                rebuild_display_projection_index(&items, &mut index_by_id);
            }
            TimelineDiff::Set { index, item } => {
                if *index >= items.len() {
                    return false;
                }
                let item_id = timeline_item_render_id(item);
                let target_index = index_by_id
                    .get(&item_id)
                    .copied()
                    .filter(|existing_index| *existing_index != *index)
                    .unwrap_or(*index);
                let old_id = timeline_item_render_id(&items[target_index]);
                items[target_index] = item.clone();
                if index_by_id.get(&old_id) == Some(&target_index) {
                    index_by_id.remove(&old_id);
                }
                index_by_id.insert(item_id, target_index);
                cursor += 1;
            }
            TimelineDiff::Remove { index } => {
                let start = *index;
                let mut count = 0;
                while matches!(
                    diffs.get(cursor + count),
                    Some(TimelineDiff::Remove { index }) if *index == start
                ) {
                    count += 1;
                }
                if start >= items.len() || count > items.len() - start {
                    return false;
                }
                items.drain(start..start + count);
                cursor += count;
                rebuild_display_projection_index(&items, &mut index_by_id);
            }
            TimelineDiff::Truncate { length } => {
                if *length > items.len() {
                    return false;
                }
                items.truncate(*length);
                cursor += 1;
                rebuild_display_projection_index(&items, &mut index_by_id);
            }
            TimelineDiff::Clear => {
                items.clear();
                index_by_id.clear();
                cursor += 1;
            }
            TimelineDiff::Reset { items: reset_items } => {
                let mut seen = HashSet::new();
                items = reset_items
                    .iter()
                    .filter(|item| seen.insert(timeline_item_render_id(item)))
                    .cloned()
                    .collect();
                cursor += 1;
                rebuild_display_projection_index(&items, &mut index_by_id);
            }
        }
    }
    items == expected_after
}

fn rebuild_display_projection_index(
    items: &[TimelineItem],
    index_by_id: &mut HashMap<String, usize>,
) {
    index_by_id.clear();
    index_by_id.extend(
        items
            .iter()
            .enumerate()
            .map(|(index, item)| (timeline_item_render_id(item), index)),
    );
}

fn record_display_projection_reset_fallback() {
    let fallback_count = DISPLAY_PROJECTION_RESET_FALLBACKS.fetch_add(1, Ordering::Relaxed) + 1;
    koushi_diagnostics::record(
        DiagnosticEvent::new(
            DiagnosticLevel::Warn,
            "core.timeline_display_projection",
            "reset_fallback",
        )
        .field(DiagnosticField::count(
            "display_projection_reset_fallbacks",
            fallback_count,
        )),
    );
}

/// Applies an SDK/Core diff batch to a canonical item copy. Production uses
/// this before summary overlay to retain the raw affected-root evidence; tests
/// also use it as the compact desktop-model oracle.
pub(super) fn apply_timeline_diffs_to_items(items: &mut Vec<TimelineItem>, diffs: &[TimelineDiff]) {
    for diff in diffs {
        match diff {
            TimelineDiff::PushFront { item } => items.insert(0, item.clone()),
            TimelineDiff::PushBack { item } => items.push(item.clone()),
            TimelineDiff::Insert { index, item } => {
                let index = (*index).min(items.len());
                items.insert(index, item.clone());
            }
            TimelineDiff::Set { index, item } => {
                if let Some(slot) = items.get_mut(*index) {
                    *slot = item.clone();
                }
            }
            TimelineDiff::Remove { index } => {
                if *index < items.len() {
                    items.remove(*index);
                }
            }
            TimelineDiff::Truncate { length } => {
                items.truncate(*length);
            }
            TimelineDiff::Clear => {
                items.clear();
            }
            TimelineDiff::Reset { items: reset_items } => {
                *items = reset_items.clone();
            }
        }
    }
}

/// Applies the same render-identity normalization used by the desktop
/// TimelineStore. A bounded replay can overlap later scrollback diffs, so this
/// mirror must not retain duplicate event, transaction, or synthetic rows that
/// the webview collapses before it derives latest-reply placement.
#[cfg(test)]
pub(super) fn apply_timeline_diffs_to_display_items(
    items: &mut Vec<TimelineItem>,
    diffs: &[TimelineDiff],
) {
    for diff in diffs {
        match diff {
            TimelineDiff::PushFront { item } => insert_display_timeline_item(items, item, 0),
            TimelineDiff::PushBack { item } => {
                insert_display_timeline_item(items, item, items.len())
            }
            TimelineDiff::Insert { index, item } => {
                insert_display_timeline_item(items, item, (*index).min(items.len()))
            }
            TimelineDiff::Set { index, item } => set_display_timeline_item(items, *index, item),
            TimelineDiff::Remove { index } => {
                if *index < items.len() {
                    items.remove(*index);
                }
            }
            TimelineDiff::Truncate { length } => items.truncate(*length),
            TimelineDiff::Clear => items.clear(),
            TimelineDiff::Reset { items: reset_items } => {
                *items = normalize_display_timeline_items(reset_items);
            }
        }
    }
}

/// Applies actor-originated item revisions to the bounded display mirror.
///
/// The local `Set` index names an exact owner in the actor's canonical
/// `navigation_items`. The bounded display mirror retains that canonical index
/// on every pre-normalization slot, so duplicate render identities cannot make
/// us revise the wrong owner. An owner outside the bounded window is omitted;
/// replay reconciliation refreshes it separately from canonical state.
pub(super) fn apply_non_sdk_item_set_diffs_to_display_items(
    display_projection: &mut DisplayProjectionState,
    diffs: &[TimelineDiff],
) -> Vec<TimelineDiff> {
    let display_before = display_projection.display_items.clone();
    for diff in diffs {
        let TimelineDiff::Set { index, item } = diff else {
            continue;
        };
        if let Some(existing) = display_projection
            .slots
            .iter_mut()
            .find(|slot| slot.canonical_index == *index)
        {
            existing.item = item.clone();
        }
    }
    display_projection.refresh_display_items();
    let display_after = display_projection.display_items.clone();
    finalize_display_projection_diffs(&display_before, &display_after, false).0
}

#[cfg(test)]
fn insert_display_timeline_item(items: &mut Vec<TimelineItem>, item: &TimelineItem, index: usize) {
    let item_id = timeline_item_render_id(item);
    if items
        .iter()
        .any(|existing| timeline_item_render_id(existing) == item_id)
    {
        return;
    }
    items.insert(index.min(items.len()), item.clone());
}

#[cfg(test)]
fn set_display_timeline_item(items: &mut [TimelineItem], index: usize, item: &TimelineItem) {
    if index >= items.len() {
        return;
    }
    let item_id = timeline_item_render_id(item);
    if let Some(existing_index) = items
        .iter()
        .position(|existing| timeline_item_render_id(existing) == item_id)
        && existing_index != index
    {
        // The TypeScript store updates the existing rendered row for a Set
        // aimed at an overlapping Core slot; it does not replace the item at
        // the raw index or move later live-edge rows.
        items[existing_index] = item.clone();
        return;
    }
    items[index] = item.clone();
}

#[cfg(test)]
fn normalize_display_timeline_items(items: &[TimelineItem]) -> Vec<TimelineItem> {
    let mut seen = HashSet::new();
    items
        .iter()
        .filter(|item| seen.insert(timeline_item_render_id(item)))
        .cloned()
        .collect()
}

/// Matches `timelineItemDomId` in the TypeScript TimelineStore exactly.
fn timeline_item_render_id(item: &TimelineItem) -> String {
    match &item.id {
        TimelineItemId::Event { event_id } => event_id.clone(),
        TimelineItemId::Transaction { transaction_id } => format!("txn:{transaction_id}"),
        TimelineItemId::Synthetic { synthetic_id } => format!("syn:{synthetic_id}"),
    }
}

pub(super) fn timeline_diffs_include_prepend(diffs: &[TimelineDiff]) -> bool {
    diffs.iter().any(|diff| match diff {
        TimelineDiff::PushFront { .. } => true,
        TimelineDiff::Insert { index, .. } => *index == 0,
        TimelineDiff::Reset { .. } => true,
        TimelineDiff::PushBack { .. }
        | TimelineDiff::Set { .. }
        | TimelineDiff::Remove { .. }
        | TimelineDiff::Truncate { .. }
        | TimelineDiff::Clear => false,
    })
}

#[cfg(test)]
mod tests {

    use std::collections::HashSet;

    use std::sync::{Arc, Mutex};

    use koushi_state::AppAction;

    use tokio::sync::{broadcast, mpsc};

    use crate::event::{
        CoreEvent, ThreadSummaryDto, TimelineAnchorRestoreStatus, TimelineDiff, TimelineEvent,
        TimelineItem, TimelineItemId, TimelineMediaKind, TimelineViewportObservation,
    };

    use crate::ids::{TimelineBatchId, TimelineGeneration};

    use super::super::item_projection::timeline_item_event_id;
    use super::super::navigation::{
        ROOM_REPLAY_INITIAL_ITEMS_MAX, RestoreSettlement, TimelineActorGenerationGate,
        accept_projection_ack_for_active_actor, derive_timeline_navigation_snapshot,
        publish_restore_settlement_for_generation, publish_restore_settlement_with_lease,
    };
    use super::super::test_support::{
        fake_rid, focused_key, replacement_generation_fixture, replay_projection_services,
        room_key, timeline_item, timeline_media_item,
    };
    use super::super::thread_projection::{
        ReplayKnownDisplayContext, ReplayKnownThreadRootProjectionRegistry,
        reconcile_replay_known_root_projections_after_navigation_update,
        refresh_replay_known_root_projections,
    };
    use super::{
        DisplayProjectionBatch, DisplayProjectionContext, DisplayProjectionState,
        apply_timeline_diffs_to_display_items, apply_timeline_diffs_to_items,
        commit_sdk_batch_for_generation, project_sdk_batch,
    };

    #[test]
    fn sdk_canonical_indices_project_to_bounded_display_and_converge_local_echo() {
        let mut canonical_items = synthetic_projection_items(9_039);
        let mut transaction = timeline_item(
            "$transaction-placeholder:test",
            Some("synthetic local echo"),
            "@sender:test",
            false,
        );
        transaction.id = TimelineItemId::Transaction {
            transaction_id: "transaction:test".to_owned(),
        };
        canonical_items.push(transaction);

        let window_start = canonical_items.len() - ROOM_REPLAY_INITIAL_ITEMS_MAX;
        let mut projection = DisplayProjectionState::from_canonical_window(
            &canonical_items,
            window_start..canonical_items.len(),
        );
        let mut desktop_model = projection.display_items().to_vec();
        let confirmed = timeline_item(
            "$confirmed:test",
            Some("synthetic confirmed event"),
            "@sender:test",
            false,
        );

        for canonical_diffs in [
            vec![TimelineDiff::Set {
                index: 9_039,
                item: confirmed.clone(),
            }],
            vec![
                TimelineDiff::Remove { index: 9_039 },
                TimelineDiff::PushBack {
                    item: confirmed.clone(),
                },
            ],
        ] {
            let projected = project_sdk_batch(
                &mut canonical_items,
                &mut projection,
                &canonical_diffs,
                &DisplayProjectionContext::bounded_live_edge(),
            );
            apply_timeline_diffs_to_items(&mut desktop_model, &projected.display_diffs);

            assert!(!projected.used_reset_fallback);
            assert_eq!(desktop_model, projection.display_items());
            assert!(
                projection
                    .display_items()
                    .iter()
                    .all(|item| !matches!(item.id, TimelineItemId::Transaction { .. }))
            );
            assert_eq!(
                projection
                    .display_items()
                    .iter()
                    .filter(|item| timeline_item_event_id(item) == Some("$confirmed:test"))
                    .count(),
                1
            );
        }
    }

    fn synthetic_projection_items(count: usize) -> Vec<TimelineItem> {
        (0..count)
            .map(|index| {
                timeline_item(
                    &format!("$canonical-{index}:test"),
                    Some("synthetic"),
                    "@sender:test",
                    false,
                )
            })
            .collect()
    }

    fn historical_display_projection_context() -> DisplayProjectionContext {
        DisplayProjectionContext {
            max_live_edge_items: None,
            include_prepend: true,
            include_append: true,
        }
    }

    fn deep_display_projection_fixture() -> (Vec<TimelineItem>, DisplayProjectionState) {
        let canonical_items = synthetic_projection_items(9_040);
        let start = canonical_items.len() - ROOM_REPLAY_INITIAL_ITEMS_MAX;
        let state = DisplayProjectionState::from_canonical_window(
            &canonical_items,
            start..canonical_items.len(),
        );
        (canonical_items, state)
    }

    fn additive_display_payload_visit_bound(batch_len: usize) -> usize {
        ROOM_REPLAY_INITIAL_ITEMS_MAX
            .saturating_add(batch_len)
            .saturating_mul(2)
    }

    fn expected_log_display_structural_visit_bound(
        represented_width: usize,
        batch_len: usize,
    ) -> usize {
        let represented_nodes = represented_width
            .saturating_add(batch_len.saturating_mul(3))
            .saturating_add(2);
        let expected_log =
            usize::BITS.saturating_sub(represented_nodes.max(1).leading_zeros()) as usize;
        represented_width
            .saturating_mul(4)
            .saturating_add(
                batch_len
                    .saturating_mul(expected_log.max(1))
                    .saturating_mul(48),
            )
            .saturating_add(256)
    }

    fn assert_display_projection_converges(
        display_before: Vec<TimelineItem>,
        projection: &DisplayProjectionBatch,
    ) {
        let mut desktop_model = display_before;
        apply_timeline_diffs_to_items(&mut desktop_model, &projection.display_diffs);
        assert_eq!(desktop_model, projection.display_after);
    }

    #[test]
    fn display_projection_retains_duplicate_identity_until_its_last_owner_is_removed() {
        let first_owner = timeline_item("$duplicate:test", Some("first"), "@sender:test", false);
        let neighbor = timeline_item("$neighbor:test", Some("neighbor"), "@sender:test", false);
        let second_owner = timeline_item("$duplicate:test", Some("second"), "@sender:test", false);
        let mut canonical_items = vec![first_owner, neighbor.clone(), second_owner.clone()];
        let mut state = DisplayProjectionState::from_canonical_window(
            &canonical_items,
            0..canonical_items.len(),
        );
        let display_before = state.display_items().to_vec();

        let projection = project_sdk_batch(
            &mut canonical_items,
            &mut state,
            &[TimelineDiff::Remove { index: 0 }],
            &historical_display_projection_context(),
        );

        assert!(!projection.used_reset_fallback);
        assert_display_projection_converges(display_before, &projection);
        assert_eq!(state.display_items(), &[neighbor, second_owner]);
        assert_eq!(
            state
                .display_items()
                .iter()
                .filter(|item| timeline_item_event_id(item) == Some("$duplicate:test"))
                .count(),
            1
        );
    }

    #[test]
    fn display_projection_media_duplicate_keeps_indexed_confirmation_in_display_space() {
        let owner = timeline_media_item(
            "$media-owner:test",
            "@sender:test",
            None,
            1,
            "owner.png",
            TimelineMediaKind::Image,
        );
        let duplicate = timeline_media_item(
            "$media-owner:test",
            "@sender:test",
            None,
            2,
            "duplicate.png",
            TimelineMediaKind::Image,
        );
        let neighbor = timeline_item("$neighbor:test", Some("neighbor"), "@sender:test", false);
        let mut transaction = timeline_media_item(
            "$transaction-placeholder:test",
            "@sender:test",
            None,
            3,
            "upload.png",
            TimelineMediaKind::Image,
        );
        transaction.id = TimelineItemId::Transaction {
            transaction_id: "media-transaction:test".to_owned(),
        };
        let confirmed = timeline_media_item(
            "$confirmed-media:test",
            "@sender:test",
            None,
            4,
            "confirmed.png",
            TimelineMediaKind::Image,
        );
        let mut canonical_items = vec![owner.clone(), neighbor.clone(), transaction];
        let mut state = DisplayProjectionState::from_canonical_window(
            &canonical_items,
            0..canonical_items.len(),
        );
        let display_before = state.display_items().to_vec();

        let projection = project_sdk_batch(
            &mut canonical_items,
            &mut state,
            &[
                TimelineDiff::Insert {
                    index: 1,
                    item: duplicate,
                },
                TimelineDiff::Set {
                    index: 3,
                    item: confirmed.clone(),
                },
            ],
            &historical_display_projection_context(),
        );

        assert!(!projection.used_reset_fallback);
        assert_display_projection_converges(display_before, &projection);
        assert_eq!(state.display_items(), &[owner, neighbor, confirmed]);
        assert_eq!(
            state
                .display_items()
                .iter()
                .filter(|item| timeline_item_event_id(item) == Some("$media-owner:test"))
                .count(),
            1
        );
        assert!(
            state
                .display_items()
                .iter()
                .all(|item| !matches!(item.id, TimelineItemId::Transaction { .. }))
        );
        assert_eq!(
            state
                .display_items()
                .last()
                .and_then(|item| item.media.as_ref())
                .map(|media| media.filename.as_str()),
            Some("confirmed.png")
        );
    }

    #[test]
    fn display_projection_ignores_out_of_window_index_mutations() {
        let mut canonical_items = synthetic_projection_items(200);
        let mut state = DisplayProjectionState::from_canonical_window(&canonical_items, 50..100);
        let display_before = state.display_items().to_vec();
        let replacement = timeline_item(
            "$replacement:test",
            Some("replacement"),
            "@sender:test",
            false,
        );
        let inserted = timeline_item("$inserted:test", Some("inserted"), "@sender:test", false);

        let projection = project_sdk_batch(
            &mut canonical_items,
            &mut state,
            &[
                TimelineDiff::Set {
                    index: 10,
                    item: replacement,
                },
                TimelineDiff::Remove { index: 10 },
                TimelineDiff::Insert {
                    index: 10,
                    item: inserted,
                },
                TimelineDiff::Truncate { length: 150 },
            ],
            &historical_display_projection_context(),
        );

        assert!(!projection.used_reset_fallback);
        assert_display_projection_converges(display_before.clone(), &projection);
        assert_eq!(projection.display_after, display_before);
    }

    #[test]
    fn display_projection_includes_boundary_adjacent_insert() {
        let mut canonical_items = synthetic_projection_items(200);
        let mut state = DisplayProjectionState::from_canonical_window(&canonical_items, 50..100);
        let display_before = state.display_items().to_vec();
        let boundary = timeline_item("$boundary:test", Some("boundary"), "@sender:test", false);

        let projection = project_sdk_batch(
            &mut canonical_items,
            &mut state,
            &[TimelineDiff::Insert {
                index: 50,
                item: boundary.clone(),
            }],
            &historical_display_projection_context(),
        );

        assert!(!projection.used_reset_fallback);
        assert_display_projection_converges(display_before, &projection);
        assert_eq!(state.display_items().first(), Some(&boundary));
    }

    #[test]
    fn display_projection_live_edge_push_back_stays_bounded() {
        let mut canonical_items = synthetic_projection_items(200);
        let mut state = DisplayProjectionState::from_canonical_window(&canonical_items, 80..200);
        let display_before = state.display_items().to_vec();
        let live = timeline_item("$live:test", Some("live"), "@sender:test", false);

        let projection = project_sdk_batch(
            &mut canonical_items,
            &mut state,
            &[TimelineDiff::PushBack { item: live.clone() }],
            &DisplayProjectionContext::bounded_live_edge(),
        );

        assert!(!projection.used_reset_fallback);
        assert_display_projection_converges(display_before, &projection);
        assert_eq!(state.display_items().len(), ROOM_REPLAY_INITIAL_ITEMS_MAX);
        assert_eq!(state.display_items().last(), Some(&live));
        assert_eq!(
            state
                .display_items()
                .first()
                .and_then(timeline_item_event_id),
            Some("$canonical-81:test")
        );
    }

    #[test]
    fn display_projection_payload_work_does_not_rescan_window_per_prepend() {
        let (mut canonical_items, mut state) = deep_display_projection_fixture();
        let diffs = (0..512)
            .map(|index| TimelineDiff::PushFront {
                item: timeline_item(
                    &format!("$older-{index}:test"),
                    Some("older"),
                    "@sender:test",
                    false,
                ),
            })
            .collect::<Vec<_>>();

        let projection = project_sdk_batch(
            &mut canonical_items,
            &mut state,
            &diffs,
            &historical_display_projection_context(),
        );

        assert!(!projection.used_reset_fallback);
        assert!(
            projection.display_payload_visits <= additive_display_payload_visit_bound(diffs.len()),
            "visible payload work must stay within binding plus materialization passes"
        );
    }

    #[test]
    fn display_projection_payload_work_does_not_rescan_window_per_indexed_diff() {
        let (mut canonical_items, mut state) = deep_display_projection_fixture();
        let mut diffs = Vec::new();
        for index in 0..128 {
            diffs.extend([
                TimelineDiff::Set {
                    index: 10,
                    item: timeline_item(
                        &format!("$outside-set-{index}:test"),
                        Some("outside"),
                        "@sender:test",
                        false,
                    ),
                },
                TimelineDiff::Remove { index: 10 },
                TimelineDiff::Insert {
                    index: 10,
                    item: timeline_item(
                        &format!("$outside-insert-{index}:test"),
                        Some("outside"),
                        "@sender:test",
                        false,
                    ),
                },
            ]);
        }
        let display_before = state.display_items().to_vec();

        let projection = project_sdk_batch(
            &mut canonical_items,
            &mut state,
            &diffs,
            &historical_display_projection_context(),
        );

        assert!(!projection.used_reset_fallback);
        assert_eq!(projection.display_after, display_before);
        assert!(
            projection.display_payload_visits <= additive_display_payload_visit_bound(diffs.len()),
            "indexed diffs must not rescan all visible payloads per operation"
        );
    }

    #[test]
    fn uncapped_restore_structural_visits_stay_inside_expected_log_envelope() {
        let represented_width = 2_048;
        let mut canonical_items = synthetic_projection_items(represented_width);
        let mut state = DisplayProjectionState::from_canonical_window(
            &canonical_items,
            0..canonical_items.len(),
        );
        let display_before = state.display_items().to_vec();
        let mut diffs = Vec::new();
        for serial in 0..128 {
            let index = (serial * 37) % represented_width;
            diffs.extend([
                TimelineDiff::Set {
                    index,
                    item: timeline_item(
                        &format!("$restore-set-{serial}:test"),
                        Some("restore"),
                        "@sender:test",
                        false,
                    ),
                },
                TimelineDiff::Remove { index },
                TimelineDiff::Insert {
                    index,
                    item: timeline_item(
                        &format!("$restore-insert-{serial}:test"),
                        Some("restore"),
                        "@sender:test",
                        false,
                    ),
                },
            ]);
        }
        let restore_context = DisplayProjectionContext::for_timeline(
            &room_key().kind,
            &TimelineViewportObservation {
                at_bottom: true,
                ..TimelineViewportObservation::default()
            },
            true,
        );
        assert_eq!(restore_context.max_live_edge_items, None);

        let projection =
            project_sdk_batch(&mut canonical_items, &mut state, &diffs, &restore_context);

        assert!(!projection.used_reset_fallback);
        assert_display_projection_converges(display_before, &projection);
        assert!(
            projection.structural_node_visits
                <= expected_log_display_structural_visit_bound(represented_width, diffs.len()),
            "uncapped restore structural work exceeded the deterministic expected-log envelope"
        );
    }

    #[test]
    fn sparse_indexed_structural_envelope_is_independent_of_canonical_history_length() {
        let represented_width = 256;
        let batch_len = 256;
        let measure = |canonical_len: usize| {
            let mut canonical_items = synthetic_projection_items(canonical_len);
            let start = canonical_len - represented_width;
            let mut state = DisplayProjectionState::from_canonical_window(
                &canonical_items,
                start..canonical_items.len(),
            );
            let diffs = (0..batch_len)
                .map(|serial| {
                    let hidden_width = canonical_len - represented_width;
                    TimelineDiff::Set {
                        index: 1 + (serial * 7_919) % hidden_width.saturating_sub(1),
                        item: timeline_item(
                            &format!("$sparse-set-{serial}:test"),
                            Some("sparse"),
                            "@sender:test",
                            false,
                        ),
                    }
                })
                .collect::<Vec<_>>();
            let projection = project_sdk_batch(
                &mut canonical_items,
                &mut state,
                &diffs,
                &historical_display_projection_context(),
            );
            assert!(!projection.used_reset_fallback);
            projection.structural_node_visits
        };
        let bound = expected_log_display_structural_visit_bound(represented_width, batch_len);

        for visits in [measure(4_096), measure(65_536)] {
            assert!(
                visits <= bound,
                "structural work must be bounded by represented W and B, not canonical N"
            );
        }
    }

    #[test]
    fn display_projection_backward_push_front_prepends_historical_page() {
        let mut canonical_items = synthetic_projection_items(200);
        let mut state = DisplayProjectionState::from_canonical_window(&canonical_items, 80..200);
        let display_before = state.display_items().to_vec();
        let older = timeline_item("$older:test", Some("older"), "@sender:test", false);

        let projection = project_sdk_batch(
            &mut canonical_items,
            &mut state,
            &[TimelineDiff::PushFront {
                item: older.clone(),
            }],
            &historical_display_projection_context(),
        );

        assert!(!projection.used_reset_fallback);
        assert_display_projection_converges(display_before, &projection);
        assert_eq!(state.display_items().first(), Some(&older));
        assert_eq!(
            state.display_items().len(),
            ROOM_REPLAY_INITIAL_ITEMS_MAX + 1
        );
    }

    #[test]
    fn display_projection_clear_and_reset_replace_authoritative_display() {
        let mut canonical_items = vec![
            timeline_item("$one:test", Some("one"), "@sender:test", false),
            timeline_item("$two:test", Some("two"), "@sender:test", false),
        ];
        let mut state = DisplayProjectionState::from_canonical_window(
            &canonical_items,
            0..canonical_items.len(),
        );
        let clear_before = state.display_items().to_vec();
        let cleared = project_sdk_batch(
            &mut canonical_items,
            &mut state,
            &[TimelineDiff::Clear],
            &historical_display_projection_context(),
        );
        assert!(!cleared.used_reset_fallback);
        assert_display_projection_converges(clear_before, &cleared);
        assert!(state.display_items().is_empty());

        let reset_items = vec![
            timeline_item("$reset-one:test", Some("one"), "@sender:test", false),
            timeline_item("$reset-two:test", Some("two"), "@sender:test", false),
        ];
        let reset_before = state.display_items().to_vec();
        let reset = project_sdk_batch(
            &mut canonical_items,
            &mut state,
            &[TimelineDiff::Reset {
                items: reset_items.clone(),
            }],
            &historical_display_projection_context(),
        );
        assert!(!reset.used_reset_fallback);
        assert_display_projection_converges(reset_before, &reset);
        assert_eq!(state.display_items(), reset_items);
    }

    #[test]
    fn display_projection_invalid_translation_uses_validated_reset_fallback() {
        let mut canonical_items = vec![timeline_item(
            "$one:test",
            Some("one"),
            "@sender:test",
            false,
        )];
        let mut state = DisplayProjectionState::from_canonical_window(&canonical_items, 0..1);
        let display_before = state.display_items().to_vec();

        let projection = project_sdk_batch(
            &mut canonical_items,
            &mut state,
            &[TimelineDiff::Remove { index: 9 }],
            &historical_display_projection_context(),
        );

        assert!(projection.used_reset_fallback);
        assert!(matches!(
            projection.display_diffs.as_slice(),
            [TimelineDiff::Reset { items }] if items == &projection.display_after
        ));
        assert_display_projection_converges(display_before, &projection);
    }

    #[tokio::test]
    async fn restore_terminal_flush_publishes_two_projected_batches_once_then_rebounds_live_edge() {
        let key = room_key();
        let mut canonical_items = synthetic_projection_items(200);
        let mut state = DisplayProjectionState::from_canonical_window(&canonical_items, 80..200);
        let mut desktop_model = state.display_items().to_vec();
        let restore_context = DisplayProjectionContext::for_timeline(
            &key.kind,
            &TimelineViewportObservation {
                at_bottom: true,
                ..TimelineViewportObservation::default()
            },
            true,
        );
        let mut restore_emit_buffer = Vec::new();
        for event_id in ["$restore-1:test", "$restore-2:test"] {
            let projected = project_sdk_batch(
                &mut canonical_items,
                &mut state,
                &[TimelineDiff::PushFront {
                    item: timeline_item(event_id, Some("restore"), "@sender:test", false),
                }],
                &restore_context,
            );
            assert!(!projected.used_reset_fallback);
            restore_emit_buffer.extend(projected.display_diffs);
        }
        let expected_buffer = restore_emit_buffer.clone();
        let (actor_generations, stale_generation, current_generation) =
            replacement_generation_fixture(&key).await;
        let (event_tx, mut event_rx) = broadcast::channel(8);
        let (replay_registry, projection_service) = replay_projection_services();
        let mut next_batch_id = TimelineBatchId(7);

        assert_eq!(
            publish_restore_settlement_for_generation(
                &mut restore_emit_buffer,
                false,
                &mut next_batch_id,
                &event_tx,
                &replay_registry,
                &projection_service,
                &actor_generations,
                &key,
                stale_generation,
                TimelineGeneration(3),
                &canonical_items,
                state.display_items(),
                RestoreSettlement {
                    navigation_snapshot: None,
                    terminal: Some((fake_rid(70), TimelineAnchorRestoreStatus::Found)),
                },
            ),
            None
        );
        assert_eq!(restore_emit_buffer, expected_buffer);
        assert_eq!(next_batch_id, TimelineBatchId(7));
        assert!(matches!(
            event_rx.try_recv(),
            Err(broadcast::error::TryRecvError::Empty)
        ));

        let lease = actor_generations
            .try_acquire(&key, current_generation)
            .expect("current restore lease");
        let replacement_gate = actor_generations.clone();
        let replacement_key = key.clone();
        let replacement = tokio::spawn(async move {
            replacement_gate
                .activate_after_quiescence(&replacement_key)
                .await
        });
        for _ in 0..10 {
            if actor_generations
                .try_acquire(&key, current_generation)
                .is_none()
            {
                break;
            }
            tokio::task::yield_now().await;
        }
        let navigation_snapshot = derive_timeline_navigation_snapshot(
            &canonical_items,
            None,
            &TimelineViewportObservation::default(),
            None,
        );
        assert!(publish_restore_settlement_with_lease(
            &mut restore_emit_buffer,
            false,
            &mut next_batch_id,
            &event_tx,
            &replay_registry,
            &projection_service,
            &lease,
            &key,
            TimelineGeneration(3),
            &canonical_items,
            state.display_items(),
            RestoreSettlement {
                navigation_snapshot: Some(navigation_snapshot.clone()),
                terminal: Some((fake_rid(71), TimelineAnchorRestoreStatus::Found)),
            },
        ));
        let CoreEvent::Timeline(TimelineEvent::ItemsUpdated {
            batch_id, diffs, ..
        }) = event_rx.recv().await.expect("one terminal restore update")
        else {
            panic!("restore flush must publish ItemsUpdated");
        };
        assert_eq!(batch_id, TimelineBatchId(7));
        assert_eq!(next_batch_id, TimelineBatchId(8));
        assert!(restore_emit_buffer.is_empty());
        apply_timeline_diffs_to_items(&mut desktop_model, &diffs);
        assert_eq!(desktop_model, state.display_items());
        assert!(matches!(
            event_rx.recv().await,
            Ok(CoreEvent::Timeline(TimelineEvent::NavigationUpdated {
                snapshot,
                ..
            })) if snapshot == navigation_snapshot
        ));
        assert!(matches!(
            event_rx.recv().await,
            Ok(CoreEvent::Timeline(TimelineEvent::AnchorRestoreFinished {
                request_id,
                status: TimelineAnchorRestoreStatus::Found,
                ..
            })) if request_id == fake_rid(71)
        ));
        assert!(
            !replacement.is_finished(),
            "replacement must wait for the full restore terminal group"
        );
        drop(lease);
        replacement.await.expect("replacement task");

        let live = timeline_item(
            "$live-after-restore:test",
            Some("live"),
            "@sender:test",
            false,
        );
        let live_projection = project_sdk_batch(
            &mut canonical_items,
            &mut state,
            &[TimelineDiff::PushBack { item: live.clone() }],
            &DisplayProjectionContext::bounded_live_edge(),
        );
        assert_display_projection_converges(desktop_model, &live_projection);
        assert_eq!(state.display_items().len(), ROOM_REPLAY_INITIAL_ITEMS_MAX);
        assert_eq!(state.display_items().last(), Some(&live));
    }

    #[tokio::test]
    async fn sdk_batch_generation_fence_rejects_activity_and_state_together() {
        let key = room_key();
        let (generations, stale_generation, _current_generation) =
            replacement_generation_fixture(&key).await;
        let mut canonical_items = vec![timeline_item(
            "$before:test",
            Some("before"),
            "@sender:test",
            false,
        )];
        let canonical_before = canonical_items.clone();
        let mut state = DisplayProjectionState::from_canonical_window(&canonical_items, 0..1);
        let state_before = state.clone();
        let (action_tx, mut action_rx) = mpsc::channel(1);

        let committed = commit_sdk_batch_for_generation(
            &generations,
            &key,
            stale_generation,
            &mut canonical_items,
            &mut state,
            &[TimelineDiff::PushBack {
                item: timeline_item("$stale:test", Some("stale"), "@sender:test", false),
            }],
            &historical_display_projection_context(),
            |_lease, _projected, _canonical, _display| {
                action_tx
                    .try_send(vec![AppAction::ActivityRowsObserved { rows: Vec::new() }])
                    .expect("current batch publication");
            },
        );

        assert!(committed.is_none());
        assert_eq!(canonical_items, canonical_before);
        assert_eq!(state, state_before);
        assert!(matches!(
            action_rx.try_recv(),
            Err(mpsc::error::TryRecvError::Empty)
        ));
    }

    #[test]
    fn replay_known_display_mirror_matches_webview_identity_normalization() {
        let mut before = timeline_item("$before:test", Some("before"), "@a:test", false);
        before.timestamp_ms = Some(200);
        let mut latest_reply = timeline_item("$latest:test", Some("reply"), "@b:test", false);
        latest_reply.timestamp_ms = Some(400);
        latest_reply.thread_root = Some("$known-root:test".to_owned());
        let mut root = timeline_item("$known-root:test", Some("root"), "@a:test", false);
        root.timestamp_ms = None;
        let mut transaction = timeline_item(
            "$transaction-placeholder:test",
            Some("txn"),
            "@a:test",
            false,
        );
        transaction.id = TimelineItemId::Transaction {
            transaction_id: "local-1".to_owned(),
        };
        transaction.timestamp_ms = Some(450);
        let mut synthetic = timeline_item(
            "$synthetic-placeholder:test",
            Some("synthetic"),
            "@a:test",
            false,
        );
        synthetic.id = TimelineItemId::Synthetic {
            synthetic_id: "divider-1".to_owned(),
        };
        synthetic.timestamp_ms = Some(500);
        let mut display_items = vec![
            before.clone(),
            latest_reply.clone(),
            root.clone(),
            transaction.clone(),
            synthetic.clone(),
        ];

        // Overlapping scrollback must not add a second event/transaction/
        // synthetic row, regardless of the Push or Insert operation.
        apply_timeline_diffs_to_display_items(
            &mut display_items,
            &[
                TimelineDiff::PushFront {
                    item: latest_reply.clone(),
                },
                TimelineDiff::PushBack {
                    item: transaction.clone(),
                },
                TimelineDiff::Insert {
                    index: 1,
                    item: synthetic.clone(),
                },
                TimelineDiff::PushBack { item: root.clone() },
            ],
        );
        assert_eq!(display_items.len(), 5);

        // A Set for an overlapping Core slot updates its already-rendered row
        // without replacing/moving the item currently at the raw index.
        let mut updated_latest_reply = latest_reply.clone();
        updated_latest_reply.body = Some("updated reply".to_owned());
        apply_timeline_diffs_to_display_items(
            &mut display_items,
            &[TimelineDiff::Set {
                index: 0,
                item: updated_latest_reply.clone(),
            }],
        );
        assert_eq!(
            timeline_item_event_id(&display_items[0]),
            Some("$before:test")
        );
        assert_eq!(
            display_items
                .iter()
                .find(|item| timeline_item_event_id(item) == Some("$latest:test"))
                .and_then(|item| item.body.as_deref()),
            Some("updated reply")
        );

        // Remove and Reset use the normalized sequence as the webview does;
        // Reset keeps the first occurrence of each render identity.
        apply_timeline_diffs_to_display_items(
            &mut display_items,
            &[TimelineDiff::Remove { index: 0 }],
        );
        assert_eq!(
            timeline_item_event_id(&display_items[0]),
            Some("$latest:test")
        );
        apply_timeline_diffs_to_display_items(
            &mut display_items,
            &[TimelineDiff::Reset {
                items: vec![
                    latest_reply.clone(),
                    updated_latest_reply,
                    root.clone(),
                    root,
                    transaction.clone(),
                    transaction,
                    synthetic.clone(),
                    synthetic,
                ],
            }],
        );
        assert_eq!(display_items.len(), 4);
        let normalized_context = ReplayKnownDisplayContext::from_display_items(&display_items);
        assert!(normalized_context.event_ids.contains("$latest:test"));
        assert!(
            normalized_context
                .exact_thread_reply_pairs
                .contains(&("$known-root:test".to_owned(), "$latest:test".to_owned()))
        );
        assert_eq!(normalized_context.activity_range, Some((400, 400)));

        // Truncate and Clear operate on the same normalized sequence, so stale
        // IDs/pairs cannot survive in replay-known display evidence.
        apply_timeline_diffs_to_display_items(
            &mut display_items,
            &[TimelineDiff::Truncate { length: 1 }],
        );
        let truncated_context = ReplayKnownDisplayContext::from_display_items(&display_items);
        assert_eq!(
            truncated_context.event_ids,
            HashSet::from(["$latest:test".to_owned()])
        );
        assert_eq!(truncated_context.activity_range, Some((400, 400)));
        assert!(
            truncated_context
                .exact_thread_reply_pairs
                .contains(&("$known-root:test".to_owned(), "$latest:test".to_owned()))
        );
        apply_timeline_diffs_to_display_items(&mut display_items, &[TimelineDiff::Clear]);
        assert_eq!(
            ReplayKnownDisplayContext::from_display_items(&display_items),
            ReplayKnownDisplayContext::default()
        );

        // A duplicate, non-displayed cache overlap leaves a retained
        // replay-known root untouched: the mirror cannot invent a Clear/Ready
        // transition that the webview itself would not render.
        let key = room_key();
        let mut root = timeline_item("$known-root:test", Some("root"), "@a:test", false);
        root.thread_summary = Some(ThreadSummaryDto {
            reply_count: 1,
            latest_event_id: Some("$latest:test".to_owned()),
            latest_sender: None,
            latest_sender_label: None,
            latest_body_preview: None,
            latest_timestamp_ms: Some(400),
        });
        let mut base_before = timeline_item("$base-before:test", Some("before"), "@a:test", false);
        base_before.timestamp_ms = Some(200);
        let mut base_after = timeline_item("$base-after:test", Some("after"), "@a:test", false);
        base_after.timestamp_ms = Some(500);
        let mut bounded_display = vec![base_before, base_after.clone()];
        let registry = Arc::new(Mutex::new(
            ReplayKnownThreadRootProjectionRegistry::default(),
        ));
        let initial = refresh_replay_known_root_projections(
            &registry,
            &key,
            &[root.clone(), latest_reply],
            &bounded_display,
        );
        assert_eq!(initial.ready.len(), 1);
        apply_timeline_diffs_to_display_items(
            &mut bounded_display,
            &[TimelineDiff::PushBack { item: base_after }],
        );
        let unchanged = reconcile_replay_known_root_projections_after_navigation_update(
            &registry,
            &key,
            &[root],
            &ReplayKnownDisplayContext::from_display_items(&bounded_display),
        );
        assert!(unchanged.ready.is_empty());
        assert!(unchanged.stale.is_empty());
    }

    #[tokio::test]
    async fn projection_ack_requires_exact_identity_and_current_actor_generation() {
        let key = focused_key();
        let generations = Arc::new(TimelineActorGenerationGate::default());
        let actor_generation = generations.activate_after_quiescence(&key).await.generation;
        let projection_request_id = fake_rid(81);
        let projection_generation = TimelineGeneration(4);
        let mut acknowledged = false;

        assert!(!accept_projection_ack_for_active_actor(
            &generations,
            &key,
            actor_generation,
            projection_request_id,
            projection_generation,
            fake_rid(80),
            projection_generation,
            &mut acknowledged,
        ));
        assert!(!acknowledged);
        assert!(!accept_projection_ack_for_active_actor(
            &generations,
            &key,
            actor_generation,
            projection_request_id,
            projection_generation,
            projection_request_id,
            TimelineGeneration(3),
            &mut acknowledged,
        ));
        assert!(!acknowledged);

        let replacement_generation = generations.activate_after_quiescence(&key).await.generation;
        assert_ne!(replacement_generation, actor_generation);
        assert!(!accept_projection_ack_for_active_actor(
            &generations,
            &key,
            actor_generation,
            projection_request_id,
            projection_generation,
            projection_request_id,
            projection_generation,
            &mut acknowledged,
        ));
        assert!(!acknowledged);
        assert!(accept_projection_ack_for_active_actor(
            &generations,
            &key,
            replacement_generation,
            projection_request_id,
            projection_generation,
            projection_request_id,
            projection_generation,
            &mut acknowledged,
        ));
        assert!(acknowledged);
    }
}
