//! Diagnostic-only, private-data-free startup / event-load phase tracing.
//!
//! Always records a structured observation. Tokens carry durations and coarse
//! buckets ONLY — never room/event/user ids, message bodies, timestamps,
//! transaction ids, or raw SDK errors (engineering-rules Secrets / QA
//! redaction). Phase A adds observation only; it changes no product behavior.

use koushi_diagnostics::{DiagnosticEvent, DiagnosticField, DiagnosticLevel, record};
use std::time::Duration;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum StartupPhase {
    Restore,
    TimelineBuild,
    TimelineSubscribe,
    CrawlerPage,
}

impl StartupPhase {
    pub(crate) fn as_token(self) -> &'static str {
        match self {
            StartupPhase::Restore => "restore",
            StartupPhase::TimelineBuild => "timeline_build",
            StartupPhase::TimelineSubscribe => "subscribe",
            StartupPhase::CrawlerPage => "crawler_page",
        }
    }
}

/// Coarse item-count bucket so exact event counts never leak.
pub(crate) fn count_bucket(n: usize) -> &'static str {
    match n {
        0 => "0",
        1..=10 => "1-10",
        11..=50 => "11-50",
        _ => "51+",
    }
}

pub(crate) fn now() -> std::time::Instant {
    std::time::Instant::now()
}

pub(crate) fn trace_phase(phase: StartupPhase, started: Option<std::time::Instant>) {
    let Some(started) = started else { return };
    let elapsed_ms = started.elapsed().as_millis();
    record(
        DiagnosticEvent::new(DiagnosticLevel::Debug, "core.startup", phase.as_token())
            .field(DiagnosticField::milliseconds("duration", elapsed_ms)),
    );
}

pub(crate) fn trace_phase_items(
    phase: StartupPhase,
    started: Option<std::time::Instant>,
    items: usize,
) {
    let Some(started) = started else { return };
    let elapsed_ms = started.elapsed().as_millis();
    let bucket = count_bucket(items);
    record(
        DiagnosticEvent::new(DiagnosticLevel::Debug, "core.startup", phase.as_token())
            .field(DiagnosticField::milliseconds("duration", elapsed_ms))
            .field(DiagnosticField::token("items", bucket)),
    );
}

pub(crate) fn trace_paginate(
    started: Option<std::time::Instant>,
    gate_wait: Option<Duration>,
    reached_start: bool,
) {
    let Some(started) = started else { return };
    let elapsed_ms = started.elapsed().as_millis();
    let gate_ms = gate_wait.map(|d| d.as_millis()).unwrap_or(0);
    record(
        DiagnosticEvent::new(DiagnosticLevel::Debug, "core.startup", "paginate")
            .field(DiagnosticField::milliseconds("duration", elapsed_ms))
            .field(DiagnosticField::milliseconds("gate_wait", gate_ms))
            .field(DiagnosticField::boolean("reached_start", reached_start)),
    );
}

/// `origin` must be one of the fixed strings "cache", "network", or "sync"
/// (mapped from the SDK `EventsOrigin`). Caller passes a `&'static str` so no
/// dynamic content can leak.
pub(crate) fn trace_origin(origin: &'static str) {
    record(
        DiagnosticEvent::new(DiagnosticLevel::Debug, "core.startup", "origin")
            .field(DiagnosticField::token("origin", origin)),
    );
}

/// Emitted when a background crawler page yields the /messages gate to a
/// user-visible pagination (preemption). Private-data-free.
pub(crate) fn trace_crawler_preempted() {
    record(DiagnosticEvent::new(
        DiagnosticLevel::Debug,
        "core.startup",
        "crawler_preempted",
    ));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn count_bucket_never_leaks_exact_counts() {
        assert_eq!(count_bucket(0), "0");
        assert_eq!(count_bucket(1), "1-10");
        assert_eq!(count_bucket(10), "1-10");
        assert_eq!(count_bucket(11), "11-50");
        assert_eq!(count_bucket(50), "11-50");
        assert_eq!(count_bucket(51), "51+");
        assert_eq!(count_bucket(85_850), "51+");
    }

    #[test]
    fn phase_tokens_are_stable_lowercase_identifiers() {
        for phase in [
            StartupPhase::Restore,
            StartupPhase::TimelineBuild,
            StartupPhase::TimelineSubscribe,
            StartupPhase::CrawlerPage,
        ] {
            let token = phase.as_token();
            assert!(!token.is_empty());
            assert!(
                token.chars().all(|c| c.is_ascii_lowercase() || c == '_'),
                "phase token must be a private-data-free lowercase identifier"
            );
        }
    }

    #[test]
    fn startup_phase_records_without_environment_switch() {
        trace_phase(StartupPhase::Restore, Some(std::time::Instant::now()));
        assert!(koushi_diagnostics::snapshot().records.iter().any(|record| {
            record.event.source == "core.startup" && record.event.stage == "restore"
        }));
    }
}
