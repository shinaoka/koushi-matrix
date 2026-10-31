//! Content-free, on-demand retention diagnostics for the #1344 memory audit.
//!
//! Reads the SDK's owner-side counters (`SearchIndex::diagnostics` and the
//! event cache's in-memory memory snapshot) at the moment they are requested.
//! Observation only: nothing here evicts, reloads, shrinks, or bounds anything.

use crate::MatrixClientSession;

/// Snapshot of one client's search index retention state.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct MatrixSearchIndexDiagnostics {
    /// Joined rooms with a `RoomIndex`.
    pub rooms_total: u64,
    /// Lifetime `open_read`/`atomic_read` round trips since the index opened.
    pub opened_files_total: u64,
    /// Lifetime decrypted plaintext bytes read.
    pub decrypted_bytes_total: u64,
    /// Decrypted plaintext bytes currently referenced by readers.
    pub live_decrypted_len_bytes: u64,
    /// Allocation capacity currently referenced by readers.
    pub live_decrypted_capacity_bytes: u64,
}

/// Snapshot of the event cache's resident room-timeline linked chunks.
///
/// Scope is room-timeline linked chunks only: thread, pinned-events and
/// event-focused caches are separate owners and are not counted. Event counts
/// are resident item lengths, not deduplicated event counts.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct MatrixEventCacheDiagnostics {
    /// Rooms with a resident room-timeline linked chunk.
    pub rooms: u64,
    /// Resident chunk nodes across those rooms.
    pub chunks: u64,
    /// Resident gap chunks.
    pub gap_chunks: u64,
    /// Resident events across those rooms' items chunks.
    pub events: u64,
    /// Rooms with a resident linked chunk and zero active subscribers.
    pub zero_subscriber_rooms: u64,
    /// Of those, rooms that also hold resident events.
    pub zero_subscriber_rooms_with_events: u64,
}

/// Combined retention snapshot for one client.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct MatrixRetentionDiagnostics {
    pub search: MatrixSearchIndexDiagnostics,
    pub event_cache: MatrixEventCacheDiagnostics,
}

/// Sample the client's retention counters now.
///
/// Requires the cache availability check performed during session admission.
/// The SDK reads are in-memory counters and snapshots; an account without a
/// search session reports zero search counters rather than an error.
pub async fn retention_diagnostics(session: &MatrixClientSession) -> MatrixRetentionDiagnostics {
    let search = session.client().search_index().diagnostics().await;
    let event_cache = session
        .client()
        .event_cache()
        .expect("Koushi clients enable the event cache")
        .memory_diagnostics()
        .await;
    MatrixRetentionDiagnostics {
        search: MatrixSearchIndexDiagnostics {
            rooms_total: search.rooms_total,
            opened_files_total: search.opened_files_total,
            decrypted_bytes_total: search.decrypted_bytes_total,
            live_decrypted_len_bytes: search.live_decrypted_len_bytes,
            live_decrypted_capacity_bytes: search.live_decrypted_capacity_bytes,
        },
        event_cache: MatrixEventCacheDiagnostics {
            rooms: event_cache.rooms,
            chunks: event_cache.chunks,
            gap_chunks: event_cache.gap_chunks,
            events: event_cache.events,
            zero_subscriber_rooms: event_cache.zero_subscriber_rooms,
            zero_subscriber_rooms_with_events: event_cache.zero_subscriber_rooms_with_events,
        },
    }
}
