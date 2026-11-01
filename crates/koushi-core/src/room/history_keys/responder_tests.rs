use super::*;
use matrix_sdk::{
    SessionMeta, SessionTokens,
    authentication::matrix::MatrixSession,
    ruma::{owned_device_id, owned_user_id},
    store::RoomLoadSettings,
};

async fn recovery() -> Recovery {
    let client = Client::builder()
        .homeserver_url("https://matrix.example.invalid")
        .build()
        .await
        .unwrap();
    client
        .matrix_auth()
        .restore_session(
            MatrixSession {
                meta: SessionMeta {
                    user_id: owned_user_id!("@responder:example.invalid"),
                    device_id: owned_device_id!("TESTDEVICE"),
                },
                tokens: SessionTokens {
                    access_token: "synthetic".to_owned(), // secret-scan: allow (synthetic test credential)
                    refresh_token: None,
                },
            },
            RoomLoadSettings::default(),
        )
        .await
        .unwrap();
    let (actions, _) = mpsc::channel(1);
    let (events, _) = broadcast::channel(1);
    Recovery {
        client,
        actions,
        events,
        ledger: Ledger::default(),
        diagnostics: HistoryKeyRecoveryDiagnostics::default(),
        room_rejections: BTreeMap::new(),
        import_pause: None,
        disclosure_pause: None,
        poisoned: AtomicBool::new(false),
        queries: BTreeMap::new(),
        startup_deadline: executor::Instant::now(),
        automatic_attempted: BTreeSet::new(),
    }
}

#[tokio::test]
async fn expired_rate_entries_are_pruned_even_at_capacity() {
    let mut recovery = recovery().await;
    let timestamp = now();
    for n in 0..512 {
        recovery.ledger.responded.insert(
            format!("!room:example.invalid\n@member{n}:example.invalid"),
            timestamp - 60,
        );
    }
    recovery
        .ledger
        .responded
        .insert("recent".to_owned(), timestamp - 30);
    recovery
        .ledger
        .responded
        .insert("future".to_owned(), timestamp + 300);
    recovery.prune().await.unwrap();
    assert_eq!(recovery.ledger.responded.len(), 2);
    assert!(
        !recovery
            .ledger
            .responded
            .contains_key("!room:example.invalid\n@member0:example.invalid")
    );
    assert!(recovery.ledger.responded.contains_key("recent"));
    assert!(recovery.ledger.responded.contains_key("future"));
    let persisted = recovery
        .client
        .encryption()
        .application_crypto_data(STORE_KEY)
        .await
        .unwrap()
        .unwrap();
    let ledger: Ledger = serde_json::from_slice(&persisted).unwrap();
    assert_eq!(ledger.responded.len(), 2);
}
