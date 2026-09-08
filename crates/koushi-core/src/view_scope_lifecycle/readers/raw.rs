use std::sync::Arc;

use super::{Phase, ReaderWork};
use crate::view_scope_lifecycle::{ScopeError, ViewScopeRegistry, model};
use crate::{timeline::RawReceiptWindow, view_budget::ViewReservation};

pub(crate) struct ChargedRaw {
    pub(crate) raw: RawReceiptWindow,
    source_revision: u64,
    owner_generation: u64,
    _bytes: ViewReservation,
}

fn check_owner(
    previous: &Option<Arc<ChargedRaw>>,
    incoming: &ChargedRaw,
) -> Result<(), ScopeError> {
    if previous
        .as_ref()
        .is_some_and(|previous| previous.owner_generation != incoming.owner_generation)
    {
        Err(ScopeError::SourceRetired)
    } else {
        Ok(())
    }
}

fn retained_bytes(raw: &RawReceiptWindow) -> Result<usize, ScopeError> {
    let mut bytes = std::mem::size_of::<ChargedRaw>();
    let mut add = |size: usize| -> Result<(), ScopeError> {
        bytes = bytes
            .checked_add(size)
            .filter(|total| *total <= 64 * 1024 * 1024)
            .ok_or(ScopeError::Capacity)?;
        Ok(())
    };
    add(raw
        .receipts
        .capacity()
        .checked_mul(std::mem::size_of::<koushi_state::LiveReadReceipt>())
        .ok_or(ScopeError::Capacity)?)?;
    add(model::encoded_bytes(&raw.receipts)?)?;
    add(model::encoded_bytes(&raw.source())?)?;
    add(raw
        .profiles
        .capacity()
        .checked_mul(std::mem::size_of::<koushi_sdk::MatrixUserProfile>())
        .ok_or(ScopeError::Capacity)?)?;
    for profile in &raw.profiles {
        add(profile.user_id.len())?;
        add(profile.display_name.as_ref().map_or(0, String::len))?;
        add(profile.avatar_mxc_uri.as_ref().map_or(0, String::len))?;
    }
    Ok(bytes)
}

impl ViewScopeRegistry {
    /// Transfer measured raw ownership out of the already-admitted builder pool;
    /// keep work metadata charged and never release/reacquire retained bytes.
    pub(crate) fn retain_reader_raw(
        &self,
        work: &mut ReaderWork,
        raw: RawReceiptWindow,
    ) -> Result<Arc<ChargedRaw>, ScopeError> {
        if raw.source() != Some(&work.source)
            || raw.start != work.start.min(raw.total_count)
            || raw.receipts.len() as u64
                != (raw.total_count - raw.start).min(u64::from(work.limit.get()))
            || raw.profiles.len() > usize::from(work.limit.get())
        {
            return Err(ScopeError::InvalidModel);
        }
        let source_revision = raw.source_revision()?;
        let owner_generation = raw
            .owner_generation()
            .ok_or(ScopeError::SourceUnavailable)?;
        let amount = retained_bytes(&raw)?;
        let work_bytes = model::encoded_bytes(&work.source)?
            .checked_add(std::mem::size_of::<ReaderWork>())
            .ok_or(ScopeError::Capacity)?;
        if amount
            .checked_add(work_bytes)
            .is_none_or(|total| total > work.reservation.bytes())
        {
            return Err(ScopeError::Capacity);
        }
        let bytes = work
            .reservation
            .split_bytes(amount)
            .ok_or(ScopeError::Capacity)?;
        Ok(Arc::new(ChargedRaw {
            raw,
            source_revision,
            owner_generation,
            _bytes: bytes,
        }))
    }

    /// Check the private owner before exposing any model from a replacement.
    pub(crate) fn validate_reader_owner(
        &self,
        work: &ReaderWork,
        raw: &ChargedRaw,
    ) -> Result<(), ScopeError> {
        let state = self.state.lock().expect("view registry poisoned");
        let entry = state.scopes.get(&work.scope).ok_or(ScopeError::Closed)?;
        let reader = entry
            .control
            .reader
            .lock()
            .expect("reader request poisoned");
        let reader = reader.as_ref().ok_or(ScopeError::Closed)?;
        check_owner(&reader.accepted_raw, raw)
    }

    pub(crate) fn accept_reader_raw(
        &self,
        work: &mut ReaderWork,
        raw: Arc<ChargedRaw>,
    ) -> Result<(), ScopeError> {
        let registration = super::super::profiles::ProfileRegistration::prepare(
            work.source.timeline.key.room_id().to_owned(),
            raw.raw.receipts.iter().map(|row| row.user_id.clone()),
            raw.raw
                .receipts
                .iter()
                .filter_map(|row| row.avatar.as_ref().map(|avatar| avatar.mxc_uri.clone()))
                .chain(
                    raw.raw
                        .profiles
                        .iter()
                        .filter_map(|profile| profile.avatar_mxc_uri.clone()),
                ),
            &mut work.reservation,
        )?;
        let replaced = {
            let mut state = self.state.lock().expect("view registry poisoned");
            let replaced_raw = {
                let entry = state.scopes.get(&work.scope).ok_or(ScopeError::Closed)?;
                let retired = entry.control.retired.lock().expect("view control poisoned");
                if retired.is_some() {
                    return Err(ScopeError::Closed);
                }
                let mut reader_guard = entry
                    .control
                    .reader
                    .lock()
                    .expect("reader request poisoned");
                let reader = reader_guard.as_mut().ok_or(ScopeError::InvalidModel)?;
                check_owner(&reader.accepted_raw, &raw)?;
                if reader.phase != Phase::Running
                    || reader.run_id != work.run_id
                    || reader.window_sequence != work.window_sequence
                {
                    return Err(ScopeError::InvalidRevision);
                }
                if raw.raw.source() != Some(&reader.source)
                    || raw.raw.start != reader.start.min(raw.raw.total_count)
                    || raw.raw.receipts.len() as u64
                        != (raw.raw.total_count - raw.raw.start).min(u64::from(reader.limit.get()))
                    || raw.raw.profiles.len() > usize::from(reader.limit.get())
                    || reader
                        .accepted_raw
                        .as_ref()
                        .is_some_and(|previous| previous.source_revision > raw.source_revision)
                {
                    return Err(ScopeError::InvalidRevision);
                }
                reader.accepted_raw.replace(raw)
            };
            state.replace_profiles(work.scope, registration);
            replaced_raw
        };
        drop(replaced);
        Ok(())
    }
}
