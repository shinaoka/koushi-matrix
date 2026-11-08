//! Session-owned historical key recovery. All private records use the encrypted
//! SDK crypto store; the SDK retains ownership of crypto and session provenance.
mod diagnostics;
mod policy;
mod records;
mod responder;
mod storage;

use crate::executor;
use koushi_protocol::{
    event::{CoreEvent, RoomEvent},
    failure::{CoreFailure, RoomFailureKind},
    ids::RequestId,
};
use koushi_sdk::MatrixClientSession;
use koushi_state::{
    AppAction, HistoryKeyRecoveryFailure as Failure, HistoryKeyRecoveryState,
    HistoryKeyRecoveryStatus as Status,
};
use matrix_sdk::ruma::{
    RoomId, UserId,
    api::client::state::get_state_events,
    events::{AnyToDeviceEvent, AnyToDeviceEventContent},
    serde::Raw,
};
use matrix_sdk::{
    Client, RoomState,
    deserialized_responses::{AlgorithmInfo, EncryptionInfo},
    encryption::identities::Device,
    event_handler::{EventHandlerHandle, RawEvent},
};
use matrix_sdk_base::crypto::{
    CollectStrategy, olm::SenderData, types::room_history::RoomKeyBundle,
};
use policy::*;
use records::*;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::sync::{broadcast, mpsc};

const EVENT_TYPE: &str = "chat.koushi.history_keys.v1";
const STORE_KEY: &str = "chat.koushi.history_keys.v1";

#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum Wire {
    Request {
        room: String,
        nonce: String,
        expires: u64,
    },
    Chunk {
        room: String,
        nonce: String,
        response: String,
        count: u16,
        index: u16,
        bundle: RoomKeyBundle,
    },
    Refused {
        room: String,
        nonce: String,
        reason: Failure,
    },
    Ack {
        room: String,
        nonce: String,
        response: String,
    },
}

/// Count-only QA observation; no keys, peers, rooms or wire correlations.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct HistoryKeyRecoveryDiagnostics {
    pub incoming_requests: u64,
    pub rejected_messages: u64,
    pub imported_keys: u64,
    pub replayed_responses: u64,
    pub acknowledged_responses: u64,
    pub cached_bytes: u64,
}

enum Message {
    #[cfg(any(test, feature = "test-hooks"))]
    PauseImport {
        disclosure: bool,
        reached: tokio::sync::oneshot::Sender<()>,
        resume: tokio::sync::oneshot::Receiver<()>,
        installed: tokio::sync::oneshot::Sender<bool>,
    },
    #[cfg(any(test, feature = "test-hooks"))]
    Barrier(
        Option<String>,
        tokio::sync::oneshot::Sender<HistoryKeyRecoveryDiagnostics>,
    ),
    Request {
        room: String,
        force_all: bool,
        request_id: RequestId,
    },
    Incoming {
        info: EncryptionInfo,
        raw: String,
    },
    Unresolved(String, bool),
}

pub(super) struct HistoryKeyRecoveryHandle {
    tx: mpsc::Sender<Message>,
    client: Client,
    handler: EventHandlerHandle,
    task: executor::AbortOnDrop<()>,
}

impl HistoryKeyRecoveryHandle {
    pub(super) fn start(
        session: Arc<MatrixClientSession>,
        actions: mpsc::Sender<Vec<AppAction>>,
        events: broadcast::Sender<CoreEvent>,
    ) -> Self {
        let client = session.client();
        let (tx, mut rx) = mpsc::channel(64);
        let (incoming, network_rx) = mpsc::channel(64);
        let handler = client.add_event_handler(
            move |_: AnyToDeviceEvent, raw: RawEvent, info: Option<EncryptionInfo>| {
                let incoming = incoming.clone();
                async move {
                    let Some(info) = info else { return };
                    if raw.get().len() > MESSAGE_LIMIT + 2048 {
                        return;
                    }
                    let Ok(value) = serde_json::from_str::<serde_json::Value>(raw.get()) else {
                        return;
                    };
                    if value["type"].as_str() != Some(EVENT_TYPE) {
                        return;
                    }
                    let Some(content) = value.get("content") else {
                        return;
                    };
                    let Ok(raw) = serde_json::to_string(content) else {
                        return;
                    };
                    if raw.len() <= MESSAGE_LIMIT {
                        // Network ingress is bounded; cached identical reply replay
                        // makes a saturated/crashed receiver recoverable by Retry.
                        let _ = incoming.try_send(Message::Incoming { info, raw });
                    }
                }
            },
        );
        let mut actor = Recovery {
            client: client.clone(),
            actions,
            events,
            ledger: Ledger::default(),
            diagnostics: HistoryKeyRecoveryDiagnostics::default(),
            #[cfg(any(test, feature = "test-hooks"))]
            room_rejections: BTreeMap::new(),
            #[cfg(any(test, feature = "test-hooks"))]
            import_pause: None,
            #[cfg(any(test, feature = "test-hooks"))]
            disclosure_pause: None,
            poisoned: AtomicBool::new(false),
            queries: BTreeMap::new(),
            startup_deadline: executor::Instant::now() + Duration::from_secs(120),
            automatic_attempted: BTreeSet::new(),
        };
        let task = executor::AbortOnDrop::new(executor::spawn(async move {
            Box::pin(actor.run(&mut rx, network_rx)).await;
            rx.close();
            while let Some(message) = rx.recv().await {
                if let Message::Request { request_id, .. } = message {
                    let _ = actor.events.send(CoreEvent::OperationFailed {
                        request_id,
                        failure: CoreFailure::StoreUnavailable,
                    });
                }
            }
        }));
        Self {
            tx,
            client,
            handler,
            task,
        }
    }
    #[cfg(any(test, feature = "test-hooks"))]
    pub(super) async fn pause_import(
        &self,
        disclosure: bool,
        reached: tokio::sync::oneshot::Sender<()>,
        resume: tokio::sync::oneshot::Receiver<()>,
        installed: tokio::sync::oneshot::Sender<bool>,
    ) {
        let _ = self
            .tx
            .send(Message::PauseImport {
                disclosure,
                reached,
                resume,
                installed,
            })
            .await;
    }
    #[cfg(any(test, feature = "test-hooks"))]
    pub(super) async fn barrier(&self, room: Option<String>) -> HistoryKeyRecoveryDiagnostics {
        let (tx, rx) = tokio::sync::oneshot::channel();
        if self.tx.send(Message::Barrier(room, tx)).await.is_ok() {
            return rx.await.unwrap_or_default();
        }
        HistoryKeyRecoveryDiagnostics::default()
    }
    pub(super) async fn request(
        &self,
        room: String,
        force_all: bool,
        request_id: RequestId,
    ) -> bool {
        self.tx
            .send(Message::Request {
                room,
                force_all,
                request_id,
            })
            .await
            .is_ok()
    }
    pub(super) async fn note_unresolved(&self, room: String, unresolved: bool) {
        let _ = self.tx.send(Message::Unresolved(room, unresolved)).await;
    }
    pub(super) async fn stop(mut self) {
        self.client.remove_event_handler(self.handler.clone());
        self.task.abort();
        self.task.settle().await;
    }
}
impl Drop for HistoryKeyRecoveryHandle {
    fn drop(&mut self) {
        self.client.remove_event_handler(self.handler.clone());
    }
}

struct Facts {
    joined: BTreeSet<String>,
    admins: BTreeSet<String>,
}
struct Recovery {
    client: Client,
    actions: mpsc::Sender<Vec<AppAction>>,
    events: broadcast::Sender<CoreEvent>,
    ledger: Ledger,
    diagnostics: HistoryKeyRecoveryDiagnostics,
    #[cfg(any(test, feature = "test-hooks"))]
    room_rejections: BTreeMap<String, u64>,
    #[cfg(any(test, feature = "test-hooks"))]
    import_pause: Option<(
        tokio::sync::oneshot::Sender<()>,
        tokio::sync::oneshot::Receiver<()>,
    )>,
    #[cfg(any(test, feature = "test-hooks"))]
    disclosure_pause: Option<(
        tokio::sync::oneshot::Sender<()>,
        tokio::sync::oneshot::Receiver<()>,
    )>,
    poisoned: AtomicBool,
    queries: BTreeMap<String, u64>,
    startup_deadline: executor::Instant,
    automatic_attempted: BTreeSet<String>,
}

impl Recovery {
    async fn run(
        &mut self,
        rx: &mut mpsc::Receiver<Message>,
        mut network_rx: mpsc::Receiver<Message>,
    ) {
        match self
            .client
            .encryption()
            .application_crypto_data(STORE_KEY)
            .await
        {
            Ok(Some(bytes)) => match serde_json::from_slice(&bytes) {
                Ok(ledger) => self.ledger = ledger,
                Err(_) => return,
            },
            Ok(None) => {}
            Err(_) => return,
        }
        if self.prune().await.is_err() {
            return;
        }
        for (room, pending) in &self.ledger.rooms {
            if pending.sent && pending.expires > now() {
                self.publish(room, pending.generation, Self::status(pending))
                    .await;
            }
        }
        let rooms: Vec<_> = self
            .ledger
            .rooms
            .iter()
            .filter(|(_, p)| p.unresolved)
            .map(|(r, _)| r.clone())
            .collect();
        for room in rooms {
            self.request(room, false, true, None, "startup").await;
        }
        loop {
            if self.poisoned.load(Ordering::Acquire) {
                return;
            }
            let message = tokio::select! { biased;
                message = rx.recv() => match message { Some(message) => message, None => return },
                message = network_rx.recv() => match message { Some(message) => message, None => return },
            };
            match message {
                #[cfg(any(test, feature = "test-hooks"))]
                Message::PauseImport {
                    disclosure,
                    reached,
                    resume,
                    installed,
                } => {
                    let slot = if disclosure {
                        &mut self.disclosure_pause
                    } else {
                        &mut self.import_pause
                    };
                    let accepted = slot.is_none();
                    if accepted {
                        *slot = Some((reached, resume));
                    }
                    let _ = installed.send(accepted);
                }
                #[cfg(any(test, feature = "test-hooks"))]
                Message::Barrier(room, done) => {
                    let mut diagnostics = self.diagnostics;
                    if let Some(room) = room {
                        diagnostics.rejected_messages =
                            self.room_rejections.get(&room).copied().unwrap_or(0);
                    }
                    diagnostics.cached_bytes =
                        self.ledger.cached.values().map(|c| c.bytes as u64).sum();
                    let _ = done.send(diagnostics);
                }
                Message::Request {
                    room,
                    force_all,
                    request_id,
                } => {
                    self.request(room, force_all, false, Some(request_id), "manual")
                        .await
                }
                Message::Incoming { info, raw } => {
                    #[cfg(any(test, feature = "test-hooks"))]
                    let observed_room = serde_json::from_str::<serde_json::Value>(&raw)
                        .ok()
                        .and_then(|v| v["room"].as_str().map(str::to_owned));
                    let result = Box::pin(executor::timeout(
                        Duration::from_secs(60),
                        self.incoming(info, raw),
                    ))
                    .await;
                    if result.is_err() {
                        // Cancellation can interrupt a durable write after it
                        // started. Never issue another write with unknown state.
                        self.poisoned.store(true, Ordering::Release);
                    }
                    if !matches!(result, Ok(Ok(()))) {
                        self.diagnostics.rejected_messages =
                            self.diagnostics.rejected_messages.saturating_add(1);
                        #[cfg(any(test, feature = "test-hooks"))]
                        if let Some(room) = observed_room
                            && (self.room_rejections.len() < 128
                                || self.room_rejections.contains_key(&room))
                        {
                            *self.room_rejections.entry(room).or_default() += 1;
                        }
                    }
                }
                Message::Unresolved(room, unresolved) => {
                    if self
                        .ledger
                        .rooms
                        .get(&room)
                        .is_some_and(|p| p.unresolved == unresolved)
                    {
                        continue;
                    }
                    if !unresolved && !self.ledger.rooms.contains_key(&room) {
                        continue;
                    }
                    if self.ledger.rooms.len() < 16 || self.ledger.rooms.contains_key(&room) {
                        self.ledger
                            .rooms
                            .entry(room.clone())
                            .or_default()
                            .unresolved = unresolved;
                        if self.save().await.is_ok() {
                            diagnostics::record_window(if unresolved { "open" } else { "close" });
                            if unresolved {
                                self.request(room, false, true, None, "recovery_window")
                                    .await;
                            }
                        }
                    }
                }
            }
        }
    }
    async fn save(&self) -> Result<(), Failure> {
        let result = async {
            let bytes = serde_json::to_vec(&self.ledger).map_err(|_| Failure::Storage)?;
            if bytes.len() > 4 * 1024 * 1024 {
                return Err(Failure::Storage);
            }
            self.store_value(STORE_KEY, &bytes).await
        }
        .await;
        if result.is_err() {
            self.poisoned.store(true, Ordering::Release);
        }
        result
    }
    async fn store_value(&self, key: &str, bytes: &[u8]) -> Result<(), Failure> {
        storage::persist(&self.poisoned, async {
            self.client
                .encryption()
                .set_application_crypto_data(key, bytes)
                .await
                .map_err(|_| Failure::Storage)
        })
        .await
    }
    async fn publish(&self, room: &str, generation: u64, status: Status) {
        let _ = self
            .actions
            .send(vec![AppAction::HistoryKeyRecoveryChanged {
                room_id: room.to_owned(),
                state: HistoryKeyRecoveryState { generation, status },
            }])
            .await;
    }
    fn status(p: &Pending) -> Status {
        if let Some(reason) = p.partial_reason {
            if p.received == 0 {
                Status::Failed { reason }
            } else {
                Status::Partial {
                    received_keys: p.received,
                    reason,
                }
            }
        } else {
            Status::Requested {
                recipients: p.recipients(),
                responses: p.responses(),
                received_keys: p.received,
            }
        }
    }
    fn failed(&self, request_id: Option<RequestId>, reason: Failure) {
        if let Some(request_id) = request_id {
            let _ = self.events.send(CoreEvent::OperationFailed {
                request_id,
                failure: CoreFailure::RoomOperationFailed {
                    kind: if reason == Failure::Network {
                        RoomFailureKind::Network
                    } else {
                        RoomFailureKind::PolicyNotVerified
                    },
                },
            });
        }
    }
    async fn facts(&self, room: &str) -> Result<Facts, Failure> {
        let room_id = RoomId::parse(room).map_err(|_| Failure::MembershipChanged)?;
        let cached = self
            .client
            .get_room(&room_id)
            .ok_or(Failure::MembershipChanged)?;
        if cached.state() != RoomState::Joined {
            return Err(Failure::MembershipChanged);
        }
        let state = self
            .client
            .send(get_state_events::v3::Request::new(room_id))
            .await
            .map_err(|_| Failure::Network)?;
        let mut joined = BTreeSet::new();
        let mut power = BTreeMap::new();
        let mut default_power = 0;
        let mut history_shared = true; // Matrix default is shared.
        let mut encrypted = false;
        let mut creator = None;
        let mut version12 = false;
        let mut additional_creators = BTreeSet::new();
        for raw in state.room_state {
            let Ok(value) = raw.deserialize_as::<serde_json::Value>() else {
                return Err(Failure::InvalidResponse);
            };
            match value["type"].as_str() {
                Some("m.room.member") if value["content"]["membership"] == "join" => {
                    if let Some(user) = value["state_key"].as_str() {
                        joined.insert(user.to_owned());
                    }
                }
                Some("m.room.history_visibility") => {
                    history_shared = matches!(
                        value["content"]["history_visibility"].as_str(),
                        Some("shared" | "world_readable")
                    )
                }
                Some("m.room.encryption") => {
                    encrypted = value["content"]["algorithm"] == "m.megolm.v1.aes-sha2"
                }
                Some("m.room.power_levels") => {
                    default_power = value["content"]["users_default"].as_i64().unwrap_or(0);
                    if let Some(users) = value["content"]["users"].as_object() {
                        for (user, level) in users {
                            if let Some(level) = level.as_i64() {
                                power.insert(user.clone(), level);
                            }
                        }
                    }
                }
                Some("m.room.create") => {
                    creator = value["sender"].as_str().map(str::to_owned);
                    version12 = value["content"]["room_version"]
                        .as_str()
                        .is_some_and(|v| v.parse::<u64>().is_ok_and(|v| v >= 12));
                    if version12
                        && let Some(additional) = value["content"]["additional_creators"].as_array()
                    {
                        for user in additional.iter().filter_map(|v| v.as_str()) {
                            additional_creators.insert(user.to_owned());
                        }
                    }
                }
                _ => {}
            }
        }
        if !encrypted || !history_shared {
            return Err(Failure::HistoryNotShared);
        }
        let own = self.client.user_id().ok_or(Failure::MembershipChanged)?;
        if !joined.contains(own.as_str()) {
            return Err(Failure::MembershipChanged);
        }
        let admins = joined
            .iter()
            .filter(|user| {
                power.get(*user).copied().unwrap_or(default_power) >= 100
                    || (version12 && creator.as_ref() == Some(*user))
                    || (version12 && additional_creators.contains(*user))
            })
            .cloned()
            .collect();
        Ok(Facts { joined, admins })
    }

    fn local_disclosure_allowed(&self, room: &str) -> Result<(), Failure> {
        use matrix_sdk::ruma::events::room::history_visibility::HistoryVisibility;
        let id = RoomId::parse(room).map_err(|_| Failure::MembershipChanged)?;
        let cached = self
            .client
            .get_room(&id)
            .ok_or(Failure::MembershipChanged)?;
        if cached.state() != RoomState::Joined {
            return Err(Failure::MembershipChanged);
        }
        if !matches!(
            cached.history_visibility_or_default(),
            HistoryVisibility::Shared | HistoryVisibility::WorldReadable
        ) {
            return Err(Failure::HistoryNotShared);
        }
        Ok(())
    }

    fn pin(&self, device: &Device) -> Result<Pin, Failure> {
        if device.is_blacklisted() {
            return Err(Failure::VerificationRequired);
        }
        // Derive the gate and master key from one device/identity snapshot.
        // Manual peer verification is optional; unsigned devices and a
        // previously verified identity change are still ineligible.
        let (SenderData::SenderUnverified(owner) | SenderData::SenderVerified(owner)) =
            device.cross_signing_sender_data()
        else {
            return Err(Failure::VerificationRequired);
        };
        Ok(Pin {
            user: device.user_id().to_string(),
            device: device.device_id().to_string(),
            curve: device
                .curve25519_key()
                .ok_or(Failure::VerificationRequired)?
                .to_base64(),
            ed: device
                .ed25519_key()
                .ok_or(Failure::VerificationRequired)?
                .to_base64(),
            master: owner.master_key.to_base64(),
        })
    }

    async fn pinned_device(&self, pin: &Pin) -> Result<Device, Failure> {
        let user = UserId::parse(&pin.user).map_err(|_| Failure::InvalidResponse)?;
        let device = self
            .client
            .encryption()
            .get_device(&user, pin.device.as_str().into())
            .await
            .map_err(|_| Failure::Network)?
            .ok_or(Failure::VerificationRequired)?;
        if self.pin(&device)? != *pin {
            return Err(Failure::VerificationRequired);
        }
        Ok(device)
    }

    async fn request(
        &mut self,
        room: String,
        force_all: bool,
        automatic: bool,
        request_id: Option<RequestId>,
        trigger: &'static str,
    ) {
        if automatic
            && (executor::Instant::now() > self.startup_deadline
                || !self.automatic_attempted.insert(room.clone())
                || self
                    .ledger
                    .rooms
                    .get(&room)
                    .is_some_and(|p| !automatic_due(p.auto_at, now())))
        {
            return;
        }
        if self.ledger.rooms.len() >= 16 && !self.ledger.rooms.contains_key(&room) {
            self.failed(request_id, Failure::TooLarge);
            return;
        }
        if !automatic
            && self.ledger.rooms.get(&room).is_some_and(|p| {
                p.sent
                    && p.partial_reason.is_none()
                    && p.expires > now()
                    && !p.automatic
                    && p.force_all == force_all
                    && now()
                        .checked_sub(p.sent_at)
                        .is_none_or(|elapsed| elapsed < 60)
            })
        {
            if let Some(request_id) = request_id {
                let _ = self
                    .events
                    .send(CoreEvent::Room(RoomEvent::HistoryKeysRequested {
                        request_id,
                        room_id: room.clone(),
                        recipients: self.ledger.rooms[&room].recipients(),
                    }));
            }
            return;
        }
        self.ledger.generation = self.ledger.generation.saturating_add(1);
        let generation = self.ledger.generation;
        let p = self.ledger.rooms.entry(room.clone()).or_default();
        if p.expires <= now() {
            *p = Pending {
                unresolved: true,
                expires: episode_expiry(now()),
                ..Default::default()
            };
        }
        p.generation = generation;
        if automatic {
            p.auto_at = now();
        }
        if self.save().await.is_err() {
            self.failed(request_id, Failure::Storage);
            return;
        }
        self.publish(&room, generation, Status::Sending).await;
        let dispatched_at = executor::Instant::now();
        let result = executor::timeout(
            Duration::from_secs(60),
            self.send_request(&room, generation, force_all, automatic, trigger),
        )
        .await;
        let result = match result {
            Ok(result) => result,
            Err(_) => {
                self.poisoned.store(true, Ordering::Release);
                self.publish(
                    &room,
                    generation,
                    Status::Failed {
                        reason: Failure::Storage,
                    },
                )
                .await;
                if let Some(request_id) = request_id {
                    let _ = self.events.send(CoreEvent::OperationFailed {
                        request_id,
                        failure: CoreFailure::StoreUnavailable,
                    });
                }
                return;
            }
        };
        match result {
            Ok(recipients) => {
                self.publish(&room, generation, Self::status(&self.ledger.rooms[&room]))
                    .await;
                diagnostics::record_requested(
                    trigger,
                    recipients,
                    dispatched_at.elapsed().as_millis(),
                );
                if let Some(request_id) = request_id {
                    let _ = self
                        .events
                        .send(CoreEvent::Room(RoomEvent::HistoryKeysRequested {
                            request_id,
                            room_id: room,
                            recipients,
                        }));
                }
            }
            Err(reason) => {
                let pending = self
                    .ledger
                    .rooms
                    .get_mut(&room)
                    .expect("request episode exists");
                pending.sent = false;
                pending.partial_reason = Some(reason);
                let status = Self::status(pending);
                if !self.poisoned.load(Ordering::Acquire) {
                    let _ = self.save().await;
                }
                self.publish(&room, generation, status).await;
                self.failed(request_id, reason);
            }
        }
    }
    async fn send_request(
        &mut self,
        room: &str,
        generation: u64,
        force_all: bool,
        automatic: bool,
        trigger: &'static str,
    ) -> Result<u16, Failure> {
        self.prune().await?;
        let facts = self.facts(room).await?;
        let own = self.client.user_id().ok_or(Failure::MembershipChanged)?;
        let target_users = targets(
            &facts.joined,
            &facts.admins,
            own.as_str(),
            force_all,
            automatic,
        )?;
        let expected: BTreeSet<_> = target_users.iter().cloned().collect();
        let mut devices = Vec::new();
        let mut pins = Vec::new();
        for user in target_users {
            let user = UserId::parse(user).map_err(|_| Failure::InvalidResponse)?;
            self.client
                .encryption()
                .request_user_identity(&user)
                .await
                .map_err(|_| Failure::Network)?;
            for device in self
                .client
                .encryption()
                .get_user_devices(&user)
                .await
                .map_err(|_| Failure::Network)?
                .devices()
            {
                if let Ok(pin) = self.pin(&device) {
                    pins.push(pin);
                    devices.push(device);
                }
                if devices.len() > 256 {
                    return Err(Failure::TooLarge);
                }
            }
        }
        if devices.is_empty() {
            return Err(Failure::VerificationRequired);
        }
        let current = self.facts(room).await?;
        if targets(
            &current.joined,
            &current.admins,
            own.as_str(),
            force_all,
            automatic,
        )?
        .into_iter()
        .collect::<BTreeSet<_>>()
            != expected
        {
            return Err(Failure::MembershipChanged);
        }
        let mut pending = self
            .ledger
            .rooms
            .get(room)
            .cloned()
            .ok_or(Failure::Storage)?;
        pending.generation = generation;
        pending.dispatch = pins.clone();
        pending.partial_reason = None;
        pending.force_all = force_all;
        pending.automatic = automatic;
        // Automatic admin retry is a dispatch subset; it does not revoke
        // still-authorized outstanding manual recipients.
        if automatic {
            pending.pins.retain(|p| current.joined.contains(&p.user));
            for pin in &pins {
                if !pending.pins.contains(pin) {
                    pending.pins.push(pin.clone());
                }
            }
        } else {
            pending.pins = pins.clone();
        }
        if pending.pins.len() > 256 {
            return Err(Failure::TooLarge);
        }
        let mut acknowledgements = Vec::new();
        // Retain completed receipts even when a scope change removes a peer,
        // so a lost Ack can still release that peer's encrypted reply cache.
        for old in pending.attempts.values().filter(|a| a.complete()) {
            let completed_key = nonce_key(&old.pin, &old.nonce);
            if self.ledger.completed.len() >= 1024
                && !self.ledger.completed.contains_key(&completed_key)
            {
                return Err(Failure::TooLarge);
            }
            let completed = Completed {
                room: room.to_owned(),
                pin: old.pin.clone(),
                nonce: old.nonce.clone(),
                response: old.receipt.id.clone(),
                expires: old.expires,
            };
            self.ledger
                .completed
                .insert(completed_key, completed.clone());
            acknowledgements.push(completed);
        }
        for pin in &pins {
            let previous = pending.attempts.get(&pin.key());
            if previous
                .is_some_and(|a| a.pin == *pin && a.expires > now() && !a.complete() && !a.terminal)
            {
                continue;
            }
            pending.attempts.insert(
                pin.key(),
                Attempt {
                    pin: pin.clone(),
                    nonce: random_id()?,
                    expires: pending.expires,
                    confirmed: false,
                    terminal: false,
                    receipt: Receipt::default(),
                },
            );
        }
        pending
            .attempts
            .retain(|_, a| pending.pins.contains(&a.pin));
        pending.sent = false;
        // A fixed dispatch anchor prevents repeated clicks from sliding the
        // coalescing window. Per-device absolute expiry never moves.
        pending.sent_at = now();
        self.ledger.rooms.insert(room.to_owned(), pending);
        self.save().await?;
        for completed in acknowledgements {
            if let Some(device) = devices.iter().find(|d| {
                d.user_id().as_str() == completed.pin.user
                    && d.device_id().as_str() == completed.pin.device
            }) {
                let _ = self
                    .send(
                        std::slice::from_ref(device),
                        &Wire::Ack {
                            room: completed.room,
                            nonce: completed.nonce,
                            response: completed.response,
                        },
                    )
                    .await;
            }
        }
        for device in &devices {
            let pin = pins
                .iter()
                .find(|p| {
                    p.user == device.user_id().as_str() && p.device == device.device_id().as_str()
                })
                .ok_or(Failure::InvalidResponse)?;
            let attempt = &self.ledger.rooms[room].attempts[&pin.key()];
            let wire = Wire::Request {
                room: room.to_owned(),
                nonce: attempt.nonce.clone(),
                expires: attempt.expires,
            };
            self.send(std::slice::from_ref(device), &wire).await?;
            self.ledger
                .rooms
                .get_mut(room)
                .ok_or(Failure::Storage)?
                .attempts
                .get_mut(&pin.key())
                .ok_or(Failure::Storage)?
                .confirmed = true;
            self.save().await?;
        }
        let pending = self.ledger.rooms.get_mut(room).ok_or(Failure::Storage)?;
        pending.sent = true;
        let recipients = pending.recipients();
        self.save().await?;
        diagnostics::record_issued(trigger, recipients);
        Ok(recipients)
    }
    async fn send(&self, devices: &[Device], wire: &Wire) -> Result<(), Failure> {
        let raw = serde_json::to_string(wire).map_err(|_| Failure::InvalidResponse)?;
        self.send_raw(devices, raw).await
    }
    async fn send_raw(&self, devices: &[Device], raw: String) -> Result<(), Failure> {
        if raw.len() > MESSAGE_LIMIT {
            return Err(Failure::TooLarge);
        }
        let content: Raw<AnyToDeviceEventContent> =
            Raw::from_json_string(raw).map_err(|_| Failure::InvalidResponse)?;
        let failures = self
            .client
            .encryption()
            .encrypt_and_send_raw_to_device(
                devices.iter().collect(),
                EVENT_TYPE,
                content,
                CollectStrategy::IdentityBasedStrategy,
            )
            .await
            .map_err(|_| Failure::Network)?;
        if failures.is_empty() {
            Ok(())
        } else {
            Err(Failure::VerificationRequired)
        }
    }
    async fn authenticated(&self, info: &EncryptionInfo) -> Result<(Device, Pin), Failure> {
        let AlgorithmInfo::OlmV1Curve25519AesSha2 {
            curve25519_public_key_base64: curve,
        } = &info.algorithm_info
        else {
            return Err(Failure::InvalidResponse);
        };
        self.client
            .encryption()
            .request_user_identity(&info.sender)
            .await
            .map_err(|_| Failure::Network)?;
        let devices = self
            .client
            .encryption()
            .get_user_devices(&info.sender)
            .await
            .map_err(|_| Failure::Network)?;
        let device = devices
            .devices()
            .find(|d| {
                d.curve25519_key()
                    .is_some_and(|key| key.to_base64() == *curve)
            })
            .ok_or(Failure::VerificationRequired)?;
        let pin = self.pin(&device)?;
        Ok((device, pin))
    }
    async fn incoming(&mut self, info: EncryptionInfo, raw: String) -> Result<(), Failure> {
        let wire: Wire = serde_json::from_str(&raw).map_err(|_| Failure::InvalidResponse)?;
        match &wire {
            Wire::Chunk { room, nonce, .. } | Wire::Refused { room, nonce, .. } => {
                if !self
                    .ledger
                    .correlated(room, nonce, info.sender.as_str(), now())
                {
                    return Err(Failure::InvalidResponse);
                }
            }
            Wire::Ack {
                room,
                nonce,
                response,
            } => {
                if !self.ledger.cached.values().any(|c| {
                    c.room == *room
                        && c.nonce == *nonce
                        && c.response == *response
                        && c.pin.user == info.sender.as_str()
                }) {
                    return Err(Failure::InvalidResponse);
                }
            }
            Wire::Request {
                room,
                nonce,
                expires,
            } => {
                self.diagnostics.incoming_requests =
                    self.diagnostics.incoming_requests.saturating_add(1);
                if !valid_id(nonce)
                    || *expires <= now()
                    || *expires > episode_expiry(now()).saturating_add(60)
                {
                    return Err(Failure::InvalidResponse);
                }
                let id = RoomId::parse(room).map_err(|_| Failure::InvalidResponse)?;
                if !self
                    .client
                    .get_room(&id)
                    .is_some_and(|r| r.state() == RoomState::Joined)
                {
                    return Err(Failure::MembershipChanged);
                }
                self.queries
                    .retain(|_, last| now().checked_sub(*last).is_none_or(|elapsed| elapsed < 60));
                let key = format!("{}\n{}", info.sender, room);
                if self.queries.contains_key(&key) || self.queries.len() >= 32 {
                    return Err(Failure::RateLimited);
                }
                self.queries.insert(key, now());
            }
        }
        let (device, pin) = self.authenticated(&info).await?;
        match wire {
            Wire::Request {
                room,
                nonce,
                expires,
            } => self.respond(room, nonce, expires, device, pin).await,
            Wire::Ack {
                room,
                nonce,
                response,
            } => self.acknowledged(room, nonce, response, pin).await,
            Wire::Refused {
                room,
                nonce,
                reason,
            } => {
                let pending = self
                    .ledger
                    .rooms
                    .get_mut(&room)
                    .ok_or(Failure::InvalidResponse)?;
                let attempt = pending
                    .attempts
                    .get_mut(&pin.key())
                    .ok_or(Failure::InvalidResponse)?;
                if !attempt.matches(&pin, &nonce, now()) || !pending.pins.contains(&pin) {
                    return Err(Failure::InvalidResponse);
                }
                attempt.terminal = reason != Failure::RateLimited;
                pending.partial_reason = Some(reason);
                let generation = pending.generation;
                let status = Self::status(pending);
                self.save().await?;
                self.publish(&room, generation, status).await;
                Ok(())
            }
            Wire::Chunk {
                room,
                nonce,
                response,
                count,
                index,
                bundle,
            } => {
                if let Some(completed) =
                    self.ledger.completed.get(&nonce_key(&pin, &nonce)).cloned()
                {
                    if completed.pin != pin
                        || completed.room != room
                        || completed.response != response
                        || completed.expires <= now()
                    {
                        return Err(Failure::InvalidResponse);
                    }
                    return self
                        .send(
                            &[device],
                            &Wire::Ack {
                                room,
                                nonce,
                                response,
                            },
                        )
                        .await;
                }
                let result = self
                    .accept_chunk(
                        &room,
                        &pin,
                        &nonce,
                        device,
                        response,
                        count,
                        index,
                        bundle,
                        raw.as_bytes(),
                    )
                    .await;
                if let Err(reason) = result
                    && let Some(p) = self.ledger.rooms.get_mut(&room)
                {
                    p.partial_reason = Some(reason);
                    let generation = p.generation;
                    let status = Self::status(p);
                    self.publish(&room, generation, status).await;
                }
                result
            }
        }
    }
    #[allow(clippy::too_many_arguments)]
    async fn accept_chunk(
        &mut self,
        room: &str,
        pin: &Pin,
        nonce: &str,
        device: Device,
        response: String,
        count: u16,
        index: u16,
        bundle: RoomKeyBundle,
        raw: &[u8],
    ) -> Result<(), Failure> {
        self.client
            .encryption()
            .request_user_identity(device.user_id())
            .await
            .map_err(|_| Failure::Network)?;
        let current = self
            .client
            .encryption()
            .get_device(device.user_id(), device.device_id())
            .await
            .map_err(|_| Failure::Network)?
            .ok_or(Failure::VerificationRequired)?;
        if self.pin(&current)? != *pin {
            return Err(Failure::VerificationRequired);
        }
        let facts = self.facts(room).await?;
        if !facts.joined.contains(&pin.user) {
            return Err(Failure::MembershipChanged);
        }
        let global_chunks: usize = self.ledger.rooms.values().map(Pending::chunk_count).sum();
        let pending = self
            .ledger
            .rooms
            .get_mut(room)
            .ok_or(Failure::InvalidResponse)?;
        if !pending.pins.contains(pin) {
            return Err(Failure::InvalidResponse);
        }
        let local_chunks = pending.chunk_count();
        let attempt = pending
            .attempts
            .get_mut(&pin.key())
            .ok_or(Failure::InvalidResponse)?;
        if !attempt.matches(pin, nonce, now()) {
            return Err(Failure::InvalidResponse);
        }
        let admitted = attempt.receipt.chunks.contains_key(&index);
        let user_bytes = pending.bytes_by_user.entry(pin.user.clone()).or_default();
        if !admitted
            && (global_chunks >= 8192
                || local_chunks >= 2048
                || *user_bytes + raw.len() > USER_LIMIT
                || pending.bytes + raw.len() > REQUEST_LIMIT)
        {
            return Err(Failure::TooLarge);
        }
        if !reserve_chunk(&mut attempt.receipt, &response, count, index, raw)? {
            if attempt.complete() {
                return self
                    .send(
                        &[device],
                        &Wire::Ack {
                            room: room.to_owned(),
                            nonce: nonce.to_owned(),
                            response,
                        },
                    )
                    .await;
            }
            return Ok(());
        }
        if !admitted {
            *user_bytes += raw.len();
            pending.bytes += raw.len();
        }
        self.save().await?;
        let id = RoomId::parse(room).map_err(|_| Failure::InvalidResponse)?;
        #[cfg(any(test, feature = "test-hooks"))]
        if let Some((reached, resume)) = self.import_pause.take() {
            let _ = reached.send(());
            let _ = resume.await;
        }
        let current = self.pinned_device(pin).await?;
        self.local_disclosure_allowed(room)?;
        let imported = self
            .client
            .encryption()
            .import_history_bundle_from_device(&id, &current, bundle)
            .await
            .map_err(|_| Failure::Storage)?
            .ok_or(Failure::InvalidResponse)?;
        let pending = self.ledger.rooms.get_mut(room).ok_or(Failure::Storage)?;
        self.diagnostics.imported_keys = self
            .diagnostics
            .imported_keys
            .saturating_add(imported.imported_count as u64);
        pending.received = pending
            .received
            .saturating_add(imported.imported_count as u32);
        let attempt = pending
            .attempts
            .get_mut(&pin.key())
            .ok_or(Failure::Storage)?;
        attempt.receipt.completed.insert(index);
        let complete = attempt.complete();
        let generation = pending.generation;
        let status = Self::status(pending);
        self.save().await?;
        self.publish(room, generation, status).await;
        if complete {
            self.send(
                &[current],
                &Wire::Ack {
                    room: room.to_owned(),
                    nonce: nonce.to_owned(),
                    response,
                },
            )
            .await?;
        }
        Ok(())
    }
}
fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}
fn random_id() -> Result<String, Failure> {
    let mut bytes = [0; 24];
    getrandom::fill(&mut bytes).map_err(|_| Failure::Storage)?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}
fn valid_id(id: &str) -> bool {
    id.len() == 48 && id.bytes().all(|b| b.is_ascii_hexdigit())
}
