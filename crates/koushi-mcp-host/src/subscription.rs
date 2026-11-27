//! The push subscription, its lease and cursor acknowledgement (#2132, design
//! sections 4-6).
//!
//! A protocol 2 connection may hold one subscription. The reader loop in
//! `server.rs` owns a [`SubSlot`]; the pump is a task that sends `Push` frames
//! into the connection's writer channel. Pace comes from the durable feed, not
//! from a buffer: at most `push_window` rows are sent and unacknowledged, and a
//! full window stops the pump reading the feed. A row is therefore delivered,
//! redelivered after the acknowledged cursor, or its loss is named in a typed
//! `SubEnd`; none is skipped silently.

use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use koushi_core::mention_feed::{CursorRejection, MentionFeed};
use serde_json::{Value, json};
use tokio::sync::mpsc::OwnedPermit;
use tokio::sync::{Notify, mpsc};

use crate::consent::{ClientIdentity, ConsentContext};
use crate::grants::Need;
use crate::profile_limits::Limits;
use crate::profile_state::SaveOutcome;
use crate::server::Shared;
use crate::subject::ProfileId;
use crate::{handlers, profile_state, profiles};
use koushi_mcp::wire::{ErrorCode, ServerMessage, WireError};

/// One subscriber per profile: the connection that holds a profile's lease.
/// In memory only, so a restarted Koushi has no holders (no stale lock).
#[derive(Default)]
pub(crate) struct Leases {
    held: Mutex<HashMap<ProfileId, u64>>,
    next: AtomicU64,
    /// Told when a lease is taken or freed, for the settings page.
    broker: Option<Arc<crate::consent::ConsentBroker>>,
}

impl Leases {
    pub(crate) fn with_broker(broker: Arc<crate::consent::ConsentBroker>) -> Self {
        Self {
            broker: Some(broker),
            ..Self::default()
        }
    }

    fn acquire(self: &Arc<Self>, profile: &ProfileId) -> Result<LeaseGuard, WireError> {
        let mut held = self.held.lock().unwrap_or_else(PoisonError::into_inner);
        if held.contains_key(profile) {
            // No holder detail: the answer must not say who or from where.
            return Err(WireError::new(
                ErrorCode::ProfileBusy,
                "another connection already holds this profile's subscription",
            ));
        }
        let token = self.next.fetch_add(1, Ordering::Relaxed);
        held.insert(profile.clone(), token);
        drop(held);
        if let Some(broker) = &self.broker {
            broker.profile_subscribed(profile, true);
        }
        Ok(LeaseGuard(Lease {
            leases: Arc::clone(self),
            profile: profile.clone(),
            token,
        }))
    }
}

/// A non-owning handle that can release the lease it names. Releasing is
/// idempotent and token-checked, so a late release can never free a lease a
/// newer subscription took.
#[derive(Clone)]
struct Lease {
    leases: Arc<Leases>,
    profile: ProfileId,
    token: u64,
}

impl Lease {
    fn release(&self) {
        let mut held = self
            .leases
            .held
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if held.get(&self.profile) == Some(&self.token) {
            held.remove(&self.profile);
            drop(held);
            if let Some(broker) = &self.leases.broker {
                broker.profile_subscribed(&self.profile, false);
            }
        }
    }
}

/// Owned by the connection, so every way the connection task ends, including
/// an abort at shutdown, releases the lease synchronously.
struct LeaseGuard(Lease);

impl Drop for LeaseGuard {
    fn drop(&mut self) {
        self.0.release();
    }
}

/// What both sides of one subscription see: the sent-and-unacknowledged
/// window and the acknowledged position.
struct SubState {
    sub: u64,
    feed: MentionFeed,
    profile: ProfileId,
    data_dir: PathBuf,
    window: Mutex<Window>,
    /// Held across a write, so two flushes never interleave.
    flushing: tokio::sync::Mutex<()>,
    /// A permit is stored if an ack lands before the pump waits.
    acked: Notify,
    /// Set only once the end of the stream is settled: its `SubEnd` is queued,
    /// or nobody is left to tell. A resubscribe may replace a subscription only
    /// then, so it can never erase a `SubEnd`.
    ended: AtomicBool,
    /// One slot of the writer queue, kept back for this subscription's
    /// `SubEnd`, so the end can always be queued however full the queue is.
    /// Whoever takes it ends the stream; there is exactly one `SubEnd`.
    end_permit: Mutex<Option<OwnedPermit<ServerMessage>>>,
    /// Consecutive acknowledged-cursor writes that failed.
    flush_failures: AtomicU32,
}

struct Window {
    /// Sequence numbers sent and not yet acknowledged, in order.
    outstanding: VecDeque<u64>,
    acked: u64,
    /// The acknowledged position known to be on disk.
    persisted: u64,
    sent: u64,
}

impl SubState {
    fn window(&self) -> std::sync::MutexGuard<'_, Window> {
        self.window.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn free(&self, limit: usize) -> usize {
        limit.saturating_sub(self.window().outstanding.len())
    }

    /// Writes the acknowledged cursor if it is ahead of what is durable.
    /// Serialised, and "durable" is recorded only after the write succeeded, so
    /// a flush that was abandoned mid-write (the pump is aborted) is simply
    /// repeated by the next one, and `unsubscribe` never answers before the
    /// cursor is on disk.
    async fn flush(self: &Arc<Self>) {
        let _one_at_a_time = self.flushing.lock().await;
        let target = {
            let window = self.window();
            if window.acked <= window.persisted {
                return;
            }
            window.acked
        };
        let cursor = self.feed.cursor_for(target);
        let state = Arc::clone(self);
        let outcome = tokio::task::spawn_blocking(move || {
            let feed = state.feed.clone();
            profile_state::save_acked(&state.data_dir, &state.profile, &cursor, |text| {
                feed.seq_of(text).ok()
            })
        })
        .await
        .unwrap_or(SaveOutcome::Failed);
        match outcome {
            SaveOutcome::Durable => {
                let mut window = self.window();
                window.persisted = window.persisted.max(target);
                self.flush_failures.store(0, Ordering::Release);
            }
            // A revoked profile has nothing to save, and that is not a fault.
            SaveOutcome::Revoked => {}
            SaveOutcome::Failed => {
                let failures = self.flush_failures.fetch_add(1, Ordering::AcqRel) + 1;
                // Once per streak, not once per second.
                if failures == 1 {
                    koushi_diagnostics::record(
                        koushi_diagnostics::DiagnosticEvent::new(
                            koushi_diagnostics::DiagnosticLevel::Warn,
                            "mcp.subscription",
                            "ack_flush",
                        )
                        .field(koushi_diagnostics::DiagnosticField::token(
                            "reason",
                            "write_failed",
                        )),
                    );
                }
            }
        }
    }

    /// Ends the stream exactly once: queues its `SubEnd` through the reserved
    /// slot (never waiting, never failing for a full queue), then marks the
    /// subscription ended. No await between the two, so the pair cannot be
    /// split by a cancellation.
    fn end(&self, error: Option<WireError>) {
        let permit = self
            .end_permit
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        if let (Some(permit), Some(error)) = (permit, error) {
            permit.send(ServerMessage::SubEnd {
                sub: self.sub,
                error,
            });
        }
        self.ended.store(true, Ordering::Release);
    }

    /// Cumulative acknowledgement (D7).
    fn ack(&self, cursor: &str) -> Result<(), WireError> {
        let seq = self.feed.seq_of(cursor).map_err(|rejection| {
            WireError::with_reason(
                ErrorCode::Unsupported,
                rejection.reason(),
                "the cursor was not issued for this subscription's account",
            )
        })?;
        let mut window = self.window();
        if seq > window.sent {
            return Err(WireError::with_reason(
                ErrorCode::Unsupported,
                "ack_ahead",
                "the cursor is ahead of what this subscription sent",
            ));
        }
        // An older cursor changes nothing.
        if seq > window.acked {
            window.acked = seq;
            while window
                .outstanding
                .front()
                .is_some_and(|front| *front <= seq)
            {
                window.outstanding.pop_front();
            }
            drop(window);
            self.acked.notify_one();
        }
        Ok(())
    }
}

/// A pump task that is aborted when dropped, so a connection that ends by any
/// route leaves no sender behind.
struct PumpTask(Option<tokio::task::JoinHandle<()>>);

impl PumpTask {
    async fn stop(&mut self) {
        if let Some(task) = self.0.take() {
            task.abort();
            let _ = task.await;
        }
    }
}

impl Drop for PumpTask {
    fn drop(&mut self) {
        if let Some(task) = &self.0 {
            task.abort();
        }
    }
}

struct Active {
    id: u64,
    state: Arc<SubState>,
    pump: PumpTask,
    /// Held for its `Drop`: the lease is released when this is dropped.
    _guard: LeaseGuard,
}

/// The connection's subscription, at most one.
#[derive(Default)]
pub(crate) struct SubSlot {
    current: Option<Active>,
    counter: u64,
    stall_guard: Arc<AtomicBool>,
}

/// What `subscribe` hands back besides the response: the pump is started only
/// after the response is queued, so no `Push` can precede it.
pub(crate) struct PumpStart {
    ctx: PumpCtx,
}

struct PumpCtx {
    shared: Shared,
    account_id: String,
    profile: ProfileId,
    connection: koushi_core::CoreConnection,
    sub: u64,
    state: Arc<SubState>,
    lease: Lease,
    position: String,
}

impl SubSlot {
    /// Starts the pump of a subscription whose response has been queued. The
    /// `SubEnd` slot is reserved first, so the end can always be told.
    pub(crate) async fn start(&mut self, start: PumpStart, tx: &mpsc::Sender<ServerMessage>) {
        let Some(active) = self.current.as_mut() else {
            return;
        };
        if active.id != start.ctx.sub {
            return;
        }
        let Ok(permit) = tx.clone().reserve_owned().await else {
            return;
        };
        *active
            .state
            .end_permit
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(permit);
        self.stall_guard.store(true, Ordering::Release);
        active.pump = PumpTask(Some(tokio::spawn(pump(start.ctx, tx.clone()))));
    }

    /// Whether the connection ever subscribed: only then may a frame that
    /// cannot be written end the writer. A protocol 1 connection never has.
    pub(crate) fn write_timeout_armed(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.stall_guard)
    }

    /// Ends the subscription: stops the pump, tells the peer why if a reason is
    /// given and the pump has not already, makes the acknowledged cursor
    /// durable, then releases the lease, in that order, so the next holder
    /// reads what this one acknowledged. `current` stays until the flush is
    /// done, so a cancelled call is simply repeated by the next one.
    pub(crate) async fn finish_with(&mut self, end: Option<WireError>) {
        if let Some(active) = self.current.as_mut() {
            active.pump.stop().await;
            active.state.end(end);
            active.state.flush().await;
        }
        // The guard is dropped here, after the flush.
        self.current = None;
    }

    pub(crate) async fn finish(&mut self) {
        self.finish_with(None).await;
    }
}

fn invalid(message: &str) -> WireError {
    WireError::with_reason(ErrorCode::Unsupported, "invalid_params", message)
}

fn unknown_sub() -> WireError {
    WireError::with_reason(
        ErrorCode::Unsupported,
        "unknown_sub",
        "no such subscription on this connection",
    )
}

fn sub_param(params: &Value) -> Result<u64, WireError> {
    params
        .get("sub")
        .and_then(Value::as_u64)
        .ok_or_else(|| invalid("sub must be a number"))
}

/// `subscribe {after?}` -> `{sub, start_cursor, window}`.
pub(crate) async fn subscribe(
    shared: &Shared,
    client: &ClientIdentity,
    slot: &mut SubSlot,
    params: &Value,
) -> Result<(Value, Option<PumpStart>), WireError> {
    let (Some(profile), Some(account_id)) = (client.subject().profile_id(), client.bound_account())
    else {
        return Err(WireError::new(
            ErrorCode::Unsupported,
            "subscribe needs an agent profile",
        ));
    };
    let (profile, account_id) = (profile.clone(), account_id.to_owned());
    if let Some(active) = &slot.current {
        if !active.state.ended.load(Ordering::Acquire) {
            return Err(WireError::with_reason(
                ErrorCode::Unsupported,
                "already_subscribed",
                "this connection already has a subscription",
            ));
        }
        // A subscription that ended on its own: its lease is already free.
        slot.current = None;
    }
    let after = match params.get("after") {
        None | Some(Value::Null) => None,
        Some(Value::String(text)) => Some(text.as_str()),
        Some(_) => return Err(invalid("after must be a string")),
    };

    let connection = handlers::authorize(
        shared,
        client,
        &account_id,
        Need::Read,
        ConsentContext::default(),
    )
    .await?;
    let guard = shared.leases.acquire(&profile)?;
    let feed = shared
        .accounts
        .mention_feed(&account_id)
        .ok_or_else(|| WireError::new(ErrorCode::AccountUnavailable, "account is not ready"))?;
    if !feed.is_healthy() {
        return Err(unhealthy());
    }

    let data_dir = shared.broker.data_dir().to_owned();
    let start_cursor = match after {
        Some("retained") => feed.retained_start_cursor(),
        Some(cursor) => {
            // The feed's own validation, reported with the feed's own reasons.
            feed.wait(Some(cursor), Duration::ZERO, 1)
                .await
                .map_err(client_cursor_rejected)?;
            cursor.to_owned()
        }
        None => {
            let read_profile = profile.clone();
            let read_dir = data_dir.clone();
            let state =
                tokio::task::spawn_blocking(move || profile_state::load(&read_dir, &read_profile))
                    .await
                    .map_err(|_| WireError::new_without_detail(ErrorCode::Internal))?;
            // Resume after what was acknowledged; a new profile starts at its
            // creation baseline; with neither, everything still retained.
            let resume = state
                .acked_cursor
                .or_else(|| state.baseline.map(|baseline| baseline.head_cursor));
            match resume {
                Some(cursor) => {
                    feed.wait(Some(&cursor), Duration::ZERO, 1)
                        .await
                        .map_err(lost_position)?;
                    cursor
                }
                None => feed.retained_start_cursor(),
            }
        }
    };
    let start_seq = feed.seq_of(&start_cursor).map_err(client_cursor_rejected)?;

    slot.counter += 1;
    let sub = slot.counter;
    let state = Arc::new(SubState {
        sub,
        feed,
        profile: profile.clone(),
        data_dir,
        window: Mutex::new(Window {
            outstanding: VecDeque::new(),
            acked: start_seq,
            persisted: start_seq,
            sent: start_seq,
        }),
        flushing: tokio::sync::Mutex::new(()),
        acked: Notify::new(),
        ended: AtomicBool::new(false),
        end_permit: Mutex::new(None),
        flush_failures: AtomicU32::new(0),
    });
    let lease = guard.0.clone();
    slot.current = Some(Active {
        id: sub,
        state: Arc::clone(&state),
        pump: PumpTask(None),
        _guard: guard,
    });
    let window = shared.limits.push_window;
    Ok((
        json!({"sub": sub, "start_cursor": start_cursor, "window": window}),
        Some(PumpStart {
            ctx: PumpCtx {
                shared: shared.clone(),
                account_id,
                profile,
                connection,
                sub,
                state,
                lease,
                position: start_cursor,
            },
        }),
    ))
}

/// `unsubscribe {sub}`: the stream ends and the lease is released; the
/// connection stays.
pub(crate) async fn unsubscribe(slot: &mut SubSlot, params: &Value) -> Result<Value, WireError> {
    let sub = sub_param(params)?;
    if slot.current.as_ref().map(|active| active.id) != Some(sub) {
        return Err(unknown_sub());
    }
    slot.finish().await;
    Ok(json!({}))
}

/// `ack {sub, cursor}`: cumulative, idempotent for an older cursor.
pub(crate) fn ack(slot: &SubSlot, params: &Value) -> Result<Value, WireError> {
    let sub = sub_param(params)?;
    let cursor = params
        .get("cursor")
        .and_then(Value::as_str)
        .ok_or_else(|| invalid("cursor must be a string"))?;
    let Some(active) = slot
        .current
        .as_ref()
        .filter(|active| active.id == sub && !active.state.ended.load(Ordering::Acquire))
    else {
        return Err(unknown_sub());
    };
    active.state.ack(cursor)?;
    Ok(json!({}))
}

fn unhealthy() -> WireError {
    WireError::new(
        ErrorCode::AccountUnavailable,
        "the mention log could not be saved; mentions may have been missed",
    )
}

/// A cursor the client chose, refused: the feed's own reason.
fn client_cursor_rejected(rejection: CursorRejection) -> WireError {
    WireError::with_reason(
        ErrorCode::ResyncRequired,
        rejection.reason(),
        format!(
            "{}: {}. Subscribe with after:\"retained\" to receive everything still retained, and de-duplicate by event id.",
            rejection.reason(),
            rejection.explanation()
        ),
    )
}

/// A position Core held (the acknowledged cursor, or one it was streaming
/// from) that the feed can no longer serve. Rows were lost to retention, or the
/// log was recreated; both are named, never skipped.
fn lost_position(rejection: CursorRejection) -> WireError {
    let reason = match rejection {
        CursorRejection::Expired => "expired_cursor",
        // A cursor that is foreign, malformed or ahead of a log it was
        // acknowledged on means that log is not this one.
        _ => "generation_changed",
    };
    WireError::with_reason(
        ErrorCode::ResyncRequired,
        reason,
        format!(
            "{reason}: wake rows may have been missed. Subscribe with after:\"retained\" and de-duplicate by event id."
        ),
    )
}

fn stalled(reason: &str, message: &str) -> WireError {
    WireError::with_reason(ErrorCode::ResyncRequired, reason, message)
}

/// Why the pump stopped.
enum Exit {
    /// The connection is gone; there is nobody to tell.
    Gone,
    Ended(WireError),
}

/// Everything that must hold before a batch is sent, as `wait_for_mentions`
/// re-checks after its wait. The profile store is read off the async worker.
async fn check(ctx: &PumpCtx) -> Result<(), WireError> {
    if !ctx.state.feed.is_healthy() {
        return Err(unhealthy());
    }
    let current = handlers::account_connection(&ctx.shared.accounts, &ctx.account_id)?;
    // The live feed, not the one this subscription started on: a recreated feed
    // is another log, and the position held here means nothing in it.
    let live = ctx.shared.accounts.mention_feed(&ctx.account_id);
    if !ctx.connection.shares_runtime_with(&current) || live.is_none() {
        return Err(WireError::new(
            ErrorCode::AccountUnavailable,
            "account changed during the subscription",
        ));
    }
    if live.is_some_and(|live| live.generation() != ctx.state.feed.generation()) {
        return Err(stalled(
            "generation_changed",
            "generation_changed: the mention log was recreated; wake rows may have been missed. Subscribe with after:\"retained\" and de-duplicate by event id.",
        ));
    }
    if !current.snapshot().is_agent_account() {
        return Err(WireError::with_reason(
            ErrorCode::AccountUnavailable,
            "not_agent_account",
            "the account is no longer an agent account",
        ));
    }
    let data_dir = ctx.shared.broker.data_dir().to_owned();
    let profile = ctx.profile.clone();
    let lookup = tokio::task::spawn_blocking(move || profiles::lookup_checked(&data_dir, &profile))
        .await
        .map_err(|_| WireError::new_without_detail(ErrorCode::Internal))?;
    match lookup {
        profiles::Lookup::Found(row) => {
            if row.account_id != ctx.account_id {
                return Err(crate::consent::profile_account());
            }
            if !row.grants.read {
                return Err(WireError::new(
                    ErrorCode::NotPermitted,
                    "read access was removed from this profile",
                ));
            }
            Ok(())
        }
        profiles::Lookup::Gone => Err(crate::consent::profile_revoked()),
        profiles::Lookup::Unusable => Err(crate::consent::profile_store_unusable()),
    }
}

async fn pump(ctx: PumpCtx, tx: mpsc::Sender<ServerMessage>) {
    let limits = ctx.shared.limits;
    // `tx` is only for the stream's frames; the end goes through the reserved
    // slot in `SubState`.
    let exit = tokio::select! {
        biased;
        exit = stream(&ctx, &tx, limits) => exit,
        error = flush_ticker(&ctx.state, limits.ack_flush) => Exit::Ended(error),
    };
    // Ending: the position is made durable, then the lease is freed, and only
    // then is the peer told, so a consumer that resubscribes at once finds
    // both in place. The `SubEnd` is queued before the subscription counts as
    // ended, so a resubscribe cannot replace it unseen.
    ctx.state.flush().await;
    ctx.lease.release();
    match exit {
        Exit::Ended(error) => ctx.state.end(Some(error)),
        Exit::Gone => ctx.state.end(None),
    }
}

/// How many consecutive failed writes of the acknowledged cursor (one per
/// `ACK_FLUSH`) end the stream rather than redeliver forever.
const FLUSH_FAILURE_LIMIT: u32 = 5;

/// Writes a changed acknowledged cursor at most once per `every`. Returns only
/// to report that the cursor cannot be saved.
async fn flush_ticker(state: &Arc<SubState>, every: Duration) -> WireError {
    let mut ticker = tokio::time::interval(every);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        ticker.tick().await;
        state.flush().await;
        if state.flush_failures.load(Ordering::Acquire) >= FLUSH_FAILURE_LIMIT {
            return WireError::with_reason(
                ErrorCode::Internal,
                "ack_unsaved",
                "ack_unsaved: the acknowledged cursor could not be saved. Fix the disk, then subscribe again without after; unacknowledged rows are redelivered.",
            );
        }
    }
}

async fn stream(ctx: &PumpCtx, tx: &mpsc::Sender<ServerMessage>, limits: Limits) -> Exit {
    let mut position = ctx.position.clone();
    loop {
        // A full window stops the pump reading the feed. The feed is a durable
        // log, so waiting here loses nothing.
        while ctx.state.free(limits.push_window) == 0 {
            let acked = ctx.state.acked.notified();
            if ctx.state.free(limits.push_window) > 0 {
                break;
            }
            if tokio::time::timeout(limits.ack_stall, acked).await.is_err() {
                return Exit::Ended(stalled(
                    "ack_stalled",
                    "ack_stalled: the window stayed full without an ack. Subscribe again without after; unacknowledged rows are redelivered.",
                ));
            }
        }
        let room = ctx.state.free(limits.push_window);
        let page = match ctx
            .state
            .feed
            .wait(Some(&position), limits.recheck, room)
            .await
        {
            Ok(page) => page,
            Err(rejection) => return Exit::Ended(lost_position(rejection)),
        };
        // One check per page, and one per idle wake-up: the wait can last, so
        // it comes after the wait, never before.
        if let Err(error) = check(ctx).await {
            return Exit::Ended(error);
        }
        if page.entries.is_empty() {
            continue;
        }
        let rows: Vec<Value> = page
            .entries
            .iter()
            .map(|entry| handlers::mention_row(&ctx.state.feed, &ctx.account_id, entry))
            .collect();
        {
            // Recorded before the frame can reach the peer, so its ack is
            // never "ahead".
            let mut window = ctx.state.window();
            for entry in &page.entries {
                window.outstanding.push_back(entry.seq);
                window.sent = entry.seq;
            }
        }
        position = page.cursor.clone();
        let push = ServerMessage::Push {
            sub: ctx.sub,
            rows,
            cursor: page.cursor,
        };
        match tokio::time::timeout(limits.write_stall, tx.send(push)).await {
            Ok(Ok(())) => {}
            // The writer is gone: the connection is ending.
            Ok(Err(_)) => return Exit::Gone,
            // The queue stayed full: the peer is not reading.
            Err(_) => {
                return Exit::Ended(stalled(
                    "write_stalled",
                    "write_stalled: frames could not be written. Subscribe again without after; unacknowledged rows are redelivered.",
                ));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state(sub: u64, dir: &std::path::Path) -> Arc<SubState> {
        Arc::new(SubState {
            sub,
            feed: MentionFeed::open(None, 0, |_| true).expect("feed"),
            profile: ProfileId::parse(&format!("prof_{}", "a".repeat(32))).expect("id"),
            data_dir: dir.to_owned(),
            window: Mutex::new(Window {
                outstanding: VecDeque::new(),
                acked: 5,
                persisted: 0,
                sent: 5,
            }),
            flushing: tokio::sync::Mutex::new(()),
            acked: Notify::new(),
            ended: AtomicBool::new(false),
            end_permit: Mutex::new(None),
            flush_failures: AtomicU32::new(0),
        })
    }

    /// The end is queued through the slot kept back for it, so a queue that is
    /// full (a peer that stopped reading) cannot swallow it, and it is queued
    /// once.
    #[tokio::test]
    async fn the_end_is_queued_even_when_the_writer_queue_is_full() {
        let dir = tempfile::tempdir().expect("dir");
        let state = state(7, dir.path());
        let (tx, mut rx) = mpsc::channel(2);
        *state.end_permit.lock().expect("permit") =
            Some(tx.clone().reserve_owned().await.expect("reserved"));
        tx.try_send(ServerMessage::success(1, json!({})))
            .expect("one frame fits");
        assert!(
            tx.try_send(ServerMessage::success(2, json!({}))).is_err(),
            "the queue is full for everything else"
        );
        state.end(Some(stalled("write_stalled", "write_stalled: test")));
        state.end(Some(stalled("write_stalled", "a second end is dropped")));
        assert!(state.ended.load(Ordering::Acquire));
        assert!(matches!(
            rx.recv().await,
            Some(ServerMessage::Response { .. })
        ));
        assert!(matches!(
            rx.recv().await,
            Some(ServerMessage::SubEnd { sub: 7, .. })
        ));
        drop(tx);
        assert!(rx.recv().await.is_none(), "exactly one SubEnd");
    }

    /// A cancelled `finish` (a dropped `unsubscribe`) keeps the subscription and
    /// its lease until the flush has run, so the cursor is durable before the
    /// lease is released, and the next call finishes the job.
    #[tokio::test]
    async fn a_cancelled_finish_keeps_the_lease_until_the_flush_is_done() {
        let dir = tempfile::tempdir().expect("dir");
        let state = state(1, dir.path());
        let leases = Arc::new(Leases::default());
        let profile = state.profile.clone();
        let mut slot = SubSlot {
            current: Some(Active {
                id: 1,
                state: Arc::clone(&state),
                pump: PumpTask(None),
                _guard: leases.acquire(&profile).expect("lease"),
            }),
            ..SubSlot::default()
        };
        let blocked = state.flushing.lock().await;
        let cancelled =
            tokio::time::timeout(Duration::from_millis(50), slot.finish_with(None)).await;
        assert!(cancelled.is_err(), "the flush is still waiting");
        assert!(slot.current.is_some(), "the subscription is kept");
        assert!(leases.acquire(&profile).is_err(), "the lease is still held");
        drop(blocked);
        slot.finish().await;
        assert!(slot.current.is_none());
        assert!(leases.acquire(&profile).is_ok(), "released after the flush");
    }
}
