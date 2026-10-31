use koushi_protocol::{
    command::AccountCommand,
    event::{AccountEvent, CoreEvent, EventCacheSubscribeStatus, LocalEncryptionEvent},
    ids::{RequestId, RuntimeConnectionId},
};
use koushi_state::{AppAction, AvatarThumbnailFailureKind, AvatarThumbnailState, SessionInfo};
use tempfile::tempdir;
use tokio::time::{Duration, timeout};
use wiremock::ResponseTemplate;

use koushi_sdk::MatrixClientSession;
use matrix_sdk::test_utils::mocks::MatrixMockServer;

use super::{
    actor::AccountMessage,
    profile::{AvatarResolution, prepare_avatar_content},
    test_support::{inspect_sync_owners, shutdown_and_ack, spawn_actor_with_dirs},
};
use crate::renderable_thumbnail::{
    AVATAR_THUMBNAIL_IDENTITY, AVATAR_THUMBNAIL_LONG_EDGE, RenderableThumbnailContent,
    RenderableThumbnailKind, store_renderable_thumbnail_variant_for_account,
};

/// Wait for the actor's next `Ready` update for one MXC and return its
/// `source_ref`.
async fn await_ready_avatar(
    action_rx: &mut tokio::sync::mpsc::Receiver<Vec<AppAction>>,
    mxc_uri: &str,
) -> String {
    timeout(Duration::from_secs(5), async {
        loop {
            for action in action_rx.recv().await.expect("account action channel") {
                if let AppAction::AvatarThumbnailUpdated {
                    mxc_uri: updated,
                    thumbnail: AvatarThumbnailState::Ready { source_ref, .. },
                } = action
                    && updated == mxc_uri
                {
                    return source_ref;
                }
            }
        }
    })
    .await
    .expect("Ready avatar update")
}

/// A session whose account scope belongs to one test only. Session teardown in
/// another test clears its own account's retained thumbnails from the
/// process-global cache, so an assertion about retained bytes needs an account
/// no other test can clear.
async fn test_session_for_account(server: &MatrixMockServer, user_id: &str) -> MatrixClientSession {
    let client = server.client_builder().build().await;
    MatrixClientSession::from_client_for_testing(
        client.clone(),
        SessionInfo {
            homeserver: server.uri(),
            user_id: user_id.to_owned(),
            device_id: client.device_id().expect("mock device id").to_string(),
            authentication_method: koushi_state::SessionAuthenticationMethod::Unknown,
        },
    )
}

async fn test_session(server: &MatrixMockServer) -> MatrixClientSession {
    let client = server.client_builder().build().await;
    MatrixClientSession::from_client_for_testing(
        client.clone(),
        SessionInfo {
            homeserver: server.uri(),
            user_id: client.user_id().expect("mock user id").to_string(),
            device_id: client.device_id().expect("mock device id").to_string(),
            authentication_method: koushi_state::SessionAuthenticationMethod::Unknown,
        },
    )
}

#[tokio::test]
async fn cached_avatar_download_dispatches_authoritative_update() {
    let _cache_guard = crate::renderable_thumbnail::test_cache_lock_async().await;
    let server = MatrixMockServer::new().await;
    server
        .mock_authed_media_download()
        .ok_image()
        .expect(1)
        .mount()
        .await;
    let session = test_session(&server).await;
    let cred_dir = tempdir().expect("credential tempdir");
    let data_dir = tempdir().expect("data tempdir");
    let (handle, mut action_rx, mut event_rx) =
        spawn_actor_with_dirs(cred_dir.path(), data_dir.path());
    assert!(
        handle
            .install_residency_test_session(std::sync::Arc::new(session))
            .await
    );
    assert!(matches!(
        event_rx.recv().await.expect("cache admission diagnostic"),
        CoreEvent::LocalEncryption(LocalEncryptionEvent::EventCacheStatus {
            encrypted_store: false,
            subscribed: true,
            subscribe_status: EventCacheSubscribeStatus::AlreadyEnabled,
            reason_class: None,
        })
    ));
    let mxc_uri = "mxc://localhost/cached-avatar";
    let first_request_id = RequestId {
        connection_id: RuntimeConnectionId(7),
        sequence: 41,
    };
    handle
        .send(AccountMessage::Command(
            AccountCommand::DownloadAvatarThumbnail {
                request_id: first_request_id,
                mxc_uri: mxc_uri.to_owned(),
            },
        ))
        .await;
    let first_thumbnail = loop {
        let actions = action_rx.recv().await.expect("first avatar action");
        if let [AppAction::AvatarThumbnailUpdated { thumbnail, .. }] = actions.as_slice() {
            break thumbnail.clone();
        }
    };
    assert!(matches!(
        event_rx.recv().await.expect("first avatar event"),
        CoreEvent::Account(AccountEvent::AvatarThumbnailDownloaded {
            request_id,
            ..
        }) if request_id == first_request_id
    ));

    let request_id = RequestId {
        connection_id: RuntimeConnectionId(7),
        sequence: 42,
    };
    handle
        .send(AccountMessage::Command(
            AccountCommand::DownloadAvatarThumbnail {
                request_id,
                mxc_uri: mxc_uri.to_owned(),
            },
        ))
        .await;

    assert_eq!(
        action_rx.recv().await.expect("cached avatar action"),
        vec![AppAction::AvatarThumbnailUpdated {
            mxc_uri: mxc_uri.to_owned(),
            thumbnail: first_thumbnail.clone(),
        }]
    );
    match event_rx.recv().await.expect("cached avatar event") {
        CoreEvent::Account(AccountEvent::AvatarThumbnailDownloaded {
            request_id: event_request_id,
            mxc_uri: event_mxc_uri,
            thumbnail: event_thumbnail,
        }) => {
            assert_eq!(event_request_id, request_id);
            assert_eq!(event_mxc_uri, mxc_uri);
            assert_eq!(event_thumbnail, first_thumbnail);
        }
        event => panic!("unexpected cached avatar event: {event:?}"),
    }

    shutdown_and_ack(&handle).await;
}

/// Holds every matched request open for the rest of the test and reports its
/// arrival, so the test synchronizes on the request instead of a server delay.
struct HeldAvatarResponse {
    arrived: tokio::sync::mpsc::UnboundedSender<()>,
}

impl wiremock::Respond for HeldAvatarResponse {
    fn respond(&self, _request: &wiremock::Request) -> ResponseTemplate {
        let _ = self.arrived.send(());
        ResponseTemplate::new(200)
            .set_body_raw(b"held-avatar", "image/jpeg")
            .set_delay(Duration::from_secs(3600))
    }
}

#[tokio::test]
async fn avatar_actor_bounds_distinct_demand_and_rejects_capacity() {
    let _cache_guard = crate::renderable_thumbnail::test_cache_lock_async().await;
    let server = MatrixMockServer::new().await;
    // #1674: the fetch asks for a server thumbnail first (#1396), so hold that
    // request. Holding the original download let a loaded run shut down
    // between the unmatched thumbnail and its download fallback.
    let edge = u16::try_from(crate::renderable_thumbnail::AVATAR_THUMBNAIL_LONG_EDGE)
        .expect("the avatar long edge fits a u16");
    let (arrived_tx, mut arrived_rx) = tokio::sync::mpsc::unbounded_channel();
    server
        .mock_authed_media_thumbnail(matrix_sdk::ruma::media::Method::Scale, edge, edge, false)
        .respond_with(HeldAvatarResponse {
            arrived: arrived_tx,
        })
        .expect(1)
        .mount()
        .await;
    let session = test_session(&server).await;
    let cred_dir = tempdir().expect("credential tempdir");
    let data_dir = tempdir().expect("data tempdir");
    let (handle, mut action_rx, mut event_rx) =
        spawn_actor_with_dirs(cred_dir.path(), data_dir.path());
    handle.set_search_crawler_speed(koushi_state::SearchCrawlerSpeed::Fast);
    assert!(
        handle
            .install_residency_test_session(std::sync::Arc::new(session))
            .await
    );

    let capacity_sequence = super::profile::AVATAR_DOWNLOAD_CONCURRENCY
        + super::profile::AVATAR_DOWNLOAD_QUEUE_CAPACITY
        + 1;
    for sequence in 1..=super::profile::AVATAR_DOWNLOAD_CONCURRENCY as u64 {
        assert!(
            handle
                .send(AccountMessage::Command(
                    AccountCommand::DownloadAvatarThumbnail {
                        request_id: RequestId {
                            connection_id: RuntimeConnectionId(9),
                            sequence,
                        },
                        mxc_uri: format!("mxc://localhost/avatar-{sequence}"),
                    },
                ))
                .await
        );
    }
    // The shared media permit admits one fetch; wait for it to reach the held
    // server. The arrival is the barrier, so no wall-clock window races it.
    arrived_rx
        .recv()
        .await
        .expect("the shared media permit should reach the held server");
    for sequence in
        (super::profile::AVATAR_DOWNLOAD_CONCURRENCY as u64 + 1)..=capacity_sequence as u64
    {
        assert!(
            handle
                .send(AccountMessage::Command(
                    AccountCommand::DownloadAvatarThumbnail {
                        request_id: RequestId {
                            connection_id: RuntimeConnectionId(9),
                            sequence,
                        },
                        mxc_uri: format!("mxc://localhost/avatar-{sequence}"),
                    },
                ))
                .await
        );
    }

    // The actor handles commands in order and admission is synchronous, so
    // once a later message is answered the capacity failure is already out,
    // while the held downloads are still active.
    let _ = inspect_sync_owners(&handle).await;
    let capacity_event = std::iter::from_fn(|| event_rx.try_recv().ok())
        .find_map(|event| match event {
            CoreEvent::Account(AccountEvent::AvatarThumbnailDownloaded {
                request_id,
                thumbnail: AvatarThumbnailState::Failed { kind, .. },
                ..
            }) if request_id.sequence == capacity_sequence as u64 => Some(kind),
            _ => None,
        })
        .expect("capacity request should settle without waiting for active downloads");
    assert_eq!(capacity_event, AvatarThumbnailFailureKind::Capacity);
    let _ = action_rx.try_recv().expect("capacity state action");

    shutdown_and_ack(&handle).await;
}

#[tokio::test]
async fn canceling_last_avatar_waiter_aborts_active_fetch_and_admits_pending_work() {
    let _cache_guard = crate::renderable_thumbnail::test_cache_lock_async().await;
    let server = MatrixMockServer::new().await;
    server
        .mock_authed_media_download()
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_raw(b"delayed-avatar", "image/jpeg")
                .set_delay(Duration::from_secs(5)),
        )
        .expect(2)
        .mount()
        .await;
    let session = test_session(&server).await;
    let cred_dir = tempdir().expect("credential tempdir");
    let data_dir = tempdir().expect("data tempdir");
    let (handle, _action_rx, mut event_rx) =
        spawn_actor_with_dirs(cred_dir.path(), data_dir.path());
    handle.set_search_crawler_speed(koushi_state::SearchCrawlerSpeed::Fast);
    assert!(
        handle
            .install_residency_test_session(std::sync::Arc::new(session))
            .await
    );

    assert!(matches!(
        event_rx.recv().await.expect("cache admission diagnostic"),
        CoreEvent::LocalEncryption(LocalEncryptionEvent::EventCacheStatus {
            encrypted_store: false,
            subscribed: true,
            subscribe_status: EventCacheSubscribeStatus::AlreadyEnabled,
            reason_class: None,
        })
    ));
    for sequence in 1..=super::profile::AVATAR_DOWNLOAD_CONCURRENCY as u64 {
        assert!(
            handle
                .send(AccountMessage::Command(
                    AccountCommand::DownloadAvatarThumbnail {
                        request_id: RequestId {
                            connection_id: RuntimeConnectionId(11),
                            sequence,
                        },
                        mxc_uri: format!("mxc://localhost/cancel-avatar-{sequence}"),
                    },
                ))
                .await
        );
    }
    timeout(Duration::from_secs(2), async {
        loop {
            if server.received_requests().await.is_some_and(|requests| {
                requests
                    .iter()
                    .filter(|request| request.url.path().contains("/cancel-avatar-"))
                    .count()
                    == 1
            }) {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("the single shared media permit should reach the delayed server");

    let pending_sequence = super::profile::AVATAR_DOWNLOAD_CONCURRENCY as u64 + 1;
    assert!(
        handle
            .send(AccountMessage::Command(
                AccountCommand::DownloadAvatarThumbnail {
                    request_id: RequestId {
                        connection_id: RuntimeConnectionId(11),
                        sequence: pending_sequence,
                    },
                    mxc_uri: format!("mxc://localhost/cancel-avatar-{pending_sequence}"),
                },
            ))
            .await
    );
    assert!(
        handle
            .send(AccountMessage::Command(
                AccountCommand::CancelAvatarThumbnail {
                    request_id: RequestId {
                        connection_id: RuntimeConnectionId(11),
                        sequence: 99,
                    },
                    target_request_id: RequestId {
                        connection_id: RuntimeConnectionId(12),
                        sequence: 1,
                    },
                    mxc_uri: "mxc://localhost/cancel-avatar-1".to_owned(),
                },
            ))
            .await
    );
    assert!(
        timeout(Duration::from_millis(100), async {
            loop {
                if server.received_requests().await.is_some_and(|requests| {
                    requests
                        .iter()
                        .any(|request| request.url.path().ends_with("/cancel-avatar-7"))
                }) {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .is_err(),
        "a different connection cannot cancel another avatar waiter"
    );
    // Queued URI requests are waiting on the shared account-wide media permit.
    // Cancel them so the actor-level pending URI becomes the next scheduler waiter.
    for sequence in 2..=super::profile::AVATAR_DOWNLOAD_CONCURRENCY as u64 {
        assert!(
            handle
                .send(AccountMessage::Command(
                    AccountCommand::CancelAvatarThumbnail {
                        request_id: RequestId {
                            connection_id: RuntimeConnectionId(11),
                            sequence: 100 + sequence,
                        },
                        target_request_id: RequestId {
                            connection_id: RuntimeConnectionId(11),
                            sequence,
                        },
                        mxc_uri: format!("mxc://localhost/cancel-avatar-{sequence}"),
                    },
                ))
                .await
        );
    }
    assert!(
        handle
            .send(AccountMessage::Command(
                AccountCommand::CancelAvatarThumbnail {
                    request_id: RequestId {
                        connection_id: RuntimeConnectionId(11),
                        sequence: 100,
                    },
                    target_request_id: RequestId {
                        connection_id: RuntimeConnectionId(11),
                        sequence: 1,
                    },
                    mxc_uri: "mxc://localhost/cancel-avatar-1".to_owned(),
                },
            ))
            .await
    );

    timeout(Duration::from_secs(2), async {
        loop {
            if server.received_requests().await.is_some_and(|requests| {
                requests
                    .iter()
                    .any(|request| request.url.path().ends_with("/cancel-avatar-7"))
            }) {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("canceling an active avatar must admit the pending URI");
    assert!(
        timeout(Duration::from_millis(100), event_rx.recv())
            .await
            .is_err(),
        "canceled avatar demand must not publish a terminal event"
    );

    shutdown_and_ack(&handle).await;
}

#[tokio::test]
async fn scoped_avatar_capacity_defers_without_losing_demand_and_reuses_terminal_failure_cache() {
    let _cache_guard = crate::renderable_thumbnail::test_cache_lock_async().await;
    let server = MatrixMockServer::new().await;
    // Non-retryable HTTP response; the avatar owner's two-attempt budget still
    // applies before its coarse Network failure becomes terminal and cached.
    server
        .mock_authed_media_download()
        .respond_with(ResponseTemplate::new(403).set_body_json(serde_json::json!({
            "errcode": "M_FORBIDDEN", "error": "synthetic denial"
        })))
        .mount()
        .await;
    let session = test_session(&server).await;
    let account_id = session.info.user_id.clone();
    let cred_dir = tempdir().unwrap();
    let data_dir = tempdir().unwrap();
    let (handle, mut actions, _events) = spawn_actor_with_dirs(cred_dir.path(), data_dir.path());
    handle.set_search_crawler_speed(koushi_state::SearchCrawlerSpeed::Fast);
    assert!(
        handle
            .install_residency_test_session(std::sync::Arc::new(session))
            .await
    );
    let context = handle.avatar_demand_context(account_id);
    let mut demand = koushi_state::AvatarDemandState::new(context.clone());
    let resources = |range: std::ops::Range<usize>| {
        range
            .map(|id| Some(format!("mxc://localhost/capacity-{id}")))
            .collect()
    };
    demand.open(1).unwrap();
    demand
        .replace(&context, 1, 1, resources(0..256), resources(256..264))
        .unwrap();
    handle.publish_avatar_demand(Some(std::sync::Arc::new(demand.clone())));
    for pass in 0..2 {
        let mut settled = std::collections::BTreeSet::new();
        timeout(Duration::from_secs(15), async {
            while settled.len() < 264 {
                for action in actions.recv().await.expect("actor action stream") {
                    if let AppAction::AvatarThumbnailUpdated { mxc_uri, thumbnail } = action
                        && mxc_uri.starts_with("mxc://localhost/capacity-")
                    {
                        assert!(matches!(
                            thumbnail,
                            AvatarThumbnailState::Failed {
                                kind: AvatarThumbnailFailureKind::Network,
                                ..
                            }
                        ));
                        settled.insert(mxc_uri);
                    }
                }
            }
        })
        .await
        .expect("all demand, including deferred resources, settles");
        let requests = server.received_requests().await.unwrap_or_default();
        assert_eq!(
            requests
                .iter()
                .filter(|request| request.url.path().contains("/media/download/"))
                .count(),
            528
        );
        if pass == 0 {
            demand.close(1);
            demand.open(2).unwrap();
            demand
                .replace(&context, 2, 1, resources(0..256), resources(256..264))
                .unwrap();
            handle.publish_avatar_demand(Some(std::sync::Arc::new(demand.clone())));
        }
    }
    shutdown_and_ack(&handle).await;
}

#[tokio::test]
async fn scoped_avatar_rehydrates_evicted_ready_bytes_from_sdk_cache() {
    use crate::renderable_thumbnail::{
        MAX_RENDERABLE_THUMBNAIL_ENTRIES, RenderableThumbnailKind, lookup_renderable_thumbnail,
        store_renderable_thumbnail,
    };
    let _cache_guard = crate::renderable_thumbnail::test_cache_lock_async().await;
    let server = MatrixMockServer::new().await;
    server
        .mock_authed_media_download()
        .ok_image()
        .expect(1)
        .mount()
        .await;
    let session = test_session(&server).await;
    let account_id = session.info.user_id.clone();
    let cred_dir = tempdir().unwrap();
    let data_dir = tempdir().unwrap();
    let (handle, mut actions, _events) = spawn_actor_with_dirs(cred_dir.path(), data_dir.path());
    assert!(
        handle
            .install_residency_test_session(std::sync::Arc::new(session))
            .await
    );
    let context = handle.avatar_demand_context(account_id);
    let mut demand = koushi_state::AvatarDemandState::new(context.clone());
    let uri = "mxc://localhost/evicted-scoped-avatar";
    for scope in 1..=2 {
        demand.open(scope).unwrap();
        demand
            .replace(&context, scope, 1, vec![Some(uri.into())], vec![])
            .unwrap();
        handle.publish_avatar_demand(Some(std::sync::Arc::new(demand.clone())));
        let source_ref = timeout(Duration::from_secs(3), async {
            'ready: loop {
                for action in actions.recv().await.unwrap() {
                    if let AppAction::AvatarThumbnailUpdated {
                        mxc_uri,
                        thumbnail: AvatarThumbnailState::Ready { source_ref, .. },
                    } = action
                        && mxc_uri == uri
                    {
                        break 'ready source_ref;
                    }
                }
            }
        })
        .await
        .expect("Ready from authoritative actor");
        assert!(
            lookup_renderable_thumbnail(&source_ref).is_some(),
            "Ready must refer to retained bytes"
        );
        if scope == 1 {
            demand.close(scope);
            for id in 0..MAX_RENDERABLE_THUMBNAIL_ENTRIES {
                store_renderable_thumbnail(
                    RenderableThumbnailKind::Avatar,
                    &format!("synthetic-eviction-{id}"),
                    vec![0],
                )
                .unwrap();
            }
            assert!(lookup_renderable_thumbnail(&source_ref).is_none());
        }
    }
    shutdown_and_ack(&handle).await;
}

#[tokio::test]
async fn stale_scoped_demand_does_not_cancel_the_current_accounts_fetch() {
    let _cache_guard = crate::renderable_thumbnail::test_cache_lock_async().await;
    let server = MatrixMockServer::new().await;
    server
        .mock_authed_media_download()
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_bytes(vec![1, 2, 3])
                .set_delay(Duration::from_millis(200)),
        )
        .expect(2)
        .mount()
        .await;
    let session = test_session(&server).await;
    let account_id = session.info.user_id.clone();
    let cred_dir = tempdir().unwrap();
    let data_dir = tempdir().unwrap();
    let (handle, mut actions, _events) = spawn_actor_with_dirs(cred_dir.path(), data_dir.path());
    assert!(
        handle
            .install_residency_test_session(std::sync::Arc::new(session))
            .await
    );
    let context = handle.avatar_demand_context(account_id);
    for case in 0..2 {
        let uri = format!("mxc://localhost/current-context-{case}");
        let mut demand = koushi_state::AvatarDemandState::new(context.clone());
        demand.open(1).unwrap();
        demand
            .replace(&context, 1, 1, vec![Some(uri.clone())], vec![])
            .unwrap();
        handle.publish_avatar_demand(Some(std::sync::Arc::new(demand)));
        timeout(Duration::from_secs(3), async {
            loop {
                let requests = server.received_requests().await.unwrap_or_default();
                if requests.iter().any(|request| {
                    request
                        .url
                        .path()
                        .ends_with(&format!("/current-context-{case}"))
                }) {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("current demand is running before stale input");
        let mut stale = context.clone();
        if case == 0 {
            stale.account_id = "@other:example.invalid".into();
        } else {
            stale.session_generation += 1;
        }
        // An empty snapshot would cancel the valid fetch if admitted.
        handle.publish_avatar_demand(Some(std::sync::Arc::new(
            koushi_state::AvatarDemandState::new(stale),
        )));
        timeout(Duration::from_secs(3), async {
            'settled: loop {
                for action in actions.recv().await.unwrap() {
                    if matches!(action, AppAction::AvatarThumbnailUpdated { mxc_uri, thumbnail: AvatarThumbnailState::Ready { .. } } if mxc_uri == uri) {
                        break 'settled;
                    }
                }
            }
        }).await.expect("stale input must preserve the active account demand");
    }
    shutdown_and_ack(&handle).await;
}

#[tokio::test]
async fn unchanged_scoped_resources_do_not_echo_cached_updates_on_new_observation_revisions() {
    let _cache_guard = crate::renderable_thumbnail::test_cache_lock_async().await;
    let server = MatrixMockServer::new().await;
    server
        .mock_authed_media_download()
        .respond_with(ResponseTemplate::new(200).set_body_bytes(vec![1, 2, 3]))
        .expect(1)
        .mount()
        .await;
    let session = test_session(&server).await;
    let account_id = session.info.user_id.clone();
    let cred_dir = tempdir().unwrap();
    let data_dir = tempdir().unwrap();
    let (handle, mut actions, _events) = spawn_actor_with_dirs(cred_dir.path(), data_dir.path());
    assert!(
        handle
            .install_residency_test_session(std::sync::Arc::new(session))
            .await
    );
    let context = handle.avatar_demand_context(account_id);
    let uri = "mxc://localhost/unchanged-scope-observation";
    let mut demand = koushi_state::AvatarDemandState::new(context.clone());
    demand.open(1).unwrap();
    demand
        .replace(&context, 1, 1, vec![Some(uri.into())], vec![])
        .unwrap();
    handle.publish_avatar_demand(Some(std::sync::Arc::new(demand.clone())));
    timeout(Duration::from_secs(3), async {
        'ready: loop {
            for action in actions.recv().await.unwrap() {
                if matches!(action, AppAction::AvatarThumbnailUpdated { mxc_uri, thumbnail: AvatarThumbnailState::Ready { .. } } if mxc_uri == uri) { break 'ready; }
            }
        }
    }).await.expect("initial download must settle");
    demand
        .replace(&context, 1, 2, vec![Some(uri.into())], vec![])
        .unwrap();
    handle.publish_avatar_demand(Some(std::sync::Arc::new(demand)));
    // The actor consumes pending watch input before processing this barrier.
    shutdown_and_ack(&handle).await;
    while let Ok(batch) = actions.try_recv() {
        assert!(!batch.iter().any(|action| matches!(action, AppAction::AvatarThumbnailUpdated { mxc_uri, .. } if mxc_uri == uri)), "unchanged demand must not generate another reader-invalidating cache echo");
    }
}

#[tokio::test]
async fn scoped_avatar_watch_cancels_active_and_queued_demand() {
    let _cache_guard = crate::renderable_thumbnail::test_cache_lock_async().await;
    let server = MatrixMockServer::new().await;
    server
        .mock_authed_media_download()
        .respond_with(ResponseTemplate::new(500).set_delay(Duration::from_secs(1)))
        .mount()
        .await;
    let session = test_session(&server).await;
    let account_id = session.info.user_id.clone();
    let cred_dir = tempdir().unwrap();
    let data_dir = tempdir().unwrap();
    let (handle, _actions, _events) = spawn_actor_with_dirs(cred_dir.path(), data_dir.path());
    handle.set_search_crawler_speed(koushi_state::SearchCrawlerSpeed::Fast);
    assert!(
        handle
            .install_residency_test_session(std::sync::Arc::new(session))
            .await
    );
    let context = handle.avatar_demand_context(account_id);
    let mut demand = koushi_state::AvatarDemandState::new(context.clone());
    demand.open(1).unwrap();
    demand
        .replace(
            &context,
            1,
            1,
            (0..20)
                .map(|id| Some(format!("mxc://localhost/scoped-{id}")))
                .collect(),
            vec![],
        )
        .unwrap();
    handle.publish_avatar_demand(Some(std::sync::Arc::new(demand)));
    async fn requests(server: &MatrixMockServer) -> usize {
        server
            .received_requests()
            .await
            .unwrap_or_default()
            .iter()
            .filter(|request| request.url.path().contains("/media/download/"))
            .count()
    }
    timeout(Duration::from_secs(3), async {
        while requests(&server).await < 1 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("the single shared media fetch starts");
    handle.publish_avatar_demand(None);
    tokio::time::sleep(Duration::from_millis(1200)).await;
    assert_eq!(
        requests(&server).await,
        1,
        "closed demand must neither retry nor start queued fetches"
    );
    shutdown_and_ack(&handle).await;
}

#[tokio::test]
async fn canceled_avatar_completion_cannot_settle_a_replacement_in_the_same_session() {
    let _cache_guard = crate::renderable_thumbnail::test_cache_lock_async().await;
    let server = MatrixMockServer::new().await;
    server
        .mock_authed_media_download()
        .respond_with(ResponseTemplate::new(500).set_delay(Duration::from_millis(100)))
        .mount()
        .await;
    let session = test_session(&server).await;
    let cred_dir = tempdir().unwrap();
    let data_dir = tempdir().unwrap();
    let (handle, _action_rx, mut event_rx) =
        spawn_actor_with_dirs(cred_dir.path(), data_dir.path());
    assert!(
        handle
            .install_residency_test_session(std::sync::Arc::new(session))
            .await
    );
    let mxc_uri = "mxc://localhost/replaced-avatar";
    let first = RequestId {
        connection_id: RuntimeConnectionId(10),
        sequence: 1,
    };
    let replacement = RequestId {
        sequence: 2,
        ..first
    };
    handle
        .send(AccountMessage::Command(
            AccountCommand::DownloadAvatarThumbnail {
                request_id: first,
                mxc_uri: mxc_uri.into(),
            },
        ))
        .await;
    handle
        .send(AccountMessage::Command(
            AccountCommand::CancelAvatarThumbnail {
                request_id: RequestId {
                    sequence: 3,
                    ..first
                },
                target_request_id: first,
                mxc_uri: mxc_uri.into(),
            },
        ))
        .await;
    handle
        .send(AccountMessage::Command(
            AccountCommand::DownloadAvatarThumbnail {
                request_id: replacement,
                mxc_uri: mxc_uri.into(),
            },
        ))
        .await;
    // Model an already queued completion from the canceled task. Session identity
    // is unchanged and a new waiter now exists for precisely the same resource.
    handle
        .send(AccountMessage::AvatarFetched {
            mxc_uri: mxc_uri.into(),
            generation: 0,
            fetch_id: tokio::spawn(async {}).id(),
            thumbnail: AvatarThumbnailState::Ready {
                source_ref: "avatar/obsolete".into(),
                width: None,
                height: None,
                mime_type: None,
            },
        })
        .await;
    let result = timeout(Duration::from_secs(3), async {
        loop {
            if let CoreEvent::Account(AccountEvent::AvatarThumbnailDownloaded {
                request_id,
                thumbnail,
                ..
            }) = event_rx.recv().await.unwrap()
                && request_id == replacement
            {
                break thumbnail;
            }
        }
    })
    .await
    .expect("replacement must settle from its own fetch");
    assert!(
        matches!(
            result,
            AvatarThumbnailState::Failed {
                kind: AvatarThumbnailFailureKind::Network,
                ..
            }
        ),
        "obsolete completion settled replacement: {result:?}"
    );
    shutdown_and_ack(&handle).await;
}

#[tokio::test]
async fn avatar_actor_drops_a_late_completion_from_a_retired_session() {
    async fn fetch_id(handle: &super::actor::AccountActorHandle, uri: &str) -> tokio::task::Id {
        let (response_tx, response_rx) = tokio::sync::oneshot::channel();
        handle
            .send(AccountMessage::AvatarFetchIdForTesting {
                mxc_uri: uri.into(),
                response_tx,
            })
            .await;
        timeout(Duration::from_secs(2), response_rx)
            .await
            .expect("actor inspection deadline")
            .expect("actor inspection reply")
            .expect("fetch remains active")
    }

    let _cache_guard = crate::renderable_thumbnail::test_cache_lock_async().await;
    let server = MatrixMockServer::new().await;
    // Keep real network completions outside this bounded fault-injection test.
    // We do not sleep for this delay; actor shutdown aborts the owned tasks.
    server
        .mock_authed_media_download()
        .respond_with(ResponseTemplate::new(500).set_delay(Duration::from_secs(60)))
        .mount()
        .await;
    let first_session = test_session(&server).await;
    let second_session = test_session(&server).await;
    let account_id = first_session.info.user_id.clone();
    let cred_dir = tempdir().unwrap();
    let data_dir = tempdir().unwrap();
    let (handle, mut action_rx, mut event_rx) =
        spawn_actor_with_dirs(cred_dir.path(), data_dir.path());
    assert!(
        handle
            .install_residency_test_session(std::sync::Arc::new(first_session))
            .await
    );
    let old_generation = handle
        .avatar_demand_context(account_id.clone())
        .session_generation;
    let mxc_uri = "mxc://localhost/retired-session-avatar";
    let old_request = RequestId {
        connection_id: RuntimeConnectionId(10),
        sequence: 76,
    };
    let current_request = RequestId {
        sequence: 77,
        ..old_request
    };
    handle
        .send(AccountMessage::Command(
            AccountCommand::DownloadAvatarThumbnail {
                request_id: old_request,
                mxc_uri: mxc_uri.into(),
            },
        ))
        .await;
    let old_id = fetch_id(&handle, mxc_uri).await;

    assert!(
        handle
            .install_residency_test_session(std::sync::Arc::new(second_session))
            .await
    );
    let current_generation = handle.avatar_demand_context(account_id).session_generation;
    assert_ne!(old_generation, current_generation);
    handle
        .send(AccountMessage::Command(
            AccountCommand::DownloadAvatarThumbnail {
                request_id: current_request,
                mxc_uri: mxc_uri.into(),
            },
        ))
        .await;
    let current_id = fetch_id(&handle, mxc_uri).await;
    assert_ne!(old_id, current_id);

    // First deliver the retired task identity, then isolate each fence by
    // deliberately pairing one stale component with one current component.
    for (generation, fetch_id) in [
        (old_generation, old_id),
        (old_generation, current_id),
        (current_generation, old_id),
    ] {
        handle
            .send(AccountMessage::AvatarFetched {
                mxc_uri: mxc_uri.into(),
                generation,
                fetch_id,
                thumbnail: AvatarThumbnailState::Ready {
                    source_ref: "avatar/stale".into(),
                    width: None,
                    height: None,
                    mime_type: None,
                },
            })
            .await;
    }
    handle
        .send(AccountMessage::AvatarFetched {
            mxc_uri: mxc_uri.into(),
            generation: current_generation,
            fetch_id: current_id,
            thumbnail: AvatarThumbnailState::Failed {
                request_id: current_request.sequence,
                kind: AvatarThumbnailFailureKind::Network,
            },
        })
        .await;
    let result = timeout(Duration::from_secs(2), async {
        loop {
            if let CoreEvent::Account(AccountEvent::AvatarThumbnailDownloaded {
                request_id,
                thumbnail,
                ..
            }) = event_rx.recv().await.expect("avatar event")
                && request_id == current_request
            {
                break thumbnail;
            }
        }
    })
    .await
    .expect("current waiter settles from the current completion");
    assert!(
        matches!(
            result,
            AvatarThumbnailState::Failed {
                request_id: 77,
                kind: AvatarThumbnailFailureKind::Network,
            }
        ),
        "stale completion settled the current waiter: {result:?}"
    );
    while let Ok(actions) = action_rx.try_recv() {
        assert!(
            !actions.iter().any(|action| matches!(
                action,
                AppAction::AvatarThumbnailUpdated {
                    thumbnail: AvatarThumbnailState::Ready { .. },
                    ..
                }
            )),
            "stale completion reached the reducer"
        );
    }
    shutdown_and_ack(&handle).await;
}

#[tokio::test]
async fn a_reader_scope_source_shared_with_another_surface_keeps_its_bytes() {
    use crate::renderable_thumbnail::{
        clear_renderable_thumbnail_cache, lookup_renderable_thumbnail,
    };

    let _cache_guard = crate::renderable_thumbnail::test_cache_lock_async().await;
    clear_renderable_thumbnail_cache();
    let server = MatrixMockServer::new().await;
    // The thumbnail endpoint is unmocked, so each distinct source costs one
    // original download: the shared source and the source the reader moves to.
    // A third one means the shared source was fetched again for the second
    // surface instead of being served from the retained bytes.
    server
        .mock_authed_media_download()
        .ok_image()
        .expect(2)
        .mount()
        .await;
    let session = test_session_for_account(&server, "@avatar-shared-surface:example.invalid").await;
    let account_id = session.info.user_id.clone();
    let cred_dir = tempdir().expect("credential tempdir");
    let data_dir = tempdir().expect("data tempdir");
    let (handle, mut action_rx, _event_rx) =
        spawn_actor_with_dirs(cred_dir.path(), data_dir.path());
    assert!(
        handle
            .install_residency_test_session(std::sync::Arc::new(session))
            .await
    );
    let context = handle.avatar_demand_context(account_id);
    let mut demand = koushi_state::AvatarDemandState::new(context.clone());
    let uri = "mxc://localhost/reader-shared";
    demand.open(1).unwrap();
    demand
        .replace(&context, 1, 1, vec![Some(uri.into())], vec![])
        .unwrap();
    handle.publish_avatar_demand(Some(std::sync::Arc::new(demand.clone())));
    let source_ref = await_ready_avatar(&mut action_rx, uri).await;
    assert!(lookup_renderable_thumbnail(&source_ref).is_some());

    // A DM row, header, or profile independently demands the same MXC while the
    // reader still shows it.
    let request = |sequence| RequestId {
        connection_id: RuntimeConnectionId(21),
        sequence,
    };
    handle
        .send(AccountMessage::Command(
            AccountCommand::DownloadAvatarThumbnail {
                request_id: request(1),
                mxc_uri: uri.to_owned(),
            },
        ))
        .await;
    assert_eq!(await_ready_avatar(&mut action_rx, uri).await, source_ref);

    // The reader scope stops demanding the source.
    let other = "mxc://localhost/reader-other";
    demand.close(1);
    demand.open(2).unwrap();
    demand
        .replace(&context, 2, 1, vec![Some(other.into())], vec![])
        .unwrap();
    handle.publish_avatar_demand(Some(std::sync::Arc::new(demand)));
    await_ready_avatar(&mut action_rx, other).await;

    assert!(
        !lookup_renderable_thumbnail(&source_ref)
            .expect("a source another visible surface still shows must keep its retained bytes")
            .bytes
            .is_empty()
    );
    shutdown_and_ack(&handle).await;
}

#[tokio::test]
async fn an_announced_avatar_losing_its_bytes_is_downgraded_not_left_stale() {
    use crate::renderable_thumbnail::{
        MAX_RENDERABLE_THUMBNAIL_ENTRIES, RenderableThumbnailKind,
        clear_renderable_thumbnail_cache, lookup_renderable_thumbnail, store_renderable_thumbnail,
    };

    let _cache_guard = crate::renderable_thumbnail::test_cache_lock_async().await;
    clear_renderable_thumbnail_cache();
    let server = MatrixMockServer::new().await;
    server.mock_authed_media_download().ok_image().mount().await;
    let session = test_session(&server).await;
    let cred_dir = tempdir().expect("credential tempdir");
    let data_dir = tempdir().expect("data tempdir");
    let (handle, mut action_rx, _event_rx) =
        spawn_actor_with_dirs(cred_dir.path(), data_dir.path());
    assert!(
        handle
            .install_residency_test_session(std::sync::Arc::new(session))
            .await
    );
    let uri = "mxc://localhost/announced-avatar";
    let request = |sequence| RequestId {
        connection_id: RuntimeConnectionId(22),
        sequence,
    };
    handle
        .send(AccountMessage::Command(
            AccountCommand::DownloadAvatarThumbnail {
                request_id: request(1),
                mxc_uri: uri.to_owned(),
            },
        ))
        .await;
    let source_ref = await_ready_avatar(&mut action_rx, uri).await;
    assert!(lookup_renderable_thumbnail(&source_ref).is_some());

    // Cache pressure removes the retained bytes while the announced source is
    // still projected as Ready.
    for id in 0..MAX_RENDERABLE_THUMBNAIL_ENTRIES {
        store_renderable_thumbnail(
            RenderableThumbnailKind::Avatar,
            &format!("mxc://localhost/pressure-{id}"),
            vec![0],
        )
        .unwrap();
    }
    assert!(lookup_renderable_thumbnail(&source_ref).is_none());

    // Later avatar activity reconciles the projection: the actor must downgrade
    // it rather than leave Ready pointing at missing bytes.
    handle
        .send(AccountMessage::Command(
            AccountCommand::DownloadAvatarThumbnail {
                request_id: request(2),
                mxc_uri: "mxc://localhost/after-pressure".to_owned(),
            },
        ))
        .await;
    timeout(Duration::from_secs(5), async {
        loop {
            for action in action_rx.recv().await.expect("account action channel") {
                if let AppAction::AvatarThumbnailUpdated {
                    mxc_uri,
                    thumbnail: AvatarThumbnailState::NotRequested,
                } = action
                    && mxc_uri == uri
                {
                    return;
                }
            }
        }
    })
    .await
    .expect("an announced avatar that lost its bytes must be downgraded, not left Ready");

    // The downgraded source rehydrates when it is demanded again.
    handle
        .send(AccountMessage::Command(
            AccountCommand::DownloadAvatarThumbnail {
                request_id: request(3),
                mxc_uri: uri.to_owned(),
            },
        ))
        .await;
    assert_eq!(await_ready_avatar(&mut action_rx, uri).await, source_ref);
    assert!(lookup_renderable_thumbnail(&source_ref).is_some());
    shutdown_and_ack(&handle).await;
}

#[tokio::test]
async fn a_capacity_evicted_demanded_source_is_terminal_until_the_demand_changes() {
    use crate::renderable_thumbnail::{
        MAX_RENDERABLE_THUMBNAIL_ENTRIES, RenderableThumbnailKind,
        clear_renderable_thumbnail_cache, lookup_renderable_thumbnail, store_renderable_thumbnail,
    };

    let _cache_guard = crate::renderable_thumbnail::test_cache_lock_async().await;
    clear_renderable_thumbnail_cache();
    let server = MatrixMockServer::new().await;
    // #1433: the source is fetched once. A capacity-evicted demanded source must
    // not refetch while the demand is unchanged, so no second network request
    // may appear; the healing refetch below is served by the SDK media store.
    server
        .mock_authed_media_download()
        .ok_image()
        .expect(1)
        .mount()
        .await;
    let session = test_session(&server).await;
    let account_id = session.info.user_id.clone();
    let cred_dir = tempdir().expect("credential tempdir");
    let data_dir = tempdir().expect("data tempdir");
    let (handle, mut action_rx, _event_rx) =
        spawn_actor_with_dirs(cred_dir.path(), data_dir.path());
    assert!(
        handle
            .install_residency_test_session(std::sync::Arc::new(session))
            .await
    );
    let context = handle.avatar_demand_context(account_id);
    let mut demand = koushi_state::AvatarDemandState::new(context.clone());
    let uri = "mxc://localhost/demanded-avatar";
    demand.open(1).unwrap();
    demand
        .replace(&context, 1, 1, vec![Some(uri.into())], vec![])
        .unwrap();
    handle.publish_avatar_demand(Some(std::sync::Arc::new(demand.clone())));
    let source_ref = await_ready_avatar(&mut action_rx, uri).await;

    // The bytes are gone while the source is still demanded: the projection must
    // be downgraded instead of claiming bytes that no longer exist.
    for id in 0..MAX_RENDERABLE_THUMBNAIL_ENTRIES {
        store_renderable_thumbnail(
            RenderableThumbnailKind::Avatar,
            &format!("mxc://localhost/demand-pressure-{id}"),
            vec![0],
        )
        .unwrap();
    }
    assert!(lookup_renderable_thumbnail(&source_ref).is_none());

    // A revision bump of the same resource set is not renewed demand: the actor
    // downgrades at this observation and must not refetch into the same full
    // cache, or the working set would refill-evict forever.
    demand
        .replace(&context, 1, 2, vec![Some(uri.into())], vec![])
        .unwrap();
    handle.publish_avatar_demand(Some(std::sync::Arc::new(demand.clone())));
    timeout(Duration::from_secs(5), async {
        loop {
            for action in action_rx.recv().await.expect("account action channel") {
                if let AppAction::AvatarThumbnailUpdated {
                    mxc_uri,
                    thumbnail: AvatarThumbnailState::NotRequested,
                } = action
                    && mxc_uri == uri
                {
                    return;
                }
            }
        }
    })
    .await
    .expect("the capacity-evicted projection is downgraded");
    // Nothing may follow while the demand is unchanged: a refetch would evict
    // another demanded source and repeat forever.
    let refetched = timeout(Duration::from_secs(1), async {
        loop {
            for action in action_rx.recv().await.expect("account action channel") {
                if let AppAction::AvatarThumbnailUpdated {
                    mxc_uri,
                    thumbnail: AvatarThumbnailState::Ready { .. },
                } = action
                    && mxc_uri == uri
                {
                    return;
                }
            }
        }
    })
    .await;
    assert!(
        refetched.is_err(),
        "a capacity-evicted source must not refetch for the same demand"
    );

    // A renderer re-request is answered with the terminal capacity failure.
    let request_id = RequestId {
        connection_id: RuntimeConnectionId(19),
        sequence: 7,
    };
    handle
        .send(AccountMessage::Command(
            AccountCommand::DownloadAvatarThumbnail {
                request_id,
                mxc_uri: uri.to_owned(),
            },
        ))
        .await;
    timeout(Duration::from_secs(5), async {
        loop {
            for action in action_rx.recv().await.expect("account action channel") {
                if let AppAction::AvatarThumbnailUpdated {
                    mxc_uri,
                    thumbnail: AvatarThumbnailState::Failed { kind, .. },
                } = action
                    && mxc_uri == uri
                {
                    assert_eq!(kind, koushi_state::AvatarThumbnailFailureKind::Capacity);
                    return;
                }
            }
        }
    })
    .await
    .expect("a re-request of a capacity-evicted source fails with Capacity");

    // Only a changed demanded resource set retries the source; the refetch is
    // served by the account-keyed SDK media store.
    demand.close(1);
    handle.publish_avatar_demand(Some(std::sync::Arc::new(demand.clone())));
    demand.open(2).unwrap();
    demand
        .replace(&context, 2, 1, vec![Some(uri.into())], vec![])
        .unwrap();
    handle.publish_avatar_demand(Some(std::sync::Arc::new(demand)));
    let healed_ref = await_ready_avatar(&mut action_rx, uri).await;
    assert!(lookup_renderable_thumbnail(&healed_ref).is_some());
    shutdown_and_ack(&handle).await;
}

#[tokio::test]
async fn changing_the_own_avatar_retires_the_superseded_entry() {
    use crate::renderable_thumbnail::{
        is_renderable_thumbnail_cached, lookup_renderable_thumbnail,
    };

    let _cache_guard = crate::renderable_thumbnail::test_cache_lock_async().await;
    let server = MatrixMockServer::new().await;
    server.mock_authed_media_download().ok_image().mount().await;
    let session = test_session(&server).await;
    let account_id = session.info.user_id.clone();
    let cred_dir = tempdir().expect("credential tempdir");
    let data_dir = tempdir().expect("data tempdir");
    let (handle, mut action_rx, _event_rx) =
        spawn_actor_with_dirs(cred_dir.path(), data_dir.path());
    assert!(
        handle
            .install_residency_test_session(std::sync::Arc::new(session))
            .await
    );

    // Establish the own avatar the profile projection reports.
    handle
        .note_own_avatar_source_for_testing(Some("mxc://localhost/own-a".to_owned()))
        .await;
    let request_id = RequestId {
        connection_id: RuntimeConnectionId(13),
        sequence: 1,
    };
    handle
        .send(AccountMessage::Command(
            AccountCommand::DownloadAvatarThumbnail {
                request_id,
                mxc_uri: "mxc://localhost/own-a".to_owned(),
            },
        ))
        .await;
    let source_ref = timeout(Duration::from_secs(3), async {
        'ready: loop {
            for action in action_rx.recv().await.unwrap() {
                if let AppAction::AvatarThumbnailUpdated {
                    mxc_uri,
                    thumbnail: AvatarThumbnailState::Ready { source_ref, .. },
                } = action
                    && mxc_uri == "mxc://localhost/own-a"
                {
                    break 'ready source_ref;
                }
            }
        }
    })
    .await
    .expect("Ready own avatar");
    assert!(is_renderable_thumbnail_cached(&source_ref));

    // The profile moves to a new avatar: the superseded entry is retired.
    handle
        .note_own_avatar_source_for_testing(Some("mxc://localhost/own-b".to_owned()))
        .await;
    timeout(Duration::from_secs(5), async {
        while is_renderable_thumbnail_cached(&source_ref) {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("a replaced own avatar must not keep its retained bytes");
    assert!(lookup_renderable_thumbnail(&source_ref).is_none());
    let _ = account_id;
    shutdown_and_ack(&handle).await;
}

fn encoded_test_image(width: u32, height: u32, format: image::ImageFormat) -> Vec<u8> {
    let image = image::RgbaImage::from_fn(width, height, |x, y| {
        image::Rgba([(x % 251) as u8, (y % 239) as u8, 127, 255])
    });
    let mut bytes = std::io::Cursor::new(Vec::new());
    image::DynamicImage::ImageRgba8(image)
        .write_to(&mut bytes, format)
        .expect("encode avatar fixture");
    bytes.into_inner()
}

fn avatar_policy() -> koushi_media::ImagePreparationPolicy {
    koushi_media::ImagePreparationPolicy {
        target_long_edge: AVATAR_THUMBNAIL_LONG_EDGE,
        quality_percent: 82,
    }
}

#[test]
fn avatar_content_is_resized_to_the_display_budget() {
    let (content, resolution) = prepare_avatar_content(
        encoded_test_image(400, 200, image::ImageFormat::Png),
        AvatarResolution::ServerThumbnail,
        &avatar_policy(),
    )
    .expect("decodable avatar is resized");
    assert_eq!(resolution, AvatarResolution::ServerThumbnail);
    assert_eq!(content.width, Some(u64::from(AVATAR_THUMBNAIL_LONG_EDGE)));
    assert_eq!(content.height, Some(60));
    assert_eq!(content.mime_type.as_deref(), Some("image/webp"));
}

#[test]
fn avatar_content_is_never_upscaled() {
    let (content, resolution) = prepare_avatar_content(
        encoded_test_image(64, 32, image::ImageFormat::Png),
        AvatarResolution::LocalResize,
        &avatar_policy(),
    )
    .expect("small avatar is retained at its own size");
    assert_eq!(resolution, AvatarResolution::LocalResize);
    assert_eq!((content.width, content.height), (Some(64), Some(32)));
}

#[test]
fn heif_avatar_content_reports_the_fetch_resolution() {
    // #1450: the HEIC fixture rides the same avatar entry point as every other
    // source, and the retained content reports the fetch resolution with its
    // WebP dimensions (the 64x64 fixture is inside the 120 px budget).
    let heic = include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../koushi-media/tests/fixtures/heif/opaque.heic"
    ))
    .to_vec();
    let (content, resolution) =
        prepare_avatar_content(heic, AvatarResolution::LocalResize, &avatar_policy())
            .expect("HEIF decodes through the avatar entry point");
    assert_eq!(resolution, AvatarResolution::LocalResize);
    assert_eq!(content.mime_type.as_deref(), Some("image/webp"));
    assert_eq!((content.width, content.height), (Some(64), Some(64)));
}

#[test]
fn undecodable_avatar_content_passes_through_with_its_own_dimensions() {
    // A 1x1 GIF: `koushi_media` treats GIF as non-decodable, so the bytes
    // are kept verbatim while the header still gives the dimensions.
    let gif = base64::Engine::decode(
        &base64::engine::general_purpose::STANDARD,
        "R0lGODlhAQABAIAAAAAAAP///yH5BAEAAAAALAAAAAABAAEAAAIBRAA7",
    )
    .expect("1x1 gif fixture");
    let (content, resolution) =
        prepare_avatar_content(gif.clone(), AvatarResolution::LocalResize, &avatar_policy())
            .expect("non-decodable content is passed through, not resized");
    assert_eq!(resolution, AvatarResolution::OriginalPassthrough);
    assert_eq!(content.bytes, gif);
    assert_eq!(content.mime_type, None, "detected at store time");
}

#[test]
fn undecodable_avatar_content_is_rejected_by_the_avatar_entry_cap() {
    let _cache_guard = crate::renderable_thumbnail::test_cache_lock();
    crate::renderable_thumbnail::clear_renderable_thumbnail_cache();
    let oversized = vec![0_u8; 512 * 1024];
    let content = RenderableThumbnailContent {
        bytes: oversized,
        mime_type: None,
        width: None,
        height: None,
    };
    assert_eq!(
        store_renderable_thumbnail_variant_for_account(
            RenderableThumbnailKind::Avatar,
            "@a",
            "mxc://oversize",
            AVATAR_THUMBNAIL_IDENTITY,
            content,
        ),
        Err(
            crate::renderable_thumbnail::RenderableThumbnailStoreError::TooLarge {
                byte_count: 512 * 1024,
                max_bytes: 256 * 1024,
            }
        )
    );
}
