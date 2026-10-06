use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, Weak};
use std::time::{SystemTime, UNIX_EPOCH};

pub const DEFAULT_DIAGNOSTIC_CAPACITY: usize = 10_000;
const DEFAULT_ROTATION_DIAGNOSTIC_CAPACITY: usize = 128;

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
    Correlation(u64),
    Milliseconds(u64),
    RequestId { connection_id: u64, sequence: u64 },
    Token(&'static str),
    OrdinalAlias { kind: &'static str, ordinal: u64 },
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RotationBoundaryDiagnostic {
    pub room_alias: u64,
    pub previous_session_alias: Option<u64>,
    pub new_session_alias: Option<u64>,
    pub reason: &'static str,
    pub creation_outcome: &'static str,
    pub first_share_outcome: &'static str,
    pub first_send_correlation_present: bool,
    pub discard_elapsed_ms: Option<u64>,
    pub elapsed_ms: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RotationDiagnosticSnapshot {
    pub records: Vec<DiagnosticRecord>,
    pub dropped_boundaries: u64,
}

struct RotationBoundaryRecord {
    timestamp_ms: u64,
    boundary: RotationBoundaryDiagnostic,
}

#[derive(Default)]
struct RotationDiagnosticState {
    records: VecDeque<RotationBoundaryRecord>,
    dropped_boundaries: u64,
}

pub struct RotationDiagnosticLedger {
    state: Mutex<RotationDiagnosticState>,
    capacity: usize,
}

impl RotationDiagnosticLedger {
    pub fn new(capacity: usize) -> Self {
        Self {
            state: Mutex::new(RotationDiagnosticState {
                records: VecDeque::with_capacity(capacity),
                dropped_boundaries: 0,
            }),
            capacity,
        }
    }

    pub fn record(&self, boundary: RotationBoundaryDiagnostic) {
        self.record_at(timestamp_millis_at(SystemTime::now()), boundary);
    }

    pub fn record_at(&self, timestamp_ms: u64, boundary: RotationBoundaryDiagnostic) {
        let mut state = lock_best_effort(&self.state);
        if self.capacity == 0 {
            state.dropped_boundaries = state.dropped_boundaries.saturating_add(1);
            return;
        }
        if state.records.len() == self.capacity {
            state.records.pop_front();
            state.dropped_boundaries = state.dropped_boundaries.saturating_add(1);
        }
        state.records.push_back(RotationBoundaryRecord {
            timestamp_ms,
            boundary,
        });
    }

    pub fn mark_first_send_correlation(&self, session_alias: u64) -> bool {
        let mut state = lock_best_effort(&self.state);
        let Some(record) = state
            .records
            .iter_mut()
            .rev()
            .find(|record| record.boundary.new_session_alias == Some(session_alias))
        else {
            return false;
        };
        record.boundary.first_send_correlation_present = true;
        true
    }

    pub fn reset(&self) {
        *lock_best_effort(&self.state) = RotationDiagnosticState::default();
    }

    pub fn snapshot(&self) -> RotationDiagnosticSnapshot {
        let state = lock_best_effort(&self.state);
        RotationDiagnosticSnapshot {
            records: state
                .records
                .iter()
                .map(|record| DiagnosticRecord {
                    timestamp_ms: record.timestamp_ms,
                    event: rotation_boundary_event(record.boundary),
                })
                .collect(),
            dropped_boundaries: state.dropped_boundaries,
        }
    }
}

fn rotation_boundary_event(boundary: RotationBoundaryDiagnostic) -> DiagnosticEvent {
    let mut event =
        DiagnosticEvent::new(DiagnosticLevel::Info, "core.room_key_rotation", "boundary")
            .field(DiagnosticField::ordinal_alias(
                "room_alias",
                "room",
                boundary.room_alias,
            ))
            .field(DiagnosticField::token("reason", boundary.reason))
            .field(DiagnosticField::token(
                "creation_outcome",
                boundary.creation_outcome,
            ))
            .field(DiagnosticField::token(
                "first_share_outcome",
                boundary.first_share_outcome,
            ))
            .field(DiagnosticField::boolean(
                "first_send_correlation_present",
                boundary.first_send_correlation_present,
            ))
            .field(DiagnosticField::milliseconds(
                "elapsed_ms",
                boundary.elapsed_ms.into(),
            ));
    if let Some(previous) = boundary.previous_session_alias {
        event = event.field(DiagnosticField::ordinal_alias(
            "previous_session_alias",
            "session",
            previous,
        ));
    }
    if let Some(new) = boundary.new_session_alias {
        event = event.field(DiagnosticField::ordinal_alias(
            "new_session_alias",
            "session",
            new,
        ));
    }
    if let Some(discard_elapsed_ms) = boundary.discard_elapsed_ms {
        event = event.field(DiagnosticField::milliseconds(
            "discard_elapsed_ms",
            discard_elapsed_ms.into(),
        ));
    }
    event
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

    pub fn optional_count(key: &'static str, value: Option<u32>) -> Self {
        match value {
            Some(value) => Self::count(key, u64::from(value)),
            None => Self::token(key, "none"),
        }
    }

    pub fn correlation(key: &'static str, value: u64) -> Self {
        Self {
            key,
            value: DiagnosticValue::Correlation(value),
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

    pub fn ordinal_alias(key: &'static str, kind: &'static str, ordinal: u64) -> Self {
        Self {
            key,
            value: DiagnosticValue::OrdinalAlias { kind, ordinal },
        }
    }
}

pub struct DiagnosticBuffer {
    records: Mutex<VecDeque<DiagnosticRecord>>,
    dropped_records: Mutex<u64>,
    capacity: usize,
}

/// Aggregate diagnostic counters owned by one client/runtime.
///
/// Keeping resets and mutations on this context prevents replacing one
/// account runtime from erasing counters that belong to another runtime.
#[derive(Default)]
pub struct DiagnosticCounterContext {
    counters: Mutex<BTreeMap<&'static str, u64>>,
}

impl DiagnosticCounterContext {
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a runtime-owned context that also contributes to the legacy
    /// process-wide diagnostic export while the runtime remains alive.
    pub fn registered() -> Arc<Self> {
        let context = Arc::new(Self::new());
        lock_best_effort(REGISTERED_COUNTER_CONTEXTS.get_or_init(|| Mutex::new(Vec::new())))
            .push(Arc::downgrade(&context));
        context
    }

    pub fn increment(&self, name: &'static str) {
        let mut counters = lock_best_effort(&self.counters);
        let counter = counters.entry(name).or_default();
        *counter = counter.saturating_add(1);
    }

    pub fn reset(&self, name: &'static str) {
        lock_best_effort(&self.counters).remove(name);
    }

    pub fn set(&self, name: &'static str, value: u64) {
        lock_best_effort(&self.counters).insert(name, value);
    }

    /// Snapshot only this client/runtime's aggregate counters.
    pub fn snapshot(&self) -> DiagnosticSnapshot {
        DiagnosticSnapshot {
            records: counter_records(
                &lock_best_effort(&self.counters),
                timestamp_millis_at(SystemTime::now()),
            ),
            dropped_records: 0,
        }
    }

    fn values(&self) -> BTreeMap<&'static str, u64> {
        lock_best_effort(&self.counters).clone()
    }
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
        self.record_at(timestamp_millis_at(SystemTime::now()), event);
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

    pub fn record_batch(&self, events: impl IntoIterator<Item = DiagnosticEvent>) {
        self.record_batch_at(timestamp_millis_at(SystemTime::now()), events);
    }

    pub fn record_batch_at(
        &self,
        timestamp_ms: u64,
        events: impl IntoIterator<Item = DiagnosticEvent>,
    ) {
        let mut records = lock_best_effort(&self.records);
        let mut dropped_records = lock_best_effort(&self.dropped_records);
        for event in events {
            if self.capacity == 0 {
                *dropped_records = dropped_records.saturating_add(1);
                continue;
            }
            if records.len() == self.capacity {
                records.pop_front();
                *dropped_records = dropped_records.saturating_add(1);
            }
            records.push_back(DiagnosticRecord {
                timestamp_ms,
                event,
            });
        }
    }

    pub fn snapshot(&self) -> DiagnosticSnapshot {
        let records_guard = lock_best_effort(&self.records);
        let records = records_guard.iter().cloned().collect();
        let dropped_records = *lock_best_effort(&self.dropped_records);
        DiagnosticSnapshot {
            records,
            dropped_records,
        }
    }

    /// Records appended at or after `cursor`, where a cursor is the number of
    /// records ever appended to this ring. With a non-zero capacity every
    /// eviction increments `dropped_records`, so the oldest retained record is
    /// number `dropped_records`. Returns `Err(evicted)` when records after the
    /// cursor were already evicted, instead of silently returning fewer.
    fn records_since(&self, cursor: u64) -> Result<Vec<DiagnosticRecord>, u64> {
        let records = lock_best_effort(&self.records);
        let first = *lock_best_effort(&self.dropped_records);
        if cursor < first {
            return Err(first - cursor);
        }
        let skip = usize::try_from(cursor - first).unwrap_or(usize::MAX);
        Ok(records.iter().skip(skip).cloned().collect())
    }

    /// The cursor just past the newest record; see [`Self::records_since`].
    fn end_cursor(&self) -> u64 {
        let records = lock_best_effort(&self.records);
        let first = *lock_best_effort(&self.dropped_records);
        first.saturating_add(records.len() as u64)
    }
}

static GLOBAL_BUFFER: OnceLock<DiagnosticBuffer> = OnceLock::new();
static GLOBAL_ROTATION_LEDGER: OnceLock<RotationDiagnosticLedger> = OnceLock::new();
static GLOBAL_COUNTER_CONTEXT: OnceLock<DiagnosticCounterContext> = OnceLock::new();
static REGISTERED_COUNTER_CONTEXTS: OnceLock<Mutex<Vec<Weak<DiagnosticCounterContext>>>> =
    OnceLock::new();

/// Test-only coordination for assertions against the process-wide diagnostic
/// buffer. Production diagnostics remain concurrent; tests that inspect the
/// global stream must hold this guard across the operation that emits and
/// checks records so parallel tests cannot consume one another's evidence.
#[doc(hidden)]
pub mod test_support {
    use tokio::sync::{Mutex, MutexGuard};

    use super::{
        DEFAULT_DIAGNOSTIC_CAPACITY, DEFAULT_ROTATION_DIAGNOSTIC_CAPACITY, DiagnosticBuffer,
        DiagnosticRecord, DiagnosticSnapshot, GLOBAL_BUFFER, GLOBAL_ROTATION_LEDGER,
        RotationDiagnosticLedger, RotationDiagnosticSnapshot,
    };

    // An async-aware mutex: asynchronous tests hold this process-wide lock
    // across await points for their whole body, so waiting must yield instead
    // of blocking a runtime worker. As with the previous poison-recovering std
    // lock, a panicking holder simply releases it.
    static GLOBAL_DIAGNOSTIC_TEST_LOCK: Mutex<()> = Mutex::const_new(());

    /// Serialize a synchronous test against every other diagnostics test.
    ///
    /// Panics when called from inside an async runtime; asynchronous tests
    /// must use [`lock_async`] so the guard can span await points safely.
    pub fn lock() -> MutexGuard<'static, ()> {
        GLOBAL_DIAGNOSTIC_TEST_LOCK.blocking_lock()
    }

    /// Serialize an asynchronous test against every other diagnostics test.
    pub async fn lock_async() -> MutexGuard<'static, ()> {
        GLOBAL_DIAGNOSTIC_TEST_LOCK.lock().await
    }

    /// Snapshot only the bounded detail ring. Use it for content checks over
    /// the whole ring; a test that needs "records since X" must use
    /// [`detail_cursor`] and [`detail_records_since`] instead of a snapshot
    /// length, which stops growing once the ring is full.
    pub fn detail_snapshot() -> DiagnosticSnapshot {
        global_detail_buffer().snapshot()
    }

    /// A monotonic position in the global detail ring: the number of records
    /// ever appended. Unlike `detail_snapshot().records.len()` it keeps
    /// advancing after the ring is full and starts evicting.
    #[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
    pub struct DetailCursor(u64);

    pub fn detail_cursor() -> DetailCursor {
        DetailCursor(global_detail_buffer().end_cursor())
    }

    /// Every detail record appended at or after `cursor`, oldest first.
    ///
    /// Panics if the ring has already evicted any of them, so an overflowing
    /// ring fails loudly instead of silently hiding the records under test.
    pub fn detail_records_since(cursor: DetailCursor) -> Vec<DiagnosticRecord> {
        global_detail_buffer()
            .records_since(cursor.0)
            .unwrap_or_else(|evicted| {
                panic!("the diagnostic ring evicted {evicted} record(s) after the test cursor")
            })
    }

    fn global_detail_buffer() -> &'static DiagnosticBuffer {
        GLOBAL_BUFFER.get_or_init(|| DiagnosticBuffer::new(DEFAULT_DIAGNOSTIC_CAPACITY))
    }

    pub fn rotation_snapshot() -> RotationDiagnosticSnapshot {
        GLOBAL_ROTATION_LEDGER
            .get_or_init(|| RotationDiagnosticLedger::new(DEFAULT_ROTATION_DIAGNOSTIC_CAPACITY))
            .snapshot()
    }
}

pub fn record(event: DiagnosticEvent) {
    GLOBAL_BUFFER
        .get_or_init(|| DiagnosticBuffer::new(DEFAULT_DIAGNOSTIC_CAPACITY))
        .record(event);
}

/// Records a structured event and mirrors the same private-data-free format
/// to stderr for startup paths whose UI diagnostics may not yet be reachable.
pub fn record_and_stderr(event: DiagnosticEvent) {
    eprintln!("[koushi] {} {}", event.source, format_event(&event));
    record(event);
}

pub fn record_batch(events: impl IntoIterator<Item = DiagnosticEvent>) {
    GLOBAL_BUFFER
        .get_or_init(|| DiagnosticBuffer::new(DEFAULT_DIAGNOSTIC_CAPACITY))
        .record_batch(events);
}

pub fn record_rotation_boundary(boundary: RotationBoundaryDiagnostic) {
    GLOBAL_ROTATION_LEDGER
        .get_or_init(|| RotationDiagnosticLedger::new(DEFAULT_ROTATION_DIAGNOSTIC_CAPACITY))
        .record(boundary);
}

pub fn mark_rotation_first_send_correlation(session_alias: u64) -> bool {
    GLOBAL_ROTATION_LEDGER
        .get_or_init(|| RotationDiagnosticLedger::new(DEFAULT_ROTATION_DIAGNOSTIC_CAPACITY))
        .mark_first_send_correlation(session_alias)
}

pub fn reset_rotation_ledger() {
    GLOBAL_ROTATION_LEDGER
        .get_or_init(|| RotationDiagnosticLedger::new(DEFAULT_ROTATION_DIAGNOSTIC_CAPACITY))
        .reset();
}

/// Increment a closed, privacy-safe aggregate diagnostic counter. Counter
/// summaries are appended outside the bounded detail ring when exported.
pub fn increment_counter(name: &'static str) {
    GLOBAL_COUNTER_CONTEXT
        .get_or_init(DiagnosticCounterContext::new)
        .increment(name);
}

/// Reset one aggregate counter when its owning account runtime is replaced.
pub fn reset_counter(name: &'static str) {
    GLOBAL_COUNTER_CONTEXT
        .get_or_init(DiagnosticCounterContext::new)
        .reset(name);
}

/// Set an aggregate counter to an absolute value. Used when mirroring an
/// authoritative SDK snapshot so repeated summaries do not inflate the count.
pub fn set_counter(name: &'static str, value: u64) {
    GLOBAL_COUNTER_CONTEXT
        .get_or_init(DiagnosticCounterContext::new)
        .set(name, value);
}

pub fn snapshot() -> DiagnosticSnapshot {
    let mut snapshot = GLOBAL_BUFFER
        .get_or_init(|| DiagnosticBuffer::new(DEFAULT_DIAGNOSTIC_CAPACITY))
        .snapshot();
    let rotation_snapshot = GLOBAL_ROTATION_LEDGER
        .get_or_init(|| RotationDiagnosticLedger::new(DEFAULT_ROTATION_DIAGNOSTIC_CAPACITY))
        .snapshot();
    snapshot.records.extend(rotation_snapshot.records);
    let timestamp_ms = timestamp_millis_at(SystemTime::now());
    snapshot
        .records
        .extend(counter_records(&aggregate_counter_values(), timestamp_ms));
    if rotation_snapshot.dropped_boundaries > 0 {
        snapshot.records.push(DiagnosticRecord {
            timestamp_ms,
            event: DiagnosticEvent::new(DiagnosticLevel::Info, "core.room_key_summary", "counter")
                .field(DiagnosticField::token(
                    "name",
                    "rotation_boundaries_dropped",
                ))
                .field(DiagnosticField::count(
                    "count",
                    rotation_snapshot.dropped_boundaries,
                )),
        });
    }
    snapshot
}

fn aggregate_counter_values() -> BTreeMap<&'static str, u64> {
    let mut totals = GLOBAL_COUNTER_CONTEXT
        .get_or_init(DiagnosticCounterContext::new)
        .values();
    let mut registered =
        lock_best_effort(REGISTERED_COUNTER_CONTEXTS.get_or_init(|| Mutex::new(Vec::new())));
    registered.retain(|context| {
        let Some(context) = context.upgrade() else {
            return false;
        };
        for (name, count) in context.values() {
            let total = totals.entry(name).or_default();
            *total = total.saturating_add(count);
        }
        true
    });
    totals
}

fn counter_records(
    counters: &BTreeMap<&'static str, u64>,
    timestamp_ms: u64,
) -> Vec<DiagnosticRecord> {
    counters
        .iter()
        .map(|(name, count)| DiagnosticRecord {
            timestamp_ms,
            event: DiagnosticEvent::new(DiagnosticLevel::Info, "core.room_key_summary", "counter")
                .field(DiagnosticField::token("name", name))
                .field(DiagnosticField::count("count", *count)),
        })
        .collect()
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
            DiagnosticValue::Correlation(value) => line.push_str(&format!("send-{value}")),
            DiagnosticValue::RequestId {
                connection_id,
                sequence,
            } => line.push_str(&format!("{}:{}", connection_id, sequence)),
            DiagnosticValue::Token(value) => line.push_str(value),
            DiagnosticValue::OrdinalAlias { kind, ordinal } => {
                line.push_str(kind);
                line.push('-');
                line.push_str(&ordinal.to_string());
            }
        }
    }
    line
}

fn timestamp_millis_at(now: SystemTime) -> u64 {
    now.duration_since(UNIX_EPOCH)
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
mod tests;
