//! Fast production-runtime SendQueue feedback integration lane.

use std::io;
use std::net::{Shutdown, SocketAddr, TcpListener, TcpStream, ToSocketAddrs};
use std::sync::{
    Arc, Condvar, Mutex,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use koushi_core::command::{AccountCommand, CoreCommand, SyncCommand, TimelineCommand};
use koushi_core::event::{
    AccountEvent, CoreEvent, SyncEvent, TimelineDiff, TimelineEvent, TimelineItem, TimelineItemId,
    TimelineMessageActions, TimelineSendState,
};
use koushi_core::ids::{AccountKey, RequestId, TimelineKey};
use koushi_core::runtime::{CoreConnection, CoreRuntime};
use koushi_state::{AuthSecret, MentionIntent, SessionState};
use matrix_sdk::{
    ruma::{event_id, room_id},
    test_utils::mocks::MatrixMockServer,
};
use matrix_sdk_test::JoinedRoomBuilder;

struct FastTcpProxy {
    listen_addr: SocketAddr,
    enabled: Arc<AtomicBool>,
    rejected_connections: Arc<AtomicUsize>,
    running: Arc<AtomicBool>,
    active_streams: Arc<Mutex<Vec<TcpStream>>>,
    accept_thread: Option<JoinHandle<()>>,
}

#[derive(Default)]
struct FastProxyRegistrationGate {
    state: Mutex<FastProxyRegistrationGateState>,
    changed: Condvar,
}

#[derive(Default)]
struct FastProxyRegistrationGateState {
    reached: bool,
    released: bool,
}

impl FastProxyRegistrationGate {
    fn pause_before_registration(&self) {
        let mut state = self.state.lock().expect("registration gate lock");
        state.reached = true;
        self.changed.notify_all();
        while !state.released {
            state = self
                .changed
                .wait(state)
                .expect("registration gate wait lock");
        }
    }

    fn wait_until_reached(&self, timeout: Duration) -> bool {
        let state = self.state.lock().expect("registration gate lock");
        let (state, _) = self
            .changed
            .wait_timeout_while(state, timeout, |state| !state.reached)
            .expect("registration gate timed wait lock");
        state.reached
    }

    fn release(&self) {
        let mut state = self.state.lock().expect("registration gate lock");
        state.released = true;
        self.changed.notify_all();
    }
}

impl FastTcpProxy {
    fn start(target_homeserver: &str) -> Result<Self, String> {
        Self::start_inner(target_homeserver, None)
    }

    fn start_with_registration_gate(
        target_homeserver: &str,
        registration_gate: Arc<FastProxyRegistrationGate>,
    ) -> Result<Self, String> {
        Self::start_inner(target_homeserver, Some(registration_gate))
    }

    fn start_inner(
        target_homeserver: &str,
        registration_gate: Option<Arc<FastProxyRegistrationGate>>,
    ) -> Result<Self, String> {
        let target = parse_http_homeserver_addr(target_homeserver)?;
        let listener = TcpListener::bind("127.0.0.1:0")
            .map_err(|error| format!("fast SendQueue proxy bind failed: {error}"))?;
        listener
            .set_nonblocking(true)
            .map_err(|error| format!("fast SendQueue proxy setup failed: {error}"))?;
        let listen_addr = listener
            .local_addr()
            .map_err(|error| format!("fast SendQueue proxy address failed: {error}"))?;
        let enabled = Arc::new(AtomicBool::new(true));
        let rejected_connections = Arc::new(AtomicUsize::new(0));
        let running = Arc::new(AtomicBool::new(true));
        let active_streams = Arc::new(Mutex::new(Vec::new()));
        let thread_enabled = Arc::clone(&enabled);
        let thread_rejected = Arc::clone(&rejected_connections);
        let thread_running = Arc::clone(&running);
        let thread_streams = Arc::clone(&active_streams);
        let thread_registration_gate = registration_gate.clone();
        let accept_thread = thread::spawn(move || {
            while thread_running.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((client, _)) if !thread_enabled.load(Ordering::SeqCst) => {
                        let _ = client.shutdown(Shutdown::Both);
                        thread_rejected.fetch_add(1, Ordering::SeqCst);
                    }
                    Ok((client, _)) => {
                        spawn_fast_proxy_pair(
                            client,
                            target,
                            Arc::clone(&thread_streams),
                            Arc::clone(&thread_enabled),
                            Arc::clone(&thread_rejected),
                            thread_registration_gate.clone(),
                        );
                    }
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(10));
                    }
                    Err(_) if thread_running.load(Ordering::SeqCst) => {
                        thread::sleep(Duration::from_millis(10));
                    }
                    Err(_) => break,
                }
            }
        });
        Ok(Self {
            listen_addr,
            enabled,
            rejected_connections,
            running,
            active_streams,
            accept_thread: Some(accept_thread),
        })
    }

    fn homeserver_url(&self) -> String {
        format!("http://{}", self.listen_addr)
    }

    fn rejected_connection_count(&self) -> usize {
        self.rejected_connections.load(Ordering::SeqCst)
    }

    fn disable(&self) {
        let mut streams = self.active_streams.lock().expect("active streams lock");
        self.enabled.store(false, Ordering::SeqCst);
        shutdown_fast_proxy_streams(&mut streams);
    }

    fn enable(&self) {
        self.enabled.store(true, Ordering::SeqCst);
    }
}

impl Drop for FastTcpProxy {
    fn drop(&mut self) {
        self.running.store(false, Ordering::SeqCst);
        self.disable();
        let _ = TcpStream::connect(self.listen_addr);
        if let Some(thread) = self.accept_thread.take() {
            let _ = thread.join();
        }
    }
}

fn parse_http_homeserver_addr(homeserver: &str) -> Result<SocketAddr, String> {
    let authority = homeserver
        .strip_prefix("http://")
        .ok_or_else(|| format!("fast SendQueue proxy requires http://, got {homeserver}"))?
        .split('/')
        .next()
        .unwrap_or_default();
    authority
        .to_socket_addrs()
        .map_err(|error| format!("fast SendQueue proxy resolution failed: {error}"))?
        .next()
        .ok_or_else(|| "fast SendQueue proxy resolved no address".to_owned())
}

fn spawn_fast_proxy_pair(
    client: TcpStream,
    target: SocketAddr,
    active_streams: Arc<Mutex<Vec<TcpStream>>>,
    enabled: Arc<AtomicBool>,
    rejected_connections: Arc<AtomicUsize>,
    registration_gate: Option<Arc<FastProxyRegistrationGate>>,
) {
    thread::spawn(move || {
        let Ok(upstream) = TcpStream::connect(target) else {
            let _ = client.shutdown(Shutdown::Both);
            return;
        };
        let Ok(client_guard) = client.try_clone() else {
            return;
        };
        let Ok(upstream_guard) = upstream.try_clone() else {
            return;
        };
        if let Some(registration_gate) = registration_gate {
            registration_gate.pause_before_registration();
        }
        let registered = if let Ok(mut streams) = active_streams.lock() {
            if enabled.load(Ordering::SeqCst) {
                streams.push(client_guard);
                streams.push(upstream_guard);
                true
            } else {
                rejected_connections.fetch_add(1, Ordering::SeqCst);
                false
            }
        } else {
            false
        };
        if !registered {
            let _ = client.shutdown(Shutdown::Both);
            let _ = upstream.shutdown(Shutdown::Both);
            return;
        }
        let Ok(mut client_reader) = client.try_clone() else {
            return;
        };
        let Ok(mut upstream_writer) = upstream.try_clone() else {
            return;
        };
        let upload = thread::spawn(move || {
            let _ = io::copy(&mut client_reader, &mut upstream_writer);
            let _ = upstream_writer.shutdown(Shutdown::Write);
        });
        let mut upstream_reader = upstream;
        let mut client_writer = client;
        let _ = io::copy(&mut upstream_reader, &mut client_writer);
        let _ = client_writer.shutdown(Shutdown::Write);
        let _ = upload.join();
    });
}

#[test]
fn fast_proxy_disable_rejects_connection_accepted_before_registration() {
    let upstream_listener = TcpListener::bind("127.0.0.1:0").expect("upstream bind");
    let upstream_addr = upstream_listener.local_addr().expect("upstream address");
    let (payload_tx, payload_rx) = std::sync::mpsc::channel();
    let upstream_thread = thread::spawn(move || {
        let (mut upstream, _) = upstream_listener.accept().expect("upstream accept");
        upstream
            .set_read_timeout(Some(Duration::from_secs(1)))
            .expect("upstream read timeout");
        let mut payload = [0_u8; 128];
        let read = io::Read::read(&mut upstream, &mut payload).unwrap_or(0);
        let _ = payload_tx.send(payload[..read].to_vec());
    });

    let registration_gate = Arc::new(FastProxyRegistrationGate::default());
    let proxy = FastTcpProxy::start_with_registration_gate(
        &format!("http://{upstream_addr}"),
        Arc::clone(&registration_gate),
    )
    .expect("proxy start");
    let mut client = TcpStream::connect(proxy.listen_addr).expect("proxy client connect");
    assert!(
        registration_gate.wait_until_reached(Duration::from_secs(1)),
        "accepted connection did not reach the pre-registration gate"
    );

    proxy.disable();
    registration_gate.release();
    let payload = b"GET /must-not-forward HTTP/1.1\r\nHost: localhost\r\n\r\n";
    let _ = io::Write::write_all(&mut client, payload);
    let received = payload_rx
        .recv_timeout(Duration::from_secs(2))
        .expect("upstream payload observation");
    let _ = upstream_thread.join();

    assert!(
        received.is_empty(),
        "connection accepted before disable forwarded payload after late registration"
    );
    assert_eq!(
        proxy.rejected_connection_count(),
        1,
        "late registration rejection must be counted"
    );
}

fn shutdown_fast_proxy_streams(streams: &mut Vec<TcpStream>) {
    for stream in streams.drain(..) {
        let _ = stream.shutdown(Shutdown::Both);
    }
}

async fn wait_for_logged_in(
    conn: &mut CoreConnection,
    request_id: RequestId,
    label: &str,
) -> Result<AccountKey, String> {
    loop {
        match conn
            .recv_event()
            .await
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?
        {
            CoreEvent::Account(AccountEvent::LoggedIn {
                request_id: event_request_id,
                account_key,
            }) if event_request_id == request_id => return Ok(account_key),
            CoreEvent::OperationFailed {
                request_id: event_request_id,
                failure,
            } if event_request_id == request_id => {
                return Err(format!("{label}: login failed: {failure:?}"));
            }
            _ => {}
        }
    }
}

async fn wait_for_session_restored(
    conn: &mut CoreConnection,
    request_id: RequestId,
    expected_account_key: &AccountKey,
    label: &str,
) -> Result<(), String> {
    loop {
        match conn
            .recv_event()
            .await
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?
        {
            CoreEvent::Account(AccountEvent::SessionRestored {
                request_id: event_request_id,
                account_key,
            }) if event_request_id == request_id => {
                if account_key == *expected_account_key {
                    return Ok(());
                }
                return Err(format!("{label}: restored account key mismatch"));
            }
            CoreEvent::OperationFailed {
                request_id: event_request_id,
                failure,
            } if event_request_id == request_id => {
                return Err(format!("{label}: restore failed: {failure:?}"));
            }
            _ => {}
        }
    }
}

async fn wait_for_ready_snapshot(conn: &mut CoreConnection, label: &str) -> Result<(), String> {
    loop {
        if matches!(conn.snapshot().session, SessionState::Ready(_)) {
            return Ok(());
        }
        conn.recv_event()
            .await
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;
    }
}

async fn wait_for_room_in_room_list(
    conn: &mut CoreConnection,
    expected_room_id: &str,
    label: &str,
) -> Result<(), String> {
    loop {
        if conn
            .snapshot()
            .rooms
            .iter()
            .any(|room| room.room_id == expected_room_id)
        {
            return Ok(());
        }
        conn.recv_event()
            .await
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;
    }
}

async fn stop_sync_for_qa(conn: &mut CoreConnection, label: &str) -> Result<(), String> {
    let request_id = conn.next_request_id();
    conn.command(CoreCommand::Sync(SyncCommand::Stop { request_id }))
        .await
        .map_err(|error| format!("{label}: submit Sync stop failed: {error}"))?;
    loop {
        match conn
            .recv_event()
            .await
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?
        {
            CoreEvent::Sync(SyncEvent::Stopped {
                request_id: Some(event_request_id),
            }) if event_request_id == request_id => return Ok(()),
            CoreEvent::OperationFailed {
                request_id: event_request_id,
                failure,
            } if event_request_id == request_id => {
                return Err(format!("{label}: stop sync failed: {failure:?}"));
            }
            _ => {}
        }
    }
}

async fn start_sync_and_wait_for_replacement_initial_items(
    conn: &mut CoreConnection,
    key: &TimelineKey,
    first: &SendQueueLocalEcho,
    second: &SendQueueLocalEcho,
    label: &str,
) -> Result<Vec<TimelineItem>, String> {
    let request_id = conn.next_request_id();
    conn.command(CoreCommand::Sync(SyncCommand::Start { request_id }))
        .await
        .map_err(|error| format!("{label}: submit Sync start failed: {error}"))?;

    let deadline = tokio::time::Instant::now() + FAST_SEND_QUEUE_PHASE_TIMEOUT;
    let mut started = false;
    let mut running = false;
    let mut replacement_items = None;
    let mut initial_items_seen = false;
    let mut first_restored = false;
    let mut first_state_restored = false;
    let mut second_restored = false;
    let mut second_state_restored = false;
    loop {
        let event = recv_fast_send_queue_event(conn, deadline, label)
            .await
            .map_err(|error| {
                format!(
                    "{error} started={started} running={running} initial_items_seen={initial_items_seen} \
                     first_restored={first_restored} first_state_restored={first_state_restored} \
                     second_restored={second_restored} \
                     second_state_restored={second_state_restored}"
                )
            })?;
        match event {
            CoreEvent::Sync(SyncEvent::Started {
                request_id: Some(event_request_id),
                ..
            }) if event_request_id == request_id => started = true,
            CoreEvent::Sync(SyncEvent::Running) => running = true,
            CoreEvent::Timeline(TimelineEvent::InitialItems {
                key: event_key,
                items,
                ..
            }) if event_key == *key => {
                initial_items_seen = true;
                let first_item = items.iter().find(|item| {
                    timeline_item_transaction_id(item) == Some(first.sdk_transaction_id.as_str())
                });
                let second_item = items.iter().find(|item| {
                    timeline_item_transaction_id(item) == Some(second.sdk_transaction_id.as_str())
                });
                first_restored = first_item.is_some();
                first_state_restored =
                    first_item.is_some_and(|item| item.send_state.as_ref().is_some());
                second_restored = second_item.is_some();
                second_state_restored =
                    second_item.is_some_and(|item| item.send_state.as_ref().is_some());
                if first_state_restored && second_state_restored {
                    replacement_items = Some(items);
                }
            }
            CoreEvent::OperationFailed {
                request_id: event_request_id,
                failure,
            } if event_request_id == request_id => {
                return Err(format!("{label}: Sync start failed: {failure:?}"));
            }
            _ => {}
        }
        if started
            && running
            && let Some(items) = replacement_items
        {
            return Ok(items);
        }
    }
}

async fn wait_for_initial_items_or_active_replay(
    conn: &mut CoreConnection,
    key: &TimelineKey,
    request_id: RequestId,
    label: &str,
) -> Result<Vec<TimelineItem>, String> {
    loop {
        match conn
            .recv_event()
            .await
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?
        {
            CoreEvent::Timeline(TimelineEvent::InitialItems {
                key: event_key,
                items,
                ..
            }) if event_key == *key => return Ok(items),
            CoreEvent::OperationFailed {
                request_id: event_request_id,
                failure,
            } if event_request_id == request_id => {
                return Err(format!("{label}: subscribe failed: {failure:?}"));
            }
            _ => {}
        }
    }
}

fn timeline_item_body_matches(item: &TimelineItem, expected_body: &str) -> bool {
    item.body
        .as_ref()
        .is_some_and(|body| body.contains(expected_body))
}

fn timeline_item_transaction_id(item: &TimelineItem) -> Option<&str> {
    match &item.id {
        TimelineItemId::Transaction { transaction_id } => Some(transaction_id),
        TimelineItemId::Event { .. } | TimelineItemId::Synthetic { .. } => None,
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
struct SendFlowOutcome {
    sdk_transaction_id: String,
    send_transaction_id: String,
    event_id: String,
}

#[derive(Debug, Clone, Eq, PartialEq)]
struct SendQueueLocalEcho {
    request_id: RequestId,
    client_transaction_id: String,
    sdk_transaction_id: String,
}

#[derive(Debug)]
struct SendFlowWaiter {
    request_id: RequestId,
    key: TimelineKey,
    expected_client_txn_id: String,
    expected_body: String,
    sdk_transaction_id: Option<String>,
    local_echo_send_state: Option<TimelineSendState>,
    send_transaction_id: Option<String>,
    event_id: Option<String>,
}

impl SendFlowWaiter {
    fn new(
        request_id: RequestId,
        key: TimelineKey,
        expected_client_txn_id: impl Into<String>,
        expected_body: impl Into<String>,
    ) -> Self {
        Self {
            request_id,
            key,
            expected_client_txn_id: expected_client_txn_id.into(),
            expected_body: expected_body.into(),
            sdk_transaction_id: None,
            local_echo_send_state: None,
            send_transaction_id: None,
            event_id: None,
        }
    }

    fn observe(&mut self, event: CoreEvent) -> Result<(), String> {
        match event {
            CoreEvent::Timeline(TimelineEvent::ItemsUpdated {
                key: event_key,
                diffs,
                ..
            }) if event_key == self.key => self.observe_local_echo(&diffs),
            CoreEvent::Timeline(TimelineEvent::SendCompleted {
                request_id: event_request_id,
                key: event_key,
                transaction_id,
                event_id,
            }) if event_request_id == self.request_id && event_key == self.key => {
                if transaction_id != self.expected_client_txn_id {
                    return Err(format!(
                        "send completed transaction mismatch: expected {}, got {transaction_id}",
                        self.expected_client_txn_id
                    ));
                }
                self.send_transaction_id = Some(transaction_id);
                self.event_id = Some(event_id);
            }
            CoreEvent::OperationFailed {
                request_id: event_request_id,
                failure,
            } if event_request_id == self.request_id => {
                return Err(format!("send flow failed: {failure:?}"));
            }
            _ => {}
        }
        if matches!(
            self.local_echo_send_state,
            Some(TimelineSendState::NotSent { .. })
        ) && self.send_transaction_id.is_none()
        {
            return Err("send flow entered NotSent before completion".to_owned());
        }
        Ok(())
    }

    fn observe_local_echo(&mut self, diffs: &[TimelineDiff]) {
        for item in diffs.iter().filter_map(|diff| match diff {
            TimelineDiff::PushBack { item }
            | TimelineDiff::PushFront { item }
            | TimelineDiff::Insert { item, .. }
            | TimelineDiff::Set { item, .. } => Some(item),
            TimelineDiff::Remove { .. }
            | TimelineDiff::Truncate { .. }
            | TimelineDiff::Clear
            | TimelineDiff::Reset { .. } => None,
        }) {
            if !timeline_item_body_matches(item, &self.expected_body) {
                continue;
            }
            if let Some(send_state) = &item.send_state {
                self.local_echo_send_state = Some(send_state.clone());
            }
            if let Some(transaction_id) = timeline_item_transaction_id(item)
                && self.sdk_transaction_id.is_none()
            {
                self.sdk_transaction_id = Some(transaction_id.to_owned());
            }
        }
    }

    fn is_complete(&self) -> bool {
        self.sdk_transaction_id.is_some()
            && self.send_transaction_id.is_some()
            && self.event_id.is_some()
    }

    fn finish(self) -> Result<SendFlowOutcome, String> {
        Ok(SendFlowOutcome {
            sdk_transaction_id: self
                .sdk_transaction_id
                .ok_or_else(|| "send flow: missing local echo".to_owned())?,
            send_transaction_id: self
                .send_transaction_id
                .ok_or_else(|| "send flow: missing SendCompleted".to_owned())?,
            event_id: self
                .event_id
                .ok_or_else(|| "send flow: missing event id".to_owned())?,
        })
    }
}

async fn retry_send_queue_item(
    conn: &mut CoreConnection,
    key: &TimelineKey,
    sdk_transaction_id: &str,
    label: &str,
) -> Result<RequestId, String> {
    let request_id = conn.next_request_id();
    conn.command(CoreCommand::Timeline(TimelineCommand::RetrySend {
        request_id,
        key: key.clone(),
        transaction_id: sdk_transaction_id.to_owned(),
    }))
    .await
    .map_err(|error| format!("{label}: submit RetrySend failed: {error}"))?;
    Ok(request_id)
}

async fn cancel_send_queue_item(
    conn: &mut CoreConnection,
    key: &TimelineKey,
    sdk_transaction_id: &str,
    label: &str,
) -> Result<RequestId, String> {
    let request_id = conn.next_request_id();
    conn.command(CoreCommand::Timeline(TimelineCommand::CancelSend {
        request_id,
        key: key.clone(),
        transaction_id: sdk_transaction_id.to_owned(),
    }))
    .await
    .map_err(|error| format!("{label}: submit CancelSend failed: {error}"))?;
    Ok(request_id)
}

fn projection_timeline_item(event_id: &str, is_redacted: bool) -> TimelineItem {
    TimelineItem {
        id: TimelineItemId::Event {
            event_id: event_id.to_owned(),
        },
        sender: Some("@projection:example.invalid".to_owned()),
        sender_label: None,
        sender_avatar: None,
        body: (!is_redacted).then(|| "projection body".to_owned()),
        notice_i18n: None,
        message_kind: Default::default(),
        spoiler_spans: Vec::new(),
        timestamp_ms: None,
        in_reply_to_event_id: None,
        formatted: None,
        reply_quote: None,
        thread_root: None,
        thread_summary: None,
        media: None,
        link_previews: None,
        link_ranges: Vec::new(),
        reactions: Vec::new(),
        can_react: false,
        is_redacted,
        is_hidden: false,
        can_redact: false,
        is_edited: false,
        can_edit: false,
        actions: TimelineMessageActions::default(),
        send_state: None,
        unable_to_decrypt: None,
    }
}

const FAST_SEND_QUEUE_PHASE_TIMEOUT: Duration = Duration::from_secs(5);
const FAST_SEND_QUEUE_TOTAL_TIMEOUT: Duration = Duration::from_secs(55);
const FAST_SEND_QUEUE_SHORT_RETRY_ATTEMPTS: usize = 3;
const FAST_SEND_QUEUE_RETRY_VIRTUAL_BUDGET: Duration = Duration::from_secs(30);
const FAST_SEND_QUEUE_RETRY_PUMP_STEP: Duration = Duration::from_millis(50);

struct FastSendQueuePausedTime;

impl FastSendQueuePausedTime {
    fn start() -> Self {
        tokio::time::pause();
        Self
    }
}

impl Drop for FastSendQueuePausedTime {
    fn drop(&mut self) {
        tokio::time::resume();
    }
}

async fn coordinate_fast_send_queue_paused_attempts<T>(
    send: impl std::future::Future<Output = Result<T, String>>,
    attempts: impl std::future::Future<Output = Result<(), String>>,
    attempt_wall_timeout: Duration,
) -> Result<T, String> {
    let paused_time = FastSendQueuePausedTime::start();
    tokio::pin!(send);
    tokio::pin!(attempts);
    tokio::select! {
        attempts_result = &mut attempts => {
            drop(paused_time);
            attempts_result?;
            tokio::time::timeout(FAST_SEND_QUEUE_PHASE_TIMEOUT, &mut send)
                .await
                .map_err(|_| {
                    "fast_send_queue send timed out after retry attempts completed".to_owned()
                })?
        }
        send_result = &mut send => {
            let attempts_result =
                fast_send_queue_wall_timeout(attempt_wall_timeout, &mut attempts).await;
            drop(paused_time);
            let attempts_result = attempts_result.ok_or_else(|| {
                "fast_send_queue retry attempts timed out after send completed".to_owned()
            })?;
            attempts_result?;
            send_result
        }
    }
}

async fn fast_send_queue_wall_timeout<T>(
    duration: Duration,
    future: impl std::future::Future<Output = T>,
) -> Option<T> {
    let (elapsed_tx, elapsed_rx) = tokio::sync::oneshot::channel();
    thread::spawn(move || {
        thread::sleep(duration);
        let _ = elapsed_tx.send(());
    });
    tokio::pin!(future);
    tokio::select! {
        value = &mut future => Some(value),
        _ = elapsed_rx => None,
    }
}

struct FastPendingResultDropProbe {
    dropped: Arc<AtomicBool>,
}

impl std::future::Future for FastPendingResultDropProbe {
    type Output = Result<(), String>;

    fn poll(
        self: std::pin::Pin<&mut Self>,
        _context: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Self::Output> {
        std::task::Poll::Pending
    }
}

impl Drop for FastPendingResultDropProbe {
    fn drop(&mut self) {
        self.dropped.store(true, Ordering::SeqCst);
    }
}

#[tokio::test]
async fn fast_send_queue_attempt_error_cancels_pending_send_and_resumes_time() {
    let send_dropped = Arc::new(AtomicBool::new(false));
    let result = fast_send_queue_wall_timeout(
        Duration::from_millis(100),
        coordinate_fast_send_queue_paused_attempts(
            FastPendingResultDropProbe {
                dropped: Arc::clone(&send_dropped),
            },
            async { Err("attempt driver failed".to_owned()) },
            Duration::from_millis(10),
        ),
    )
    .await
    .expect("attempt error must return without waiting for the pending send");
    assert_eq!(result, Err("attempt driver failed".to_owned()));
    assert!(
        send_dropped.load(Ordering::SeqCst),
        "pending send future must be cancelled before returning the attempt error"
    );

    fast_send_queue_wall_timeout(
        Duration::from_millis(100),
        tokio::time::sleep(Duration::from_millis(1)),
    )
    .await
    .expect("Tokio time must resume before the attempt error is returned");
}

#[tokio::test]
async fn fast_send_queue_send_success_times_out_pending_attempts_and_resumes_time() {
    let attempts_dropped = Arc::new(AtomicBool::new(false));
    let result = fast_send_queue_wall_timeout(
        Duration::from_millis(100),
        coordinate_fast_send_queue_paused_attempts(
            async { Ok(()) },
            FastPendingResultDropProbe {
                dropped: Arc::clone(&attempts_dropped),
            },
            Duration::from_millis(10),
        ),
    )
    .await
    .expect("pending attempts must be wall-bounded after the send completes");
    assert_eq!(
        result,
        Err("fast_send_queue retry attempts timed out after send completed".to_owned())
    );
    assert!(
        attempts_dropped.load(Ordering::SeqCst),
        "pending attempts future must be cancelled before returning the timeout"
    );

    fast_send_queue_wall_timeout(
        Duration::from_millis(100),
        tokio::time::sleep(Duration::from_millis(1)),
    )
    .await
    .expect("Tokio time must resume before the attempt timeout is returned");
}

async fn fast_send_queue_phase<T>(
    label: &str,
    future: impl std::future::Future<Output = T>,
) -> Result<T, String> {
    tokio::time::timeout(FAST_SEND_QUEUE_PHASE_TIMEOUT, future)
        .await
        .map_err(|_| format!("{label}: timed out"))
}

async fn configure_fast_send_queue_trust(runtime: &CoreRuntime) -> Result<(), String> {
    let configured = runtime
        .configure_trust_observation_for_testing(koushi_sdk::CurrentDeviceTrustObservation {
            current: koushi_state::CurrentDeviceTrustState::Verified,
            updates: Box::pin(futures_util::stream::pending()),
        })
        .await;
    if configured {
        Ok(())
    } else {
        Err("fast_send_queue trust observation actor unavailable".to_owned())
    }
}

fn apply_fast_send_queue_diffs(
    items: &mut Vec<TimelineItem>,
    diffs: &[TimelineDiff],
    label: &str,
) -> Result<(), String> {
    for diff in diffs {
        match diff {
            TimelineDiff::PushFront { item } => items.insert(0, item.clone()),
            TimelineDiff::PushBack { item } => items.push(item.clone()),
            TimelineDiff::Insert { index, item } if *index <= items.len() => {
                items.insert(*index, item.clone());
            }
            TimelineDiff::Set { index, item } if *index < items.len() => {
                items[*index] = item.clone();
            }
            TimelineDiff::Remove { index } if *index < items.len() => {
                items.remove(*index);
            }
            TimelineDiff::Truncate { length } => items.truncate(*length),
            TimelineDiff::Clear => items.clear(),
            TimelineDiff::Reset { items: reset } => items.clone_from(reset),
            TimelineDiff::Insert { index, .. }
            | TimelineDiff::Set { index, .. }
            | TimelineDiff::Remove { index } => {
                return Err(format!(
                    "{label}: display diff index {index} outside projection length {}",
                    items.len()
                ));
            }
        }
    }
    Ok(())
}

fn apply_fast_send_queue_event(
    items: &mut Vec<TimelineItem>,
    key: &TimelineKey,
    event: &CoreEvent,
    label: &str,
) -> Result<(), String> {
    if let CoreEvent::Timeline(TimelineEvent::ItemsUpdated {
        key: event_key,
        diffs,
        ..
    }) = event
        && event_key == key
    {
        apply_fast_send_queue_diffs(items, diffs, label)?;
    }
    Ok(())
}

async fn recv_fast_send_queue_event(
    conn: &mut CoreConnection,
    deadline: tokio::time::Instant,
    label: &str,
) -> Result<CoreEvent, String> {
    tokio::time::timeout_at(deadline, conn.recv_event())
        .await
        .map_err(|_| format!("{label}: timed out waiting for CoreEvent"))?
        .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))
}

async fn drive_fast_send_queue_short_retry_attempts(
    proxy: &FastTcpProxy,
    baseline: usize,
    phase: &str,
) -> Result<(), String> {
    let wall_deadline = std::time::Instant::now() + FAST_SEND_QUEUE_PHASE_TIMEOUT;
    let expected = baseline + FAST_SEND_QUEUE_SHORT_RETRY_ATTEMPTS;
    let mut virtual_advanced = Duration::ZERO;

    loop {
        let observed = proxy.rejected_connection_count();
        if observed > expected {
            return Err(format!(
                "fast_send_queue phase={phase} rejected_connections={} expected={}",
                observed - baseline,
                FAST_SEND_QUEUE_SHORT_RETRY_ATTEMPTS
            ));
        }
        if observed == expected {
            eprintln!(
                "fast_send_queue phase={phase} rejected_connections={}",
                observed - baseline
            );
            return Ok(());
        }
        if std::time::Instant::now() >= wall_deadline {
            return Err(format!(
                "fast_send_queue phase={phase} rejected_connections={} expected={}",
                observed - baseline,
                FAST_SEND_QUEUE_SHORT_RETRY_ATTEMPTS
            ));
        }
        if virtual_advanced >= FAST_SEND_QUEUE_RETRY_VIRTUAL_BUDGET {
            return Err(format!(
                "fast_send_queue phase={phase} virtual_budget_exhausted=1 rejected_connections={} expected={}",
                observed - baseline,
                FAST_SEND_QUEUE_SHORT_RETRY_ATTEMPTS
            ));
        }
        for _ in 0..64 {
            tokio::task::yield_now().await;
        }
        tokio::time::advance(FAST_SEND_QUEUE_RETRY_PUMP_STEP).await;
        virtual_advanced += FAST_SEND_QUEUE_RETRY_PUMP_STEP;
    }
}

async fn wait_for_fast_send_queue_pending_removal(
    conn: &mut CoreConnection,
    projection: &mut Vec<TimelineItem>,
    key: &TimelineKey,
    expected_body: &str,
    label: &str,
) -> Result<(), String> {
    let deadline = tokio::time::Instant::now() + FAST_SEND_QUEUE_PHASE_TIMEOUT;
    loop {
        let (events, transactions) = fast_send_queue_projection_counts(projection, expected_body);
        if events > 1 || transactions > 1 {
            return Err(format!(
                "{label}: duplicate send projection events={events} transactions={transactions}"
            ));
        }
        if transactions == 0 {
            return Ok(());
        }
        let event = recv_fast_send_queue_event(conn, deadline, label).await?;
        apply_fast_send_queue_event(projection, key, &event, label)?;
    }
}

async fn wait_for_fast_send_queue_authoritative_completion(
    conn: &mut CoreConnection,
    projection: &mut Vec<TimelineItem>,
    key: &TimelineKey,
    send_request_id: RequestId,
    mut retry_request_id: Option<RequestId>,
    sdk_transaction_id: &str,
    expected_transaction_id: &str,
    expected_body: &str,
    expected_event_id: &str,
    label: &str,
) -> Result<String, String> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(event_id) = fast_send_queue_authoritative_projection(
            projection,
            expected_body,
            expected_event_id,
            label,
        )? {
            return Ok(event_id);
        }
        if retry_request_id.is_none()
            && projection.iter().any(|item| {
                timeline_item_transaction_id(item) == Some(sdk_transaction_id)
                    && matches!(
                        item.send_state,
                        Some(TimelineSendState::NotSent {
                            reason: koushi_core::event::TimelineSendFailureReason::Recoverable,
                        })
                    )
            })
        {
            retry_request_id = Some(
                fast_send_queue_phase(
                    "fast_send_queue restored retry command",
                    retry_send_queue_item(conn, key, sdk_transaction_id, label),
                )
                .await??,
            );
        }

        let event = recv_fast_send_queue_event(conn, deadline, label).await?;
        apply_fast_send_queue_event(projection, key, &event, label)?;

        match event {
            CoreEvent::Timeline(TimelineEvent::SendCompleted {
                request_id,
                key: ref event_key,
                transaction_id,
                event_id,
            }) if request_id == send_request_id && event_key == key => {
                if transaction_id != expected_transaction_id {
                    return Err(format!(
                        "{label}: completion transaction mismatch: {transaction_id}"
                    ));
                }
                if event_id != expected_event_id {
                    return Err(format!("{label}: completion event mismatch: {event_id}"));
                }
            }
            CoreEvent::OperationFailed {
                request_id,
                failure,
            } if request_id == send_request_id || retry_request_id == Some(request_id) => {
                return Err(format!("{label}: operation failed: {failure:?}"));
            }
            _ => {}
        }
    }
}

async fn wait_for_fast_send_queue_flow_completion(
    conn: &mut CoreConnection,
    projection: &mut Vec<TimelineItem>,
    request_id: RequestId,
    key: &TimelineKey,
    client_transaction_id: &str,
    expected_body: &str,
    label: &str,
) -> Result<SendFlowOutcome, String> {
    let mut waiter = SendFlowWaiter::new(
        request_id,
        key.clone(),
        client_transaction_id,
        expected_body,
    );
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        let event = recv_fast_send_queue_event(conn, deadline, label).await?;
        apply_fast_send_queue_event(projection, key, &event, label)?;
        waiter.observe(event)?;
        if waiter.is_complete() {
            return waiter.finish();
        }
    }
}

async fn send_fast_send_queue_text_expect_local_echo(
    conn: &mut CoreConnection,
    projection: &mut Vec<TimelineItem>,
    key: &TimelineKey,
    client_transaction_id: &str,
    body: &str,
    label: &str,
) -> Result<SendQueueLocalEcho, String> {
    let request_id = conn.next_request_id();
    fast_send_queue_phase(
        label,
        conn.command(CoreCommand::Timeline(TimelineCommand::SendText {
            request_id,
            key: key.clone(),
            transaction_id: client_transaction_id.to_owned(),
            body: body.to_owned(),
            mentions: MentionIntent::default(),
        })),
    )
    .await
    .map_err(|error| format!("{label}: submit SendText timed out: {error}"))?
    .map_err(|error| format!("{label}: submit SendText failed: {error}"))?;

    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        let event = recv_fast_send_queue_event(conn, deadline, label).await?;
        apply_fast_send_queue_event(projection, key, &event, label)?;
        if let CoreEvent::OperationFailed {
            request_id: event_request_id,
            failure,
        } = &event
            && *event_request_id == request_id
        {
            return Err(format!(
                "{label}: send failed before local echo: {failure:?}"
            ));
        }
        if let Some(sdk_transaction_id) = projection.iter().find_map(|item| {
            (timeline_item_body_matches(item, body))
                .then(|| timeline_item_transaction_id(item))
                .flatten()
                .map(str::to_owned)
        }) {
            return Ok(SendQueueLocalEcho {
                request_id,
                client_transaction_id: client_transaction_id.to_owned(),
                sdk_transaction_id,
            });
        }
    }
}

async fn wait_for_fast_send_queue_not_sent(
    conn: &mut CoreConnection,
    projection: &mut Vec<TimelineItem>,
    key: &TimelineKey,
    send: &SendQueueLocalEcho,
    label: &str,
) -> Result<(), String> {
    let deadline = tokio::time::Instant::now() + FAST_SEND_QUEUE_RETRY_VIRTUAL_BUDGET;
    loop {
        if let Some(item) = projection.iter().find(|item| {
            timeline_item_transaction_id(item) == Some(send.sdk_transaction_id.as_str())
        }) {
            match item.send_state {
                Some(TimelineSendState::NotSent {
                    reason: koushi_core::event::TimelineSendFailureReason::Recoverable,
                }) => return Ok(()),
                Some(TimelineSendState::NotSent {
                    reason: koushi_core::event::TimelineSendFailureReason::Unrecoverable,
                }) => {
                    return Err(format!(
                        "{label}: expected recoverable transport failure, got unrecoverable NotSent"
                    ));
                }
                _ => {}
            }
        }
        let event = recv_fast_send_queue_event(conn, deadline, label).await?;
        apply_fast_send_queue_event(projection, key, &event, label)?;
        if let CoreEvent::OperationFailed {
            request_id,
            failure,
        } = event
            && request_id == send.request_id
        {
            return Err(format!("{label}: operation failed: {failure:?}"));
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn send_fast_send_queue_text_expect_recoverable_transport_failure(
    conn: &mut CoreConnection,
    projection: &mut Vec<TimelineItem>,
    key: &TimelineKey,
    client_transaction_id: &str,
    body: &str,
    label: &str,
    attempt_phase: &str,
    proxy: &FastTcpProxy,
) -> Result<SendQueueLocalEcho, String> {
    let baseline = proxy.rejected_connection_count();
    coordinate_fast_send_queue_paused_attempts(
        async {
            let send = send_fast_send_queue_text_expect_local_echo(
                conn,
                projection,
                key,
                client_transaction_id,
                body,
                label,
            )
            .await?;
            wait_for_fast_send_queue_not_sent(conn, projection, key, &send, label).await?;
            Ok::<_, String>(send)
        },
        drive_fast_send_queue_short_retry_attempts(proxy, baseline, attempt_phase),
        FAST_SEND_QUEUE_PHASE_TIMEOUT,
    )
    .await
}

async fn wait_for_fast_send_queue_completions_in_order(
    conn: &mut CoreConnection,
    projection: &mut Vec<TimelineItem>,
    key: &TimelineKey,
    retry_request_id: RequestId,
    first: &SendQueueLocalEcho,
    second: &SendQueueLocalEcho,
    label: &str,
) -> Result<(), String> {
    let mut first_completion_count = 0;
    let mut second_completion_count = 0;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        let event = recv_fast_send_queue_event(conn, deadline, label)
            .await
            .map_err(|error| {
                format!(
                    "{error} first_completion_count={first_completion_count} \
                     second_completion_count={second_completion_count}"
                )
            })?;
        apply_fast_send_queue_event(projection, key, &event, label)?;
        match event {
            CoreEvent::Timeline(TimelineEvent::SendCompleted {
                request_id,
                key: ref event_key,
                transaction_id,
                ..
            }) if request_id == first.request_id && event_key == key => {
                if transaction_id != first.client_transaction_id {
                    return Err(format!(
                        "{label}: first completion transaction mismatch: {transaction_id}"
                    ));
                }
                first_completion_count += 1;
                if first_completion_count != 1 {
                    return Err(format!(
                        "{label}: first completion was emitted more than once"
                    ));
                }
            }
            CoreEvent::Timeline(TimelineEvent::SendCompleted {
                request_id,
                key: ref event_key,
                transaction_id,
                ..
            }) if request_id == second.request_id && event_key == key => {
                if first_completion_count != 1 {
                    return Err(format!(
                        "{label}: later queued send completed before the failed predecessor"
                    ));
                }
                if transaction_id != second.client_transaction_id {
                    return Err(format!(
                        "{label}: second completion transaction mismatch: {transaction_id}"
                    ));
                }
                second_completion_count += 1;
                if second_completion_count != 1 {
                    return Err(format!(
                        "{label}: second completion was emitted more than once"
                    ));
                }
                return Ok(());
            }
            CoreEvent::Timeline(TimelineEvent::SendCompleted {
                request_id,
                key: ref event_key,
                ..
            }) if request_id == retry_request_id && event_key == key => {
                return Err(format!(
                    "{label}: retry request id must not own SendCompleted"
                ));
            }
            CoreEvent::OperationFailed {
                request_id,
                failure,
            } if request_id == retry_request_id
                || request_id == first.request_id
                || request_id == second.request_id =>
            {
                return Err(format!("{label}: operation failed: {failure:?}"));
            }
            _ => {}
        }
    }
}

async fn wait_for_fast_send_queue_cancelled_or_removed(
    conn: &mut CoreConnection,
    projection: &mut Vec<TimelineItem>,
    key: &TimelineKey,
    cancel_request_id: RequestId,
    sdk_transaction_id: &str,
    label: &str,
) -> Result<(), String> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        if projection
            .iter()
            .all(|item| timeline_item_transaction_id(item) != Some(sdk_transaction_id))
        {
            return Ok(());
        }
        let event = recv_fast_send_queue_event(conn, deadline, label).await?;
        apply_fast_send_queue_event(projection, key, &event, label)?;
        if let CoreEvent::OperationFailed {
            request_id,
            failure,
        } = event
            && request_id == cancel_request_id
        {
            return Err(format!("{label}: cancel failed: {failure:?}"));
        }
    }
}

fn fast_send_queue_authoritative_projection(
    items: &[TimelineItem],
    expected_body: &str,
    expected_event_id: &str,
    label: &str,
) -> Result<Option<String>, String> {
    let mut event_ids = Vec::new();
    let mut transactions = 0;
    for item in items
        .iter()
        .filter(|item| timeline_item_body_matches(item, expected_body))
    {
        match &item.id {
            TimelineItemId::Event { event_id } => event_ids.push(event_id.as_str()),
            TimelineItemId::Transaction { .. } => transactions += 1,
            TimelineItemId::Synthetic { .. } => {}
        }
    }

    if event_ids.len() > 1 || transactions > 1 {
        return Err(format!(
            "{label}: duplicate send projection events={} transactions={transactions}",
            event_ids.len()
        ));
    }
    if let Some(event_id) = event_ids.first()
        && *event_id != expected_event_id
    {
        return Err(format!("{label}: authoritative event mismatch: {event_id}"));
    }
    if event_ids.len() == 1 && transactions == 0 {
        return Ok(Some(expected_event_id.to_owned()));
    }
    Ok(None)
}

fn fast_send_queue_projection_counts(
    items: &[TimelineItem],
    expected_body: &str,
) -> (usize, usize) {
    items
        .iter()
        .filter(|item| timeline_item_body_matches(item, expected_body))
        .fold((0, 0), |(events, transactions), item| match item.id {
            TimelineItemId::Event { .. } => (events + 1, transactions),
            TimelineItemId::Transaction { .. } => (events, transactions + 1),
            TimelineItemId::Synthetic { .. } => (events, transactions),
        })
}

fn validate_fast_send_queue_projection_counts(
    items: &[TimelineItem],
    expected_body: &str,
    expected_counts: (usize, usize),
    phase: &str,
) -> Result<(), String> {
    let actual = fast_send_queue_projection_counts(items, expected_body);
    if actual == expected_counts {
        Ok(())
    } else {
        Err(format!(
            "{phase}: projection counts mismatch: actual={actual:?} expected={expected_counts:?}"
        ))
    }
}

fn assert_fast_send_queue_success_projection(
    items: &[TimelineItem],
    expected_body: &str,
    phase: &str,
) {
    validate_fast_send_queue_projection_counts(items, expected_body, (1, 0), phase)
        .expect("success projection must contain one Event and no Transaction row");
}

fn assert_fast_send_queue_cancel_projection(
    items: &[TimelineItem],
    expected_body: &str,
    phase: &str,
) {
    validate_fast_send_queue_projection_counts(items, expected_body, (0, 0), phase)
        .expect("cancelled projection must contain neither an Event nor a Transaction row");
}

#[test]
fn fast_send_queue_cancel_projection_rejects_event_rows() {
    let mut event = projection_timeline_item("$cancelled:localhost", false);
    event.body = Some("fast cancel body".to_owned());
    let projection = vec![event];

    let rejected = validate_fast_send_queue_projection_counts(
        &projection,
        "fast cancel body",
        (0, 0),
        "fast_send_queue cancel regression",
    );
    assert!(
        rejected.is_err(),
        "cancel assertion must reject a surviving Event row"
    );
}

#[test]
fn fast_send_queue_lane_hard_bounds_generic_lifecycle_phases() {
    let source = include_str!("send_queue_fast.rs");
    let lane = source
        .rsplit("async fn run_fast_send_queue_feedback")
        .next()
        .and_then(|section| {
            section
                .split(
                    "async fn fast_send_queue_feedback_runs_production_runtime_without_homeserver",
                )
                .next()
        })
        .unwrap_or("");
    let compact = lane
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .replace("( ", "(");

    assert!(
        source.contains("timeout(FAST_SEND_QUEUE_TOTAL_TIMEOUT"),
        "fast SendQueue lane must have a hard whole-test timeout"
    );
    for phase in [
        "fast_send_queue initial trust configure",
        "fast_send_queue login command",
        "fast_send_queue LoggedIn event",
        "fast_send_queue ready snapshot",
        "fast_send_queue room-list snapshot",
        "fast_send_queue initial stop sync",
        "fast_send_queue initial subscribe command",
        "fast_send_queue initial subscribe replay",
        "fast_send_queue replacement sync start",
        "fast_send_queue stop replacement sync",
        "fast_send_queue first retry command",
        "fast_send_queue cancel command",
        "fast_send_queue first shutdown barrier",
        "fast_send_queue restored trust configure",
        "fast_send_queue restore command",
        "fast_send_queue SessionRestored event",
        "fast_send_queue restored ready snapshot",
        "fast_send_queue restored stop sync",
        "fast_send_queue restored subscribe command",
        "fast_send_queue restored subscribe replay",
        "fast_send_queue restored retry command",
        "fast_send_queue final shutdown barrier",
    ] {
        assert!(
            compact.contains(&format!("fast_send_queue_phase(\"{phase}\",")),
            "generic lifecycle phase is not fast-bounded: {phase}"
        );
    }
}

#[test]
fn send_queue_stage_uses_active_replay_waiter_for_both_subscriptions() {
    let source = include_str!("../src/bin/headless-core-qa.rs");
    let send_queue_stage = source
        .split("\nasync fn run_send_queue_stage(")
        .nth(1)
        .expect("run_send_queue_stage body")
        .split("\nasync fn unsubscribe_timeline_for_qa(")
        .next()
        .expect("run_send_queue_stage body end");

    assert_eq!(
        send_queue_stage
            .matches("wait_for_initial_items_or_active_replay(")
            .count(),
        2,
        "initial and restored SendQueue subscribes must both accept same-key replay InitialItems"
    );
    assert_eq!(
        send_queue_stage.matches("wait_for_initial_items(").count(),
        0,
        "SendQueue subscribes must not require their fresh request id on an active timeline"
    );
}

#[test]
fn headless_send_queue_diagnostic_contract_counts_forwarded_and_completed_room_sends() {
    let source = include_str!("../src/bin/headless-core-qa.rs");
    let classifier = source
        .split("\nfn qa_proxy_request_kind(")
        .nth(1)
        .expect("QA proxy request classifier")
        .split("\nfn qa_messages_proxy_action(")
        .next()
        .expect("QA proxy request classifier end");
    assert!(classifier.contains("(\"PUT\", path)"));
    assert!(classifier.contains("path.contains(\"/rooms/\")"));
    assert!(classifier.contains("path.contains(\"/send/\")"));
    assert!(classifier.contains("QaProxyRequestKind::RoomSend"));

    let proxy_request = source
        .split("\nfn proxy_single_http_request(")
        .nth(1)
        .expect("QA proxy request forwarding body")
        .split("\nfn qa_proxy_request_kind(")
        .next()
        .expect("QA proxy request forwarding body end");
    let compact_proxy_request = proxy_request
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    assert!(compact_proxy_request.contains(
        "request_kind == QaProxyRequestKind::RoomSend && action == QaProxyRequestAction::Forward"
    ));
    let counter_increment = proxy_request
        .find("room_send_forwarded.fetch_add(1, Ordering::SeqCst);")
        .expect("forwarded RoomSend counter increment");
    let upstream_write = proxy_request
        .find("io::Write::write_all(&mut server, &request)?;")
        .expect("QA proxy upstream write");
    assert!(
        counter_increment < upstream_write,
        "RoomSend counter must increment immediately before its upstream write"
    );
    let upstream_copy = proxy_request
        .find("io::copy(&mut server, client)?;")
        .expect("QA proxy successful response copy");
    let completed_increment = proxy_request
        .find("room_send_responses_completed.fetch_add(1, Ordering::SeqCst);")
        .expect("completed forwarded RoomSend response counter increment");
    assert!(
        upstream_copy < completed_increment,
        "completed RoomSend counter must increment only after response copy succeeds"
    );
    assert!(source.contains("room_send_forwarded: Arc<AtomicUsize>"));
    assert!(source.contains("fn room_send_forwarded_count(&self) -> usize"));
    assert!(source.contains("room_send_responses_completed: Arc<AtomicUsize>"));
    assert!(source.contains("fn room_send_responses_completed_count(&self) -> usize"));
}

#[test]
fn headless_send_queue_diagnostic_contract_wraps_fifo_failure_with_proxy_deltas() {
    let source = include_str!("../src/bin/headless-core-qa.rs");
    let send_queue_stage = source
        .split("\nasync fn run_send_queue_stage(")
        .nth(1)
        .expect("run_send_queue_stage body")
        .split("\nasync fn unsubscribe_timeline_for_qa(")
        .next()
        .expect("run_send_queue_stage body end");
    let retry_stage = send_queue_stage
        .split("    proxy.enable();")
        .nth(1)
        .expect("FIFO retry stage")
        .split("    println!(\"resend=ok\");")
        .next()
        .expect("FIFO retry stage end");
    let compact_retry_stage = retry_stage.split_whitespace().collect::<Vec<_>>().join(" ");

    let baseline = compact_retry_stage
        .find("let room_send_forwarded_before_retry = proxy.room_send_forwarded_count();")
        .expect("RoomSend baseline immediately before FIFO retry");
    let completed_baseline = compact_retry_stage
        .find("let room_send_responses_completed_before_retry = proxy.room_send_responses_completed_count();")
        .expect("completed RoomSend response baseline immediately before FIFO retry");
    let retry = compact_retry_stage
        .find("let retry_id = retry_send_queue_item(")
        .expect("first FIFO retry command");
    assert!(baseline < retry, "RoomSend baseline must precede the retry");
    assert!(
        completed_baseline < retry,
        "completed response baseline must precede the retry"
    );
    assert!(retry_stage.contains("room_send_forwarded_after_retry={}"));
    assert!(retry_stage.contains("room_send_responses_completed_after_retry={}"));
    assert!(retry_stage.contains("saturating_sub(room_send_forwarded_before_retry)"));
    assert!(retry_stage.contains("saturating_sub(room_send_responses_completed_before_retry)"));
}

#[test]
fn headless_send_queue_diagnostic_contract_arms_before_private_safe_not_sent_failure() {
    let source = include_str!("../src/bin/headless-core-qa.rs");
    let observer = source
        .split("\nfn observe_send_queue_retry_item_state(")
        .nth(1)
        .expect("causally fenced private-safe SendQueue state observer")
        .split("\nasync fn wait_for_send_completions_in_order(")
        .next()
        .expect("causally fenced private-safe SendQueue state observer end");
    assert!(observer.contains("first_left_not_sent_after_retry: &mut bool"));
    assert!(observer.contains("if *first_left_not_sent_after_retry"));
    assert!(observer.contains("*first_left_not_sent_after_retry = true;"));
    assert!(observer.contains("TimelineSendFailureReason::Recoverable"));
    assert!(observer.contains("Some(\"recoverable\")"));
    assert!(observer.contains("TimelineSendFailureReason::Unrecoverable"));
    assert!(observer.contains("Some(\"unrecoverable\")"));
    assert!(!observer.contains("format!("));

    let waiter = source
        .split("\nasync fn wait_for_send_completions_in_order(")
        .nth(1)
        .expect("ordered SendQueue completion waiter")
        .split("\nasync fn wait_for_cancelled_or_removed_send(")
        .next()
        .expect("ordered SendQueue completion waiter end");
    assert!(waiter.contains("TimelineEvent::InitialItems"));
    assert!(waiter.contains("TimelineEvent::ItemsUpdated"));
    assert!(waiter.contains("visit_timeline_diff_items(&diffs"));
    assert!(waiter.contains("let mut first_left_not_sent_after_retry = false;"));
    assert_eq!(
        waiter
            .matches("observe_send_queue_retry_item_state(")
            .count(),
        2,
        "InitialItems and ItemsUpdated must share the causally fenced observer"
    );
    assert_eq!(
        waiter
            .matches("\"{label}: first queued send returned to NotSent reason={reason}\"")
            .count(),
        2,
        "InitialItems and ItemsUpdated must use the same private-safe fixed-token diagnostic"
    );
    assert!(waiter.contains("request_id == retry_request_id"));
    assert!(waiter.contains("\"{label}: retry operation failed\""));
    assert!(waiter.contains("\"{label}: queued send operation failed\""));
    assert!(!waiter.contains("{failure:?}"));
    assert!(!waiter.contains("{transaction_id}"));
    assert!(!waiter.contains("sdk_transaction_id}"));
}

#[test]
fn fast_send_queue_restored_completion_cannot_finish_from_send_completed_alone() {
    let source = include_str!("send_queue_fast.rs");
    let helper = source
        .split("async fn wait_for_fast_send_queue_authoritative_completion")
        .nth(1)
        .and_then(|section| {
            section
                .split("async fn wait_for_fast_send_queue_flow_completion")
                .next()
        })
        .expect("fast SendQueue authoritative completion helper source");

    assert!(
        helper.contains("fast_send_queue_authoritative_projection"),
        "restored completion must validate the accumulated projection"
    );
    let send_completed_arm = helper
        .split("TimelineEvent::SendCompleted")
        .nth(1)
        .and_then(|section| section.split("CoreEvent::OperationFailed").next())
        .expect("fast SendQueue SendCompleted arm");
    assert!(
        !send_completed_arm.contains("return Ok(event_id)"),
        "SendCompleted alone must not finish restored completion with zero Event rows"
    );
}

#[test]
fn fast_send_queue_authoritative_projection_requires_one_exact_event_and_no_transaction() {
    let body = "fast authoritative body";
    let expected_event_id = "$fast-authoritative:localhost";
    let mut transaction = projection_timeline_item("$placeholder:localhost", false);
    transaction.id = TimelineItemId::Transaction {
        transaction_id: "fast-authoritative-transaction".to_owned(),
    };
    transaction.body = Some(body.to_owned());
    let mut event = projection_timeline_item(expected_event_id, false);
    event.body = Some(body.to_owned());

    assert_eq!(
        fast_send_queue_authoritative_projection(
            &[],
            body,
            expected_event_id,
            "fast_send_queue contract"
        )
        .expect("empty projection is still waiting"),
        None
    );
    assert_eq!(
        fast_send_queue_authoritative_projection(
            &[transaction.clone()],
            body,
            expected_event_id,
            "fast_send_queue contract"
        )
        .expect("transaction-only projection is still waiting"),
        None
    );
    assert_eq!(
        fast_send_queue_authoritative_projection(
            &[event.clone(), transaction],
            body,
            expected_event_id,
            "fast_send_queue contract"
        )
        .expect("mixed projection is still waiting for transaction removal"),
        None
    );
    assert_eq!(
        fast_send_queue_authoritative_projection(
            &[event.clone()],
            body,
            expected_event_id,
            "fast_send_queue contract"
        )
        .expect("exact event projection"),
        Some(expected_event_id.to_owned())
    );

    let mut wrong = event.clone();
    wrong.id = TimelineItemId::Event {
        event_id: "$fast-wrong:localhost".to_owned(),
    };
    assert!(
        fast_send_queue_authoritative_projection(
            &[wrong],
            body,
            expected_event_id,
            "fast_send_queue contract"
        )
        .is_err(),
        "wrong Event rows must be rejected"
    );
    assert!(
        fast_send_queue_authoritative_projection(
            &[event.clone(), event],
            body,
            expected_event_id,
            "fast_send_queue contract"
        )
        .is_err(),
        "duplicate Event rows must be rejected"
    );
}

async fn run_fast_send_queue_feedback() {
    let server = MatrixMockServer::new().await;
    let proxy = FastTcpProxy::start(&server.uri()).expect("fast_send_queue proxy start");
    let room_id = room_id!("!fast-send-queue:localhost");
    let data_dir = tempfile::tempdir().expect("fast_send_queue data directory");
    let credential_dir = tempfile::tempdir().expect("fast_send_queue credential directory");

    server.mock_versions().ok().mount().await;
    server.mock_login().ok().mock_once().mount().await;
    server
        .mock_room_state_encryption()
        .ignore_access_token()
        .plain()
        .mount()
        .await;
    server
        .mock_sync()
        .ok(|builder| {
            builder.add_joined_room(JoinedRoomBuilder::new(room_id));
        })
        .mock_once()
        .mount()
        .await;

    let runtime = CoreRuntime::start_with_data_dir_and_file_credentials(
        data_dir.path().to_path_buf(),
        credential_dir.path().to_path_buf(),
    );
    fast_send_queue_phase(
        "fast_send_queue initial trust configure",
        configure_fast_send_queue_trust(&runtime),
    )
    .await
    .expect("fast_send_queue initial trust phase")
    .expect("fast_send_queue initial trust admission");
    let mut conn = runtime.attach();
    let login_id = conn.next_request_id();
    fast_send_queue_phase(
        "fast_send_queue login command",
        conn.command(CoreCommand::Account(AccountCommand::LoginPassword {
            request_id: login_id,
            request: koushi_state::LoginRequest {
                homeserver: proxy.homeserver_url(),
                username: "fast-send-queue".to_owned(),
                password: AuthSecret::new("fast-send-queue-password"),
                device_display_name: Some("Fast Send Queue QA".to_owned()),
            },
        })),
    )
    .await
    .expect("fast_send_queue login command phase")
    .expect("fast_send_queue submit login");
    let account_key = fast_send_queue_phase(
        "fast_send_queue LoggedIn event",
        wait_for_logged_in(&mut conn, login_id, "fast_send_queue login"),
    )
    .await
    .expect("fast_send_queue LoggedIn phase")
    .expect("fast_send_queue login event");
    fast_send_queue_phase(
        "fast_send_queue ready snapshot",
        wait_for_ready_snapshot(&mut conn, "fast_send_queue ready"),
    )
    .await
    .expect("fast_send_queue ready phase")
    .expect("fast_send_queue ready projection");
    fast_send_queue_phase(
        "fast_send_queue room-list snapshot",
        wait_for_room_in_room_list(&mut conn, room_id.as_str(), "fast_send_queue room list"),
    )
    .await
    .expect("fast_send_queue room-list phase")
    .expect("fast_send_queue room list projection");
    fast_send_queue_phase(
        "fast_send_queue initial stop sync",
        stop_sync_for_qa(&mut conn, "fast_send_queue stop background sync"),
    )
    .await
    .expect("fast_send_queue initial stop-sync phase")
    .expect("fast_send_queue background sync stopped");

    let key = TimelineKey::room(account_key.clone(), room_id.to_string());
    let subscribe_id = conn.next_request_id();
    fast_send_queue_phase(
        "fast_send_queue initial subscribe command",
        conn.command(CoreCommand::Timeline(TimelineCommand::Subscribe {
            request_id: subscribe_id,
            key: key.clone(),
        })),
    )
    .await
    .expect("fast_send_queue initial subscribe command phase")
    .expect("fast_send_queue submit subscribe");
    let mut projection = fast_send_queue_phase(
        "fast_send_queue initial subscribe replay",
        wait_for_initial_items_or_active_replay(
            &mut conn,
            &key,
            subscribe_id,
            "fast_send_queue initial subscribe",
        ),
    )
    .await
    .expect("fast_send_queue initial subscribe replay phase")
    .expect("fast_send_queue initial projection");
    assert!(
        projection.is_empty(),
        "fast_send_queue starts with an empty timeline"
    );

    let initial_send_guard = tokio::time::timeout(
        Duration::from_secs(5),
        server
            .mock_room_send()
            .body_matches_partial_json(serde_json::json!({ "body": "fast initial body" }))
            .ok(event_id!("$fast-initial:localhost"))
            .mock_once()
            .mount_as_scoped(),
    )
    .await;
    let initial_send_guard =
        initial_send_guard.expect("fast_send_queue initial guard mount timed out");
    let initial_request_id = conn.next_request_id();
    fast_send_queue_phase(
        "fast_send_queue initial send command",
        conn.command(CoreCommand::Timeline(TimelineCommand::SendText {
            request_id: initial_request_id,
            key: key.clone(),
            transaction_id: "fast-initial-client".to_owned(),
            body: "fast initial body".to_owned(),
            mentions: MentionIntent::default(),
        })),
    )
    .await
    .expect("fast_send_queue initial send command phase")
    .expect("fast_send_queue submit initial send");
    let initial_outcome = wait_for_fast_send_queue_flow_completion(
        &mut conn,
        &mut projection,
        initial_request_id,
        &key,
        "fast-initial-client",
        "fast initial body",
        "fast_send_queue initial completion",
    )
    .await
    .expect("fast_send_queue initial local echo and completion");
    assert_eq!(initial_outcome.event_id, "$fast-initial:localhost");
    wait_for_fast_send_queue_pending_removal(
        &mut conn,
        &mut projection,
        &key,
        "fast initial body",
        "fast_send_queue initial pending removal",
    )
    .await
    .expect("fast_send_queue initial pending echo removed");
    assert_fast_send_queue_success_projection(
        &projection,
        "fast initial body",
        "fast_send_queue initial completion",
    );
    drop(initial_send_guard);

    proxy.disable();
    let first = send_fast_send_queue_text_expect_recoverable_transport_failure(
        &mut conn,
        &mut projection,
        &key,
        "fast-offline-first-client",
        "fast offline first body",
        "fast_send_queue first offline failure",
        "offline_first",
        &proxy,
    )
    .await
    .expect("fast_send_queue first recoverable offline state");

    let second = send_fast_send_queue_text_expect_local_echo(
        &mut conn,
        &mut projection,
        &key,
        "fast-offline-second-client",
        "fast offline second body",
        "fast_send_queue second offline local echo",
    )
    .await
    .expect("fast_send_queue second offline local echo");
    proxy.enable();
    // Keep the first successful response in flight while Sync Start produces
    // Started, Running, and replacement InitialItems. This deterministically
    // places SentEvent after actor replacement without a blind orchestration sleep.
    let fifo_first_guard = server
        .mock_room_send()
        .body_matches_partial_json(serde_json::json!({ "body": "fast offline first body" }))
        .ok_with_delay(
            event_id!("$fast-fifo-first:localhost"),
            Duration::from_secs(2),
        )
        .mock_once()
        .mount_as_scoped()
        .await;
    let fifo_second_guard = server
        .mock_room_send()
        .body_matches_partial_json(serde_json::json!({ "body": "fast offline second body" }))
        .ok(event_id!("$fast-fifo-second:localhost"))
        .mock_once()
        .mount_as_scoped()
        .await;
    let replacement_sync_guard = server
        .mock_sync()
        .ok(|builder| {
            builder.add_joined_room(JoinedRoomBuilder::new(room_id));
        })
        .mock_once()
        .mount_as_scoped()
        .await;
    let retry_id = fast_send_queue_phase(
        "fast_send_queue first retry command",
        retry_send_queue_item(
            &mut conn,
            &key,
            &first.sdk_transaction_id,
            "fast_send_queue retry first offline",
        ),
    )
    .await
    .expect("fast_send_queue first retry phase")
    .expect("fast_send_queue retry command");
    projection = fast_send_queue_phase(
        "fast_send_queue replacement sync start",
        start_sync_and_wait_for_replacement_initial_items(
            &mut conn,
            &key,
            &first,
            &second,
            "fast_send_queue replacement sync start",
        ),
    )
    .await
    .expect("fast_send_queue replacement sync phase")
    .expect("fast_send_queue replacement InitialItems");
    wait_for_fast_send_queue_completions_in_order(
        &mut conn,
        &mut projection,
        &key,
        retry_id,
        &first,
        &second,
        "fast_send_queue FIFO completion",
    )
    .await
    .expect("fast_send_queue FIFO order");
    wait_for_fast_send_queue_pending_removal(
        &mut conn,
        &mut projection,
        &key,
        "fast offline first body",
        "fast_send_queue FIFO first pending removal",
    )
    .await
    .expect("fast_send_queue FIFO first pending echo removed");
    wait_for_fast_send_queue_pending_removal(
        &mut conn,
        &mut projection,
        &key,
        "fast offline second body",
        "fast_send_queue FIFO second pending removal",
    )
    .await
    .expect("fast_send_queue FIFO second pending echo removed");
    assert_fast_send_queue_success_projection(
        &projection,
        "fast offline first body",
        "fast_send_queue FIFO first",
    );
    assert_fast_send_queue_success_projection(
        &projection,
        "fast offline second body",
        "fast_send_queue FIFO second",
    );
    drop(fifo_second_guard);
    drop(fifo_first_guard);
    drop(replacement_sync_guard);
    fast_send_queue_phase(
        "fast_send_queue stop replacement sync",
        stop_sync_for_qa(&mut conn, "fast_send_queue stop replacement sync"),
    )
    .await
    .expect("fast_send_queue replacement stop-sync phase")
    .expect("fast_send_queue replacement sync stopped");

    proxy.disable();
    let cancel = send_fast_send_queue_text_expect_recoverable_transport_failure(
        &mut conn,
        &mut projection,
        &key,
        "fast-cancel-client",
        "fast cancel body",
        "fast_send_queue cancel transport failure",
        "cancel",
        &proxy,
    )
    .await
    .expect("fast_send_queue cancel recoverable state");
    let cancel_id = fast_send_queue_phase(
        "fast_send_queue cancel command",
        cancel_send_queue_item(
            &mut conn,
            &key,
            &cancel.sdk_transaction_id,
            "fast_send_queue cancel",
        ),
    )
    .await
    .expect("fast_send_queue cancel command phase")
    .expect("fast_send_queue cancel command");
    wait_for_fast_send_queue_cancelled_or_removed(
        &mut conn,
        &mut projection,
        &key,
        cancel_id,
        &cancel.sdk_transaction_id,
        "fast_send_queue cancel removal",
    )
    .await
    .expect("fast_send_queue cancel removal event");
    assert_fast_send_queue_cancel_projection(
        &projection,
        "fast cancel body",
        "fast_send_queue cancel",
    );

    let restart = send_fast_send_queue_text_expect_recoverable_transport_failure(
        &mut conn,
        &mut projection,
        &key,
        "fast-restart-client",
        "fast restart body",
        "fast_send_queue restart transport failure",
        "restart",
        &proxy,
    )
    .await
    .expect("fast_send_queue restart recoverable state");

    drop(conn);
    fast_send_queue_phase("fast_send_queue first shutdown barrier", runtime.shutdown())
        .await
        .expect("fast_send_queue first ordered shutdown");

    let runtime = CoreRuntime::start_with_data_dir_and_file_credentials(
        data_dir.path().to_path_buf(),
        credential_dir.path().to_path_buf(),
    );
    fast_send_queue_phase(
        "fast_send_queue restored trust configure",
        configure_fast_send_queue_trust(&runtime),
    )
    .await
    .expect("fast_send_queue restored trust phase")
    .expect("fast_send_queue restore trust admission");
    let mut conn = runtime.attach();
    let restore_id = conn.next_request_id();
    fast_send_queue_phase(
        "fast_send_queue restore command",
        conn.command(CoreCommand::Account(AccountCommand::RestoreSession {
            request_id: restore_id,
            account_key: account_key.clone(),
        })),
    )
    .await
    .expect("fast_send_queue restore command phase")
    .expect("fast_send_queue submit restore");
    fast_send_queue_phase(
        "fast_send_queue SessionRestored event",
        wait_for_session_restored(
            &mut conn,
            restore_id,
            &account_key,
            "fast_send_queue restore",
        ),
    )
    .await
    .expect("fast_send_queue SessionRestored phase")
    .expect("fast_send_queue restored session event");
    fast_send_queue_phase(
        "fast_send_queue restored ready snapshot",
        wait_for_ready_snapshot(&mut conn, "fast_send_queue restored ready"),
    )
    .await
    .expect("fast_send_queue restored ready phase")
    .expect("fast_send_queue restored ready projection");
    fast_send_queue_phase(
        "fast_send_queue restored stop sync",
        stop_sync_for_qa(&mut conn, "fast_send_queue stop restored background sync"),
    )
    .await
    .expect("fast_send_queue restored stop-sync phase")
    .expect("fast_send_queue restored background sync stopped");

    let subscribe_id = conn.next_request_id();
    fast_send_queue_phase(
        "fast_send_queue restored subscribe command",
        conn.command(CoreCommand::Timeline(TimelineCommand::Subscribe {
            request_id: subscribe_id,
            key: key.clone(),
        })),
    )
    .await
    .expect("fast_send_queue restored subscribe command phase")
    .expect("fast_send_queue submit restored subscribe");
    let mut projection = fast_send_queue_phase(
        "fast_send_queue restored subscribe replay",
        wait_for_initial_items_or_active_replay(
            &mut conn,
            &key,
            subscribe_id,
            "fast_send_queue restored subscribe",
        ),
    )
    .await
    .expect("fast_send_queue restored subscribe replay phase")
    .expect("fast_send_queue restored projection");
    assert_eq!(
        fast_send_queue_projection_counts(&projection, "fast restart body"),
        (0, 1),
        "fast_send_queue restored unsent echo: projection count mismatch",
    );
    let restored_send_state = projection
        .iter()
        .find(|item| timeline_item_body_matches(item, "fast restart body"))
        .and_then(|item| {
            (timeline_item_transaction_id(item) == Some(restart.sdk_transaction_id.as_str()))
                .then(|| item.send_state.clone())
                .flatten()
        })
        .expect("fast_send_queue restored unsent Transaction state");
    assert!(
        matches!(
            &restored_send_state,
            TimelineSendState::Sending
                | TimelineSendState::NotSent {
                    reason: koushi_core::event::TimelineSendFailureReason::Recoverable,
                }
        ),
        "fast_send_queue restored Transaction must remain recoverable"
    );

    let restart_success_guard = fast_send_queue_phase(
        "fast_send_queue restored success fixture",
        server
            .mock_room_send()
            .body_matches_partial_json(serde_json::json!({ "body": "fast restart body" }))
            .ok(event_id!("$fast-restart:localhost"))
            .mock_once()
            .mount_as_scoped(),
    )
    .await
    .expect("fast_send_queue restored success fixture phase");
    proxy.enable();
    let restart_retry_id = if matches!(
        &restored_send_state,
        TimelineSendState::NotSent {
            reason: koushi_core::event::TimelineSendFailureReason::Recoverable,
        }
    ) {
        Some(
            fast_send_queue_phase(
                "fast_send_queue restored retry command",
                retry_send_queue_item(
                    &mut conn,
                    &key,
                    &restart.sdk_transaction_id,
                    "fast_send_queue retry restored",
                ),
            )
            .await
            .expect("fast_send_queue restored retry phase")
            .expect("fast_send_queue restored retry command"),
        )
    } else {
        let paused_time = FastSendQueuePausedTime::start();
        tokio::time::advance(Duration::from_millis(1_500)).await;
        drop(paused_time);
        None
    };
    let restart_event_id = wait_for_fast_send_queue_authoritative_completion(
        &mut conn,
        &mut projection,
        &key,
        restart.request_id,
        restart_retry_id,
        &restart.sdk_transaction_id,
        &restart.client_transaction_id,
        "fast restart body",
        "$fast-restart:localhost",
        "fast_send_queue restored completion",
    )
    .await
    .expect("fast_send_queue restored authoritative completion");
    assert_eq!(restart_event_id, "$fast-restart:localhost");
    wait_for_fast_send_queue_pending_removal(
        &mut conn,
        &mut projection,
        &key,
        "fast restart body",
        "fast_send_queue restored pending removal",
    )
    .await
    .expect("fast_send_queue restored pending echo removed");
    assert_fast_send_queue_success_projection(
        &projection,
        "fast restart body",
        "fast_send_queue restored completion dedupe",
    );
    drop(restart_success_guard);

    drop(conn);
    fast_send_queue_phase("fast_send_queue final shutdown barrier", runtime.shutdown())
        .await
        .expect("fast_send_queue final ordered shutdown");
}

#[tokio::test]
async fn fast_send_queue_feedback_runs_production_runtime_without_homeserver() {
    let started = std::time::Instant::now();
    tokio::time::timeout(
        FAST_SEND_QUEUE_TOTAL_TIMEOUT,
        run_fast_send_queue_feedback(),
    )
    .await
    .expect("fast_send_queue whole lane timed out");
    assert!(
        started.elapsed() < Duration::from_secs(60),
        "fast_send_queue exceeded the 60-second lane budget"
    );
}

#[tokio::test]
async fn fast_send_queue_sync_started_replacement_preserves_original_completion_correlation() {
    tokio::time::timeout(
        FAST_SEND_QUEUE_TOTAL_TIMEOUT,
        run_fast_send_queue_feedback(),
    )
    .await
    .expect("fast_send_queue replacement regression timed out");
}
