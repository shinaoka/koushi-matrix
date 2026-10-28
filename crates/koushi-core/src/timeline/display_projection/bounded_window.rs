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
    pub(in crate::timeline) fn return_to_live_window(
        &mut self,
        canonical: &[TimelineItem],
        context: &DisplayProjectionContext,
    ) -> Vec<TimelineDiff> {
        let before = self.display_items.clone();
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
        let slot_for_activity = |activity: &str| {
            // Activity identity and canonical content identity differ for a
            // LatestReply thread root. Prefer its activity slot when retained,
            // then resolve the projected row back to its canonical content.
            self.slots
                .iter()
                .position(|slot| event_id(&slot.item) == Some(activity))
                .or_else(|| {
                    let row = self
                        .display_items
                        .iter()
                        .find(|item| event_id(item) == Some(activity))?;
                    self.slots.iter().position(|slot| slot.item.id == row.id)
                })
        };
        let first = slot_for_activity(observation.first_visible_event_id.as_deref()?)?;
        let last = slot_for_activity(observation.last_visible_event_id.as_deref()?)?;
        (first <= last && last - first < WINDOW_SLOTS).then_some((first, last))
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
                });
            }
            if item_occupies_display_row(&canonical[index]) {
                page.push(DisplayProjectionSlot {
                    canonical_index: index,
                    item: canonical[index].clone(),
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
        self.window_revision = self.window_revision.wrapping_add(1);
        self.rebuild_window_items(context);
        finalize_display_projection_diffs(&before, &self.display_items, false).0
    }

    fn rebuild_window_items(&mut self, context: &DisplayProjectionContext) {
        self.display_items = project_display_items(
            &self.slots,
            &self.pending_items,
            &self.suppressed_transaction_ids,
            context,
        );
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
        } else if context.viewport.at_bottom {
            len - WINDOW_SLOTS
        } else {
            0
        };
        self.slots.truncate(start + WINDOW_SLOTS);
        self.slots.drain(..start);
    }

    pub(in crate::timeline) fn bound_after_batch(&mut self, context: &DisplayProjectionContext) {
        if !context.room_window || self.slots.len() <= WINDOW_SLOTS {
            return;
        }
        self.trim_slots(context, !context.viewport.at_bottom);
        self.window_revision = self.window_revision.wrapping_add(1);
        self.rebuild_window_items(context);
    }
}
