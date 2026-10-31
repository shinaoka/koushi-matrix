//! Real-homeserver delayed history import, using the production Koushi client
//! builder and Simplified Sliding Sync. Only the saved acceptance age is faked.

use std::{
    collections::{HashMap, HashSet},
    future::Future,
    pin::Pin,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, SystemTime},
};

use koushi_sdk::{MatrixClientSession, MatrixClientStoreConfig, MatrixClientStoreKey};
use koushi_state::{AuthSecret, LoginRequest};
use matrix_sdk::{
    Client, RoomState,
    config::RequestConfig,
    media::{MediaFetcher, MediaRequestParameters},
    ruma::{
        MilliSecondsSinceUnixEpoch, OwnedEventId, OwnedRoomId,
        api::client::room::create_room::v3::Request as CreateRoomRequest,
        events::room::{
            history_visibility::{HistoryVisibility, RoomHistoryVisibilityEventContent},
            message::RoomMessageEventContent,
        },
    },
};
use matrix_sdk_base::crypto::store::types::Changes;
use matrix_sdk_ui::sync_service::SyncService;

use super::registry::QaConfig;

const DAY: Duration = Duration::from_secs(24 * 60 * 60);
const WAIT: Duration = Duration::from_secs(45);
const BODY: &str = "Synthetic historical message";

#[path = "history_key_recovery_core.rs"]
mod core_recovery;

#[derive(Debug, Default)]
struct DeferredMedia(AtomicUsize);

impl MediaFetcher for DeferredMedia {
    fn fetch_media_content<'a>(
        &'a self,
        _client: &'a Client,
        _request: &'a MediaRequestParameters,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<u8>, matrix_sdk::Error>> + Send + 'a>> {
        Box::pin(async move {
            self.0.fetch_add(1, Ordering::SeqCst);
            Err(matrix_sdk::Error::UnknownError(
                std::io::Error::new(std::io::ErrorKind::Interrupted, "synthetic media deferral")
                    .into(),
            ))
        })
    }

    fn fetch_media_content_with_config<'a>(
        &'a self,
        client: &'a Client,
        request: &'a MediaRequestParameters,
        _request_config: RequestConfig,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<u8>, matrix_sdk::Error>> + Send + 'a>> {
        self.fetch_media_content(client, request)
    }
}

struct Participant {
    session: Option<MatrixClientSession>,
    sync: Option<SyncService>,
    store: MatrixClientStoreConfig,
}

impl Participant {
    fn session(&self) -> &MatrixClientSession {
        self.session
            .as_ref()
            .expect("session exists during scenario")
    }

    async fn start_sync(&mut self) -> Result<(), String> {
        let sync = SyncService::builder(self.session().client())
            .build()
            .await
            .map_err(|_| "history window: sync build failed")?;
        self.sync = Some(sync);
        self.sync
            .as_ref()
            .expect("sync stored before start")
            .start()
            .await;
        Ok(())
    }

    async fn stop_sync(&mut self) -> Result<(), String> {
        if let Some(sync) = &self.sync {
            tokio::time::timeout(Duration::from_secs(10), sync.stop())
                .await
                .map_err(|_| "history window: sync stop timed out")?;
        }
        self.sync.take();
        Ok(())
    }

    async fn restart(&mut self) -> Result<(), String> {
        self.restart_with_sync(true).await
    }

    async fn restart_with_sync(&mut self, start_sync: bool) -> Result<(), String> {
        self.stop_sync().await?;
        let old = self.session();
        let saved = old
            .persistable_session()
            .map_err(|_| "history window: session save failed")?;
        let expected_device = old.info.device_id.clone();
        old.client().abort_bundle_receiver_task();
        koushi_sdk::close_session_stores(old)
            .await
            .map_err(|_| "history window: close before restart failed")?;
        let restored = koushi_sdk::restore_session_with_verified_store(&saved, &self.store)
            .await
            .map_err(|_| "history window: restore failed")?;
        if restored.info.device_id != expected_device {
            self.session = Some(restored);
            return Err("history window: restore changed device".to_owned());
        }
        self.session = Some(restored);
        self.session()
            .client()
            .encryption()
            .wait_for_e2ee_initialization_tasks()
            .await;
        if start_sync {
            self.start_sync().await
        } else {
            Ok(())
        }
    }

    async fn cleanup(&mut self) -> Result<(), String> {
        let stopped = self.stop_sync().await;
        if let Some(session) = self.session.take() {
            let logout =
                tokio::time::timeout(Duration::from_secs(10), koushi_sdk::logout(&session)).await;
            let close = tokio::time::timeout(
                Duration::from_secs(10),
                koushi_sdk::close_session_stores(&session),
            )
            .await;
            logout
                .map_err(|_| "history window: logout timed out")?
                .map_err(|_| "history window: logout failed")?;
            close
                .map_err(|_| "history window: store cleanup timed out")?
                .map_err(|_| "history window: store cleanup failed")?;
        }
        stopped
    }
}

struct HistoricalRoom {
    room_id: OwnedRoomId,
    events: Vec<OwnedEventId>,
}

async fn wait_until<F, Fut>(label: &'static str, mut check: F) -> Result<(), String>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<bool, String>>,
{
    tokio::time::timeout(WAIT, async {
        let mut poll = tokio::time::interval(Duration::from_millis(25));
        loop {
            poll.tick().await;
            if check().await? {
                return Ok(());
            }
        }
    })
    .await
    .map_err(|_| format!("history window: {label} timed out"))?
}

async fn login(
    config: &QaConfig,
    root: &std::path::Path,
    name: &str,
    username: &str,
    password: &str,
) -> Result<Participant, String> {
    let mut key = [0; 32];
    getrandom::fill(&mut key).map_err(|_| "history window: store key generation failed")?;
    let store = MatrixClientStoreConfig::new(root.join(name), MatrixClientStoreKey::new(key));
    let session = koushi_sdk::login_with_password_with_store(
        &LoginRequest {
            homeserver: config.homeserver.clone(),
            username: username.to_owned(),
            password: AuthSecret::new(password.to_owned()),
            device_display_name: Some("Koushi Historical Key QA".to_owned()),
        },
        Some(&store),
    )
    .await
    .map_err(|_| "history window: login failed")?;
    Ok(Participant {
        session: Some(session),
        sync: None,
        store,
    })
}

async fn bootstrap(participant: &mut Participant, password: &str) -> Result<(), String> {
    participant.start_sync().await?;
    participant
        .session()
        .client()
        .encryption()
        .wait_for_e2ee_initialization_tasks()
        .await;
    koushi_sdk::bootstrap_cross_signing(
        participant.session(),
        Some(&AuthSecret::new(password.to_owned())),
    )
    .await
    .map_err(|_| "history window: cross signing failed")?;
    Ok(())
}

async fn create_history(sender: &Participant) -> Result<HistoricalRoom, String> {
    let room = sender
        .session()
        .client()
        .create_room(CreateRoomRequest::new())
        .await
        .map_err(|_| "history window: room creation failed")?;
    create_history_in_room(room).await
}

async fn create_history_in_room(room: matrix_sdk::Room) -> Result<HistoricalRoom, String> {
    room.send_state_event(RoomHistoryVisibilityEventContent::new(
        HistoryVisibility::Shared,
    ))
    .await
    .map_err(|_| "history window: history setting failed")?;
    room.enable_encryption()
        .await
        .map_err(|_| "history window: encryption enable failed")?;
    let mut events = Vec::new();
    let mut sessions = HashSet::new();
    for _ in 0..2 {
        room.discard_room_key()
            .await
            .map_err(|_| "history window: session rotation failed")?;
        events.push(
            room.send(RoomMessageEventContent::text_plain(BODY))
                .await
                .map_err(|_| "history window: historical send failed")?
                .response
                .event_id,
        );
        let event = room
            .event(events.last().expect("sent event exists"), None)
            .await
            .map_err(|_| "history window: sender event read failed")?;
        let session_id = event
            .encryption_info()
            .and_then(|info| info.session_id())
            .ok_or("history window: historical event not encrypted")?;
        sessions.insert(session_id.to_owned());
    }
    if sessions.len() != 2 {
        return Err("history window: historical sessions were not distinct".to_owned());
    }
    Ok(HistoricalRoom {
        room_id: room.room_id().to_owned(),
        events,
    })
}

async fn historical_messages_decrypt(
    client: &Client,
    history: &HistoricalRoom,
) -> Result<bool, String> {
    let room = client
        .get_room(&history.room_id)
        .ok_or("history window: room missing")?;
    for event_id in &history.events {
        let event = room
            .event(event_id, None)
            .await
            .map_err(|_| "history window: event read failed")?;
        if event.encryption_info().is_none() {
            return Ok(false);
        }
        let Some(content) = event
            .raw()
            .get_field::<RoomMessageEventContent>("content")
            .map_err(|_| "history window: decrypted event malformed")?
        else {
            return Err("history window: decrypted event has no content".to_owned());
        };
        if content.body() != BODY {
            return Err("history window: decrypted content mismatch".to_owned());
        }
    }
    Ok(true)
}

async fn backdate(client: &Client, history: &HistoricalRoom, age: Duration) -> Result<(), String> {
    let mut details = client
        .get_pending_key_bundle_details_for_room(&history.room_id)
        .await
        .map_err(|_| "history window: pending read failed")?
        .ok_or("history window: pending record missing before backdate")?;
    details.invite_accepted_at =
        MilliSecondsSinceUnixEpoch::from_system_time(SystemTime::now() - age)
            .ok_or("history window: acceptance timestamp invalid")?;
    let machine = client.olm_machine_for_testing().await;
    machine
        .as_ref()
        .ok_or("history window: crypto unavailable")?
        .store()
        .save_changes(Changes {
            rooms_pending_key_bundle: HashMap::from([(history.room_id.clone(), Some(details))]),
            ..Default::default()
        })
        .await
        .map_err(|_| "history window: backdate save failed".to_owned())
}

async fn flow(
    config: &QaConfig,
    sender: &mut Participant,
    recipient: &mut Participant,
) -> Result<(), String> {
    bootstrap(sender, &config.password_a).await?;
    bootstrap(recipient, &config.password_b).await?;
    koushi_sdk::enable_key_backup(sender.session(), None)
        .await
        .map_err(|_| "history window: sender backup setup failed")?;
    let gate = Arc::new(DeferredMedia::default());
    recipient
        .session()
        .client()
        .set_media_fetcher(gate.clone())
        .await;
    let valid = create_history(sender).await?;
    let expired = create_history(sender).await?;
    for history in [&valid, &expired] {
        let client = sender.session().client();
        client
            .get_room(&history.room_id)
            .ok_or("history window: sender room missing")?
            .invite_user_by_id(
                recipient
                    .session()
                    .client()
                    .user_id()
                    .ok_or("history window: recipient identity missing")?,
            )
            .await
            .map_err(|_| "history window: invite failed")?;
        let client = recipient.session().client();
        // The SDK creates an invited room before its stripped invite state is stored,
        // so a join that wins that race records no inviter and never queues the key
        // bundle. Wait for the invite the UI would actually be showing.
        wait_until("invite arrival", || async {
            let Some(room) = client.get_room(&history.room_id) else {
                return Ok(false);
            };
            Ok(room.state() == RoomState::Invited && room.invite_details().await.is_ok())
        })
        .await?;
        client
            .join_room_by_id(&history.room_id)
            .await
            .map_err(|_| "history window: join failed")?;
        let inviter = sender.session().client().user_id().unwrap().to_owned();
        wait_until("stored bundle", || async {
            let machine = client.olm_machine_for_testing().await;
            Ok(machine
                .as_ref()
                .ok_or("history window: crypto unavailable")?
                .store()
                .get_received_room_key_bundle_data(&history.room_id, &inviter)
                .await
                .map_err(|_| "history window: bundle read failed")?
                .is_some())
        })
        .await?;
        if historical_messages_decrypt(&client, history).await? {
            return Err("history window: history imported before restart".to_owned());
        }
        let machine = client.olm_machine_for_testing().await;
        if !machine
            .as_ref()
            .ok_or("history window: crypto unavailable")?
            .store()
            .get_inbound_group_sessions_by_room_id(&history.room_id)
            .await
            .map_err(|_| "history window: key lookup failed")?
            .is_empty()
        {
            return Err("history window: unexpected pre-restart room key".to_owned());
        }
    }
    if gate.0.load(Ordering::SeqCst) == 0 {
        return Err("history window: media deferral not exercised".to_owned());
    }
    recipient.stop_sync().await?;
    backdate(&recipient.session().client(), &valid, 29 * DAY).await?;
    backdate(&recipient.session().client(), &expired, 30 * DAY).await?;
    println!("history_window_deferred=ok");
    recipient.restart().await?;
    let client = recipient.session().client();
    wait_until("29-day import", || {
        historical_messages_decrypt(&client, &valid)
    })
    .await?;
    wait_until("pending cleanup", || async {
        for history in [&valid, &expired] {
            if client
                .get_pending_key_bundle_details_for_room(&history.room_id)
                .await
                .map_err(|_| "history window: pending cleanup read failed")?
                .is_some()
            {
                return Ok(false);
            }
        }
        Ok(true)
    })
    .await?;
    println!("history_window_29_days=ok");
    if historical_messages_decrypt(&client, &expired).await? {
        return Err("history window: expired bundle imported".to_owned());
    }
    let machine = client.olm_machine_for_testing().await;
    if !machine
        .as_ref()
        .ok_or("history window: crypto unavailable")?
        .store()
        .get_inbound_group_sessions_by_room_id(&expired.room_id)
        .await
        .map_err(|_| "history window: expired key lookup failed")?
        .is_empty()
    {
        return Err("history window: expired historical keys present".to_owned());
    }
    println!("history_window_30_days_expired=ok");
    drop(machine);
    core_recovery::run(config, sender, recipient, &expired).await?;
    Ok(())
}

pub(super) async fn run(config: &QaConfig) -> Result<(), String> {
    let root = tempfile::tempdir().map_err(|_| "history window: temporary stores failed")?;
    let mut sender = login(
        config,
        root.path(),
        "sender",
        &config.user_a,
        &config.password_a,
    )
    .await?;
    let mut recipient = match login(
        config,
        root.path(),
        "recipient",
        &config.user_b,
        &config.password_b,
    )
    .await
    {
        Ok(recipient) => recipient,
        Err(error) => {
            let _ = sender.cleanup().await;
            return Err(error);
        }
    };
    let result = tokio::time::timeout(
        Duration::from_secs(300),
        flow(config, &mut sender, &mut recipient),
    )
    .await
    .map_err(|_| "history window: scenario timed out".to_owned())
    .and_then(|result| result);
    let cleanup_recipient = recipient.cleanup().await;
    let cleanup_sender = sender.cleanup().await;
    root.close()
        .map_err(|_| "history window: temporary store removal failed")?;
    result?;
    cleanup_recipient.and(cleanup_sender)?;
    println!("history_window_cleanup=ok");
    println!("history_key_recovery_window=ok");
    Ok(())
}
