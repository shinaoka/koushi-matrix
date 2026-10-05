use super::*;
use koushi_state::ReplyQuoteState;

/// Privacy-safe record of every projection of one reply observed through
/// `ItemsUpdated`. Only identity kinds and quote state kinds are retained.
#[derive(Debug, Default)]
pub(crate) struct ReplyQuoteLifecycle {
    observed: Vec<(&'static str, Option<ReplyQuoteState>)>,
    quote_target_mismatch: bool,
    settled_remote_ready: bool,
}

impl ReplyQuoteLifecycle {
    pub(crate) fn observe(
        &mut self,
        item: &TimelineItem,
        expected_body: &str,
        quoted_event_id: &str,
    ) {
        if !timeline_item_body_matches(item, expected_body) {
            return;
        }
        let identity = match &item.id {
            TimelineItemId::Event { .. } => "event",
            TimelineItemId::Transaction { .. } => "transaction",
            TimelineItemId::Synthetic { .. } => "synthetic",
        };
        let state = item.reply_quote.as_ref().map(|quote| {
            if quote.event_id != quoted_event_id {
                self.quote_target_mismatch = true;
            }
            quote.state
        });
        self.observed.push((identity, state));
        self.settled_remote_ready = identity == "event" && state == Some(ReplyQuoteState::Ready);
    }

    pub(crate) fn settled(&self) -> bool {
        self.settled_remote_ready
    }

    /// The reply-quote lifecycle contract: every projection of the reply
    /// carries a quote of the expected original; the quote may be `Loading`
    /// only before it first becomes `Ready`, and it never claims that the
    /// original is missing, unsupported, or failed.
    pub(crate) fn lifecycle_violation(&self) -> Option<String> {
        if self.observed.is_empty() {
            return Some("no reply projection observed".to_owned());
        }
        if self.quote_target_mismatch {
            return Some("quote target mismatch".to_owned());
        }
        let mut ready_seen = false;
        let violated = self.observed.iter().any(|(_, state)| match state {
            Some(ReplyQuoteState::Ready) => {
                ready_seen = true;
                false
            }
            Some(ReplyQuoteState::Loading) => ready_seen,
            _ => true,
        });
        violated.then(|| format!("sequence={}", self.summary()))
    }

    pub(crate) fn summary(&self) -> String {
        self.observed
            .iter()
            .map(|(identity, state)| {
                let state = state.map_or("none", |state| state.as_str());
                format!("{identity}:{state}")
            })
            .collect::<Vec<_>>()
            .join(",")
    }
}

/// Wait for `request_id` to complete and for the remote echo of the reply to
/// settle with a ready quote, recording every intermediate projection.
pub(crate) async fn observe_reply_quote_lifecycle(
    conn: &mut CoreConnection,
    key: &TimelineKey,
    request_id: RequestId,
    expected_body: &str,
    quoted_event_id: &str,
    label: &str,
) -> Result<(ReplyQuoteLifecycle, String), String> {
    let deadline = QaEventDeadline::after(EVENT_TIMEOUT);
    let mut lifecycle = ReplyQuoteLifecycle::default();
    let mut sent_event_id = None;
    loop {
        if let Some(event_id) = sent_event_id.as_ref()
            && lifecycle.settled()
        {
            return Ok((lifecycle, String::clone(event_id)));
        }
        let event = deadline
            .recv(conn)
            .await
            .map_err(|_| format!("{label}: timed out ({})", lifecycle.summary()))?
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;
        match event {
            CoreEvent::Timeline(TimelineEvent::ItemsUpdated {
                key: ref event_key,
                ref diffs,
                ..
            }) if event_key == key => {
                visit_timeline_diff_items(diffs, |item| {
                    lifecycle.observe(item, expected_body, quoted_event_id);
                    Ok(())
                })?;
            }
            CoreEvent::Timeline(TimelineEvent::SendCompleted {
                request_id: event_request_id,
                key: ref event_key,
                ref event_id,
                ..
            }) if event_request_id == request_id && event_key == key => {
                sent_event_id = Some(event_id.clone());
            }
            CoreEvent::OperationFailed {
                request_id: event_request_id,
                failure,
            } if event_request_id == request_id => {
                return Err(format!("{label} failed: {failure:?}"));
            }
            _ => {}
        }
    }
}
