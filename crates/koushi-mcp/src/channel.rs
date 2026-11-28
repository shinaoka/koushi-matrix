//! Claude Code channel adapter (#2126, stage 4).
//!
//! A launcher started with an agent profile **and** the explicit
//! `--claude-channel` opt-in declares `experimental["claude/channel"]`, opens
//! its own IPC connection, subscribes with the profile, and turns every wake
//! row into one `notifications/claude/channel` notification. Replies are not
//! a channel feature: the agent uses the ordinary `send_message` and
//! `send_thread_reply` tools with a fresh `client_request_id`. There is no
//! permission relay.
//!
//! # Delivery guarantee: at-most-once past the notification API
//!
//! Claude Code gives no delivery receipt. `notification()` resolves when the
//! line is written to the transport, and a session that did not opt in with
//! `--channels` drops channel events silently. The cursor is therefore
//! acknowledged at the strongest hand-off this harness lets the adapter
//! observe: **the notification line was written and flushed to stdout**. It is
//! never acknowledged earlier. A notification Claude Code then drops is lost
//! to the channel and Koushi cannot know; the safety net is that nothing is
//! deleted on acknowledgement, so the agent can reconcile with
//! `wait_for_mentions` (the instructions say so). Core's own guarantee ends at
//! this plugin.
//!
//! # What the launcher can detect
//!
//! Nothing about `--channels`: the MCP `initialize` request does not say
//! whether the session loaded the server as a channel. The only per-session
//! signal is how the launcher was started, so the subscription requires both
//! a profile and `--claude-channel` (or `KOUSHI_CLAUDE_CHANNEL=1`), and a
//! launcher without them behaves exactly as an unprofiled one.

use std::collections::{HashSet, VecDeque};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use serde_json::{Value, json};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use crate::server::{Backend, Outbound};
use crate::wire::{ClientInfo, ErrorCode, ProfileHello, ServerMessage, WireError};

/// The model-facing text of the `initialize` result.
pub const INSTRUCTIONS: &str = "\
Koushi wake events arrive as <channel source=\"koushi\" ...> tags. Each one is a Matrix \
message that wants your attention: `content` is its text, `room_id`, `event_id`, \
`thread_root` (when it is in a thread), `sender` and `kind` identify it. Answer with the \
send_message tool (or send_thread_reply for a thread), always with a fresh client_request_id, \
and only when a reply is warranted; an event of kind invite is information only. Events with \
kind profile_busy, subscription_ended or wake_budget are status notes about this \
connection, not messages. Channel delivery is at-most-once: an event Claude Code drops is \
not redelivered, so after a restart or a long idle period call wait_for_mentions to \
reconcile with what you may have missed.";

/// How long a message text lookup may take before the notification falls
/// back to a pointer. Keeps one slow room from stalling the stream.
const CONTENT_TIMEOUT: Duration = Duration::from_secs(5);
/// Longest text forwarded in one notification.
const MAX_CONTENT_BYTES: usize = 16 * 1024;
/// Rows remembered so a redelivery is not shown twice.
const SEEN_CAPACITY: usize = 1024;
const RETRY_MIN: Duration = Duration::from_secs(1);
const RETRY_MAX: Duration = Duration::from_secs(30);

/// Something the stdio loop starts once the client has initialized.
pub trait ChannelFeed: Send {
    /// Starts the feed; it writes through `outbound` and ends with the task.
    fn start(&mut self, outbound: mpsc::Sender<Outbound>) -> JoinHandle<()>;
}

/// The production feed: one profile subscription over its own connection.
pub struct ChannelRunner {
    pub endpoint_path: PathBuf,
    pub client_info: ClientInfo,
    pub profile: ProfileHello,
    /// Request path, used to read the text of a woken message.
    pub backend: Arc<dyn Backend>,
}

impl ChannelFeed for ChannelRunner {
    fn start(&mut self, outbound: mpsc::Sender<Outbound>) -> JoinHandle<()> {
        let runner = Self {
            endpoint_path: self.endpoint_path.clone(),
            client_info: self.client_info.clone(),
            profile: self.profile.clone(),
            backend: Arc::clone(&self.backend),
        };
        tokio::spawn(async move { runner.run(outbound).await })
    }
}

/// How one subscription attempt ended.
enum Ended {
    /// stdout is gone; nothing more can be shown.
    OutputClosed,
    /// The session was told; do not subscribe again.
    Stop,
    /// Subscribe again, from where `after` says.
    Resubscribe { after: Option<&'static str> },
}

struct Seen {
    order: VecDeque<String>,
    set: HashSet<String>,
}

impl Seen {
    fn new() -> Self {
        Self {
            order: VecDeque::new(),
            set: HashSet::new(),
        }
    }

    /// Whether `cursor` is new; remembers it.
    fn insert(&mut self, cursor: &str) -> bool {
        if !self.set.insert(cursor.to_owned()) {
            return false;
        }
        self.order.push_back(cursor.to_owned());
        if self.order.len() > SEEN_CAPACITY
            && let Some(old) = self.order.pop_front()
        {
            self.set.remove(&old);
        }
        true
    }
}

impl ChannelRunner {
    async fn run(self, outbound: mpsc::Sender<Outbound>) {
        let mut seen = Seen::new();
        let mut after: Option<&'static str> = None;
        let mut delay = RETRY_MIN;
        let mut announced_unavailable = false;
        loop {
            let connected = crate::ipc::connect_with_profile(
                &self.endpoint_path,
                self.client_info.clone(),
                Some(self.profile.clone()),
            )
            .await;
            let mut client = match connected {
                Ok(client) => client,
                Err(error) => {
                    if !announced_unavailable {
                        announced_unavailable = true;
                        eprintln!(
                            "koushi-mcp: channel cannot reach Koushi: {}",
                            error.code.as_str()
                        );
                        if !status(
                            &outbound,
                            "subscription_ended",
                            "Koushi is not reachable, so no wake events arrive. The channel retries \
                             on its own; request tools report koushi_unavailable until Koushi returns.",
                        )
                        .await
                        {
                            return;
                        }
                    }
                    tokio::time::sleep(delay).await;
                    delay = (delay * 2).min(RETRY_MAX);
                    continue;
                }
            };
            announced_unavailable = false;
            delay = RETRY_MIN;
            match self
                .subscribed(&mut client, after, &outbound, &mut seen)
                .await
            {
                Ended::OutputClosed | Ended::Stop => return,
                Ended::Resubscribe { after: next } => {
                    after = next;
                    tokio::time::sleep(RETRY_MIN).await;
                }
            }
        }
    }

    /// Subscribes on `client` and pumps until the subscription or the
    /// connection ends.
    async fn subscribed(
        &self,
        client: &mut crate::ipc::Client,
        after: Option<&str>,
        outbound: &mpsc::Sender<Outbound>,
        seen: &mut Seen,
    ) -> Ended {
        let params = after.map_or_else(|| json!({}), |after| json!({ "after": after }));
        let sub = match client.call("subscribe", params).await {
            Ok(value) => match value.get("sub").and_then(Value::as_u64) {
                Some(sub) => sub,
                None => return Ended::Resubscribe { after: None },
            },
            Err(error) if error.code == ErrorCode::ProfileBusy => {
                eprintln!("koushi-mcp: profile_busy: another session holds this profile");
                status(outbound, "profile_busy", PROFILE_BUSY_TEXT).await;
                return Ended::Stop;
            }
            Err(error) => return self.after_subscribe_error(&error, outbound).await,
        };

        loop {
            let frame = match client.next_frame().await {
                Ok(frame) => frame,
                // The connection dropped (Koushi restarted); reconnect and
                // resume after the acknowledged cursor.
                Err(_) => return Ended::Resubscribe { after: None },
            };
            match frame {
                ServerMessage::Push { sub: got, rows, .. } if got == sub => {
                    for row in rows {
                        let Some(cursor) = row.get("cursor").and_then(Value::as_str) else {
                            continue;
                        };
                        let cursor = cursor.to_owned();
                        if seen.insert(&cursor) {
                            let notification = self.notification(&row).await;
                            // Acknowledge only after the line is written.
                            if !deliver(outbound, notification).await {
                                return Ended::OutputClosed;
                            }
                        }
                        // A redelivered row was shown already; acknowledging
                        // it again moves the cursor past it.
                        if let Err(error) = client
                            .call("ack", json!({ "sub": sub, "cursor": cursor }))
                            .await
                        {
                            eprintln!("koushi-mcp: ack failed: {}", error.code.as_str());
                            if error.code == ErrorCode::KoushiUnavailable {
                                return Ended::Resubscribe { after: None };
                            }
                        }
                    }
                }
                ServerMessage::SubEnd { sub: got, error } if got == sub => {
                    return self.ended(&error, outbound).await;
                }
                ServerMessage::WakeBudgetExceeded {
                    window,
                    limit,
                    retry_after_ms,
                    ..
                } => {
                    let text = format!(
                        "Koushi is holding wake events for this profile: the {window} budget of \
                         {limit} was reached. They are delivered in order after about {} s.",
                        retry_after_ms.div_ceil(1000)
                    );
                    if !status(outbound, "wake_budget", &text).await {
                        return Ended::OutputClosed;
                    }
                }
                _ => {}
            }
        }
    }

    async fn after_subscribe_error(
        &self,
        error: &WireError,
        outbound: &mpsc::Sender<Outbound>,
    ) -> Ended {
        match error.code {
            ErrorCode::KoushiUnavailable => Ended::Resubscribe { after: None },
            ErrorCode::ResyncRequired => Ended::Resubscribe {
                after: Some("retained"),
            },
            _ => self.ended(error, outbound).await,
        }
    }

    /// Maps a `SubEnd` to what the session is told and what happens next.
    async fn ended(&self, error: &WireError, outbound: &mpsc::Sender<Outbound>) -> Ended {
        let reason = error.reason.as_deref().unwrap_or("");
        let (retry, text): (Option<Option<&'static str>>, String) = match (error.code, reason) {
            (ErrorCode::ResyncRequired, "expired_cursor" | "generation_changed") => (
                Some(Some("retained")),
                "Wake events may have been missed (the feed lost its position). Koushi \
                 resubscribes from the oldest retained row; call wait_for_mentions to reconcile."
                    .to_owned(),
            ),
            (ErrorCode::ResyncRequired, _) => (Some(None), String::new()),
            (ErrorCode::Unauthorized, _) => (
                None,
                "This agent profile was revoked or its secret changed, so no more wake events \
                 arrive. Create or restore the profile in Koushi's settings and start the \
                 session again."
                    .to_owned(),
            ),
            (ErrorCode::NotPermitted, _) => (
                None,
                "This agent profile may no longer read its account, so no more wake events \
                 arrive. Check its permissions in Koushi's settings."
                    .to_owned(),
            ),
            (ErrorCode::AccountUnavailable, _) => (
                Some(None),
                "The agent account is signed out, locked, or unavailable, so wake events pause. \
                 The channel retries on its own."
                    .to_owned(),
            ),
            _ => (
                None,
                format!("The wake subscription ended: {}.", error.code.as_str()),
            ),
        };
        eprintln!(
            "koushi-mcp: subscription ended: {} {}",
            error.code.as_str(),
            reason
        );
        if !text.is_empty() && !status(outbound, "subscription_ended", &text).await {
            return Ended::OutputClosed;
        }
        match retry {
            Some(after) => Ended::Resubscribe { after },
            None => Ended::Stop,
        }
    }

    /// The channel notification for one wake row.
    async fn notification(&self, row: &Value) -> Value {
        let text = self.message_text(row).await;
        notification_for_row(row, text)
    }

    /// The message text, read with the profile's own `read` grant. `None` when
    /// it cannot be read (an invite, an undecryptable event, a timeout).
    async fn message_text(&self, row: &Value) -> Option<String> {
        let account_id = row.get("account_id").and_then(Value::as_str)?;
        let room_id = row.get("room_id").and_then(Value::as_str)?;
        let event_id = row.get("event_id").and_then(Value::as_str)?;
        let call = self.backend.call(
            "tools/call",
            json!({
                "name": "get_message",
                "arguments": {
                    "account_id": account_id,
                    "room_id": room_id,
                    "event_id": event_id,
                }
            }),
        );
        let result = tokio::time::timeout(CONTENT_TIMEOUT, call)
            .await
            .ok()?
            .ok()?;
        if result.get("isError").and_then(Value::as_bool) == Some(true) {
            return None;
        }
        let body = result
            .get("structuredContent")
            .and_then(|content| content.get("body"))
            .and_then(Value::as_str)?;
        Some(body.to_owned())
    }
}

const PROFILE_BUSY_TEXT: &str = "Another session already holds this agent profile's wake \
subscription, so this session receives no wake events. Its request tools keep working. End \
the other session, or start this one with a different profile.";

/// Builds `notifications/claude/channel` for a wake row. `text` is the message
/// text when it could be read. Only identifiers and text travel: no bytes, no
/// filenames, and `meta` keys are letters, digits and underscores.
pub fn notification_for_row(row: &Value, text: Option<String>) -> Value {
    let field = |name: &str| row.get(name).and_then(Value::as_str);
    let kind = field("kind").unwrap_or("mention");
    let mut meta = serde_json::Map::new();
    for (key, source) in [
        ("room_id", "room_id"),
        ("event_id", "event_id"),
        ("thread_root", "thread_root_event_id"),
        ("sender", "sender_id"),
    ] {
        if let Some(value) = field(source) {
            meta.insert(key.to_owned(), json!(value));
        }
    }
    meta.insert("kind".to_owned(), json!(kind));
    let content = match text {
        Some(text) => truncate(text),
        None => format!(
            "A {kind} woke this profile in {}. The text is not available here; read it with \
             get_message or read_room_messages.",
            field("room_id").unwrap_or("a room"),
        ),
    };
    json!({
        "jsonrpc": "2.0",
        "method": "notifications/claude/channel",
        "params": { "content": content, "meta": meta }
    })
}

fn truncate(mut text: String) -> String {
    if text.len() > MAX_CONTENT_BYTES {
        let mut end = MAX_CONTENT_BYTES;
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        text.truncate(end);
        text.push_str("\n[truncated; read the full message with get_message]");
    }
    text
}

/// A status note for the session, as a channel notification of a status kind.
fn status_notification(kind: &str, text: &str) -> Value {
    json!({
        "jsonrpc": "2.0",
        "method": "notifications/claude/channel",
        "params": { "content": text, "meta": { "kind": kind } }
    })
}

async fn status(outbound: &mpsc::Sender<Outbound>, kind: &str, text: &str) -> bool {
    deliver(outbound, status_notification(kind, text)).await
}

/// Hands one notification to the stdio loop and waits until it was written.
/// `false` when stdout is gone.
async fn deliver(outbound: &mpsc::Sender<Outbound>, message: Value) -> bool {
    let (written, done) = tokio::sync::oneshot::channel();
    if outbound
        .send(Outbound {
            message,
            written: Some(written),
        })
        .await
        .is_err()
    {
        return false;
    }
    done.await.unwrap_or(false)
}

#[cfg(test)]
#[path = "channel_tests.rs"]
mod tests;
