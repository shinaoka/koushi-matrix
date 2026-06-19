use std::{
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
    process::ExitCode,
    time::{SystemTime, UNIX_EPOCH},
};

use koushi_sdk::{
    MatrixClientSession, MatrixClientStoreConfig, MatrixClientStoreKey, MatrixRoomListSnapshot,
    MatrixSearchIndexKey, MatrixSearchIndexStoreConfig, MatrixTimelineItem,
};
use koushi_state::{AuthSecret, LoginRequest};
use tokio::runtime::Runtime;

const DEFAULT_HOMESERVER: &str = "https://matrix.org";
const DEFAULT_DEVICE_DISPLAY_NAME: &str = "Matrix Desktop Smoke Test";
const TIMELINE_ROOM_SAMPLE_LIMIT: usize = 20;
const TIMELINE_BACKFILL_EVENT_COUNT: u16 = 30;
const SEARCH_SMOKE_QUERY: &str = "matrixdesktop-smoke-nonmatching-query";
const SEARCH_SMOKE_LIMIT: usize = 5;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct SmokeOptions {
    keep_session: bool,
    check_room_list: bool,
    real_account_qa: bool,
}

fn main() -> ExitCode {
    let arguments = std::env::args().skip(1).collect::<Vec<_>>();
    if arguments
        .iter()
        .any(|argument| matches!(argument.as_str(), "-h" | "--help"))
    {
        print_usage();
        return ExitCode::SUCCESS;
    }
    let keep_session = arguments
        .iter()
        .any(|argument| argument == "--keep-session");
    let check_room_list = arguments
        .iter()
        .any(|argument| argument == "--check-room-list");
    let real_account_qa = arguments
        .iter()
        .any(|argument| argument == "--real-account-qa");

    match run(SmokeOptions {
        keep_session,
        check_room_list,
        real_account_qa,
    }) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("Smoke login failed: {error}");
            ExitCode::FAILURE
        }
    }
}

fn print_usage() {
    println!(
        "Usage: password-login-smoke [--keep-session] [--check-room-list] [--real-account-qa]"
    );
    println!();
    println!("Prompts interactively for homeserver, username, and password.");
    println!("By default, logs out immediately after a successful smoke login.");
    println!("Use --check-room-list to run one sync and print private-data-free counts.");
    println!(
        "Use --real-account-qa to verify room list and timeline with private-data-free counts."
    );
}

fn run(options: SmokeOptions) -> Result<(), String> {
    println!("Matrix password login smoke test");
    println!("Do not paste secrets into command-line arguments or shell history.");

    let homeserver = prompt_with_default("Homeserver", DEFAULT_HOMESERVER)?;
    let username = prompt_required("Username")?;
    let device_display_name =
        prompt_with_default("Device display name", DEFAULT_DEVICE_DISPLAY_NAME)?;
    let password = rpassword::prompt_password("Password: ").map_err(|error| error.to_string())?;

    let request = LoginRequest {
        homeserver,
        username,
        password: AuthSecret::new(password),
        device_display_name: Some(device_display_name),
    };
    let runtime = Runtime::new().map_err(|_| "password login runtime failed".to_owned())?;
    let mut smoke_store = None;
    let mut session = Some(
        runtime
            .block_on(koushi_sdk::login_with_password(&request))
            .map_err(|_| "login request failed".to_owned())?,
    );

    println!("Login OK. SDK session established.");

    let smoke_result = run_authenticated_smoke(
        &runtime,
        session
            .as_mut()
            .expect("session should exist until explicit runtime-entered drop"),
        options,
        &mut smoke_store,
    );
    let result = finish_smoke_with_logout(options, smoke_result, || {
        let session = session
            .as_ref()
            .expect("session should exist until explicit runtime-entered drop");
        runtime
            .block_on(koushi_sdk::logout(session))
            .map_err(|_| "logout request failed after successful login".to_owned())
    });
    let runtime_guard = runtime.enter();
    drop(session.take());
    drop(smoke_store.take());
    drop(runtime_guard);
    result
}

fn run_authenticated_smoke(
    runtime: &Runtime,
    session: &mut MatrixClientSession,
    options: SmokeOptions,
    smoke_store: &mut Option<SmokeStore>,
) -> Result<(), String> {
    let mut session_restored = false;
    if options.real_account_qa {
        let persisted = session
            .persistable_session()
            .map_err(|_| "persistable session export failed".to_owned())?;
        let persisted_json = persisted
            .to_json()
            .map_err(|_| "persistable session serialization failed".to_owned())?;
        let persisted = koushi_sdk::PersistableMatrixSession::from_json(&persisted_json)
            .map_err(|_| "persistable session deserialization failed".to_owned())?;
        let store = SmokeStore::new()?;
        *session = runtime
            .block_on(koushi_sdk::restore_session_with_store(
                &persisted,
                Some(store.config()),
            ))
            .map_err(|_| "persisted session restore failed".to_owned())?;
        *smoke_store = Some(store);
        session_restored = true;
    }

    if options.check_room_list || options.real_account_qa {
        runtime
            .block_on(koushi_sdk::sync_once(&session))
            .map_err(|_| "sync request failed".to_owned())?;
        let snapshot = runtime
            .block_on(koushi_sdk::room_list_snapshot(&session))
            .map_err(|_| "room list request failed".to_owned())?;
        if options.check_room_list {
            println!(
                "Room list OK. {}",
                koushi_sdk::room_list_smoke_report(&snapshot)
            );
        }
        if options.real_account_qa {
            let timeline_items = first_visible_timeline_items(runtime, &session, &snapshot)?;
            let search_candidates = runtime
                .block_on(koushi_sdk::search_message_candidates(
                    &session,
                    SEARCH_SMOKE_QUERY,
                    SEARCH_SMOKE_LIMIT,
                ))
                .map_err(|_| "search request failed".to_owned())?;
            let report = koushi_sdk::real_account_qa_report_with_search(
                &snapshot,
                true,
                &timeline_items,
                session_restored,
                &search_candidates,
            );
            println!("Real account QA OK. {}", report);
        }
    }

    Ok(())
}

fn finish_smoke_with_logout<F>(
    options: SmokeOptions,
    smoke_result: Result<(), String>,
    logout: F,
) -> Result<(), String>
where
    F: FnOnce() -> Result<(), String>,
{
    if options.keep_session {
        if smoke_result.is_ok() {
            println!("Session kept on homeserver for manual follow-up.");
        }
        return smoke_result;
    }

    let logout_result = logout();
    if logout_result.is_ok() {
        println!("Logout OK. Smoke session discarded.");
    }
    smoke_result.and(logout_result)
}

struct SmokeStore {
    root: PathBuf,
    config: MatrixClientStoreConfig,
}

impl SmokeStore {
    fn new() -> Result<Self, String> {
        let root = std::env::temp_dir().join(format!(
            "matrix-desktop-smoke-store-{}-{}",
            std::process::id(),
            timestamp_millis()?
        ));
        fs::create_dir_all(&root).map_err(|_| "temporary smoke store could not be created")?;
        let store_key = smoke_store_key()?;
        let search_key = hex_key(&smoke_store_key()?);
        let config = smoke_store_config_under(&root, store_key, &search_key);
        Ok(Self { root, config })
    }

    fn config(&self) -> &MatrixClientStoreConfig {
        &self.config
    }
}

impl Drop for SmokeStore {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn smoke_store_config_under(
    root: &Path,
    store_key: [u8; 32],
    search_key: &str,
) -> MatrixClientStoreConfig {
    MatrixClientStoreConfig::new(root.join("sdk-store"), MatrixClientStoreKey::new(store_key))
        .with_cache_path(root.join("sdk-cache"))
        .with_search_index_store(MatrixSearchIndexStoreConfig::new(
            root.join("search-index"),
            MatrixSearchIndexKey::new(search_key.to_owned()),
        ))
}

fn smoke_store_key() -> Result<[u8; 32], String> {
    use std::io::Read;

    let mut key = [0_u8; 32];
    match fs::File::open("/dev/urandom").and_then(|mut file| file.read_exact(&mut key)) {
        Ok(()) => Ok(key),
        Err(_) => {
            let seed = timestamp_millis()?;
            for (index, byte) in key.iter_mut().enumerate() {
                *byte = ((seed.rotate_left((index % 63) as u32) >> (index % 8)) & 0xff) as u8;
            }
            Ok(key)
        }
    }
}

fn hex_key(key: &[u8; 32]) -> String {
    key.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn timestamp_millis() -> Result<u128, String> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .map_err(|_| "system clock is before UNIX_EPOCH".to_owned())
}

fn first_visible_timeline_items(
    runtime: &Runtime,
    session: &MatrixClientSession,
    snapshot: &MatrixRoomListSnapshot,
) -> Result<Vec<MatrixTimelineItem>, String> {
    if snapshot.rooms.is_empty() {
        return Err("room list request returned no rooms".to_owned());
    }

    let mut subscribed_room_count = 0;
    for room in snapshot.rooms.iter().take(TIMELINE_ROOM_SAMPLE_LIMIT) {
        let Ok(items) = runtime.block_on(koushi_sdk::room_timeline_visible_items(
            session,
            &room.room_id,
            TIMELINE_BACKFILL_EVENT_COUNT,
        )) else {
            continue;
        };
        subscribed_room_count += 1;
        if !items.is_empty() {
            return Ok(items);
        }
    }

    if subscribed_room_count == 0 {
        return Err("timeline request failed for sampled rooms".to_owned());
    }

    Err("timeline request returned no visible items".to_owned())
}

fn prompt_required(label: &str) -> Result<String, String> {
    loop {
        let value = prompt(label)?;
        if !value.is_empty() {
            return Ok(value);
        }
        println!("{label} is required.");
    }
}

fn prompt_with_default(label: &str, default: &str) -> Result<String, String> {
    let value = prompt(&format!("{label} [{default}]"))?;
    if value.is_empty() {
        Ok(default.to_owned())
    } else {
        Ok(value)
    }
}

fn prompt(label: &str) -> Result<String, String> {
    print!("{label}: ");
    io::stdout().flush().map_err(|error| error.to_string())?;

    let mut value = String::new();
    let bytes_read = io::stdin()
        .read_line(&mut value)
        .map_err(|error| error.to_string())?;
    if bytes_read == 0 {
        return Err(format!("{label} input ended before a value was provided"));
    }

    Ok(value.trim().to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cleanup_runs_after_smoke_failure_when_session_is_not_kept() {
        let options = SmokeOptions {
            keep_session: false,
            check_room_list: false,
            real_account_qa: true,
        };
        let mut logout_called = false;

        let result =
            finish_smoke_with_logout(options, Err("sync request failed".to_owned()), || {
                logout_called = true;
                Ok(())
            });

        assert_eq!(result, Err("sync request failed".to_owned()));
        assert!(logout_called);
    }

    #[test]
    fn real_account_qa_store_config_separates_sdk_cache_and_search_paths() {
        let root = tempfile::tempdir().expect("temp root should be created");
        let config = smoke_store_config_under(root.path(), [7; 32], "synthetic-search-key");

        assert_eq!(config.path(), &root.path().join("sdk-store"));
        assert_eq!(
            config.cache_path(),
            Some(root.path().join("sdk-cache").as_path())
        );
    }

    #[test]
    fn store_backed_restore_does_not_escape_its_runtime() {
        let source = include_str!("password-login-smoke.rs");
        let anti_pattern = ["fn ", "restore_session_with_store_blocking", "("].concat();

        assert!(
            !source.contains(&anti_pattern),
            "store-backed SDK sessions must not be returned from a helper that drops its Tokio runtime"
        );
    }

    #[test]
    fn store_backed_session_drop_enters_runtime_context() {
        let source = include_str!("password-login-smoke.rs");
        let enter_runtime = ["runtime", ".enter()"].concat();
        let take_session = ["session", ".take()"].concat();

        assert!(source.contains(&enter_runtime));
        assert!(source.contains(&take_session));
    }
}
