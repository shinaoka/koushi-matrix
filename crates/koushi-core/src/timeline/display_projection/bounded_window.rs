//! Finite cached-history movement. This bounds display ownership, not SDK storage.
use super::*;
use koushi_protocol::event::{PaginationDirection, TimelineDisplayWindow};

const WINDOW_SLOTS: usize = 240;
const PAGE_ROWS: usize = 60;
const EDGE_SLOTS: usize = 24;

fn event_id(item: &TimelineItem) -> Option<&str> {
    if let Some(display) = &item.display_metadata {
        return display.activity_event_id.as_deref();
    }
    match &item.id {
        TimelineItemId::Event { event_id } => Some(event_id.as_str()),
        _ => None,
    }
}

impl DisplayProjectionState {
    /// Reconcile a replacement snapshot without measuring its window in raw
    /// canonical offsets: hidden runs can separate adjacent displayed rows.
    pub(in crate::timeline) fn reset_bounded(
        &mut self,
        canonical: &[TimelineItem],
        context: &DisplayProjectionContext,
    ) {
        self.canonical_len = canonical.len();
        if context.viewport.at_bottom || canonical.is_empty() {
            self.return_to_live_window(canonical, context);
            return;
        }
        let same_positions = !self.slots.is_empty()
            && self.slots.len() <= WINDOW_SLOTS
            && self.slots.iter().all(|slot| {
                canonical
                    .get(slot.canonical_index)
                    .is_some_and(|item| item.id == slot.item.id)
            });
        let newly_visible_owner = if same_positions {
            let start = self.slots.first().unwrap().canonical_index;
            let end = self.slots.last().unwrap().canonical_index + 1;
            canonical
                .iter()
                .enumerate()
                .take(end)
                .skip(start)
                .any(|(index, item)| {
                    item_occupies_display_row(item)
                        && !self.slots.iter().any(|slot| slot.canonical_index == index)
                })
        } else {
            true
        };
        if same_positions && !newly_visible_owner {
            for slot in &mut self.slots {
                slot.item = canonical[slot.canonical_index].clone();
            }
            // The replacement may change the unrepresented runs (#1692).
            self.refresh_gap_ends(canonical);
            self.window_revision = self.window_revision.saturating_add(1);
            self.rebuild_window_items(context);
            return;
        }
        let represented = self
            .slots
            .iter()
            .map(|slot| &slot.item.id)
            .collect::<HashSet<_>>();
        let retained = canonical
            .iter()
            .enumerate()
            .filter_map(|(index, item)| represented.contains(&item.id).then_some(index))
            .take(WINDOW_SLOTS)
            .collect::<Vec<_>>();
        let endpoint = |activity: Option<&str>| {
            let activity = activity?;
            canonical
                .iter()
                .position(|item| event_id(item) == Some(activity))
                .or_else(|| {
                    let row = self
                        .display_items
                        .iter()
                        .find(|item| event_id(item) == Some(activity))?;
                    canonical.iter().position(|item| item.id == row.id)
                })
        };
        let first = endpoint(context.viewport.first_visible_event_id.as_deref());
        let last = endpoint(context.viewport.last_visible_event_id.as_deref());
        let Some(anchor) = first.or(last).or_else(|| retained.first().copied()) else {
            self.return_to_live_window(canonical, context);
            return;
        };
        let first_index = first.unwrap_or(anchor);
        let last_index = last.unwrap_or(anchor);
        let first = first_index.min(last_index);
        let last = first_index.max(last_index);
        let mut indices = HashSet::with_capacity(WINDOW_SLOTS);
        indices.insert(first);
        indices.insert(last);
        // Preserve the known real end, including a hidden cursor, before spending
        // the remaining budget. Historical windows with a suffix keep that suffix.
        if let Some(&index) = retained.last()
            && index + 1 == canonical.len()
        {
            indices.insert(index);
        }
        for (index, item) in canonical.iter().enumerate().take(last + 1).skip(first) {
            if indices.len() == WINDOW_SLOTS {
                break;
            }
            if item_occupies_display_row(item) {
                indices.insert(index);
            }
        }
        // Existing thread activity must survive an unrelated replacement. These
        // indices are bounded by the old window, unlike the canonical history.
        for &index in &retained {
            if indices.len() == WINDOW_SLOTS {
                break;
            }
            if canonical[index].thread_root.is_some() {
                indices.insert(index);
            }
        }
        let mut latest_replies = indices
            .iter()
            .filter_map(|&index| {
                canonical[index]
                    .thread_root
                    .as_deref()
                    .map(|root| (root, index))
            })
            .collect::<HashMap<_, _>>();
        for (index, item) in canonical.iter().enumerate().take(last + 1).skip(first) {
            let Some(root) = item.thread_root.as_deref() else {
                continue;
            };
            if let Some(previous) = latest_replies.get(root).copied()
                && previous != first
                && previous != last
                && previous + 1 != canonical.len()
            {
                indices.remove(&previous);
            }
            if indices.len() < WINDOW_SLOTS {
                indices.insert(index);
                latest_replies.insert(root, index);
            }
        }
        let mut lookbehind = 0;
        for index in (0..first).rev() {
            if indices.len() == WINDOW_SLOTS || lookbehind == PAGE_ROWS {
                break;
            }
            if item_occupies_display_row(&canonical[index]) && indices.insert(index) {
                lookbehind += 1;
            }
        }
        for (index, item) in canonical.iter().enumerate().skip(last + 1) {
            if indices.len() == WINDOW_SLOTS {
                break;
            }
            if item_occupies_display_row(item) {
                indices.insert(index);
            }
        }
        let mut indices = indices.into_iter().collect::<Vec<_>>();
        indices.sort_unstable();
        self.slots = indices
            .into_iter()
            .map(|canonical_index| DisplayProjectionSlot {
                canonical_index,
                item: canonical[canonical_index].clone(),
                gap_before: GapEnds::default(),
            })
            .collect();
        self.refresh_gap_ends(canonical);
        self.window_revision = self.window_revision.saturating_add(1);
        self.rebuild_window_items(context);
    }

    pub(in crate::timeline) fn return_to_live_window(
        &mut self,
        canonical: &[TimelineItem],
        context: &DisplayProjectionContext,
    ) -> Vec<TimelineDiff> {
        let before = self.display_items.clone();
        self.canonical_len = canonical.len();
        let mut slots = Vec::with_capacity(WINDOW_SLOTS);
        let mut rows = 0;
        let mut roots = HashSet::new();
        for (index, item) in canonical.iter().enumerate().rev() {
            // Keep the real canonical end as a cursor even when it is hidden.
            let terminal = index + 1 == canonical.len();
            let row = item_occupies_display_row(item);
            let reply = item.thread_root.as_deref().is_some_and(|root| {
                roots.len() < ROOM_REPLAY_INITIAL_ITEMS_MAX - 1 && roots.insert(root)
            });
            if terminal || row || reply {
                slots.push(DisplayProjectionSlot {
                    canonical_index: index,
                    item: item.clone(),
                    gap_before: GapEnds::default(),
                });
            }
            if row {
                rows += 1;
            }
            if rows == ROOM_REPLAY_INITIAL_ITEMS_MAX || slots.len() == WINDOW_SLOTS {
                break;
            }
        }
        slots.reverse();
        self.slots = slots;
        self.refresh_gap_ends(canonical);
        self.window_revision = self.window_revision.saturating_add(1);
        self.rebuild_window_items(context);
        finalize_display_projection_diffs(&before, &self.display_items, false).0
    }

    pub(in crate::timeline) fn window_reaches_live_end(&self, canonical_len: usize) -> bool {
        canonical_len == 0
            || self
                .slots
                .last()
                .is_some_and(|slot| slot.canonical_index + 1 == canonical_len)
    }

    pub(in crate::timeline) fn window_metadata(
        &self,
        canonical_len: usize,
    ) -> TimelineDisplayWindow {
        let mut ids = self
            .display_items
            .iter()
            .filter(|item| !item.is_hidden)
            .filter_map(event_id);
        let first_event_id = ids.next().map(str::to_owned);
        let last_event_id = ids
            .next_back()
            .map(str::to_owned)
            .or_else(|| first_event_id.clone());
        TimelineDisplayWindow {
            revision: self.window_revision,
            first_event_id,
            last_event_id,
            reaches_live_end: self.window_reaches_live_end(canonical_len),
        }
    }

    pub(in crate::timeline) fn observation_is_current(
        &self,
        observation: &TimelineViewportObservation,
    ) -> bool {
        let positions = || {
            let first = observation.first_visible_event_id.as_deref()?;
            let last = observation.last_visible_event_id.as_deref()?;
            let first = self
                .display_items
                .iter()
                .position(|item| event_id(item) == Some(first))?;
            let last = self
                .display_items
                .iter()
                .position(|item| event_id(item) == Some(last))?;
            Some(first <= last)
        };
        observation.display_window_revision == Some(self.window_revision)
            && positions() == Some(true)
    }

    fn observed_slots(&self, observation: &TimelineViewportObservation) -> Option<(usize, usize)> {
        let slot_for_row = |row: &TimelineItem| {
            event_id(row)
                .and_then(|activity| {
                    self.slots
                        .iter()
                        .position(|slot| event_id(&slot.item) == Some(activity))
                })
                .or_else(|| self.slots.iter().position(|slot| slot.item.id == row.id))
        };
        let endpoint = |activity: &str, prefer_next: bool| {
            // Only genuinely projected rows can provide geometry evidence. A
            // retained standalone root has product identity but no canonical
            // slot; borrow a neighbor's geometry without changing read evidence.
            let position = self
                .display_items
                .iter()
                .position(|row| !row.is_hidden && event_id(row) == Some(activity))?;
            if let Some(index) = slot_for_row(&self.display_items[position]) {
                return Some(index);
            }
            let next = || {
                self.display_items[position + 1..]
                    .iter()
                    .filter(|row| !row.is_hidden)
                    .find_map(slot_for_row)
            };
            let previous = || {
                self.display_items[..position]
                    .iter()
                    .rev()
                    .filter(|row| !row.is_hidden)
                    .find_map(slot_for_row)
            };
            if prefer_next {
                next()
                    .or_else(previous)
                    .or_else(|| (!self.slots.is_empty()).then_some(0))
            } else {
                previous()
                    .or_else(next)
                    .or_else(|| self.slots.len().checked_sub(1))
            }
        };
        let first = endpoint(observation.first_visible_event_id.as_deref()?, true)?;
        let last = endpoint(observation.last_visible_event_id.as_deref()?, false)?;
        let (first, last) = (first.min(last), first.max(last));
        (last - first < WINDOW_SLOTS).then_some((first, last))
    }

    pub(in crate::timeline) fn reconcile_bounded_window(
        &mut self,
        canonical: &[TimelineItem],
        observation: &TimelineViewportObservation,
        context: &DisplayProjectionContext,
        direction: Option<PaginationDirection>,
    ) -> Vec<TimelineDiff> {
        if !context.room_window || observation.display_window_revision != Some(self.window_revision)
        {
            return Vec::new();
        }
        let Some((first, last)) = self.observed_slots(observation) else {
            return Vec::new();
        };
        let backwards = match direction {
            Some(PaginationDirection::Backward) => first < EDGE_SLOTS,
            None => false,
            Some(PaginationDirection::Forward) => return Vec::new(),
        };
        let forwards = direction.is_none() && last + EDGE_SLOTS >= self.slots.len();
        if !backwards && !forwards {
            return Vec::new();
        }
        let start = self.slots.first().map_or(0, |slot| slot.canonical_index);
        let end = self.slots.last().map_or(0, |slot| slot.canonical_index + 1);
        if (backwards && start == 0) || (forwards && end >= canonical.len()) {
            return Vec::new();
        }
        let before = self.display_items.clone();
        self.canonical_len = canonical.len();
        let mut page = Vec::with_capacity(PAGE_ROWS + 1);
        let mut rows = 0;
        let mut reply_roots = HashSet::new();
        let range: Box<dyn Iterator<Item = usize>> = if backwards {
            Box::new((0..start).rev())
        } else {
            Box::new(end..canonical.len())
        };
        // Hidden runs are traversed without cloning. Keep a terminal cursor slot
        // so an entirely suppressed prefix/suffix can still reach its boundary.
        let mut terminal = None;
        for index in range {
            terminal = Some(index);
            if let Some(root) = canonical[index].thread_root.as_deref()
                && reply_roots.len() < PAGE_ROWS
                && reply_roots.insert(root)
            {
                page.push(DisplayProjectionSlot {
                    canonical_index: index,
                    item: canonical[index].clone(),
                    gap_before: GapEnds::default(),
                });
            }
            if item_occupies_display_row(&canonical[index]) {
                page.push(DisplayProjectionSlot {
                    canonical_index: index,
                    item: canonical[index].clone(),
                    gap_before: GapEnds::default(),
                });
                rows += 1;
                if rows == PAGE_ROWS {
                    break;
                }
            }
        }
        if let Some(index) = terminal
            && !page.iter().any(|slot| slot.canonical_index == index)
        {
            page.push(DisplayProjectionSlot {
                canonical_index: index,
                item: canonical[index].clone(),
                gap_before: GapEnds::default(),
            });
        }
        if page.is_empty() {
            return Vec::new();
        }
        if backwards {
            page.reverse();
            page.append(&mut self.slots);
            self.slots = page;
        } else {
            self.slots.extend(page);
        }
        let observation_context = DisplayProjectionContext {
            viewport: observation.clone(),
            ..context.clone()
        };
        self.trim_slots(&observation_context, backwards);
        self.refresh_gap_ends(canonical);
        self.window_revision = self.window_revision.wrapping_add(1);
        self.rebuild_window_items(context);
        finalize_display_projection_diffs(&before, &self.display_items, false).0
    }

    fn rebuild_window_items(&mut self, context: &DisplayProjectionContext) {
        self.display_items = project_display_items(self, context);
    }

    fn trim_slots(&mut self, context: &DisplayProjectionContext, keep_older: bool) {
        if self.slots.len() <= WINDOW_SLOTS {
            return;
        }
        let len = self.slots.len();
        let observed = self.observed_slots(&context.viewport);
        let start = if let Some((first, last)) = observed {
            if keep_older {
                (last + 1).saturating_sub(WINDOW_SLOTS)
            } else {
                first.min(len - WINDOW_SLOTS)
            }
        } else if keep_older {
            0
        } else {
            len - WINDOW_SLOTS
        };
        self.slots.truncate(start + WINDOW_SLOTS);
        self.slots.drain(..start);
    }

    pub(in crate::timeline) fn bound_after_batch(
        &mut self,
        canonical: &[TimelineItem],
        context: &DisplayProjectionContext,
    ) {
        if !context.room_window || self.slots.len() <= WINDOW_SLOTS {
            return;
        }
        self.trim_slots(context, !context.viewport.at_bottom);
        // Only the edge runs absorbed dropped slots (#1692).
        self.refresh_edge_gap_ends(canonical);
        self.window_revision = self.window_revision.wrapping_add(1);
        self.rebuild_window_items(context);
    }
}
