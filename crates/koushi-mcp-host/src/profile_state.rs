//! Per-profile host state: `mcp/profile-state/<profile_id>.json` (#2132, D7).
//!
//! It holds what the host learns *about* a profile, never what authorises it:
//! the creation baseline (the feed position at creation, so the first
//! subscription neither replays history nor misses rows that arrive before the
//! plugin first starts) and `last_rejected_ms`. Authentication never reads this
//! file, and nothing in it is in `profiles.json`: that file is owner-written and
//! low-frequency, and a write here can never resurrect a revoked profile. A
//! state file without a profile row is ignored and swept at start.
//!
//! PR 1b writes the baseline and the rejection time. The acknowledged cursor is
//! added by the subscription stage; unknown keys are kept so that stage's file
//! survives a downgrade.

use std::path::{Path, PathBuf};

use serde_json::{Map, Value, json};

use crate::subject::ProfileId;

const DIR: &str = "profile-state";
const SCHEMA: u64 = 1;

/// Where the profile's feed started, fixed at creation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Baseline {
    /// The mention log's generation, so a recreated log is recognised.
    pub feed_generation: String,
    /// The feed's head cursor when the profile was created.
    pub head_cursor: String,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ProfileState {
    pub baseline: Option<Baseline>,
    /// When a connection last presented this profile with the wrong secret.
    pub last_rejected_ms: Option<u64>,
    pub(crate) extra: Map<String, Value>,
}

pub(crate) fn dir(data_dir: &Path) -> PathBuf {
    data_dir.join("mcp").join(DIR)
}

pub(crate) fn path(data_dir: &Path, id: &ProfileId) -> PathBuf {
    dir(data_dir).join(format!("{}.json", id.as_str()))
}

/// Reads the state. A missing or unreadable file is an empty state: nothing
/// here grants anything, so there is nothing to fail closed on.
pub(crate) fn load(data_dir: &Path, id: &ProfileId) -> ProfileState {
    let Ok(bytes) = std::fs::read(path(data_dir, id)) else {
        return ProfileState::default();
    };
    let Ok(Value::Object(mut root)) = serde_json::from_slice::<Value>(&bytes) else {
        return ProfileState::default();
    };
    root.remove("schema");
    let baseline = match (root.remove("feed_generation"), root.remove("head_cursor")) {
        (Some(Value::String(feed_generation)), Some(Value::String(head_cursor))) => {
            Some(Baseline {
                feed_generation,
                head_cursor,
            })
        }
        _ => None,
    };
    let last_rejected_ms = root.remove("last_rejected_ms").and_then(|v| v.as_u64());
    ProfileState {
        baseline,
        last_rejected_ms,
        extra: root,
    }
}

/// Writes the state with a private temporary file and an atomic rename.
pub(crate) fn save(data_dir: &Path, id: &ProfileId, state: &ProfileState) -> std::io::Result<()> {
    use std::io::Write;
    let directory = dir(data_dir);
    crate::endpoint::ensure_private_dir(&directory)?;
    let mut root = state.extra.clone();
    root.insert("schema".to_owned(), json!(SCHEMA));
    if let Some(baseline) = &state.baseline {
        root.insert(
            "feed_generation".to_owned(),
            json!(baseline.feed_generation),
        );
        root.insert("head_cursor".to_owned(), json!(baseline.head_cursor));
    }
    if let Some(at) = state.last_rejected_ms {
        root.insert("last_rejected_ms".to_owned(), json!(at));
    }
    let bytes = serde_json::to_vec_pretty(&Value::Object(root))
        .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?;
    // `tempfile` creates the file 0600 on Unix.
    let mut temporary = tempfile::NamedTempFile::new_in(&directory)?;
    temporary.write_all(&bytes)?;
    temporary.write_all(b"\n")?;
    temporary.as_file().sync_all()?;
    temporary
        .persist(path(data_dir, id))
        .map_err(|error| error.error)?;
    Ok(())
}

/// Records that a connection presented this profile with a wrong secret.
/// Coalesced to one write per second so a guessing loop cannot become a
/// write loop.
pub(crate) fn note_rejected(data_dir: &Path, id: &ProfileId, now_ms: u64) {
    let mut state = load(data_dir, id);
    if state
        .last_rejected_ms
        .is_some_and(|last| now_ms.saturating_sub(last) < 1_000)
    {
        return;
    }
    state.last_rejected_ms = Some(now_ms);
    let _ = save(data_dir, id, &state);
}

pub(crate) fn remove(data_dir: &Path, id: &ProfileId) {
    let _ = std::fs::remove_file(path(data_dir, id));
}
