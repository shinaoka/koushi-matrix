use std::collections::VecDeque;
use std::sync::{Mutex, MutexGuard, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

pub const DEFAULT_DIAGNOSTIC_CAPACITY: usize = 10_000;

#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DiagnosticLevel {
    Trace,
    Debug,
    Info,
    Warn,
    Error,
}

#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum DiagnosticValue {
    Boolean(bool),
    Count(u64),
    Milliseconds(u64),
    RequestId { connection_id: u64, sequence: u64 },
    Token(&'static str),
}

#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize)]
pub struct DiagnosticField {
    pub key: &'static str,
    pub value: DiagnosticValue,
}

#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize)]
pub struct DiagnosticEvent {
    pub level: DiagnosticLevel,
    pub source: &'static str,
    pub stage: &'static str,
    pub fields: Vec<DiagnosticField>,
}

#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize)]
pub struct DiagnosticRecord {
    #[serde(rename = "timestampMs")]
    pub timestamp_ms: u64,
    pub event: DiagnosticEvent,
}

#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize)]
pub struct DiagnosticSnapshot {
    pub records: Vec<DiagnosticRecord>,
    #[serde(rename = "droppedRecords")]
    pub dropped_records: u64,
}

impl DiagnosticEvent {
    pub fn new(level: DiagnosticLevel, source: &'static str, stage: &'static str) -> Self {
        Self {
            level,
            source,
            stage,
            fields: Vec::new(),
        }
    }

    pub fn field(mut self, field: DiagnosticField) -> Self {
        self.fields.push(field);
        self
    }
}

impl DiagnosticField {
    pub fn token(key: &'static str, value: &'static str) -> Self {
        Self {
            key,
            value: DiagnosticValue::Token(value),
        }
    }

    pub fn boolean(key: &'static str, value: bool) -> Self {
        Self {
            key,
            value: DiagnosticValue::Boolean(value),
        }
    }

    pub fn count(key: &'static str, value: u64) -> Self {
        Self {
            key,
            value: DiagnosticValue::Count(value),
        }
    }

    pub fn milliseconds(key: &'static str, value: u128) -> Self {
        Self {
            key,
            value: DiagnosticValue::Milliseconds(value.min(u64::MAX as u128) as u64),
        }
    }

    pub fn request_id(key: &'static str, connection_id: u64, sequence: u64) -> Self {
        Self {
            key,
            value: DiagnosticValue::RequestId {
                connection_id,
                sequence,
            },
        }
    }
}

pub struct DiagnosticBuffer {
    records: Mutex<VecDeque<DiagnosticRecord>>,
    dropped_records: Mutex<u64>,
    capacity: usize,
}

impl DiagnosticBuffer {
    pub fn new(capacity: usize) -> Self {
        Self {
            records: Mutex::new(VecDeque::with_capacity(capacity)),
            dropped_records: Mutex::new(0),
            capacity,
        }
    }

    pub fn record(&self, event: DiagnosticEvent) {
        self.record_at(timestamp_millis(), event);
    }

    pub fn record_at(&self, timestamp_ms: u64, event: DiagnosticEvent) {
        let mut records = lock_best_effort(&self.records);
        if self.capacity == 0 {
            increment_dropped(&self.dropped_records);
            return;
        }
        if records.len() == self.capacity {
            records.pop_front();
            increment_dropped(&self.dropped_records);
        }
        records.push_back(DiagnosticRecord {
            timestamp_ms,
            event,
        });
    }

    pub fn snapshot(&self) -> DiagnosticSnapshot {
        let records = lock_best_effort(&self.records).iter().cloned().collect();
        let dropped_records = *lock_best_effort(&self.dropped_records);
        DiagnosticSnapshot {
            records,
            dropped_records,
        }
    }
}

static GLOBAL_BUFFER: OnceLock<DiagnosticBuffer> = OnceLock::new();

pub fn record(event: DiagnosticEvent) {
    GLOBAL_BUFFER
        .get_or_init(|| DiagnosticBuffer::new(DEFAULT_DIAGNOSTIC_CAPACITY))
        .record(event);
}

pub fn snapshot() -> DiagnosticSnapshot {
    GLOBAL_BUFFER
        .get_or_init(|| DiagnosticBuffer::new(DEFAULT_DIAGNOSTIC_CAPACITY))
        .snapshot()
}

pub fn format_event(event: &DiagnosticEvent) -> String {
    let mut line = format!("stage={}", event.stage);
    for field in &event.fields {
        line.push(' ');
        line.push_str(field.key);
        line.push('=');
        match &field.value {
            DiagnosticValue::Boolean(value) => line.push_str(if *value { "true" } else { "false" }),
            DiagnosticValue::Count(value) | DiagnosticValue::Milliseconds(value) => {
                line.push_str(&value.to_string())
            }
            DiagnosticValue::RequestId {
                connection_id,
                sequence,
            } => line.push_str(&format!("{}:{}", connection_id, sequence)),
            DiagnosticValue::Token(value) => line.push_str(value),
        }
    }
    line
}

fn timestamp_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis().min(u64::MAX as u128) as u64)
        .unwrap_or(0)
}

fn lock_best_effort<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    match mutex.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

fn increment_dropped(counter: &Mutex<u64>) {
    let mut counter = lock_best_effort(counter);
    *counter = counter.saturating_add(1);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn event(stage: &'static str) -> DiagnosticEvent {
        DiagnosticEvent::new(DiagnosticLevel::Debug, "test", stage)
    }

    #[test]
    fn keeps_latest_records_and_reports_drops() {
        let buffer = DiagnosticBuffer::new(2);
        buffer.record_at(1, event("one"));
        buffer.record_at(2, event("two"));
        buffer.record_at(3, event("three"));

        let snapshot = buffer.snapshot();
        assert_eq!(snapshot.dropped_records, 1);
        assert_eq!(
            snapshot
                .records
                .iter()
                .map(|record| record.event.stage)
                .collect::<Vec<_>>(),
            vec!["two", "three"]
        );
    }

    #[test]
    fn records_concurrently_without_exceeding_capacity() {
        let buffer = Arc::new(DiagnosticBuffer::new(64));
        let workers = (0..8)
            .map(|_| {
                let buffer = Arc::clone(&buffer);
                std::thread::spawn(move || {
                    for index in 0..100 {
                        buffer.record_at(index, event("concurrent"));
                    }
                })
            })
            .collect::<Vec<_>>();
        for worker in workers {
            worker.join().unwrap();
        }
        let snapshot = buffer.snapshot();
        assert_eq!(snapshot.records.len(), 64);
        assert_eq!(snapshot.dropped_records, 736);
    }

    #[test]
    fn formats_only_structured_fields() {
        let line = format_event(
            &DiagnosticEvent::new(DiagnosticLevel::Debug, "core.timeline", "actor_finish")
                .field(DiagnosticField::token("operation", "send_reaction"))
                .field(DiagnosticField::milliseconds("elapsed_ms", 42))
                .field(DiagnosticField::boolean("success", true)),
        );
        assert_eq!(
            line,
            "stage=actor_finish operation=send_reaction elapsed_ms=42 success=true"
        );
    }
}
