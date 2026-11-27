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
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use koushi_core::mention_feed::{CursorRejection, MentionFeed};
use serde_json::{Value, json};
use tokio::sync::{Notify, mpsc};

use crate::consent::{ClientIdentity, ConsentContext};
use crate::grants::Need;
use crate::profile_limits::Limits;
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
}

impl Leases {
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
    feed: MentionFeed,
    profile: ProfileId,
    data_dir: PathBuf,
    window: Mutex<Window>,
    /// Held across a write, so two flushes never interleave.
    flushing: tokio::sync::Mutex<()>,
    /// A permit is stored if an ack lands before the pump waits.
    acked: Notify,
    ended: AtomicBool,
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
        let saved = tokio::task::spawn_blocking(move || {
            let feed = state.feed.clone();
            profile_state::save_acked(&state.data_dir, &state.profile, &cursor, |text| {
                feed.seq_of(text).ok()
            })
        })
        .await
        .unwrap_or(false);
        if saved {
            let mut window = self.window();
            window.persisted = window.persisted.max(target);
        }
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
    guard: LeaseGuard,
}

/// The connection's subscription, at most one.
#[derive(Default)]
pub(crate) struct SubSlot {
    current: Option<Active>,
    counter: u64,
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
    /// Starts the pump of a subscription whose response has been queued.
    pub(crate) fn start(&mut self, start: PumpStart, tx: mpsc::Sender<ServerMessage>) {
        let Some(active) = self.current.as_mut() else {
            return;
        };
        if active.id == start.ctx.sub {
            active.pump = PumpTask(Some(tokio::spawn(pump(start.ctx, tx))));
        }
    }

    /// Ends the subscription: stops the pump, makes the acknowledged cursor
    /// durable, then releases the lease, in that order, so the next holder
    /// reads what this one acknowledged.
    pub(crate) async fn finish(&mut self) {
        if let Some(mut active) = self.current.take() {
            active.pump.stop().await;
            active.state.flush().await;
            drop(active.guard);
        }
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
    });
    let lease = guard.0.clone();
    slot.current = Some(Active {
        id: sub,
        state: Arc::clone(&state),
        pump: PumpTask(None),
        guard,
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
/// re-checks after its wait.
fn check(ctx: &PumpCtx) -> Result<(), WireError> {
    if !ctx.state.feed.is_healthy() {
        return Err(unhealthy());
    }
    let current = handlers::account_connection(&ctx.shared.accounts, &ctx.account_id)?;
    if !ctx.connection.shares_runtime_with(&current)
        || ctx.shared.accounts.mention_feed(&ctx.account_id).is_none()
    {
        return Err(WireError::new(
            ErrorCode::AccountUnavailable,
            "account changed during the subscription",
        ));
    }
    if !current.snapshot().is_agent_account() {
        return Err(WireError::with_reason(
            ErrorCode::AccountUnavailable,
            "not_agent_account",
            "the account is no longer an agent account",
        ));
    }
    match profiles::lookup_checked(ctx.shared.broker.data_dir(), &ctx.profile) {
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
    let exit = tokio::select! {
        biased;
        exit = stream(&ctx, &tx, limits) => exit,
        never = flush_ticker(&ctx.state, limits.ack_flush) => match never {},
    };
    // Ending: the position is made durable, then the lease is freed, and only
    // then is the peer told, so a consumer that resubscribes at once finds
    // both in place.
    ctx.state.ended.store(true, Ordering::Release);
    ctx.state.flush().await;
    ctx.lease.release();
    if let Exit::Ended(error) = exit {
        let end = ServerMessage::SubEnd {
            sub: ctx.sub,
            error,
        };
        let _ = tokio::time::timeout(limits.write_stall, tx.send(end)).await;
    }
}

/// Writes a changed acknowledged cursor at most once per `every`.
async fn flush_ticker(state: &Arc<SubState>, every: Duration) -> std::convert::Infallible {
    let mut ticker = tokio::time::interval(every);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        ticker.tick().await;
        state.flush().await;
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
        if let Err(error) = check(ctx) {
            return Exit::Ended(error);
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
        if page.entries.is_empty() {
            continue;
        }
        // The wait can last: re-check before anything is sent.
        if let Err(error) = check(ctx) {
            return Exit::Ended(error);
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
