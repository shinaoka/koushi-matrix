use std::sync::Arc;

use super::{AppActor, CoreCommandEnvelope};
use crate::{
    timeline::RawReceiptWindow,
    view_scope_lifecycle::{ChargedRaw, ProducerCompletion, ReaderWork, ScopeError},
};
use koushi_protocol::view::{ReaderWindow, ResolvedReaderAnchor, ViewModel, ViewRetirement};
use koushi_state::SessionState;
use tokio::sync::oneshot;

pub(super) enum ReaderInput {
    Fresh(RawReceiptWindow),
    Cached(Arc<ChargedRaw>),
}

pub(super) struct ReaderPrepared {
    work: ReaderWork,
    completion: ProducerCompletion,
    result: Result<ReaderInput, ScopeError>,
}

impl AppActor {
    pub(super) fn start_reader_work(&mut self) {
        // One admitted job per fair actor turn. Admission errors already retire
        // the affected scope; never wait for budget inside this actor.
        let Ok(Some(work)) = self.view_scopes.take_reader_work() else {
            return;
        };
        let Some(command_tx) = self.command_tx.upgrade() else {
            return;
        };
        let _ = work.spawn(move |work, completion| async move {
            // No CoreConnection, consumer or strong registry capture: only the
            // work's weak lifecycle guard and existing bounded command sender.
            let result = async {
                if let Some(raw) = &work.raw
                    && raw.raw.acquire_source().is_some()
                {
                    return Ok(ReaderInput::Cached(raw.clone()));
                }
                let (response, result) = oneshot::channel();
                command_tx
                    .send(CoreCommandEnvelope::ReadReceiptWindow {
                        source: work.source.clone(),
                        start: work.start,
                        limit: work.limit,
                        response,
                    })
                    .await
                    .map_err(|_| ScopeError::Closed)?;
                result
                    .await
                    .map_err(|_| ScopeError::Closed)?
                    .map(ReaderInput::Fresh)
            }
            .await;
            let _ = command_tx
                .send(CoreCommandEnvelope::ReaderPrepared(ReaderPrepared {
                    work,
                    completion,
                    result,
                }))
                .await;
        });
    }

    pub(super) fn handle_reader_prepared(&mut self, prepared: ReaderPrepared) {
        let ReaderPrepared {
            mut work,
            completion,
            result,
        } = prepared;
        let result = result.and_then(|input| self.project_reader_input(&mut work, input));
        if let Err(error) = result {
            if matches!(
                error,
                ScopeError::InvalidRevision
                    | ScopeError::InvalidModel
                    | ScopeError::SourceUnavailable
            ) && self.view_scopes.reader_work_superseded(&work)
            {
                completion.complete();
                let _ = self.view_scopes.finish_reader_work(&work);
                return;
            }
            let reason = match error {
                ScopeError::Capacity => ViewRetirement::Capacity,
                ScopeError::CounterExhausted => ViewRetirement::CounterExhausted,
                ScopeError::InactiveSession => ViewRetirement::SessionRetired,
                ScopeError::SourceUnavailable | ScopeError::SourceRetired => {
                    ViewRetirement::SourceUnavailable
                }
                _ => ViewRetirement::ProducerFailed,
            };
            self.view_scopes.retire(work.scope, reason);
        }
        completion.complete();
        let _ = self.view_scopes.finish_reader_work(&work);
    }

    fn project_reader_input(
        &mut self,
        work: &mut ReaderWork,
        input: ReaderInput,
    ) -> Result<(), ScopeError> {
        let SessionState::Ready(info) = &self.state.session else {
            return Err(ScopeError::InactiveSession);
        };
        if info.user_id != work.source.timeline.key.account_key.0 {
            return Err(ScopeError::InactiveSession);
        }
        let raw = match input {
            ReaderInput::Fresh(raw) => self.view_scopes.retain_reader_raw(work, raw)?,
            ReaderInput::Cached(raw) => raw,
        };
        self.view_scopes.validate_reader_owner(work, &raw)?;
        let _lease = raw
            .raw
            .acquire_source()
            .ok_or(ScopeError::SourceUnavailable)?;
        // The input now has its own measured charge. Admit projection scratch
        // before cloning; this is not a second snapshot or another ordering owner.
        if !work.reservation.resize_bytes(64 * 1024 * 1024) {
            return Err(ScopeError::Capacity);
        }
        let mut projection = raw.raw.clone();
        projection.resolve_profiles(
            &self.state.profile,
            work.source.timeline.key.room_id(),
            Some(&info.user_id),
        );
        let mut resolved = projection.into_resolved(koushi_state::resolve_catalog_locale(
            &self.state.settings.values.locale,
        ));
        let model = ViewModel::ReaderReady(ReaderWindow {
            source: work.source.clone(),
            total_count: resolved.total_count,
            start: resolved.start,
            rows: std::mem::take(&mut resolved.rows),
            window_sequence: work.window_sequence,
            source_revision: resolved.source_revision()?,
            dependency_revision: work.dependency_revision,
            resolved_anchor: ResolvedReaderAnchor::NotRequested,
        });
        let resources = std::mem::take(&mut resolved.avatar_resources);
        self.view_scopes
            .publish_current(work.scope, model, resources, &resolved)?;
        // Still the same await-free actor turn and the work remains Running:
        // no subsequent job can read the slot until completion below.
        self.view_scopes.accept_reader_raw(work, raw)?;
        Ok(())
    }
}
