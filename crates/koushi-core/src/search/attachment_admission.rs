//! Files admission uses SDK validity/redaction evidence, not observation ordering.
use super::*;

const MUTATION_PROOF_TIMEOUT: Duration = Duration::from_millis(250);
const FILES_PROOF_TIMEOUT: Duration = Duration::from_secs(2);

impl SearchIndexMessage {
    fn attachment_target(&self) -> Option<(&str, &str)> {
        match self {
            Self::Upsert {
                room_id, event_id, ..
            } => Some((room_id, event_id)),
            Self::Edit {
                room_id,
                target_event_id,
                ..
            } => Some((room_id, target_event_id)),
            Self::Redact { .. } => None,
        }
    }

    fn incoming_edit_id(&self) -> Option<&str> {
        match self {
            Self::Upsert { edit, .. } => edit.as_ref().map(|edit| edit.edit_event_id.as_str()),
            Self::Edit { edit_event_id, .. } => Some(edit_event_id),
            Self::Redact { .. } => None,
        }
    }

    fn carries_attachment(&self) -> bool {
        match self {
            Self::Upsert {
                attachment,
                attachment_filename,
                ..
            }
            | Self::Edit {
                attachment,
                attachment_filename,
                ..
            } => attachment.is_some() || attachment_filename.is_some(),
            Self::Redact { .. } => false,
        }
    }

    fn drop_text(&mut self) {
        match self {
            Self::Upsert { body, .. } | Self::Edit { body, .. } => *body = None,
            Self::Redact { .. } => {}
        }
    }
}

/// Retire SDK-proven redacted/invalid versions, never absence or ordering.
fn retire_proven_edits(store: &mut SearchDocumentStore, target: &str, denied: &HashSet<String>) {
    if denied.contains(target) {
        store.redact(target);
        return;
    }
    for id in store.mutation_event_ids(target) {
        if id != target && denied.contains(&id) {
            store.retire_edit(target, &id);
        }
    }
}

/// A text replacement found in an earlier crawl page may have been discarded
/// before its attachment target arrived. Recover only its body-free provenance
/// from the existing SDK resolver, never a history-sized text-edit ledger.
async fn cached_text_replacement(
    session: &MatrixClientSession,
    room: &str,
    target: &str,
) -> Result<Option<SearchEdit>, koushi_sdk::MatrixSearchError> {
    let current = koushi_sdk::resolve_cached_message(session, room, target).await?;
    Ok(current
        .filter(|m| m.current_event_id != m.event_id && m.attachment_filename.is_none())
        .and_then(|m| {
            m.timestamp_ms.map(|timestamp_ms| SearchEdit {
                room_id: room.to_owned(),
                edit_event_id: m.current_event_id,
                target_event_id: target.to_owned(),
                sender: m.sender,
                timestamp_ms,
                body: None,
                attachment_filename: None,
                attachment: None,
            })
        }))
}

impl SearchActor {
    fn attachment_message_is_relevant(&self, message: &SearchIndexMessage) -> bool {
        let Some((room, target)) = message.attachment_target() else {
            return true;
        };
        message.carries_attachment()
            || self.document_store.affects_attachment(target)
            || self
                .attachment_retries
                .iter()
                .chain(self.queued_crawl_index.iter())
                .any(|queued| queued.attachment_target() == Some((room, target)))
    }

    pub(super) fn queue_crawl_messages(&mut self, mut messages: Vec<SearchIndexMessage>) {
        // A backward page can carry text replacement B before media edit A.
        // Admit its metadata chronologically so B can remove A without keeping
        // provenance for every unrelated edited text message in history.
        messages.sort_by_key(|message| match message {
            SearchIndexMessage::Upsert { timestamp_ms, .. }
            | SearchIndexMessage::Edit { timestamp_ms, .. } => *timestamp_ms,
            SearchIndexMessage::Redact { .. } => 0,
        });
        let targets: HashSet<_> = messages
            .iter()
            .filter(|m| m.carries_attachment())
            .filter_map(|m| m.attachment_target().map(|(_, id)| id.to_owned()))
            .collect();
        for mut message in messages {
            message.drop_text();
            if self.attachment_message_is_relevant(&message)
                || message
                    .attachment_target()
                    .is_some_and(|(_, id)| targets.contains(id))
            {
                self.queued_crawl_index.push_back(message);
            }
        }
    }

    fn apply_proven_index_message(
        &mut self,
        message: SearchIndexMessage,
        redacted: &HashSet<String>,
        invalid: &HashSet<String>,
    ) -> Option<(String, String)> {
        if let Some((_, target)) = message.attachment_target() {
            retire_proven_edits(&mut self.document_store, target, redacted);
            retire_proven_edits(&mut self.document_store, target, invalid);
            if redacted.contains(target) {
                return None;
            }
            if let Some(id) = message.incoming_edit_id()
                && (redacted.contains(id) || invalid.contains(id))
            {
                self.document_store.retire_edit(target, id);
                return None;
            }
        }
        self.apply_index_message(message)
    }

    pub(super) async fn handle_index(&mut self, mut message: SearchIndexMessage) {
        message.drop_text();
        if !self.attachment_message_is_relevant(&message) {
            return;
        }
        // Policy is checked before querying or retaining confidential metadata.
        if !self.crawler_settings.include_filenames {
            return;
        }
        if let Some((_, target)) = message.attachment_target()
            && !message.carries_attachment()
            && !self.document_store.affects_attachment(target)
        {
            // Relevance came from a queued file, not a resident root. A focused
            // text edit may never enter the SDK room cache: retain its body-free
            // payload until the known file is admitted, even if proof succeeds.
            self.attachment_retries.push_back(message);
            return;
        }
        let updated = if let Some((room, target)) = message.attachment_target() {
            let mut ids = self.document_store.mutation_event_ids(target);
            if let Some(id) = message.incoming_edit_id() {
                ids.push(id.to_owned());
            }
            let proof = executor::timeout(MUTATION_PROOF_TIMEOUT, async {
                let text = cached_text_replacement(&self.session, room, target).await?;
                if let Some(text) = &text {
                    ids.push(text.edit_event_id.clone());
                }
                let invalid =
                    koushi_sdk::cached_invalid_replacement_ids(&self.session, room, target, &ids)
                        .await?;
                let redacted =
                    koushi_sdk::cached_redacted_event_ids(&self.session, room, &ids).await?;
                Ok::<_, koushi_sdk::MatrixSearchError>((redacted, invalid, text))
            })
            .await;
            match proof {
                Ok(Ok((redacted, invalid, text))) => {
                    let updated = self.apply_proven_index_message(message, &redacted, &invalid);
                    if let Some(text) = text
                        && !redacted.contains(&text.edit_event_id)
                        && !redacted.contains(&text.target_event_id)
                        && !invalid.contains(&text.edit_event_id)
                    {
                        self.document_store.upsert_edit(text, true);
                    }
                    updated
                }
                _ => {
                    // Do not coalesce different versions: a newer failed edit can
                    // later be redacted and the older queued version promoted.
                    self.attachment_retries.push_back(message);
                    None
                }
            }
        } else {
            self.apply_index_message(message)
        };
        if let Some((room_id, event_id)) = updated {
            self.emit(CoreEvent::Search(SearchEvent::IndexUpdated {
                room_id,
                event_id,
            }));
        }
    }

    /// A read boundary: get all proofs before mutating or publishing any rows.
    /// One deadline covers every room and retry. Errors preserve state and fail
    /// the Files request, rather than exposing unverified rows or deleting them.
    pub(super) async fn reconcile_attachment_redactions(&mut self) -> bool {
        let mut targets: HashMap<(String, String), HashSet<String>> = self
            .document_store
            .mutation_targets()
            .into_iter()
            .map(|(room, target)| {
                let ids = self
                    .document_store
                    .mutation_event_ids(&target)
                    .into_iter()
                    .collect();
                ((room, target), ids)
            })
            .collect();
        for message in self
            .attachment_retries
            .iter()
            .chain(self.queued_crawl_index.iter())
        {
            if let Some((room, target)) = message.attachment_target() {
                let ids = targets
                    .entry((room.to_owned(), target.to_owned()))
                    .or_default();
                ids.extend(self.document_store.mutation_event_ids(target));
                if let Some(id) = message.incoming_edit_id() {
                    ids.insert(id.to_owned());
                }
            }
        }
        let proofs = executor::timeout(FILES_PROOF_TIMEOUT, async {
            let mut texts = Vec::new();
            for ((room, target), ids) in &mut targets {
                if let Some(text) = cached_text_replacement(&self.session, room, target).await? {
                    ids.insert(text.edit_event_id.clone());
                    texts.push(text);
                }
            }
            let mut room_ids: HashMap<String, HashSet<String>> = HashMap::new();
            let mut invalid = HashMap::new();
            for ((room, target), ids) in targets {
                room_ids
                    .entry(room.clone())
                    .or_default()
                    .extend(ids.iter().cloned());
                let ids: Vec<_> = ids.into_iter().collect();
                let rejected =
                    koushi_sdk::cached_invalid_replacement_ids(&self.session, &room, &target, &ids)
                        .await?;
                invalid.insert((room, target), rejected);
            }
            let mut proofs = HashMap::new();
            for (room, ids) in room_ids {
                let ids: Vec<_> = ids.into_iter().collect();
                let redacted =
                    koushi_sdk::cached_redacted_event_ids(&self.session, &room, &ids).await?;
                proofs.insert(room, redacted);
            }
            Ok::<_, koushi_sdk::MatrixSearchError>((proofs, invalid, texts))
        })
        .await;
        let Ok(Ok((proofs, invalid, texts))) = proofs else {
            return false;
        };
        for (room, target) in self.document_store.mutation_targets() {
            if let Some(redacted) = proofs.get(&room) {
                retire_proven_edits(&mut self.document_store, &target, redacted);
            }
            if let Some(rejected) = invalid.get(&(room, target.clone())) {
                retire_proven_edits(&mut self.document_store, &target, rejected);
            }
        }
        let empty = HashSet::new();
        let mut messages: Vec<_> = self
            .attachment_retries
            .drain(..)
            .chain(self.queued_crawl_index.drain(..))
            .collect();
        // Seed known media before consuming text provenance, regardless of
        // producer order or server-clock skew. Edit guards still pick the newest
        // version; no timestamp is treated as proof of a redaction.
        messages.sort_by_key(|message| !message.carries_attachment());
        for message in messages {
            let redacted = message
                .attachment_target()
                .and_then(|(room, _)| proofs.get(room))
                .unwrap_or(&empty);
            let rejected = message
                .attachment_target()
                .and_then(|(room, target)| invalid.get(&(room.to_owned(), target.to_owned())))
                .unwrap_or(&empty);
            self.apply_proven_index_message(message, redacted, rejected);
        }
        for text in texts {
            if let Some(redacted) = proofs.get(&text.room_id)
                && !redacted.contains(&text.edit_event_id)
                && !redacted.contains(&text.target_event_id)
                && !invalid
                    .get(&(text.room_id.clone(), text.target_event_id.clone()))
                    .is_some_and(|ids| ids.contains(&text.edit_event_id))
            {
                self.document_store.upsert_edit(text, true);
            }
        }
        true
    }
}

#[cfg(test)]
mod tests;
