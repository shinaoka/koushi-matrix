use std::{
    env,
    process::ExitCode,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use matrix_desktop_state::{AuthSecret, LoginRequest};

const ENV_HOMESERVER: &str = "MATRIX_DESKTOP_LOCAL_QA_HOMESERVER";
const ENV_SERVER_NAME: &str = "MATRIX_DESKTOP_LOCAL_QA_SERVER_NAME";
const ENV_SERVER_KIND: &str = "MATRIX_DESKTOP_LOCAL_QA_SERVER_KIND";
const ENV_USER_A: &str = "MATRIX_DESKTOP_LOCAL_QA_USER_A";
const ENV_PASSWORD_A: &str = "MATRIX_DESKTOP_LOCAL_QA_PASSWORD_A";
const ENV_USER_B: &str = "MATRIX_DESKTOP_LOCAL_QA_USER_B";
const ENV_PASSWORD_B: &str = "MATRIX_DESKTOP_LOCAL_QA_PASSWORD_B";
const DEVICE_A: &str = "Matrix Desktop Headless QA A";
const DEVICE_B: &str = "Matrix Desktop Headless QA B";
const POLL_ATTEMPTS: usize = 30;
const POLL_INTERVAL: Duration = Duration::from_millis(500);
const TIMELINE_BACKFILL_EVENT_COUNT: u16 = 50;

fn main() -> ExitCode {
    match run_on_runtime() {
        Ok(report) => {
            println!("{report}");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("Headless local QA failed: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run_on_runtime() -> Result<HeadlessQaReport, String> {
    let config = HeadlessQaConfig::from_env()?;
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|error| format!("runtime creation failed: {error}"))?;
    runtime.block_on(run(config))
}

#[derive(Clone, Debug)]
struct HeadlessQaConfig {
    homeserver: String,
    server_name: String,
    server_kind: String,
    user_a: String,
    password_a: String,
    user_b: String,
    password_b: String,
}

impl HeadlessQaConfig {
    fn from_env() -> Result<Self, String> {
        Ok(Self {
            homeserver: env_required(ENV_HOMESERVER)?,
            server_name: env_required(ENV_SERVER_NAME)?,
            server_kind: env::var(ENV_SERVER_KIND).unwrap_or_else(|_| "local".to_owned()),
            user_a: env_required(ENV_USER_A)?,
            password_a: env_required(ENV_PASSWORD_A)?,
            user_b: env_required(ENV_USER_B)?,
            password_b: env_required(ENV_PASSWORD_B)?,
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct HeadlessQaReport {
    server_kind: String,
    created_rooms: usize,
    created_spaces: usize,
    invited_users: usize,
    joined_rooms: usize,
    sent_messages: usize,
    received_messages: usize,
    room_list_rooms: usize,
    room_list_spaces: usize,
}

impl std::fmt::Display for HeadlessQaReport {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "Headless local QA OK. server={} created_rooms={} created_spaces={} invited_users={} joined_rooms={} sent_messages={} received_messages={} room_list_rooms={} room_list_spaces={}",
            self.server_kind,
            self.created_rooms,
            self.created_spaces,
            self.invited_users,
            self.joined_rooms,
            self.sent_messages,
            self.received_messages,
            self.room_list_rooms,
            self.room_list_spaces
        )
    }
}

async fn run(config: HeadlessQaConfig) -> Result<HeadlessQaReport, String> {
    let mut session_a = Some(
        login(
            &config.homeserver,
            &config.user_a,
            &config.password_a,
            DEVICE_A,
        )
        .await?,
    );
    let mut session_b = Some(
        login(
            &config.homeserver,
            &config.user_b,
            &config.password_b,
            DEVICE_B,
        )
        .await?,
    );

    let result = run_authenticated(
        &config,
        session_a.as_ref().expect("session A should exist"),
        session_b.as_ref().expect("session B should exist"),
    )
    .await;

    if let Some(session) = session_a.as_ref() {
        let _ = matrix_desktop_sdk::logout(session).await;
    }
    if let Some(session) = session_b.as_ref() {
        let _ = matrix_desktop_sdk::logout(session).await;
    }
    drop(session_b.take());
    drop(session_a.take());

    result
}

async fn run_authenticated(
    config: &HeadlessQaConfig,
    session_a: &matrix_desktop_sdk::MatrixClientSession,
    session_b: &matrix_desktop_sdk::MatrixClientSession,
) -> Result<HeadlessQaReport, String> {
    let suffix = timestamp_millis()?;
    let room_id = matrix_desktop_sdk::create_room(
        session_a,
        &format!("Matrix Desktop Headless QA Room {suffix}"),
        false,
    )
    .await
    .map_err(|error| format!("create room failed: {error}"))?;
    let space_id = matrix_desktop_sdk::create_space(
        session_a,
        &format!("Matrix Desktop Headless QA Space {suffix}"),
    )
    .await
    .map_err(|error| format!("create space failed: {error}"))?;

    matrix_desktop_sdk::set_space_child(session_a, &space_id, &room_id, &config.server_name)
        .await
        .map_err(|error| format!("set space child failed: {error}"))?;

    let user_b_id = matrix_user_id(&config.user_b, &config.server_name);
    matrix_desktop_sdk::invite_user_to_room(session_a, &space_id, &user_b_id)
        .await
        .map_err(|error| format!("invite user to space failed: {error}"))?;
    matrix_desktop_sdk::invite_user_to_room(session_a, &room_id, &user_b_id)
        .await
        .map_err(|error| format!("invite user to room failed: {error}"))?;

    join_with_retry(session_b, &space_id, "space").await?;
    join_with_retry(session_b, &room_id, "room").await?;
    let snapshot_b = wait_for_room_list_entries(session_b, &room_id, &space_id).await?;

    assert_can_send(session_a, &room_id, "sender A").await?;
    assert_can_send(session_b, &room_id, "sender B").await?;

    let message_a_to_b = format!("matrix-desktop-headless-a-to-b-{suffix}");
    matrix_desktop_sdk::send_text_message(
        session_a,
        &room_id,
        &message_a_to_b,
        &format!("headless-a-to-b-{suffix}"),
    )
    .await
    .map_err(|error| format!("send A to B failed: {error}"))?;
    wait_for_message(session_b, &room_id, &message_a_to_b, "B receive").await?;

    let message_b_to_a = format!("matrix-desktop-headless-b-to-a-{suffix}");
    matrix_desktop_sdk::send_text_message(
        session_b,
        &room_id,
        &message_b_to_a,
        &format!("headless-b-to-a-{suffix}"),
    )
    .await
    .map_err(|error| format!("send B to A failed: {error}"))?;
    wait_for_message(session_a, &room_id, &message_b_to_a, "A receive").await?;

    Ok(HeadlessQaReport {
        server_kind: config.server_kind.clone(),
        created_rooms: 1,
        created_spaces: 1,
        invited_users: 1,
        joined_rooms: 2,
        sent_messages: 2,
        received_messages: 2,
        room_list_rooms: snapshot_b.rooms.len(),
        room_list_spaces: snapshot_b.spaces.len(),
    })
}

async fn login(
    homeserver: &str,
    username: &str,
    password: &str,
    device_display_name: &str,
) -> Result<matrix_desktop_sdk::MatrixClientSession, String> {
    let request = LoginRequest {
        homeserver: homeserver.to_owned(),
        username: username.to_owned(),
        password: AuthSecret::new(password.to_owned()),
        device_display_name: Some(device_display_name.to_owned()),
    };
    matrix_desktop_sdk::login_with_password(&request)
        .await
        .map_err(|error| format!("login failed: {error}"))
}

async fn join_with_retry(
    session: &matrix_desktop_sdk::MatrixClientSession,
    room_id: &str,
    label: &str,
) -> Result<(), String> {
    let mut last_error = "join was not attempted".to_owned();
    for _ in 0..POLL_ATTEMPTS {
        let _ = matrix_desktop_sdk::sync_once(session).await;
        match matrix_desktop_sdk::join_room_by_id(session, room_id).await {
            Ok(_) => return Ok(()),
            Err(error) => last_error = error.to_string(),
        }
        tokio::time::sleep(POLL_INTERVAL).await;
    }
    Err(format!("join {label} failed: {last_error}"))
}

async fn wait_for_room_list_entries(
    session: &matrix_desktop_sdk::MatrixClientSession,
    room_id: &str,
    space_id: &str,
) -> Result<matrix_desktop_sdk::MatrixRoomListSnapshot, String> {
    let mut last_error = "room list was not attempted".to_owned();
    for _ in 0..POLL_ATTEMPTS {
        matrix_desktop_sdk::sync_once(session)
            .await
            .map_err(|error| format!("sync before room list failed: {error}"))?;
        match matrix_desktop_sdk::room_list_snapshot(session).await {
            Ok(snapshot)
                if snapshot.rooms.iter().any(|room| room.room_id == room_id)
                    && snapshot
                        .spaces
                        .iter()
                        .any(|space| space.space_id == space_id) =>
            {
                return Ok(snapshot);
            }
            Ok(snapshot) => {
                last_error = format!(
                    "room list missing entries rooms={} spaces={}",
                    snapshot.rooms.len(),
                    snapshot.spaces.len()
                );
            }
            Err(error) => last_error = error.to_string(),
        }
        tokio::time::sleep(POLL_INTERVAL).await;
    }
    Err(last_error)
}

async fn assert_can_send(
    session: &matrix_desktop_sdk::MatrixClientSession,
    room_id: &str,
    label: &str,
) -> Result<(), String> {
    let can_send = matrix_desktop_sdk::room_can_send_text_message(session, room_id)
        .await
        .map_err(|error| format!("{label} send permission check failed: {error}"))?;
    if can_send {
        Ok(())
    } else {
        Err(format!("{label} cannot send to joined room"))
    }
}

async fn wait_for_message(
    session: &matrix_desktop_sdk::MatrixClientSession,
    room_id: &str,
    expected_body: &str,
    label: &str,
) -> Result<(), String> {
    let mut last_error = "timeline was not attempted".to_owned();
    for _ in 0..POLL_ATTEMPTS {
        matrix_desktop_sdk::sync_once(session)
            .await
            .map_err(|error| format!("{label} sync failed: {error}"))?;
        match matrix_desktop_sdk::room_timeline_visible_items(
            session,
            room_id,
            TIMELINE_BACKFILL_EVENT_COUNT,
        )
        .await
        {
            Ok(items) if items.iter().any(|item| item.body == expected_body) => return Ok(()),
            Ok(items) => last_error = format!("{label} timeline_items={}", items.len()),
            Err(error) => last_error = error.to_string(),
        }
        tokio::time::sleep(POLL_INTERVAL).await;
    }
    Err(format!(
        "{label} did not observe expected synthetic message: {last_error}"
    ))
}

fn matrix_user_id(localpart: &str, server_name: &str) -> String {
    format!("@{localpart}:{server_name}")
}

fn env_required(name: &str) -> Result<String, String> {
    env::var(name).map_err(|_| format!("{name} is required"))
}

fn timestamp_millis() -> Result<u128, String> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .map_err(|_| "system clock is before UNIX_EPOCH".to_owned())
}
