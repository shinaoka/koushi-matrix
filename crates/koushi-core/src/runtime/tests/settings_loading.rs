//! Account settings never transiently replace valid policy or stall navigation.
use super::*;

#[tokio::test]
async fn failed_settings_load_defers_safe_policy_and_retry_replaces_held_values() {
    let data_dir = tempfile::tempdir().expect("runtime data directory");
    let info = SessionInfo {
        homeserver: "https://example.invalid".to_owned(),
        user_id: "@settings:example.invalid".to_owned(),
        device_id: "SYNTHETIC".to_owned(),
        authentication_method: koushi_state::SessionAuthenticationMethod::Unknown,
    };
    let key = session_key_id_from_info(&info);
    let (mut actor, _, _, mut account_rx, mut event_rx, ..) =
        app_actor_fixture_with_account_capacity(
            data_dir.path(),
            AppState {
                session: SessionState::Ready(info),
                ..AppState::default()
            },
            1,
        );
    let path = actor
        .composer_draft_store_actor
        .account_local_data_dir(&key)
        .join("settings/account-settings.v1.enc");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, b"corrupt settings").unwrap();
    assert!(
        actor
            .account_actor
            .send(AccountMessage::CancelActivityResolution)
            .await
    );

    executor::timeout(
        Duration::from_millis(250),
        actor.load_account_settings_for_current_session(),
    )
    .await
    .expect("failed load must not wait for mailbox capacity");
    assert!(actor.account_settings_loaded_for.is_none());
    assert!(!actor.state.settings.values.notifications.send_read_receipts);
    assert_eq!(std::fs::read(&path).unwrap(), b"corrupt settings");
    let safe_values = actor.state.settings.values.clone();
    let request_id = RequestId {
        connection_id: RuntimeConnectionId(19),
        sequence: 1,
    };
    assert!(
        !actor
            .handle_public_command(CoreCommandEnvelope::Public {
                command: CoreCommand::App(AppCommand::UpdateSettings {
                    request_id,
                    patch: SettingsPatch {
                        scope: Some(koushi_state::SettingsPatchScope::Account),
                        notifications: Some(koushi_state::NotificationSettings::default()),
                        ..SettingsPatch::default()
                    },
                }),
                composer_permit: None,
                admission: None,
            })
            .await
    );
    assert_eq!(
        actor.state.settings.values, safe_values,
        "a rejected write must not enable privacy-sensitive policy"
    );
    assert!(std::iter::from_fn(|| event_rx.try_recv().ok()).any(|event| matches!(event,
        CoreEvent::OperationFailed { request_id: id, failure: CoreFailure::StoreUnavailable } if id == request_id)));
    assert!(matches!(
        account_rx.recv().await,
        Some(AccountMessage::CancelActivityResolution)
    ));
    actor
        .deliver_deferred_account_dispatch(actor.account_actor.reserve_owned().await)
        .await;
    assert!(matches!(
        account_rx.recv().await,
        Some(AccountMessage::ReadStatePolicyChanged {
            send_read_receipts: false
        })
    ));

    // Leave display/link policies held, then repair and retry. Only the new
    // policies may follow; the transient safe defaults must not be replayed.
    actor
        .composer_draft_store_actor
        .save_account_settings(&key, &koushi_state::AccountSettingsValues::default())
        .unwrap();
    assert!(
        actor
            .account_actor
            .send(AccountMessage::CancelActivityResolution)
            .await
    );
    executor::timeout(
        Duration::from_millis(250),
        actor.load_account_settings_for_current_session(),
    )
    .await
    .expect("successful retry must not wait for mailbox capacity");
    assert_eq!(actor.account_settings_loaded_for, Some(key));
    assert!(actor.state.settings.values.notifications.send_read_receipts);
    account_rx.recv().await.unwrap();
    for index in 0..3 {
        actor
            .deliver_deferred_account_dispatch(actor.account_actor.reserve_owned().await)
            .await;
        let message = account_rx.recv().await.unwrap();
        match index {
            0 => assert!(matches!(
                message,
                AccountMessage::ReadStatePolicyChanged {
                    send_read_receipts: true
                }
            )),
            1 => assert!(matches!(
                message,
                AccountMessage::DisplayPolicyChanged { .. }
            )),
            _ => assert!(matches!(
                message,
                AccountMessage::TimelineCommand(TimelineCommand::BroadcastLinkPreviewPolicy {
                    unencrypted_global_enabled: true,
                    ..
                })
            )),
        }
    }
    // Content-policy changes also invalidate the crawler's completed cache.
    actor
        .deliver_deferred_account_dispatch(actor.account_actor.reserve_owned().await)
        .await;
    assert!(matches!(
        account_rx.recv().await,
        Some(AccountMessage::InvalidateSearchCrawlerCache)
    ));
    assert!(!actor.deferred_account_dispatch.is_pending());
}
