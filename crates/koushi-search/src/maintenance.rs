use std::collections::BTreeSet;

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct SearchEventRef {
    pub room_id: String,
    pub event_id: String,
}

#[derive(Default)]
pub struct SearchMaintenanceQueue {
    late_decryption: BTreeSet<SearchEventRef>,
    reindex_rooms: BTreeSet<String>,
}

impl SearchMaintenanceQueue {
    pub fn enqueue_late_decryption(
        &mut self,
        room_id: impl Into<String>,
        event_id: impl Into<String>,
    ) {
        self.late_decryption.insert(SearchEventRef {
            room_id: room_id.into(),
            event_id: event_id.into(),
        });
    }

    pub fn pending_late_decryption_count(&self) -> usize {
        self.late_decryption.len()
    }

    pub fn drain_late_decryption(&mut self, room_id: &str) -> Vec<SearchEventRef> {
        let events = self
            .late_decryption
            .iter()
            .filter(|event| event.room_id == room_id)
            .cloned()
            .collect::<Vec<_>>();

        for event in &events {
            self.late_decryption.remove(event);
        }

        events
    }

    pub fn mark_room_reindex_needed(&mut self, room_id: impl Into<String>) {
        self.reindex_rooms.insert(room_id.into());
    }

    pub fn drain_reindex_rooms(&mut self) -> Vec<String> {
        std::mem::take(&mut self.reindex_rooms)
            .into_iter()
            .collect()
    }
}
