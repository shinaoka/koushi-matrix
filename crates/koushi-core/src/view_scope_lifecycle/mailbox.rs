use std::sync::Arc;

use koushi_protocol::view::ViewRevision;

use super::{ScopeError, model::PreparedModel};

#[derive(Default)]
pub(super) struct Mailbox {
    revision: u64,
    latest: Option<(ViewRevision, Arc<PreparedModel>)>,
    in_flight: Option<(ViewRevision, Arc<PreparedModel>)>,
    installed: Option<(ViewRevision, Arc<super::model::InstalledRows>)>,
}

impl Mailbox {
    pub(super) fn publish(
        &mut self,
        model: &Arc<PreparedModel>,
    ) -> Result<(ViewRevision, Option<Arc<PreparedModel>>), ScopeError> {
        self.revision = self
            .revision
            .checked_add(1)
            .ok_or(ScopeError::CounterExhausted)?;
        let revision = ViewRevision(self.revision);
        let replaced = self
            .latest
            .replace((revision, model.clone()))
            .map(|(_, model)| model);
        Ok((revision, replaced))
    }

    pub(super) fn take(&mut self) -> Option<(ViewRevision, Arc<PreparedModel>)> {
        if self.in_flight.is_some() {
            return None;
        }
        let next = self.latest.take()?;
        self.in_flight = Some(next.clone());
        Some(next)
    }

    pub(super) fn ack(&mut self, revision: ViewRevision) -> Result<(), ScopeError> {
        if self
            .installed
            .as_ref()
            .is_some_and(|(installed, _)| revision == *installed)
        {
            return Ok(());
        }
        if !self
            .in_flight
            .as_ref()
            .is_some_and(|(issued, _)| *issued == revision)
        {
            return Err(ScopeError::InvalidRevision);
        }
        let (_, model) = self.in_flight.take().expect("checked in-flight model");
        self.installed = Some((revision, model.installed.clone()));
        Ok(())
    }

    pub(super) fn installed_revision(&self) -> Option<ViewRevision> {
        self.installed.as_ref().map(|(revision, _)| *revision)
    }

    pub(super) fn anchor_index(
        &self,
        revision: ViewRevision,
        user_id: &str,
    ) -> Result<Option<u64>, ScopeError> {
        let (_, installed) = self
            .installed
            .as_ref()
            .filter(|(current, _)| *current == revision)
            .ok_or(ScopeError::InvalidRevision)?;
        Ok(installed
            .rows
            .iter()
            .position(|row| row.user_id == user_id)
            .map(|index| index as u64))
    }

    pub(super) fn installed_rows(
        &self,
        revision: ViewRevision,
    ) -> Result<Arc<super::model::InstalledRows>, ScopeError> {
        let (_, installed) = self
            .installed
            .as_ref()
            .filter(|(current, _)| *current == revision)
            .ok_or(ScopeError::InvalidRevision)?;
        Ok(installed.clone())
    }

    pub(super) fn clear(&mut self) {
        self.latest = None;
        self.in_flight = None;
        self.installed = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replacement_returns_old_payload_for_release_outside_commit() {
        let budget = crate::view_budget::ViewBudget::default();
        let make = || {
            let source = serde_json::from_value(serde_json::json!({
                "key": {"account_key": "account", "kind": {"Room": {"room_id": "!r:example.org"}}},
                "projection_request_id": {"connection_id": "1", "sequence": "2"},
                "generation": "3", "event_id": "$event"
            }))
            .unwrap();
            super::super::model::prepare(
                koushi_protocol::view::ViewModel::ReaderLoading { source },
                &budget,
                Vec::new(),
            )
            .unwrap()
        };
        let mut mailbox = Mailbox::default();
        mailbox.publish(&Arc::new(make())).unwrap();
        let old = Arc::downgrade(&mailbox.latest.as_ref().unwrap().1);
        let replaced = mailbox.publish(&Arc::new(make())).unwrap();
        assert!(
            old.upgrade().is_some(),
            "commit must return, not destroy, the replaced payload"
        );
        drop(replaced);
        assert!(old.upgrade().is_none());
    }
}
