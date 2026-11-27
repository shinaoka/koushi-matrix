//! Immutable, bounded pagination of the Core-owned Recent projection.
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use koushi_core::{CoreConnection, direct_activity::RecentActivitySnapshot};
use koushi_mcp::wire::{ErrorCode, WireError};
use serde_json::{Value, json};

use crate::consent::GrantSubject;

const TTL: Duration = Duration::from_secs(120);
const MAX_SNAPSHOTS: usize = 16;
const MAX_BYTES: usize = 8 * 1024 * 1024;
// tool_result emits both structured JSON and an escaped text copy. Keep their
// combined JSON-RPC envelope below the transport's 1 MiB frame.
const MAX_PAGE_BYTES: usize = 256 * 1024;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Query {
    pub rooms: Vec<String>,
    pub from: Option<u64>,
    pub to: Option<u64>,
    pub limit: usize,
}
impl Query {
    pub(crate) fn parse(args: &Value) -> Result<Self, WireError> {
        koushi_mcp::tools::validate_arguments("read_recent_activity", args)
            .map_err(|_| invalid("invalid activity query"))?;
        let mut rooms: Vec<String> = match args.get("room_ids") {
            Some(Value::Array(values)) if values.len() <= 64 => values
                .iter()
                .map(|value| {
                    value
                        .as_str()
                        .filter(|room| !room.is_empty() && room.len() <= 1024)
                        .map(str::to_owned)
                        .ok_or_else(|| invalid("invalid room filter"))
                })
                .collect::<Result<_, _>>()?,
            None => vec![],
            _ => return Err(invalid("at most 64 room filters are allowed")),
        };
        rooms.sort();
        rooms.dedup();
        let time = |key| {
            args.get(key)
                .map(|value| value.as_u64().ok_or_else(|| invalid("invalid time range")))
                .transpose()
        };
        let from = time("from_timestamp_ms")?;
        let to = time("to_timestamp_ms")?;
        if from.zip(to).is_some_and(|(from, to)| from >= to) {
            return Err(invalid("time range must be increasing"));
        }
        let limit = match args.get("limit") {
            None => 50,
            Some(value) => value
                .as_u64()
                .filter(|limit| (1..=100).contains(limit))
                .ok_or_else(|| invalid("limit must be between 1 and 100"))?
                as usize,
        };
        Ok(Self {
            rooms,
            from,
            to,
            limit,
        })
    }
}
fn invalid(message: &str) -> WireError {
    WireError::new(ErrorCode::Unsupported, message)
}
fn resync(reason: &str) -> WireError {
    WireError::with_reason(
        ErrorCode::ResyncRequired,
        reason,
        "Activity continuation is unavailable. Start a new read without cursor; bounded observed history cannot recover evicted messages.",
    )
}

struct Page {
    cursor: Option<String>,
    offset: usize,
    end: usize,
}
struct Retained {
    subject: GrantSubject,
    account: String,
    query: Query,
    runtime: CoreConnection,
    session: koushi_state::SessionInfo,
    revision: u64,
    session_epoch: u64,
    room_modes: BTreeMap<String, koushi_state::RoomNotificationMode>,
    created: Instant,
    timestamp: u64,
    rows: Vec<Value>,
    pages: Vec<Page>,
    bytes: usize,
    truncated: bool,
}
#[derive(Default)]
pub(crate) struct ActivityPages {
    snapshots: Mutex<VecDeque<Retained>>,
}
impl ActivityPages {
    #[expect(
        clippy::too_many_arguments,
        reason = "private pagination admission binds the full caller and session context"
    )]
    pub(crate) fn read(
        &self,
        subject: &GrantSubject,
        account: &str,
        query: Query,
        cursor: Option<&str>,
        runtime: CoreConnection,
        current: RecentActivitySnapshot,
        eligible: &BTreeSet<String>,
        settings: &std::collections::HashMap<String, koushi_state::RoomNotificationSettings>,
    ) -> Result<Value, WireError> {
        // These are stable host identities, not arbitrary data-bearing metadata.
        if account.len() > 1024 || !(1..=100).contains(&query.limit) {
            return Err(invalid("invalid activity scope"));
        }
        let mut snapshots = self
            .snapshots
            .lock()
            .map_err(|_| resync("snapshot_unavailable"))?;
        snapshots.retain(|snapshot| snapshot.created.elapsed() < TTL);
        if let Some(cursor) = cursor {
            if cursor.len() != 64 || !cursor.bytes().all(|byte| byte.is_ascii_hexdigit()) {
                return Err(resync("invalid_cursor"));
            }
            let Some(index) = snapshots.iter().position(|snapshot| {
                snapshot
                    .pages
                    .iter()
                    .any(|page| page.cursor.as_deref() == Some(cursor))
            }) else {
                return Err(resync("expired_cursor"));
            };
            let snapshot = &snapshots[index];
            if &snapshot.subject != subject
                || snapshot.account != account
                || snapshot.query != query
            {
                return Err(resync("query_mismatch"));
            }
            if snapshot.session != current.session
                || snapshot.revision != current.session_revision
                || snapshot.session_epoch != current.session_epoch
                || !snapshot.runtime.shares_runtime_with(&runtime)
            {
                snapshots.remove(index);
                return Err(resync("session_changed"));
            }
            if snapshot
                .rows
                .iter()
                .any(|row| !eligible.contains(row["room_id"].as_str().unwrap_or_default()))
            {
                snapshots.remove(index);
                return Err(resync("room_access_changed"));
            }
            if snapshot.room_modes.iter().any(|(room, mode)| {
                *mode
                    != settings
                        .get(room)
                        .map(|setting| setting.mode)
                        .unwrap_or_default()
            }) {
                snapshots.remove(index);
                return Err(resync("filter_changed"));
            }
            let page = snapshot
                .pages
                .iter()
                .find(|page| page.cursor.as_deref() == Some(cursor))
                .unwrap();
            return Ok(render(snapshot, page));
        }
        if query.rooms.iter().any(|room| !eligible.contains(room)) {
            return Err(WireError::new(
                ErrorCode::RoomNotFound,
                "a filtered room is not available",
            ));
        }
        let mut truncated = current.rows.len() >= 200;
        let mut bytes = 0;
        let mut rows = Vec::new();
        let mut sizes = Vec::new();
        for row in current
            .rows
            .into_iter()
            .filter(|row| {
                eligible.contains(&row.room_id)
                    && (query.rooms.is_empty() || query.rooms.contains(&row.room_id))
                    && query.from.is_none_or(|from| row.timestamp_ms >= from)
                    && query.to.is_none_or(|to| row.timestamp_ms < to)
            })
            .take(200)
        {
            let preview_truncated = row
                .preview
                .as_ref()
                .is_some_and(|preview| preview.chars().count() > 4096);
            let preview = row
                .preview
                .map(|preview| preview.chars().take(4096).collect::<String>());
            let metadata_truncated = row.room_label.chars().count() > 256
                || row
                    .sender_label
                    .as_ref()
                    .is_some_and(|label| label.chars().count() > 256);
            let room_label = row.room_label.chars().take(256).collect::<String>();
            let sender_label = row
                .sender_label
                .map(|label| label.chars().take(256).collect::<String>());
            let row = json!({"account_id":account,"room_id":row.room_id,"room_label":room_label,
                "event_id":row.event_id,"thread_root_event_id":row.thread_root_event_id,
                "sender_id":row.sender_id,"sender_label":sender_label,"timestamp_ms":row.timestamp_ms,
                "preview":preview,"content_available":preview.is_some(),"preview_truncated":preview_truncated,
                "metadata_truncated":metadata_truncated,"read_state":"unknown"});
            let size = row.to_string().len();
            truncated |= preview_truncated || metadata_truncated;
            // One oversized identifier-bearing row cannot consume an entire frame.
            if size > MAX_PAGE_BYTES {
                truncated = true;
                continue;
            }
            if bytes + size > MAX_BYTES {
                truncated = true;
                break;
            }
            bytes += size;
            sizes.push(size);
            rows.push(row);
        }
        let room_modes = rows
            .iter()
            .filter_map(|row| row["room_id"].as_str())
            .map(|room| {
                (
                    room.to_owned(),
                    settings
                        .get(room)
                        .map(|setting| setting.mode)
                        .unwrap_or_default(),
                )
            })
            .collect();
        let mut snapshot = Retained {
            subject: subject.clone(),
            account: account.into(),
            query,
            runtime,
            session: current.session,
            revision: current.session_revision,
            session_epoch: current.session_epoch,
            room_modes,
            created: Instant::now(),
            timestamp: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64,
            rows,
            pages: vec![],
            bytes,
            truncated,
        };
        let mut offset = 0;
        loop {
            let mut end = offset;
            let mut page_bytes = 0;
            while end < snapshot.rows.len()
                && end - offset < snapshot.query.limit
                && page_bytes + sizes[end] <= MAX_PAGE_BYTES
            {
                page_bytes += sizes[end];
                end += 1;
            }
            let cursor = if offset == 0 {
                None
            } else {
                let mut random = [0; 32];
                crate::endpoint::fill_random(&mut random)
                    .map_err(|_| resync("snapshot_unavailable"))?;
                Some(random.iter().map(|byte| format!("{byte:02x}")).collect())
            };
            snapshot.pages.push(Page {
                cursor,
                offset,
                end,
            });
            if end == snapshot.rows.len() {
                break;
            }
            offset = end;
        }
        let result = render(&snapshot, &snapshot.pages[0]);
        while snapshots.len() >= MAX_SNAPSHOTS
            || snapshots
                .iter()
                .map(|snapshot| snapshot.bytes)
                .sum::<usize>()
                + snapshot.bytes
                > MAX_BYTES
        {
            if snapshots.pop_front().is_none() {
                break;
            }
        }
        snapshots.push_back(snapshot);
        Ok(result)
    }
}
fn render(snapshot: &Retained, page: &Page) -> Value {
    json!({"account_id":snapshot.account,"items":snapshot.rows[page.offset..page.end],
        "cursor":snapshot.pages.iter().find(|next| next.offset == page.end && next.offset > page.offset).and_then(|next| next.cursor.as_ref()),
        "snapshot_timestamp_ms":snapshot.timestamp,"coverage":"bounded_observed_recent","history_complete":false,
        "retention_limit":200,"truncated":snapshot.truncated,"expires_after_s":120})
}

#[cfg(test)]
#[path = "recent_activity_tests.rs"]
mod tests;
