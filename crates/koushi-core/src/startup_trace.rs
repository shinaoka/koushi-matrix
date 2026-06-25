//! Diagnostic-only, private-data-free startup / event-load phase tracing.
//!
//! Enabled with `KOUSHI_STARTUP_TRACE=1`. Mirrors `app_loop_trace`
//! (`runtime.rs`): a no-op unless the env var is set, emitting stable
//! `key=value` tokens via `eprintln!`. Tokens carry durations and coarse
//! buckets ONLY — never room/event/user ids, message bodies, timestamps,
//! transaction ids, or raw SDK errors (engineering-rules Secrets / QA
//! redaction). Phase A adds observation only; it changes no product behavior.

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

/// True when startup tracing is enabled. Cheap; checked at each call site.
pub(crate) fn enabled() -> bool {
    std::env::var_os("KOUSHI_STARTUP_TRACE").is_some()
}

/// Capture a start instant only when tracing is enabled (true no-op otherwise).
pub(crate) fn now_if_enabled() -> Option<std::time::Instant> {
    enabled().then(std::time::Instant::now)
}

pub(crate) fn trace_phase(phase: StartupPhase, started: Option<std::time::Instant>) {
    if let Some(started) = started {
        eprintln!(
            "koushi.startup phase={} ms={}",
            phase.as_token(),
            started.elapsed().as_millis()
        );
    }
}

pub(crate) fn trace_phase_items(
    phase: StartupPhase,
    started: Option<std::time::Instant>,
    items: usize,
) {
    if let Some(started) = started {
        eprintln!(
            "koushi.startup phase={} ms={} items={}",
            phase.as_token(),
            started.elapsed().as_millis(),
            count_bucket(items)
        );
    }
}

pub(crate) fn trace_paginate(
    started: Option<std::time::Instant>,
    gate_wait: Option<Duration>,
    reached_start: bool,
) {
    if let Some(started) = started {
        let gate_ms = gate_wait.map(|d| d.as_millis()).unwrap_or(0);
        eprintln!(
            "koushi.startup phase=paginate ms={} gate_ms={} reached_start={}",
            started.elapsed().as_millis(),
            gate_ms,
            reached_start
        );
    }
}

/// `origin` must be one of the fixed strings "cache", "network", or "sync"
/// (mapped from the SDK `EventsOrigin`). Caller passes a `&'static str` so no
/// dynamic content can leak.
pub(crate) fn trace_origin(origin: &'static str) {
    if enabled() {
        eprintln!("koushi.startup phase=origin origin={origin}");
    }
}

/// Emitted when a background crawler page yields the /messages gate to a
/// user-visible pagination (preemption). Private-data-free.
pub(crate) fn trace_crawler_preempted() {
    if enabled() {
        eprintln!("koushi.startup phase=crawler_preempted");
    }
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
}
