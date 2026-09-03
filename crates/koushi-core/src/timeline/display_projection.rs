use std::collections::{HashMap, HashSet};
use std::sync::{Arc, atomic::Ordering};

use koushi_diagnostics::{DiagnosticEvent, DiagnosticField, DiagnosticLevel};

use crate::threads_list::ThreadRootDisplayData;
use koushi_protocol::event::{
    TimelineDiff, TimelineDisplayKind, TimelineDisplayMetadata, TimelineItem, TimelineItemId,
    TimelineViewportObservation,
};
use koushi_protocol::ids::{TimelineKey, TimelineKind};
use koushi_state::TimelineThreadRootOrder;

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
    /// Manager-owned accepted sends which are not yet represented by an SDK slot.
    pending_items: Vec<TimelineItem>,
    /// Transaction identities whose late SDK local echo must not reappear.
    suppressed_transaction_ids: HashSet<String>,
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
        let display_items = normalize_display_projection_slots(&slots)
            .iter()
            .filter_map(decorate_event_item)
            .collect();
        Self {
            slots,
            display_items,
            pending_items: Vec::new(),
            suppressed_transaction_ids: HashSet::new(),
        }
    }

    pub(super) fn display_items(&self) -> &[TimelineItem] {
        &self.display_items
    }

    pub(super) fn reproject(&mut self, context: &DisplayProjectionContext) -> Vec<TimelineDiff> {
        let before = self.display_items.clone();
        self.display_items = project_display_items(
            &self.slots,
            &self.pending_items,
            &self.suppressed_transaction_ids,
            context,
        );
        finalize_display_projection_diffs(&before, &self.display_items, false).0
    }

    pub(super) fn set_pending_inputs(
        &mut self,
        pending_items: Vec<TimelineItem>,
        suppressed_transaction_ids: HashSet<String>,
    ) {
        self.pending_items = pending_items;
        self.suppressed_transaction_ids = suppressed_transaction_ids;
    }

    pub(super) fn replace_pending(
        &mut self,
        pending_items: Vec<TimelineItem>,
        suppressed_transaction_ids: HashSet<String>,
        context: &DisplayProjectionContext,
    ) -> Vec<TimelineDiff> {
        let before = self.display_items.clone();
        self.set_pending_inputs(pending_items, suppressed_transaction_ids);
        self.display_items = project_display_items(
            &self.slots,
            &self.pending_items,
            &self.suppressed_transaction_ids,
            context,
        );
        finalize_display_projection_diffs(&before, &self.display_items, false).0
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

    fn materialize(
        mut self,
        display_state: &mut DisplayProjectionState,
        context: &DisplayProjectionContext,
    ) -> (usize, usize) {
        let visible_len = self.visible_len();
        let mut slots = Vec::with_capacity(visible_len);
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
        display_state.display_items = project_display_items(
            &display_state.slots,
            &display_state.pending_items,
            &display_state.suppressed_transaction_ids,
            context,
        );
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

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct DisplayProjectionContext {
    max_live_edge_items: Option<usize>,
    include_prepend: bool,
    include_append: bool,
    project_thread_roots: bool,
    pub(super) thread_root_order: TimelineThreadRootOrder,
    pub(super) thread_roots: Vec<ThreadRootDisplayData>,
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
            project_thread_roots: matches!(kind, TimelineKind::Room { .. }),
            thread_root_order: TimelineThreadRootOrder::RootEvent,
            thread_roots: Vec::new(),
        }
    }

    pub(super) fn with_thread_roots(
        mut self,
        order: TimelineThreadRootOrder,
        thread_roots: Vec<ThreadRootDisplayData>,
    ) -> Self {
        self.thread_root_order = order;
        self.thread_roots = thread_roots;
        self
    }

    #[cfg(test)]
    fn bounded_live_edge() -> Self {
        Self {
            max_live_edge_items: Some(ROOM_REPLAY_INITIAL_ITEMS_MAX),
            include_prepend: false,
            include_append: true,
            project_thread_roots: true,
            thread_root_order: TimelineThreadRootOrder::RootEvent,
            thread_roots: Vec::new(),
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
    let (display_payload_visits, structural_node_visits) =
        membership.materialize(display_state, context);
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

fn project_display_items(
    slots: &[DisplayProjectionSlot],
    pending_items: &[TimelineItem],
    suppressed_transaction_ids: &HashSet<String>,
    context: &DisplayProjectionContext,
) -> Vec<TimelineItem> {
    if !context.project_thread_roots {
        let mut rendered = slots
            .iter()
            .filter(|slot| match &slot.item.id {
                TimelineItemId::Transaction { transaction_id } => {
                    !suppressed_transaction_ids.contains(transaction_id)
                }
                _ => true,
            })
            .filter_map(|slot| decorate_event_item(&slot.item))
            .collect::<Vec<_>>();
        let mut seen = rendered
            .iter()
            .map(timeline_item_render_id)
            .collect::<HashSet<_>>();
        for item in pending_items {
            let id = timeline_item_render_id(item);
            if seen.insert(id) {
                rendered.push(decorate_event_item(item).expect("pending send is renderable"));
            }
        }
        return rendered;
    }

    let roots = context
        .thread_roots
        .iter()
        .map(|root| (root.root_event_id.as_str(), root))
        .collect::<HashMap<_, _>>();
    let mut activity_slots = HashMap::<&str, (usize, &TimelineItem)>::new();
    for slot in slots {
        if let TimelineItemId::Event { event_id } = &slot.item.id
            && let Some(root_event_id) = slot.item.thread_root.as_deref()
        {
            activity_slots
                .entry(root_event_id)
                .or_insert((slot.canonical_index, &slot.item));
            if let Some(root) = roots.get(root_event_id)
                && root.activity_event_id == *event_id
            {
                activity_slots.insert(root_event_id, (slot.canonical_index, &slot.item));
            }
        }
    }

    let mut root_at_index = HashMap::<usize, TimelineItem>::new();
    let mut suppressed = HashSet::new();
    let mut scheduled_roots = HashSet::new();
    for slot in slots {
        let event_id = match &slot.item.id {
            TimelineItemId::Event { event_id } => event_id.as_str(),
            TimelineItemId::Transaction { .. } | TimelineItemId::Synthetic { .. } => "",
        };
        if slot.item.thread_root.is_some() {
            continue;
        }
        let Some(root) = roots.get(event_id) else {
            root_at_index.insert(
                slot.canonical_index,
                decorate_event_item(&slot.item).unwrap(),
            );
            continue;
        };

        let (display_index, activity_event_id, display_timestamp_ms) =
            match context.thread_root_order {
                TimelineThreadRootOrder::RootEvent => (
                    slot.canonical_index,
                    event_id.to_owned(),
                    slot.item.timestamp_ms,
                ),
                TimelineThreadRootOrder::LatestReply => {
                    let activity = activity_slots.get(event_id);
                    (
                        activity
                            .map(|(index, _)| *index)
                            .unwrap_or(slot.canonical_index),
                        root.activity_event_id.clone(),
                        root.activity_timestamp_ms.or(slot.item.timestamp_ms),
                    )
                }
            };
        let item = root_display_item(root, &slot.item, activity_event_id, display_timestamp_ms);
        root_at_index.insert(display_index, item);
        scheduled_roots.insert(event_id.to_owned());
        if display_index != slot.canonical_index {
            suppressed.insert(slot.canonical_index);
        }
        if let Some((activity_index, _)) = activity_slots.get(event_id)
            && context.thread_root_order == TimelineThreadRootOrder::LatestReply
        {
            suppressed.insert(*activity_index);
        }
    }

    // Root lifecycle is owned by `context.thread_roots`, not by the bounded
    // canonical window. Under LatestReply ordering the activity can remain in
    // the window after the root event has been trimmed out. Materialize the
    // retained root at that activity position before suppressing the reply.
    for root in &context.thread_roots {
        let root_id = root.root_event_id.as_str();
        if scheduled_roots.contains(root_id)
            || context.thread_root_order != TimelineThreadRootOrder::LatestReply
        {
            continue;
        }
        let Some((activity_index, _)) = activity_slots.get(root_id) else {
            continue;
        };
        let item = root.item.as_ref().map_or_else(
            || root_placeholder_item(root),
            |item| {
                root_display_item(
                    root,
                    item,
                    root.activity_event_id.clone(),
                    root.activity_timestamp_ms.or(item.timestamp_ms),
                )
            },
        );
        root_at_index.insert(*activity_index, item);
        suppressed.insert(*activity_index);
        scheduled_roots.insert(root.root_event_id.clone());
    }

    let mut projected = Vec::new();
    let mut rendered_ids = HashSet::new();
    for slot in slots {
        if matches!(
            &slot.item.id,
            TimelineItemId::Transaction { transaction_id }
                if suppressed_transaction_ids.contains(transaction_id)
        ) {
            continue;
        }
        if let Some(item) = root_at_index.remove(&slot.canonical_index) {
            if rendered_ids.insert(timeline_item_render_id(&item)) {
                projected.push(item);
            }
            continue;
        }
        if suppressed.contains(&slot.canonical_index) {
            continue;
        }
        if slot.item.thread_root.is_none() {
            let item = decorate_event_item(&slot.item).unwrap();
            if rendered_ids.insert(timeline_item_render_id(&item)) {
                projected.push(item);
            }
        }
    }
    for root in &context.thread_roots {
        let root_id = root.root_event_id.as_str();
        if scheduled_roots.contains(root_id) {
            continue;
        }
        let Some(item) = root.item.as_ref() else {
            projected.push(root_placeholder_item(root));
            continue;
        };
        let display_timestamp_ms = root.activity_timestamp_ms.or(item.timestamp_ms);
        let projected_item = root_display_item(
            root,
            item,
            root.activity_event_id.clone(),
            display_timestamp_ms,
        );
        if !rendered_ids.insert(timeline_item_render_id(&projected_item)) {
            continue;
        }
        let insertion_index = projected
            .iter()
            .position(|current| {
                current
                    .display_metadata
                    .as_ref()
                    .and_then(|metadata| metadata.display_timestamp_ms)
                    .unwrap_or(u64::MAX)
                    > display_timestamp_ms.unwrap_or(u64::MAX)
            })
            .unwrap_or(projected.len());
        projected.insert(insertion_index, projected_item);
    }
    let mut seen = projected
        .iter()
        .map(timeline_item_render_id)
        .collect::<HashSet<_>>();
    for item in pending_items {
        if matches!(
            &item.id,
            TimelineItemId::Transaction { transaction_id }
                if suppressed_transaction_ids.contains(transaction_id)
        ) {
            continue;
        }
        let id = timeline_item_render_id(item);
        if seen.insert(id) {
            projected.push(decorate_event_item(item).expect("pending send is renderable"));
        }
    }
    projected
}

fn decorate_event_item(item: &TimelineItem) -> Option<TimelineItem> {
    let mut item = item.clone();
    let (content_event_id, row_id) = match &item.id {
        TimelineItemId::Event { event_id } => (Some(event_id.clone()), event_id.clone()),
        TimelineItemId::Transaction { transaction_id } => (None, format!("txn:{transaction_id}")),
        TimelineItemId::Synthetic { synthetic_id } => (None, format!("syn:{synthetic_id}")),
    };
    let timestamp = item.timestamp_ms;
    item.display_metadata = Some(TimelineDisplayMetadata {
        row_id,
        kind: TimelineDisplayKind::Event,
        content_event_id: content_event_id.clone(),
        activity_event_id: content_event_id,
        display_timestamp_ms: timestamp,
    });
    Some(item)
}

fn root_display_item(
    root: &ThreadRootDisplayData,
    fallback: &TimelineItem,
    activity_event_id: String,
    display_timestamp_ms: Option<u64>,
) -> TimelineItem {
    let mut item = root.item.clone().unwrap_or_else(|| fallback.clone());
    let summary =
        item.thread_summary
            .get_or_insert_with(|| koushi_protocol::event::ThreadSummaryDto {
                reply_count: 0,
                latest_event_id: None,
                latest_sender: None,
                latest_sender_label: None,
                latest_body_preview: None,
                latest_timestamp_ms: None,
            });
    summary.reply_count = root.aggregate.reply_count;
    summary.latest_event_id = root.aggregate.latest_event_id.clone();
    summary.latest_sender = root.aggregate.latest_sender.clone();
    summary.latest_sender_label = root.aggregate.latest_sender_label.clone();
    summary.latest_body_preview = root.aggregate.latest_body_preview.clone();
    summary.latest_timestamp_ms = root.aggregate.latest_timestamp_ms;
    item.display_metadata = Some(TimelineDisplayMetadata {
        row_id: format!("thread-root:{}", root.root_event_id),
        kind: if root.pending {
            TimelineDisplayKind::ThreadRootPending
        } else if let Some(failure_kind) = root.failure_kind {
            TimelineDisplayKind::ThreadRootFailed { failure_kind }
        } else {
            TimelineDisplayKind::ThreadRoot
        },
        content_event_id: Some(root.root_event_id.clone()),
        activity_event_id: Some(activity_event_id),
        display_timestamp_ms,
    });
    item
}

fn root_placeholder_item(root: &ThreadRootDisplayData) -> TimelineItem {
    let mut item = TimelineItem {
        id: TimelineItemId::Synthetic {
            synthetic_id: format!("thread-root-slot:{}", root.root_event_id),
        },
        sender: None,
        sender_label: None,
        sender_avatar: None,
        body: None,
        notice_i18n: None,
        message_kind: Default::default(),
        spoiler_spans: Vec::new(),
        timestamp_ms: None,
        in_reply_to_event_id: None,
        formatted: None,
        reply_quote: None,
        thread_root: None,
        thread_summary: None,
        media: None,
        link_previews: None,
        link_ranges: Vec::new(),
        reactions: Vec::new(),
        can_react: false,
        is_redacted: false,
        is_hidden: false,
        can_redact: false,
        is_edited: false,
        can_edit: false,
        unable_to_decrypt: None,
        request_state: None,
        actions: Default::default(),
        send_state: None,
        display_metadata: None,
    };
    let summary = koushi_protocol::event::ThreadSummaryDto {
        reply_count: root.aggregate.reply_count,
        latest_event_id: root.aggregate.latest_event_id.clone(),
        latest_sender: root.aggregate.latest_sender.clone(),
        latest_sender_label: root.aggregate.latest_sender_label.clone(),
        latest_body_preview: root.aggregate.latest_body_preview.clone(),
        latest_timestamp_ms: root.aggregate.latest_timestamp_ms,
    };
    item.thread_summary = Some(summary);
    root_display_item(
        root,
        &item,
        root.activity_event_id.clone(),
        root.activity_timestamp_ms,
    )
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
    context: &DisplayProjectionContext,
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
    display_projection.display_items = project_display_items(
        &display_projection.slots,
        &display_projection.pending_items,
        &display_projection.suppressed_transaction_ids,
        context,
    );
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
    if let Some(metadata) = &item.display_metadata {
        return metadata.row_id.clone();
    }
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
mod tests;
