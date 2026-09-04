use koushi_protocol::{
    command::AccountCommand,
    event::{AccountEvent, CoreEvent},
    ids::{RequestId, RuntimeConnectionId},
};
use koushi_state::{AppAction, AvatarThumbnailState};
use tempfile::tempdir;

use super::{
    actor::AccountMessage,
    test_support::{shutdown_and_ack, spawn_actor_with_dirs},
};

#[tokio::test]
async fn cached_avatar_download_dispatches_authoritative_update() {
    let cred_dir = tempdir().expect("credential tempdir");
    let data_dir = tempdir().expect("data tempdir");
    let (handle, mut action_rx, mut event_rx) =
        spawn_actor_with_dirs(cred_dir.path(), data_dir.path());
    let mxc_uri = "mxc://example.invalid/cached-avatar";
    let thumbnail = AvatarThumbnailState::Ready {
        source_ref: "avatar/test".to_owned(),
        width: None,
        height: None,
        mime_type: Some("image/png".to_owned()),
    };

    handle
        .send(AccountMessage::AvatarFetched {
            mxc_uri: mxc_uri.to_owned(),
            generation: 0,
            thumbnail: thumbnail.clone(),
        })
        .await;
    assert_eq!(
        action_rx.recv().await.expect("seed avatar action"),
        vec![AppAction::AvatarThumbnailUpdated {
            mxc_uri: mxc_uri.to_owned(),
            thumbnail: thumbnail.clone(),
        }]
    );

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
            thumbnail: thumbnail.clone(),
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
            assert_eq!(event_thumbnail, thumbnail);
        }
        event => panic!("unexpected cached avatar event: {event:?}"),
    }

    shutdown_and_ack(&handle).await;
}
