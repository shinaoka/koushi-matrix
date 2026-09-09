use koushi_protocol::{
    command::AccountCommand,
    event::{AccountEvent, CoreEvent},
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
    test_support::{shutdown_and_ack, spawn_actor_with_dirs},
};

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
    let _cache_guard = crate::renderable_thumbnail::test_cache_lock();
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

#[tokio::test]
async fn avatar_actor_bounds_distinct_demand_and_rejects_capacity() {
    let _cache_guard = crate::renderable_thumbnail::test_cache_lock();
    let server = MatrixMockServer::new().await;
    server
        .mock_authed_media_download()
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_raw(b"delayed-avatar", "image/jpeg")
                .set_delay(Duration::from_secs(5)),
        )
        .expect(6)
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
    timeout(Duration::from_secs(2), async {
        loop {
            if server.received_requests().await.is_some_and(|requests| {
                requests
                    .iter()
                    .filter(|request| request.url.path().contains("/avatar-"))
                    .count()
                    == super::profile::AVATAR_DOWNLOAD_CONCURRENCY
            }) {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("all active avatar requests should reach the delayed server");
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

    let capacity_event = timeout(Duration::from_secs(2), async {
        loop {
            match event_rx.recv().await.expect("avatar event") {
                CoreEvent::Account(AccountEvent::AvatarThumbnailDownloaded {
                    request_id,
                    thumbnail: AvatarThumbnailState::Failed { kind, .. },
                    ..
                }) if request_id.sequence == capacity_sequence as u64 => {
                    break kind;
                }
                _ => {}
            }
        }
    })
    .await
    .expect("capacity request should settle without waiting for active downloads");
    assert_eq!(capacity_event, AvatarThumbnailFailureKind::Capacity);
    let _ = action_rx.try_recv().expect("capacity state action");

    shutdown_and_ack(&handle).await;
}

#[tokio::test]
async fn canceling_last_avatar_waiter_aborts_active_fetch_and_admits_pending_work() {
    let _cache_guard = crate::renderable_thumbnail::test_cache_lock();
    let server = MatrixMockServer::new().await;
    server
        .mock_authed_media_download()
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_raw(b"delayed-avatar", "image/jpeg")
                .set_delay(Duration::from_secs(5)),
        )
        .expect(7)
        .mount()
        .await;
    let session = test_session(&server).await;
    let cred_dir = tempdir().expect("credential tempdir");
    let data_dir = tempdir().expect("data tempdir");
    let (handle, _action_rx, mut event_rx) =
        spawn_actor_with_dirs(cred_dir.path(), data_dir.path());
    assert!(
        handle
            .install_residency_test_session(std::sync::Arc::new(session))
            .await
    );

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
                    == super::profile::AVATAR_DOWNLOAD_CONCURRENCY
            }) {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("all active avatar requests should reach the delayed server");

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
async fn scoped_avatar_watch_cancels_active_and_queued_demand() {
    let _cache_guard = crate::renderable_thumbnail::test_cache_lock();
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
        while requests(&server).await < 6 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("six scoped fetches start");
    handle.publish_avatar_demand(None);
    tokio::time::sleep(Duration::from_millis(1200)).await;
    assert_eq!(
        requests(&server).await,
        6,
        "closed demand must neither retry nor start queued fetches"
    );
    shutdown_and_ack(&handle).await;
}

#[tokio::test]
async fn canceled_avatar_completion_cannot_settle_a_replacement_in_the_same_session() {
    let _cache_guard = crate::renderable_thumbnail::test_cache_lock();
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
            {
                if request_id == replacement {
                    break thumbnail;
                }
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
    let _cache_guard = crate::renderable_thumbnail::test_cache_lock();
    let server = MatrixMockServer::new().await;
    server
        .mock_authed_media_download()
        .error500()
        .expect(2)
        .mount()
        .await;
    let first_session = test_session(&server).await;
    let second_session = test_session(&server).await;
    let cred_dir = tempdir().expect("credential tempdir");
    let data_dir = tempdir().expect("data tempdir");
    let (handle, _action_rx, mut event_rx) =
        spawn_actor_with_dirs(cred_dir.path(), data_dir.path());
    assert!(
        handle
            .install_residency_test_session(std::sync::Arc::new(first_session))
            .await
    );
    assert!(
        handle
            .install_residency_test_session(std::sync::Arc::new(second_session))
            .await
    );

    let mxc_uri = "mxc://localhost/retired-session-avatar";
    handle
        .send(AccountMessage::AvatarFetched {
            mxc_uri: mxc_uri.to_owned(),
            generation: 0,
            fetch_id: tokio::spawn(async {}).id(),
            thumbnail: AvatarThumbnailState::Ready {
                source_ref: "avatar/stale".to_owned(),
                width: None,
                height: None,
                mime_type: Some("image/jpeg".to_owned()),
            },
        })
        .await;
    handle
        .send(AccountMessage::Command(
            AccountCommand::DownloadAvatarThumbnail {
                request_id: RequestId {
                    connection_id: RuntimeConnectionId(10),
                    sequence: 77,
                },
                mxc_uri: mxc_uri.to_owned(),
            },
        ))
        .await;

    let result = timeout(Duration::from_secs(2), async {
        loop {
            match event_rx.recv().await.expect("avatar event") {
                CoreEvent::Account(AccountEvent::AvatarThumbnailDownloaded {
                    request_id,
                    mxc_uri: event_mxc_uri,
                    thumbnail,
                }) if request_id.sequence == 77 && event_mxc_uri == mxc_uri => {
                    break thumbnail;
                }
                _ => {}
            }
        }
    })
    .await
    .expect("current session request should settle");
    assert!(matches!(
        result,
        AvatarThumbnailState::Failed {
            kind: AvatarThumbnailFailureKind::Network,
            ..
        }
    ));

    shutdown_and_ack(&handle).await;
}
