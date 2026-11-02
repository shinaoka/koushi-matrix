//! An owned loopback homeserver: tests explicitly answer each room-list request.
use std::time::Duration;

use serde_json::{Value, json};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    sync::{mpsc, oneshot},
    task::JoinSet,
};

pub(super) struct HeldResponse {
    reply: oneshot::Sender<()>,
    written: oneshot::Receiver<()>,
}

impl HeldResponse {
    pub(super) async fn answer(self) {
        let _ = self.reply.send(());
        let _ = self.written.await;
    }
}

pub(super) struct StalledServer {
    pub(super) uri: String,
    pub(super) attempts: mpsc::UnboundedReceiver<HeldResponse>,
    pub(super) backup_requests: mpsc::UnboundedReceiver<()>,
    task: tokio::task::JoinHandle<()>,
    stop: Option<oneshot::Sender<()>>,
}

impl StalledServer {
    pub(super) async fn start() -> Self {
        Self::start_named("@fixture-user:example.invalid").await
    }

    pub(super) async fn start_named(user_id: &'static str) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let uri = format!("http://{}", listener.local_addr().unwrap());
        let (attempt_tx, attempts) = mpsc::unbounded_channel();
        let (backup_tx, backup_requests) = mpsc::unbounded_channel();
        let (stop, mut stopped) = oneshot::channel();
        let task = tokio::spawn(async move {
            let mut peers = JoinSet::new();
            loop {
                tokio::select! {
                    _ = &mut stopped => break,
                    accepted = listener.accept() => {
                        let Ok((mut stream, _)) = accepted else { break };
                        let attempt_tx = attempt_tx.clone();
                        let backup_tx = backup_tx.clone();
                        peers.spawn(async move {
                            let mut request = Vec::new();
                            let mut buf = [0; 4096];
                            let (header_end, length) = loop {
                                let Ok(n) = stream.read(&mut buf).await else { return };
                                if n == 0 { return }
                                request.extend_from_slice(&buf[..n]);
                                let text = String::from_utf8_lossy(&request);
                                if let Some(end) = text.find("\r\n\r\n") {
                                    let length = text.lines().find_map(|line| {
                                        let (key, value) = line.split_once(':')?;
                                        key.eq_ignore_ascii_case("content-length").then(|| value.trim().parse::<usize>().ok()).flatten()
                                    }).unwrap_or(0);
                                    if request.len() >= end + 4 + length { break (end + 4, length) }
                                }
                            };
                            let text = String::from_utf8_lossy(&request[..header_end]);
                            let path = text.split_whitespace().nth(1).unwrap_or("");
                            let body: Value = serde_json::from_slice(&request[header_end..header_end+length]).unwrap_or_default();
                            let mut written = None;
                            let (status, mut response) = if path.contains("msc3575/sync") {
                                if body["conn_id"] == "room-list" {
                                    let (reply, ready) = oneshot::channel();
                                    let (done, received) = oneshot::channel();
                                    if attempt_tx.send(HeldResponse { reply, written: received }).is_err() { return }
                                    if ready.await.is_err() { return }
                                    written = Some(done);
                                } else {
                                    std::future::pending::<()>().await;
                                }
                                (200, json!({"pos":"synthetic-pos", "lists":{"all_rooms":{"count":0}},"rooms":{},"extensions":{}}))
                            } else if path.ends_with("/versions") {
                                (200, json!({"versions":["v1.7"],"unstable_features":{"org.matrix.simplified_msc3575":true}}))
                            } else if path.ends_with("/login") {
                                (200, json!({"access_token":"synthetic-token","user_id":user_id,"device_id":body["device_id"]}))
                            } else if path.ends_with("/keys/upload") { (200, json!({"one_time_key_counts":{}}))
                            } else if path.ends_with("/keys/query") {
                                // Tests supply trust observations; don't race them with an unknown recheck.
                                std::future::pending::<()>().await;
                                return;

                            } else if path.ends_with("/room_keys/version") {
                                let _ = backup_tx.send(());
                                (404, json!({"errcode":"M_NOT_FOUND","error":"No backup"}))
                            } else { (200, json!({})) };
                            if let Some(txn) = body.get("txn_id") { response["txn_id"] = txn.clone(); }
                            let body = response.to_string();
                            let response = format!("HTTP/1.1 {status} OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
                            let _ = stream.write_all(response.as_bytes()).await;
                            if let Some(done) = written { let _ = done.send(()); }
                        });
                    }
                    _ = peers.join_next(), if !peers.is_empty() => {}
                }
            }
            peers.abort_all();
            while peers.join_next().await.is_some() {}
        });
        Self {
            uri,
            attempts,
            backup_requests,
            task,
            stop: Some(stop),
        }
    }

    pub(super) async fn shutdown(mut self) {
        let _ = self.stop.take().expect("server stop owner").send(());
        let _ = (&mut self.task).await;
    }

    pub(super) async fn next_attempt(&mut self) -> HeldResponse {
        tokio::time::timeout(Duration::from_secs(3), self.attempts.recv())
            .await
            .expect("Retry must issue a new sliding-sync request, not wait on the stalled run")
            .expect("request observer")
    }
}

impl Drop for StalledServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}
