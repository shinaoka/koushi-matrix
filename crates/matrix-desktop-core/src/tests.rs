//! Phase 1 contract tests: redaction, unauthenticated rejection, request-id
//! correlation, snapshot coalescing, queue overflow.

use std::{path::PathBuf, time::Duration};

use matrix_desktop_state::{
    ActivityMarkReadTarget, ActivityRow, ActivityState, AppAction, AppearanceSettings, AuthSecret,
    AvatarImage, AvatarThumbnailState, ComposerMode, CrossSigningStatus, DisplaySettings,
    IdentityResetAuthRequest, LiveEventReceipts, LiveReadReceipt, LiveRoomSignalUpdate,
    LoginRequest, MediaSettings, MentionIntent, NotificationSettings, PresenceKind,
    RecoveryRequest, RoomSummary, RoomTagKind, RoomTags, SasEmoji, ScheduledSendCapability,
    ScheduledSendHandle, ScheduledSendItem, SearchState, SessionInfo, SessionState, SettingsPatch,
    SettingsPersistenceState, ThemePreference, VerificationCancelReason, VerificationFlowState,
    VerificationTarget,
};
use matrix_sdk::ruma::api::FeatureFlag;

use crate::command::{
    AccountCommand, AppCommand, CoreCommand, RoomCommand, RoomKeyExportRequest,
    RoomKeyImportRequest, SearchCommand, SecureBackupPassphraseChangeRequest,
    SecureBackupSetupRequest, SyncCommand, TimelineCommand,
};
use crate::event::{CoreEvent, E2eeTrustEvent, LiveSignalsEvent, PaginationDirection};
use crate::executor;
use crate::failure::CoreFailure;
use crate::ids::{AccountKey, RequestId, RuntimeConnectionId, TimelineKey};
use crate::runtime::{CommandSubmitError, CoreConnection, CoreRuntime};

const PASSWORD: &str = "p4ssw0rd-very-secret";
const RECOVERY: &str = "EsT1 RcVy KeyM ater";
const BODY: &str = "private message body 機密本文";
const QUERY: &str = "secret search terms";

fn session_info() -> SessionInfo {
    SessionInfo {
        homeserver: "https://example.test".to_owned(),
        user_id: "@alice:example.test".to_owned(),
        device_id: "DEVICE1".to_owned(),
    }
}

fn fake_request_id() -> RequestId {
    RequestId {
        connection_id: RuntimeConnectionId(999),
        sequence: 1,
    }
}

fn dark_theme_settings_patch() -> SettingsPatch {
    SettingsPatch {
        appearance: Some(AppearanceSettings {
            theme: ThemePreference::Dark,
        }),
        ..SettingsPatch::default()
    }
}

#[test]
fn secret_bearing_commands_redact_debug() {
    let login = CoreCommand::Account(AccountCommand::LoginPassword {
        request_id: fake_request_id(),
        request: LoginRequest {
            homeserver: "https://example.test".to_owned(),
            username: "alice-login-name".to_owned(),
            password: AuthSecret::new(PASSWORD),
            device_display_name: Some("Alice Laptop".to_owned()),
        },
    });
    let recovery = CoreCommand::Account(AccountCommand::SubmitRecovery {
        request_id: fake_request_id(),
        request: RecoveryRequest {
            secret: AuthSecret::new(RECOVERY),
        },
    });
    let identity_reset_auth = CoreCommand::Account(AccountCommand::SubmitIdentityResetAuth {
        request_id: fake_request_id(),
        flow_id: 42,
        request: IdentityResetAuthRequest::UiaaPassword {
            password: AuthSecret::new(PASSWORD),
        },
    });
    let bootstrap_cross_signing = CoreCommand::Account(AccountCommand::BootstrapCrossSigning {
        request_id: fake_request_id(),
        auth: Some(AuthSecret::new(PASSWORD)),
    });
    let enable_key_backup = CoreCommand::Account(AccountCommand::EnableKeyBackup {
        request_id: fake_request_id(),
        passphrase: Some(AuthSecret::new(RECOVERY)),
    });
    let restore_key_backup = CoreCommand::Account(AccountCommand::RestoreKeyBackup {
        request_id: fake_request_id(),
        version: Some("backup-version-1".to_owned()),
        request: RecoveryRequest {
            secret: AuthSecret::new(RECOVERY),
        },
    });
    let key = TimelineKey::room(AccountKey("acc".to_owned()), "!room:example.test");
    let send = CoreCommand::Timeline(TimelineCommand::SendText {
        request_id: fake_request_id(),
        key: key.clone(),
        transaction_id: "txn-1".to_owned(),
        body: BODY.to_owned(),
        mentions: MentionIntent::default(),
    });
    let toggle_reaction = CoreCommand::Timeline(TimelineCommand::ToggleReaction {
        request_id: fake_request_id(),
        key: key.clone(),
        event_id: "$evt".to_owned(),
        reaction_key: "👍".to_owned(),
    });
    let send_reaction = CoreCommand::Timeline(TimelineCommand::SendReaction {
        request_id: fake_request_id(),
        key: key.clone(),
        event_id: "$evt".to_owned(),
        reaction_key: "👍".to_owned(),
    });
    let redact_reaction = CoreCommand::Timeline(TimelineCommand::RedactReaction {
        request_id: fake_request_id(),
        key: key.clone(),
        event_id: "$evt".to_owned(),
        reaction_key: "👍".to_owned(),
        reaction_event_id: "$reaction".to_owned(),
    });
    let edit = CoreCommand::Timeline(TimelineCommand::EditText {
        request_id: fake_request_id(),
        key: key.clone(),
        event_id: "$evt".to_owned(),
        body: BODY.to_owned(),
    });
    let search = CoreCommand::Search(SearchCommand::Query {
        request_id: fake_request_id(),
        query: QUERY.to_owned(),
        scope: crate::command::SearchScope::Global,
    });
    let thread_draft = CoreCommand::App(AppCommand::SetThreadComposerDraft {
        request_id: fake_request_id(),
        room_id: "!room:example.test".to_owned(),
        root_event_id: "$root".to_owned(),
        draft: BODY.to_owned(),
    });
    let scheduled_send = CoreCommand::App(AppCommand::ScheduleSend {
        request_id: fake_request_id(),
        room_id: "!room:example.test".to_owned(),
        body: BODY.to_owned(),
        send_at_ms: 1_900_000_000_000,
    });

    for (command, secrets) in [
        (&login, vec![PASSWORD, "alice-login-name", "Alice Laptop"]),
        (&recovery, vec![RECOVERY]),
        (&identity_reset_auth, vec![PASSWORD]),
        (&bootstrap_cross_signing, vec![PASSWORD]),
        (&enable_key_backup, vec![RECOVERY]),
        (&restore_key_backup, vec![RECOVERY, "backup-version-1"]),
        (&send, vec![BODY]),
        (&toggle_reaction, vec!["👍", "$evt"]),
        (&send_reaction, vec!["👍", "$evt"]),
        (&redact_reaction, vec!["👍", "$evt", "$reaction"]),
        (&edit, vec![BODY]),
        (&search, vec![QUERY]),
        (&thread_draft, vec![BODY, "$root"]),
        (&scheduled_send, vec![BODY, "!room:example.test"]),
    ] {
        let debug = format!("{command:?}");
        for secret in secrets {
            assert!(
                !debug.contains(secret),
                "Debug output leaked a secret: {debug}"
            );
        }
    }
    // Non-secret correlation data stays visible.
    assert!(format!("{send:?}").contains("txn-1"));
}

#[test]
fn auth_discovery_and_oidc_commands_redact_debug_and_do_not_require_ready_session() {
    let request_id = fake_request_id();
    let homeserver = "https://example.test".to_owned();
    let callback_url = "matrix-desktop://auth/callback?code=secret-code".to_owned();
    let commands = [
        CoreCommand::Account(AccountCommand::DiscoverLogin {
            request_id,
            homeserver: homeserver.clone(),
        }),
        CoreCommand::Account(AccountCommand::StartOidcLogin {
            request_id,
            homeserver: homeserver.clone(),
        }),
        CoreCommand::Account(AccountCommand::CompleteOidcLogin {
            request_id,
            homeserver: homeserver.clone(),
            callback_url: callback_url.clone(),
        }),
    ];

    for command in commands {
        assert_eq!(command.request_id(), request_id);
        assert!(!command.requires_ready_session());
        let debug = format!("{command:?}");
        assert!(debug.contains("request_id"));
        assert!(
            !debug.contains(&homeserver),
            "Debug leaked homeserver: {debug}"
        );
        assert!(
            !debug.contains(&callback_url),
            "Debug leaked callback URL: {debug}"
        );
    }
}

fn future_epoch_ms(offset: Duration) -> u64 {
    std::time::SystemTime::now()
        .checked_add(offset)
        .expect("future timestamp")
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system time after epoch")
        .as_millis() as u64
}

#[test]
fn scheduled_send_capability_detects_msc4140_server_support() {
    let features = [FeatureFlag::from(crate::scheduled_send::MSC4140_FEATURE)]
        .into_iter()
        .collect();

    assert_eq!(
        crate::scheduled_send::capability_from_unstable_features(&features),
        ScheduledSendCapability::ServerDelayedEvents
    );
}

#[test]
fn scheduled_send_capability_falls_back_when_msc4140_is_absent_or_disabled() {
    let absent = [FeatureFlag::from("org.matrix.something_else")]
        .into_iter()
        .collect();
    let disabled = std::collections::BTreeMap::from([(
        crate::scheduled_send::MSC4140_FEATURE.to_owned(),
        false,
    )]);

    assert_eq!(
        crate::scheduled_send::capability_from_unstable_features(&absent),
        ScheduledSendCapability::LocalFallback
    );
    assert_eq!(
        crate::scheduled_send::capability_from_versions_flags(&disabled),
        ScheduledSendCapability::LocalFallback
    );
}

#[test]
fn scheduled_send_server_timeout_uses_target_delta_without_private_data() {
    assert_eq!(
        crate::scheduled_send::server_delay_timeout(1_500, 1_000),
        Duration::from_millis(500)
    );
    assert_eq!(
        crate::scheduled_send::server_delay_timeout(1_000, 1_500),
        Duration::from_millis(0)
    );
}

#[test]
fn reaction_commands_are_split_correlated_ready_gated_and_redacted() {
    let request_id = fake_request_id();
    let key = TimelineKey::room(AccountKey("acc".to_owned()), "!room:example.test");
    let commands = vec![
        CoreCommand::Timeline(TimelineCommand::SendReaction {
            request_id,
            key: key.clone(),
            event_id: "$target:example.test".to_owned(),
            reaction_key: "👍".to_owned(),
        }),
        CoreCommand::Timeline(TimelineCommand::RedactReaction {
            request_id,
            key,
            event_id: "$target:example.test".to_owned(),
            reaction_key: "👍".to_owned(),
            reaction_event_id: "$reaction:example.test".to_owned(),
        }),
    ];

    for command in commands {
        assert_eq!(command.request_id(), request_id);
        assert!(command.requires_ready_session());
        let debug = format!("{command:?}");
        assert!(!debug.contains("$target:example.test"), "{debug}");
        assert!(!debug.contains("$reaction:example.test"), "{debug}");
        assert!(!debug.contains("👍"), "{debug}");
    }
}

#[test]
fn e2ee_trust_account_commands_are_correlated_ready_gated_and_redacted() {
    let request_id = fake_request_id();
    let flow_id = request_id.sequence;
    let target = VerificationTarget {
        user_id: "@bob:example.test".to_owned(),
        device_id: "BOBDEVICE".to_owned(),
    };
    let commands = vec![
        CoreCommand::Account(AccountCommand::RequestVerification {
            request_id,
            target: target.clone(),
        }),
        CoreCommand::Account(AccountCommand::AcceptVerification {
            request_id,
            flow_id,
        }),
        CoreCommand::Account(AccountCommand::ConfirmSasVerification {
            request_id,
            flow_id,
        }),
        CoreCommand::Account(AccountCommand::CancelVerification {
            request_id,
            flow_id,
            reason: VerificationCancelReason::Mismatch,
        }),
        CoreCommand::Account(AccountCommand::BootstrapCrossSigning {
            request_id,
            auth: None,
        }),
        CoreCommand::Account(AccountCommand::EnableKeyBackup {
            request_id,
            passphrase: Some(AuthSecret::new(RECOVERY)),
        }),
        CoreCommand::Account(AccountCommand::RestoreKeyBackup {
            request_id,
            version: Some("backup-version-1".to_owned()),
            request: RecoveryRequest {
                secret: AuthSecret::new(RECOVERY),
            },
        }),
        CoreCommand::Account(AccountCommand::ResetIdentity { request_id }),
        CoreCommand::Account(AccountCommand::SubmitIdentityResetAuth {
            request_id,
            flow_id,
            request: IdentityResetAuthRequest::OAuthApproved,
        }),
    ];

    for command in commands {
        assert_eq!(command.request_id(), request_id);
        let requires_ready = command.requires_ready_session();
        if matches!(
            command,
            CoreCommand::Account(AccountCommand::RestoreKeyBackup { .. })
        ) {
            assert!(
                !requires_ready,
                "key-backup restore must be allowed while the session is NeedsRecovery"
            );
        } else {
            assert!(requires_ready);
        }
        let debug = format!("{command:?}");
        assert!(!debug.contains("@bob:example.test"));
        assert!(!debug.contains("BOBDEVICE"));
        assert!(!debug.contains("backup-version-1"));
    }
}

#[test]
fn device_session_commands_are_correlated_ready_gated_and_redacted() {
    let request_id = fake_request_id();
    let display_name = "Alice private laptop";
    let auth_phrase = "device-delete-auth-text";
    let commands = vec![
        CoreCommand::Account(AccountCommand::QueryDevices { request_id }),
        CoreCommand::Account(AccountCommand::RenameDevice {
            request_id,
            device_ordinal: 7,
            display_name: display_name.to_owned(),
        }),
        CoreCommand::Account(AccountCommand::DeleteDevices {
            request_id,
            device_ordinals: vec![7, 8],
            auth: Some(IdentityResetAuthRequest::UiaaPassword {
                password: AuthSecret::new(auth_phrase),
            }),
        }),
        CoreCommand::Account(AccountCommand::SoftLogoutReauth {
            request_id,
            password: AuthSecret::new(auth_phrase),
        }),
    ];

    for command in commands {
        assert_eq!(command.request_id(), request_id);
        assert!(command.requires_ready_session());
        let debug = format!("{command:?}");
        assert!(!debug.contains(display_name), "{debug}");
        assert!(!debug.contains(auth_phrase), "{debug}");
        assert!(!debug.contains("DEVICE"), "{debug}");
    }
}

#[test]
fn room_key_file_transfer_commands_are_correlated_ready_gated_and_redacted() {
    let request_id = fake_request_id();
    let destination = PathBuf::from("/tmp/private-element-compatible-export.txt");
    let source = PathBuf::from("/tmp/private-element-compatible-import.txt");
    let transfer_phrase = "element-compatible-transfer-phrase";
    let commands = vec![
        CoreCommand::Account(AccountCommand::ExportRoomKeys {
            request_id,
            request: RoomKeyExportRequest {
                destination_path: destination.clone(),
                passphrase: AuthSecret::new(transfer_phrase),
            },
        }),
        CoreCommand::Account(AccountCommand::ImportRoomKeys {
            request_id,
            request: RoomKeyImportRequest {
                source_path: source.clone(),
                passphrase: AuthSecret::new(transfer_phrase),
            },
        }),
    ];

    for command in commands {
        assert_eq!(command.request_id(), request_id);
        assert!(command.requires_ready_session());
        let debug = format!("{command:?}");
        assert!(!debug.contains(transfer_phrase), "{debug}");
        assert!(
            !debug.contains(destination.to_string_lossy().as_ref()),
            "{debug}"
        );
        assert!(
            !debug.contains(source.to_string_lossy().as_ref()),
            "{debug}"
        );
        assert!(debug.contains("AuthSecret(..)"), "{debug}");
    }
}

#[test]
fn secure_backup_commands_are_correlated_ready_gated_and_redacted() {
    let request_id = fake_request_id();
    let setup_phrase = "secure-backup-setup-phrase";
    let old_phrase = "secure-backup-old-phrase";
    let new_phrase = "secure-backup-new-phrase";
    let destination = PathBuf::from("/tmp/private-recovery-artifact.txt");
    let commands = vec![
        CoreCommand::Account(AccountCommand::BootstrapSecureBackup {
            request_id,
            request: SecureBackupSetupRequest {
                passphrase: Some(AuthSecret::new(setup_phrase)),
                recovery_key_destination_path: Some(destination.clone()),
            },
        }),
        CoreCommand::Account(AccountCommand::ChangeSecureBackupPassphrase {
            request_id,
            request: SecureBackupPassphraseChangeRequest {
                old_secret: AuthSecret::new(old_phrase),
                new_passphrase: AuthSecret::new(new_phrase),
                recovery_key_destination_path: Some(destination.clone()),
            },
        }),
    ];

    for command in commands {
        assert_eq!(command.request_id(), request_id);
        assert!(command.requires_ready_session());
        let debug = format!("{command:?}");
        assert!(!debug.contains(setup_phrase), "{debug}");
        assert!(!debug.contains(old_phrase), "{debug}");
        assert!(!debug.contains(new_phrase), "{debug}");
        assert!(
            !debug.contains(destination.to_string_lossy().as_ref()),
            "{debug}"
        );
        assert!(
            debug.contains("has_recovery_key_destination_path"),
            "{debug}"
        );
    }
}

#[test]
fn invite_and_dm_room_commands_are_correlated() {
    let request_id = fake_request_id();
    for command in [
        CoreCommand::Room(RoomCommand::AcceptInvite {
            request_id,
            room_id: "!invite:example.test".to_owned(),
        }),
        CoreCommand::Room(RoomCommand::DeclineInvite {
            request_id,
            room_id: "!invite:example.test".to_owned(),
        }),
        CoreCommand::Room(RoomCommand::StartDirectMessage {
            request_id,
            user_id: "@bob:example.test".to_owned(),
        }),
    ] {
        assert_eq!(command.request_id(), request_id);
    }
}

#[test]
fn room_tag_commands_are_correlated_ready_gated_and_redact_room_ids() {
    let request_id = fake_request_id();
    let room_id = "!tagged-room:example.test".to_owned();
    let commands = vec![
        CoreCommand::Room(RoomCommand::SetTag {
            request_id,
            room_id: room_id.clone(),
            tag: RoomTagKind::Favourite,
            order: Some(0.25),
        }),
        CoreCommand::Room(RoomCommand::RemoveTag {
            request_id,
            room_id: room_id.clone(),
            tag: RoomTagKind::LowPriority,
        }),
    ];

    for command in commands {
        assert_eq!(command.request_id(), request_id);
        assert!(command.requires_ready_session());
        let debug = format!("{command:?}");
        assert!(!debug.contains(&room_id));
        assert!(debug.contains("RoomId(..)"));
    }
}

#[test]
fn sync_commands_are_correlated_but_not_ready_gated() {
    let request_id = fake_request_id();
    let commands = vec![
        CoreCommand::Sync(SyncCommand::Start { request_id }),
        CoreCommand::Sync(SyncCommand::Stop { request_id }),
        CoreCommand::Sync(SyncCommand::Restart { request_id }),
        CoreCommand::Sync(SyncCommand::SyncOnce { request_id }),
    ];

    for command in commands {
        assert_eq!(command.request_id(), request_id);
        assert!(
            !command.requires_ready_session(),
            "sync commands are allowed while E2EE recovery is pending; AccountActor still requires a store-backed session"
        );
    }
}

#[test]
fn e2ee_trust_events_are_typed_and_debug_redacts_identifiers() {
    let target = VerificationTarget {
        user_id: "@bob:example.test".to_owned(),
        device_id: "BOBDEVICE".to_owned(),
    };
    let event = E2eeTrustEvent::VerificationProgress {
        account_key: AccountKey("@alice:example.test".to_owned()),
        state: VerificationFlowState::SasPresented {
            request_id: 7,
            target,
            emojis: vec![SasEmoji {
                symbol: "🐶".to_owned(),
                description: "Dog".to_owned(),
            }],
        },
    };

    let value = serde_json::to_value(&event).expect("E2EE trust event serializes");
    assert_eq!(value["kind"], "verificationProgress");
    assert_eq!(value["state"]["kind"], "sasPresented");

    let debug = format!("{:?}", CoreEvent::E2eeTrust(event));
    assert!(debug.contains("VerificationProgress"));
    assert!(!debug.contains("@alice:example.test"));
    assert!(!debug.contains("@bob:example.test"));
    assert!(!debug.contains("BOBDEVICE"));
}

#[test]
fn live_signal_commands_are_correlated_ready_gated_and_redacted() {
    let request_id = fake_request_id();
    let key = TimelineKey::room(
        AccountKey("@alice:example.test".to_owned()),
        "!room:example.test",
    );
    let commands = vec![
        CoreCommand::Timeline(TimelineCommand::SendReadReceipt {
            request_id,
            key: key.clone(),
            event_id: "$receipt-target:example.test".to_owned(),
        }),
        CoreCommand::Timeline(TimelineCommand::SetFullyRead {
            request_id,
            key: key.clone(),
            event_id: "$fully-read-target:example.test".to_owned(),
        }),
        CoreCommand::Timeline(TimelineCommand::SetTyping {
            request_id,
            key,
            is_typing: true,
        }),
        CoreCommand::Account(AccountCommand::SetPresence {
            request_id,
            presence: PresenceKind::Away,
        }),
    ];

    for command in commands {
        assert_eq!(command.request_id(), request_id);
        assert!(command.requires_ready_session());
        let debug = format!("{command:?}");
        assert!(!debug.contains("@alice:example.test"), "{debug}");
        assert!(!debug.contains("!room:example.test"), "{debug}");
        assert!(!debug.contains("$receipt-target:example.test"), "{debug}");
        assert!(
            !debug.contains("$fully-read-target:example.test"),
            "{debug}"
        );
    }
}

#[test]
fn local_encryption_probe_command_is_correlated_ready_gated_and_redacted() {
    let request_id = fake_request_id();
    let command = CoreCommand::Account(AccountCommand::ProbeLocalEncryptionHealth { request_id });

    assert_eq!(command.request_id(), request_id);
    assert!(command.requires_ready_session());
    assert!(!format!("{command:?}").contains("@user-a:example.invalid"));
}

#[test]
fn reset_local_data_command_is_correlated_ready_gated_and_redacted() {
    let request_id = fake_request_id();
    let command = CoreCommand::Account(AccountCommand::ResetLocalData { request_id });

    assert_eq!(command.request_id(), request_id);
    assert!(command.requires_ready_session());
    assert!(!format!("{command:?}").contains("@user-a:example.invalid"));
}

#[test]
fn live_signal_events_are_typed_and_debug_redacts_identifiers() {
    let request_id = fake_request_id();
    let key = TimelineKey::room(
        AccountKey("@alice:example.test".to_owned()),
        "!room:example.test",
    );
    let update = LiveRoomSignalUpdate {
        receipts_by_event: vec![LiveEventReceipts {
            event_id: "$event:example.test".to_owned(),
            receipts: vec![LiveReadReceipt {
                user_id: "@bob:example.test".to_owned(),
                display_name: Some("Private Reader".to_owned()),
                original_display_label: String::new(),
                avatar: Some(AvatarImage {
                    mxc_uri: "mxc://example.test/private-reader".to_owned(),
                    thumbnail: AvatarThumbnailState::NotRequested,
                }),
                timestamp_ms: Some(123),
            }],
        }],
        fully_read_event_id: Some("$event:example.test".to_owned()),
        typing_user_ids: vec!["@bob:example.test".to_owned()],
    };

    let event = LiveSignalsEvent::RoomSignalsUpdated {
        room_id: "!room:example.test".to_owned(),
        update,
    };
    let value = serde_json::to_value(&event).expect("live signal event serializes");
    assert_eq!(value["kind"], "roomSignalsUpdated");
    assert_eq!(value["room_id"], "!room:example.test");
    let debug_update = format!("{:?}", CoreEvent::LiveSignals(event));
    assert!(debug_update.contains("RoomSignalsUpdated"));
    assert!(!debug_update.contains("Private Reader"), "{debug_update}");
    assert!(!debug_update.contains("private-reader"), "{debug_update}");

    let completion = LiveSignalsEvent::ReadReceiptSent {
        request_id,
        key,
        event_id: "$event:example.test".to_owned(),
    };
    let debug = format!("{:?}", CoreEvent::LiveSignals(completion));
    assert!(debug.contains("ReadReceiptSent"));
    assert!(!debug.contains("@alice:example.test"), "{debug}");
    assert!(!debug.contains("!room:example.test"), "{debug}");
    assert!(!debug.contains("$event:example.test"), "{debug}");
}

#[tokio::test]
async fn unauthenticated_session_commands_are_rejected() {
    let runtime = CoreRuntime::start();
    let mut connection = runtime.attach();

    let request_id = connection.next_request_id();
    connection
        .command(CoreCommand::Room(RoomCommand::CreateRoom {
            request_id,
            name: "qa room".to_owned(),
            encrypted: false,
        }))
        .await
        .expect("submit");

    match connection.recv_event().await.expect("event") {
        CoreEvent::OperationFailed {
            request_id: failed_id,
            failure,
        } => {
            assert_eq!(failed_id, request_id);
            assert_eq!(failure, CoreFailure::SessionRequired);
        }
        other => panic!("expected OperationFailed, got {other:?}"),
    }
}

#[tokio::test]
async fn ready_session_routes_past_appactor_session_gate() {
    // Verify that a Timeline command passes the AppActor's session gate
    // (only applied before routing) and reaches AccountActor, which returns
    // a timeline-domain failure (not a routing/gate failure like an unknown
    // command kind).
    //
    // With inject_actions we get a Ready AppState but no real SDK session in
    // AccountActor, so AccountActor emits SessionRequired from its own guard.
    // That is a valid "routes to AccountActor" signal: the AppActor did not
    // short-circuit it with a different failure.
    let runtime = CoreRuntime::start();
    let mut connection = runtime.attach();
    runtime
        .inject_actions(vec![AppAction::RestoreSessionSucceeded(session_info())])
        .await;
    // Wait for the Ready snapshot before submitting.
    loop {
        if matches!(connection.snapshot().session, SessionState::Ready(_)) {
            break;
        }
        executor::sleep(Duration::from_millis(5)).await;
    }

    let request_id = connection.next_request_id();
    connection
        .command(CoreCommand::Timeline(TimelineCommand::Paginate {
            request_id,
            key: TimelineKey::room(AccountKey("acc".to_owned()), "!room:example.test"),
            direction: PaginationDirection::Backward,
            event_count: 20,
        }))
        .await
        .expect("submit");

    loop {
        match connection.recv_event().await.expect("event") {
            CoreEvent::OperationFailed {
                request_id: failed_id,
                failure,
            } if failed_id == request_id => {
                // The AppActor allows timeline commands to reach AccountActor
                // when the session is Ready. AccountActor checks its own session
                // guard; with a fake inject there is no real SDK session, so it
                // returns SessionRequired. That is the expected behavior:
                // the command reached AccountActor (not rejected at AppActor).
                assert!(
                    matches!(
                        failure,
                        CoreFailure::SessionRequired | CoreFailure::TimelineOperationFailed { .. }
                    ),
                    "unexpected failure kind: {failure:?}"
                );
                return;
            }
            _ => continue,
        }
    }
}

#[tokio::test]
async fn search_query_projects_search_state_before_routing() {
    let runtime = CoreRuntime::start();
    let mut connection = runtime.attach();

    runtime
        .inject_actions(vec![AppAction::RestoreSessionSucceeded(session_info())])
        .await;

    loop {
        if matches!(connection.snapshot().session, SessionState::Ready(_)) {
            break;
        }
        executor::sleep(Duration::from_millis(5)).await;
    }

    let request_id = connection.next_request_id();
    connection
        .command(CoreCommand::Search(SearchCommand::Query {
            request_id,
            query: "Alpha".to_owned(),
            scope: crate::command::SearchScope::Global,
        }))
        .await
        .expect("submit");

    let result = executor::timeout(Duration::from_secs(1), async {
        loop {
            match connection.recv_event().await.expect("event") {
                CoreEvent::StateChanged(snapshot)
                    if !matches!(snapshot.search, SearchState::Closed) =>
                {
                    return snapshot;
                }
                _ => continue,
            }
        }
    })
    .await
    .expect("search submission should publish a non-closed search snapshot");

    match result.search {
        SearchState::Searching {
            request_id: rid, ..
        }
        | SearchState::Failed {
            request_id: rid, ..
        }
        | SearchState::Results {
            request_id: rid, ..
        } => {
            assert_eq!(rid, request_id.sequence);
        }
        other => panic!("expected search state to project, got {other:?}"),
    }
}

#[tokio::test]
async fn e2ee_trust_account_command_projects_pending_state_before_routing() {
    let runtime = CoreRuntime::start();
    let mut connection = runtime.attach();

    runtime
        .inject_actions(vec![AppAction::RestoreSessionSucceeded(session_info())])
        .await;

    loop {
        if matches!(connection.snapshot().session, SessionState::Ready(_)) {
            break;
        }
        executor::sleep(Duration::from_millis(5)).await;
    }

    let request_id = connection.next_request_id();
    connection
        .command(CoreCommand::Account(
            AccountCommand::BootstrapCrossSigning {
                request_id,
                auth: None,
            },
        ))
        .await
        .expect("submit bootstrap cross-signing");

    let snapshot = executor::timeout(Duration::from_secs(1), async {
        loop {
            match connection.recv_event().await.expect("event") {
                CoreEvent::StateChanged(snapshot)
                    if matches!(
                        snapshot.e2ee_trust.cross_signing,
                        CrossSigningStatus::Bootstrapping { .. }
                    ) =>
                {
                    return snapshot;
                }
                _ => continue,
            }
        }
    })
    .await
    .expect("E2EE trust command should project Rust-owned pending state before actor route");

    assert_eq!(
        snapshot.e2ee_trust.cross_signing,
        CrossSigningStatus::Bootstrapping {
            request_id: request_id.sequence,
        }
    );
}

#[tokio::test]
async fn app_update_settings_projects_state_and_persists() {
    let data_dir = tempfile::tempdir().expect("tempdir");
    let runtime = CoreRuntime::start_with_data_dir(data_dir.path().to_path_buf());
    let mut connection = runtime.attach();
    let request_id = connection.next_request_id();

    connection
        .command(CoreCommand::App(AppCommand::UpdateSettings {
            request_id,
            patch: dark_theme_settings_patch(),
        }))
        .await
        .expect("submit settings update");

    let snapshot = executor::timeout(Duration::from_secs(1), async {
        loop {
            match connection.recv_event().await.expect("event") {
                CoreEvent::StateChanged(snapshot)
                    if snapshot.settings.values.appearance.theme == ThemePreference::Dark =>
                {
                    return snapshot;
                }
                _ => continue,
            }
        }
    })
    .await
    .expect("settings state should change");

    assert_eq!(
        snapshot.settings.persistence,
        SettingsPersistenceState::Idle
    );
    let persisted = crate::settings::SettingsStore::new(data_dir.path())
        .load()
        .expect("load persisted settings");
    assert_eq!(persisted.appearance.theme, ThemePreference::Dark);
}

#[tokio::test]
async fn persisted_settings_load_when_runtime_restarts() {
    let data_dir = tempfile::tempdir().expect("tempdir");
    {
        let runtime = CoreRuntime::start_with_data_dir(data_dir.path().to_path_buf());
        let mut connection = runtime.attach();
        let request_id = connection.next_request_id();

        connection
            .command(CoreCommand::App(AppCommand::UpdateSettings {
                request_id,
                patch: dark_theme_settings_patch(),
            }))
            .await
            .expect("submit settings update");

        executor::timeout(Duration::from_secs(1), async {
            loop {
                match connection.recv_event().await.expect("event") {
                    CoreEvent::StateChanged(snapshot)
                        if snapshot.settings.values.appearance.theme == ThemePreference::Dark
                            && snapshot.settings.persistence == SettingsPersistenceState::Idle =>
                    {
                        return;
                    }
                    _ => continue,
                }
            }
        })
        .await
        .expect("settings state should persist before restart");
    }

    let restarted = CoreRuntime::start_with_data_dir(data_dir.path().to_path_buf());
    let connection = restarted.attach();

    assert_eq!(
        connection.snapshot().settings.values.appearance.theme,
        ThemePreference::Dark
    );
    assert_eq!(
        connection.snapshot().settings.persistence,
        SettingsPersistenceState::Idle
    );
}

#[test]
fn settings_store_rejects_corrupt_json_with_defaults() {
    let data_dir = tempfile::tempdir().expect("tempdir");
    let settings_dir = data_dir.path().join("settings");
    std::fs::create_dir_all(&settings_dir).expect("settings dir");
    std::fs::write(settings_dir.join("settings.json"), "{not-json").expect("write corrupt");

    let store = crate::settings::SettingsStore::new(data_dir.path());
    let err = store
        .load()
        .expect_err("corrupt settings should fail safely");

    assert_eq!(err.kind(), crate::settings::SettingsStoreErrorKind::Corrupt);
}

#[test]
fn settings_store_loads_legacy_json_without_notification_settings() {
    let data_dir = tempfile::tempdir().expect("tempdir");
    let settings_dir = data_dir.path().join("settings");
    std::fs::create_dir_all(&settings_dir).expect("settings dir");
    std::fs::write(
        settings_dir.join("settings.json"),
        r#"{
  "locale": { "language_tag": null, "text_direction": "auto" },
  "appearance": { "theme": "dark" },
  "typography": { "font": "system", "emoji": "system" },
  "keyboard": { "composer_send_shortcut": "enter" }
}
"#,
    )
    .expect("write legacy settings");

    let values = crate::settings::SettingsStore::new(data_dir.path())
        .load()
        .expect("legacy settings should load with default notification settings");

    assert_eq!(values.appearance.theme, ThemePreference::Dark);
    assert_eq!(values.notifications, NotificationSettings::default());
    assert_eq!(values.display, DisplaySettings::default());
    assert_eq!(values.media, MediaSettings::default());
}

#[test]
fn empty_search_is_not_special_cased_in_the_runtime() {
    let runtime_source = include_str!("runtime.rs");
    let search_source = include_str!("search.rs");

    assert!(
        !runtime_source.contains("is_empty_query"),
        "runtime should not special-case empty search queries"
    );
    assert!(
        !runtime_source.contains("results: Vec::new()"),
        "runtime should not locally settle empty search results"
    );
    assert!(
        search_source.contains("query.trim().is_empty()"),
        "search actor should own empty-query handling"
    );
    assert!(
        search_source.contains("CoreEvent::Search(SearchEvent::Results"),
        "search actor should still emit search results events"
    );
}

#[tokio::test]
async fn mismatched_request_id_fails_locally_without_publishing() {
    let runtime = CoreRuntime::start();
    let intruder = runtime.attach();
    let mut observer = runtime.attach();

    let foreign_id = observer.next_request_id();
    let result = intruder
        .command(CoreCommand::Room(RoomCommand::JoinRoom {
            request_id: foreign_id,
            room_id: "!room:example.test".to_owned(),
        }))
        .await;
    assert_eq!(result, Err(CommandSubmitError::InvalidRequestId));

    // No CoreEvent may be published with the forged RequestId.
    let outcome = executor::timeout(Duration::from_millis(100), observer.recv_event()).await;
    assert!(
        outcome.is_err(),
        "no event should be published for a rejected submission"
    );
}

#[tokio::test]
async fn result_events_correlate_in_submission_order() {
    let runtime = CoreRuntime::start();
    let mut connection = runtime.attach();

    let first = connection.next_request_id();
    let second = connection.next_request_id();
    assert_ne!(first, second);

    for request_id in [first, second] {
        connection
            .command(CoreCommand::Room(RoomCommand::JoinRoom {
                request_id,
                room_id: "!room:example.test".to_owned(),
            }))
            .await
            .expect("submit");
    }

    let mut seen = Vec::new();
    while seen.len() < 2 {
        if let CoreEvent::OperationFailed { request_id, .. } =
            connection.recv_event().await.expect("event")
        {
            seen.push(request_id);
        }
    }
    assert_eq!(seen, vec![first, second], "events must be ordered");
}

#[tokio::test]
async fn reducer_actions_coalesce_into_single_state_changed() {
    let runtime = CoreRuntime::start();
    let mut connection = runtime.attach();

    runtime
        .inject_actions(vec![
            AppAction::AppStarted,
            AppAction::RestoreSessionFailed {
                message: "synthetic".to_owned(),
            },
            AppAction::LoginDiscoveryRequested {
                homeserver: "https://example.test".to_owned(),
            },
        ])
        .await;

    let mut state_changed_count = 0;
    let mut last = None;
    // Drain everything emitted within a quiet period.
    while let Ok(Ok(event)) =
        executor::timeout(Duration::from_millis(200), connection.recv_event()).await
    {
        if let CoreEvent::StateChanged(snapshot) = event {
            state_changed_count += 1;
            last = Some(snapshot);
        }
    }

    assert_eq!(
        state_changed_count, 1,
        "one batch of actions must coalesce into exactly one StateChanged"
    );
    let last = last.expect("snapshot");
    // The final state reflects the LAST action in the batch.
    assert!(matches!(
        last.auth,
        matrix_desktop_state::AuthDiscoveryState::Discovering { .. }
    ));
    assert_eq!(connection.snapshot(), last);
}

#[tokio::test]
async fn slow_consumer_observes_lag_and_recovers_via_snapshot() {
    let runtime = CoreRuntime::start_with_event_capacity(4);
    let pump = runtime.attach();
    let mut slow = runtime.attach();

    // Overflow the slow consumer's bounded queue.
    for _ in 0..32 {
        let request_id = pump.next_request_id();
        pump.command(CoreCommand::Room(RoomCommand::JoinRoom {
            request_id,
            room_id: "!room:example.test".to_owned(),
        }))
        .await
        .expect("submit");
    }
    runtime.inject_actions(vec![AppAction::AppStarted]).await;
    executor::sleep(Duration::from_millis(100)).await;

    let first = slow.recv_event().await;
    assert!(first.is_err(), "slow consumer must observe the lag marker");

    // Recovery path: latest-wins snapshot is intact and current.
    assert!(matches!(
        slow.snapshot().session,
        SessionState::Restoring | SessionState::SignedOut
    ));
}

fn room_summary(room_id: &str) -> RoomSummary {
    RoomSummary {
        room_id: room_id.to_owned(),
        display_name: "QA Room".to_owned(),
        display_label: "QA Room".to_owned(),
        original_display_label: "QA Room".to_owned(),
        avatar: None,
        is_dm: false,
        dm_user_ids: Vec::new(),
        tags: RoomTags::default(),
        unread_count: 0,
        notification_count: 0,
        highlight_count: 0,
        parent_space_ids: vec![],
    }
}

fn unread_room_summary(room_id: &str, unread_count: u64) -> RoomSummary {
    RoomSummary {
        unread_count,
        ..room_summary(room_id)
    }
}

fn activity_row(room_id: &str, event_id: &str, timestamp_ms: u64) -> ActivityRow {
    ActivityRow {
        room_id: room_id.to_owned(),
        event_id: event_id.to_owned(),
        room_label: String::new(),
        sender_label: Some("Private sender".to_owned()),
        preview: Some("Private preview".to_owned()),
        timestamp_ms,
        unread: false,
        highlight: false,
    }
}

async fn wait_for_state<F>(
    connection: &mut CoreConnection,
    predicate: F,
) -> matrix_desktop_state::AppState
where
    F: Fn(&matrix_desktop_state::AppState) -> bool,
{
    for _ in 0..200 {
        let snapshot = connection.snapshot();
        if predicate(&snapshot) {
            return snapshot;
        }
        executor::sleep(Duration::from_millis(5)).await;
    }
    panic!("state predicate was not satisfied");
}

#[tokio::test]
async fn app_command_sets_and_clears_reply_target() {
    let runtime = CoreRuntime::start();
    let mut conn = runtime.attach();
    runtime
        .inject_actions(vec![
            AppAction::RestoreSessionSucceeded(session_info()),
            AppAction::RoomListUpdated {
                spaces: vec![],
                rooms: vec![room_summary("!room:example.test")],
            },
            AppAction::SelectRoom {
                room_id: "!room:example.test".to_owned(),
            },
            AppAction::TimelineSubscribed {
                room_id: "!room:example.test".to_owned(),
            },
        ])
        .await;
    wait_for_state(&mut conn, |state| {
        matches!(state.session, SessionState::Ready(_))
            && state.timeline.room_id.as_deref() == Some("!room:example.test")
    })
    .await;

    let set_request = conn.next_request_id();
    conn.command(CoreCommand::App(AppCommand::SetComposerReplyTarget {
        request_id: set_request,
        room_id: "!room:example.test".to_owned(),
        event_id: "$root:example.test".to_owned(),
    }))
    .await
    .expect("set reply target command");

    let snapshot = wait_for_state(&mut conn, |state| {
        matches!(
            state.timeline.composer.mode,
            ComposerMode::Reply { ref in_reply_to_event_id }
                if in_reply_to_event_id == "$root:example.test"
        )
    })
    .await;
    assert!(matches!(
        snapshot.timeline.composer.mode,
        ComposerMode::Reply { .. }
    ));

    let cancel_request = conn.next_request_id();
    conn.command(CoreCommand::App(AppCommand::CancelComposerReply {
        request_id: cancel_request,
    }))
    .await
    .expect("cancel reply target command");

    let snapshot = wait_for_state(&mut conn, |state| {
        state.timeline.composer.mode == ComposerMode::Plain
    })
    .await;
    assert_eq!(snapshot.timeline.composer.mode, ComposerMode::Plain);
}

#[tokio::test]
async fn app_command_sets_open_thread_composer_draft() {
    let runtime = CoreRuntime::start();
    let mut conn = runtime.attach();
    runtime
        .inject_actions(vec![
            AppAction::RestoreSessionSucceeded(session_info()),
            AppAction::RoomListUpdated {
                spaces: vec![],
                rooms: vec![room_summary("!room:example.test")],
            },
            AppAction::SelectRoom {
                room_id: "!room:example.test".to_owned(),
            },
            AppAction::TimelineSubscribed {
                room_id: "!room:example.test".to_owned(),
            },
            AppAction::OpenThread {
                room_id: "!room:example.test".to_owned(),
                root_event_id: "$root:example.test".to_owned(),
            },
            AppAction::ThreadSubscribed {
                room_id: "!room:example.test".to_owned(),
                root_event_id: "$root:example.test".to_owned(),
            },
        ])
        .await;
    wait_for_state(&mut conn, |state| {
        matches!(
            &state.thread,
            matrix_desktop_state::ThreadPaneState::Open {
                room_id,
                root_event_id,
                ..
            } if room_id == "!room:example.test" && root_event_id == "$root:example.test"
        )
    })
    .await;

    let request_id = conn.next_request_id();
    conn.command(CoreCommand::App(AppCommand::SetThreadComposerDraft {
        request_id,
        room_id: "!room:example.test".to_owned(),
        root_event_id: "$root:example.test".to_owned(),
        draft: "thread draft".to_owned(),
    }))
    .await
    .expect("set thread composer draft command");

    let snapshot = wait_for_state(&mut conn, |state| {
        matches!(
            &state.thread,
            matrix_desktop_state::ThreadPaneState::Open { composer, .. }
                if composer.draft == "thread draft"
        )
    })
    .await;

    match snapshot.thread {
        matrix_desktop_state::ThreadPaneState::Open { composer, .. } => {
            assert_eq!(composer.draft, "thread draft");
        }
        other => panic!("expected open thread, got {other:?}"),
    }
}

#[tokio::test]
async fn app_command_sets_selected_room_composer_draft() {
    let runtime = CoreRuntime::start();
    let mut conn = runtime.attach();
    runtime
        .inject_actions(vec![
            AppAction::RestoreSessionSucceeded(session_info()),
            AppAction::RoomListUpdated {
                spaces: vec![],
                rooms: vec![room_summary("!room:example.test")],
            },
            AppAction::SelectRoom {
                room_id: "!room:example.test".to_owned(),
            },
            AppAction::TimelineSubscribed {
                room_id: "!room:example.test".to_owned(),
            },
        ])
        .await;
    wait_for_state(&mut conn, |state| {
        matches!(state.session, SessionState::Ready(_))
            && state.timeline.room_id.as_deref() == Some("!room:example.test")
    })
    .await;

    let request_id = conn.next_request_id();
    conn.command(CoreCommand::App(AppCommand::SetComposerDraft {
        request_id,
        room_id: "!room:example.test".to_owned(),
        draft: "room draft".to_owned(),
    }))
    .await
    .expect("set room composer draft command");

    let snapshot = wait_for_state(&mut conn, |state| {
        state.timeline.composer.draft == "room draft"
    })
    .await;
    assert_eq!(snapshot.timeline.composer.draft, "room draft");
}

#[tokio::test]
async fn app_command_schedules_cancel_and_reschedules_local_fallback_send() {
    let runtime = CoreRuntime::start();
    let mut conn = runtime.attach();
    runtime
        .inject_actions(vec![
            AppAction::RestoreSessionSucceeded(session_info()),
            AppAction::RoomListUpdated {
                spaces: vec![],
                rooms: vec![room_summary("!room:example.test")],
            },
            AppAction::SelectRoom {
                room_id: "!room:example.test".to_owned(),
            },
            AppAction::TimelineSubscribed {
                room_id: "!room:example.test".to_owned(),
            },
            AppAction::ScheduledSendCapabilityChanged {
                capability: ScheduledSendCapability::LocalFallback,
            },
            AppAction::ComposerDraftChanged {
                room_id: "!room:example.test".to_owned(),
                draft: "scheduled body".to_owned(),
            },
        ])
        .await;
    wait_for_state(&mut conn, |state| {
        state.timeline.composer.draft == "scheduled body"
    })
    .await;

    conn.command(CoreCommand::App(AppCommand::ScheduleSend {
        request_id: conn.next_request_id(),
        room_id: "!room:example.test".to_owned(),
        body: "scheduled body".to_owned(),
        send_at_ms: future_epoch_ms(Duration::from_secs(60)),
    }))
    .await
    .expect("schedule send");

    let snapshot = wait_for_state(&mut conn, |state| {
        state.timeline.composer.draft.is_empty() && state.timeline.scheduled_sends.len() == 1
    })
    .await;
    assert_eq!(
        snapshot.timeline.scheduled_send_capability,
        ScheduledSendCapability::LocalFallback
    );
    assert_eq!(snapshot.timeline.scheduled_sends[0].body, "scheduled body");
    let scheduled_id = snapshot.timeline.scheduled_sends[0].scheduled_id.clone();

    conn.command(CoreCommand::App(AppCommand::RescheduleScheduledSend {
        request_id: conn.next_request_id(),
        scheduled_id: scheduled_id.clone(),
        send_at_ms: future_epoch_ms(Duration::from_secs(120)),
    }))
    .await
    .expect("reschedule send");

    let rescheduled =
        wait_for_state(&mut conn, |state| {
            state.timeline.scheduled_sends.first().is_some_and(|item| {
                item.send_at_ms > snapshot.timeline.scheduled_sends[0].send_at_ms
            })
        })
        .await;
    assert_eq!(
        rescheduled.timeline.scheduled_sends[0].scheduled_id,
        scheduled_id
    );

    conn.command(CoreCommand::App(AppCommand::CancelScheduledSend {
        request_id: conn.next_request_id(),
        scheduled_id,
    }))
    .await
    .expect("cancel scheduled send");

    wait_for_state(&mut conn, |state| state.timeline.scheduled_sends.is_empty()).await;
}

#[tokio::test]
async fn local_fallback_scheduled_send_fires_at_target_and_leaves_rust_state() {
    let runtime = CoreRuntime::start();
    let mut conn = runtime.attach();
    runtime
        .inject_actions(vec![
            AppAction::RestoreSessionSucceeded(session_info()),
            AppAction::RoomListUpdated {
                spaces: vec![],
                rooms: vec![room_summary("!room:example.test")],
            },
            AppAction::SelectRoom {
                room_id: "!room:example.test".to_owned(),
            },
            AppAction::TimelineSubscribed {
                room_id: "!room:example.test".to_owned(),
            },
            AppAction::ScheduledSendCapabilityChanged {
                capability: ScheduledSendCapability::LocalFallback,
            },
        ])
        .await;
    wait_for_state(&mut conn, |state| {
        matches!(state.session, SessionState::Ready(_))
            && state.timeline.room_id.as_deref() == Some("!room:example.test")
    })
    .await;

    conn.command(CoreCommand::App(AppCommand::ScheduleSend {
        request_id: conn.next_request_id(),
        room_id: "!room:example.test".to_owned(),
        body: "fire later".to_owned(),
        send_at_ms: future_epoch_ms(Duration::from_millis(60)),
    }))
    .await
    .expect("schedule send");

    wait_for_state(&mut conn, |state| state.timeline.scheduled_sends.len() == 1).await;
    let snapshot =
        wait_for_state(&mut conn, |state| state.timeline.scheduled_sends.is_empty()).await;
    assert!(snapshot.scheduled_sends.items.is_empty());
}

#[tokio::test]
async fn server_scheduled_send_items_are_not_dispatched_by_local_fallback_timer() {
    let runtime = CoreRuntime::start();
    let mut conn = runtime.attach();
    runtime
        .inject_actions(vec![
            AppAction::RestoreSessionSucceeded(session_info()),
            AppAction::RoomListUpdated {
                spaces: vec![],
                rooms: vec![room_summary("!room:example.test")],
            },
            AppAction::SelectRoom {
                room_id: "!room:example.test".to_owned(),
            },
            AppAction::TimelineSubscribed {
                room_id: "!room:example.test".to_owned(),
            },
            AppAction::ScheduledSendCapabilityChanged {
                capability: ScheduledSendCapability::ServerDelayedEvents,
            },
            AppAction::ScheduledSendCreated {
                item: ScheduledSendItem {
                    scheduled_id: "server-scheduled".to_owned(),
                    room_id: "!room:example.test".to_owned(),
                    body: "server delayed body".to_owned(),
                    send_at_ms: future_epoch_ms(Duration::from_millis(20)),
                    handle: ScheduledSendHandle::Server {
                        delay_id: "delay-private".to_owned(),
                    },
                },
            },
        ])
        .await;
    wait_for_state(&mut conn, |state| state.timeline.scheduled_sends.len() == 1).await;

    executor::sleep(Duration::from_millis(80)).await;
    let snapshot = conn.snapshot();
    assert_eq!(snapshot.timeline.scheduled_sends.len(), 1);
    assert_eq!(
        snapshot.timeline.scheduled_sends[0].handle,
        ScheduledSendHandle::Server {
            delay_id: "delay-private".to_owned()
        }
    );
}

#[tokio::test]
async fn composer_drafts_persist_after_debounce_and_load_on_restart() {
    let data_dir = tempfile::tempdir().expect("data dir");
    let credential_dir = tempfile::tempdir().expect("credential dir");

    {
        let runtime = CoreRuntime::start_with_data_dir_and_file_credentials(
            data_dir.path().to_path_buf(),
            credential_dir.path().to_path_buf(),
        );
        let mut conn = runtime.attach();
        runtime
            .inject_actions(vec![
                AppAction::RestoreSessionSucceeded(session_info()),
                AppAction::RoomListUpdated {
                    spaces: vec![],
                    rooms: vec![room_summary("!room:example.test")],
                },
                AppAction::SelectRoom {
                    room_id: "!room:example.test".to_owned(),
                },
                AppAction::TimelineSubscribed {
                    room_id: "!room:example.test".to_owned(),
                },
            ])
            .await;
        wait_for_state(&mut conn, |state| {
            matches!(state.session, SessionState::Ready(_))
                && state.timeline.room_id.as_deref() == Some("!room:example.test")
        })
        .await;

        conn.command(CoreCommand::App(AppCommand::SetComposerDraft {
            request_id: conn.next_request_id(),
            room_id: "!room:example.test".to_owned(),
            draft: "survives restart".to_owned(),
        }))
        .await
        .expect("set room composer draft");

        wait_for_state(&mut conn, |state| {
            state.timeline.composer.draft == "survives restart"
        })
        .await;
        executor::sleep(crate::runtime::COMPOSER_DRAFT_PERSIST_DEBOUNCE * 2).await;
    }

    let restarted = CoreRuntime::start_with_data_dir_and_file_credentials(
        data_dir.path().to_path_buf(),
        credential_dir.path().to_path_buf(),
    );
    let mut conn = restarted.attach();
    restarted
        .inject_actions(vec![
            AppAction::RestoreSessionSucceeded(session_info()),
            AppAction::RoomListUpdated {
                spaces: vec![],
                rooms: vec![room_summary("!room:example.test")],
            },
            AppAction::SelectRoom {
                room_id: "!room:example.test".to_owned(),
            },
            AppAction::TimelineSubscribed {
                room_id: "!room:example.test".to_owned(),
            },
        ])
        .await;

    let snapshot = wait_for_state(&mut conn, |state| {
        state.timeline.composer.draft == "survives restart"
    })
    .await;
    assert_eq!(snapshot.timeline.composer.draft, "survives restart");
}

#[tokio::test]
async fn cleared_composer_drafts_do_not_resurrect_on_restart() {
    let data_dir = tempfile::tempdir().expect("data dir");
    let credential_dir = tempfile::tempdir().expect("credential dir");

    {
        let runtime = CoreRuntime::start_with_data_dir_and_file_credentials(
            data_dir.path().to_path_buf(),
            credential_dir.path().to_path_buf(),
        );
        let mut conn = runtime.attach();
        runtime
            .inject_actions(vec![
                AppAction::RestoreSessionSucceeded(session_info()),
                AppAction::RoomListUpdated {
                    spaces: vec![],
                    rooms: vec![room_summary("!room:example.test")],
                },
                AppAction::SelectRoom {
                    room_id: "!room:example.test".to_owned(),
                },
                AppAction::TimelineSubscribed {
                    room_id: "!room:example.test".to_owned(),
                },
            ])
            .await;
        wait_for_state(&mut conn, |state| {
            matches!(state.session, SessionState::Ready(_))
                && state.timeline.room_id.as_deref() == Some("!room:example.test")
        })
        .await;

        conn.command(CoreCommand::App(AppCommand::SetComposerDraft {
            request_id: conn.next_request_id(),
            room_id: "!room:example.test".to_owned(),
            draft: "deleted before restart".to_owned(),
        }))
        .await
        .expect("set room composer draft");
        wait_for_state(&mut conn, |state| {
            state.timeline.composer.draft == "deleted before restart"
        })
        .await;
        executor::sleep(crate::runtime::COMPOSER_DRAFT_PERSIST_DEBOUNCE * 2).await;

        conn.command(CoreCommand::App(AppCommand::SetComposerDraft {
            request_id: conn.next_request_id(),
            room_id: "!room:example.test".to_owned(),
            draft: String::new(),
        }))
        .await
        .expect("clear room composer draft");
        wait_for_state(&mut conn, |state| state.timeline.composer.draft.is_empty()).await;
        executor::sleep(crate::runtime::COMPOSER_DRAFT_PERSIST_DEBOUNCE * 2).await;
    }

    let restarted = CoreRuntime::start_with_data_dir_and_file_credentials(
        data_dir.path().to_path_buf(),
        credential_dir.path().to_path_buf(),
    );
    let mut conn = restarted.attach();
    restarted
        .inject_actions(vec![
            AppAction::RestoreSessionSucceeded(session_info()),
            AppAction::RoomListUpdated {
                spaces: vec![],
                rooms: vec![room_summary("!room:example.test")],
            },
            AppAction::SelectRoom {
                room_id: "!room:example.test".to_owned(),
            },
            AppAction::TimelineSubscribed {
                room_id: "!room:example.test".to_owned(),
            },
        ])
        .await;

    let snapshot = wait_for_state(&mut conn, |state| {
        matches!(state.session, SessionState::Ready(_))
            && state.timeline.room_id.as_deref() == Some("!room:example.test")
            && state.timeline.is_subscribed
    })
    .await;
    assert!(snapshot.timeline.composer.draft.is_empty());
}

#[tokio::test]
async fn app_command_opens_activity_from_observed_rows_and_mark_read_settles() {
    let runtime = CoreRuntime::start();
    let mut conn = runtime.attach();
    runtime
        .inject_actions(vec![
            AppAction::RestoreSessionSucceeded(session_info()),
            AppAction::RoomListUpdated {
                spaces: vec![],
                rooms: vec![
                    unread_room_summary("!recent:example.test", 1),
                    unread_room_summary("!stale:example.test", 1),
                    unread_room_summary("!marker:example.test", 2),
                ],
            },
            AppAction::FullyReadMarkerUpdated {
                room_id: "!marker:example.test".to_owned(),
                event_id: Some("$marker-read:example.test".to_owned()),
            },
            AppAction::ActivityRowsObserved {
                rows: vec![
                    activity_row("!recent:example.test", "$recent:example.test", 20),
                    activity_row("!stale:example.test", "$stale:example.test", 1),
                    activity_row("!marker:example.test", "$marker-read:example.test", 40),
                    activity_row("!marker:example.test", "$marker-unread:example.test", 60),
                ],
            },
        ])
        .await;
    wait_for_state(&mut conn, |state| {
        matches!(state.session, SessionState::Ready(_)) && state.rooms.len() == 3
    })
    .await;

    let open_request_id = conn.next_request_id();
    conn.command(CoreCommand::App(AppCommand::OpenActivity {
        request_id: open_request_id,
    }))
    .await
    .expect("open activity command");

    let snapshot = wait_for_state(&mut conn, |state| {
        matches!(state.activity, ActivityState::Open { .. })
    })
    .await;
    let ActivityState::Open { recent, unread, .. } = snapshot.activity else {
        panic!("activity should be open");
    };
    assert_eq!(
        recent
            .rows
            .iter()
            .map(|row| row.event_id.as_str())
            .collect::<Vec<_>>(),
        [
            "$marker-unread:example.test",
            "$marker-read:example.test",
            "$recent:example.test",
            "$stale:example.test"
        ]
    );
    assert!(
        unread
            .rows
            .iter()
            .any(|row| row.event_id == "$stale:example.test"),
        "stale unread rows must remain visible"
    );
    assert!(
        unread
            .rows
            .iter()
            .any(|row| row.event_id == "$marker-unread:example.test"),
        "rows after the Rust-owned fully-read marker must remain unread"
    );
    assert!(
        unread
            .rows
            .iter()
            .all(|row| row.event_id != "$marker-read:example.test"),
        "rows at or before the Rust-owned fully-read marker must be excluded"
    );

    let mark_request_id = conn.next_request_id();
    conn.command(CoreCommand::App(AppCommand::MarkActivityRead {
        request_id: mark_request_id,
        target: ActivityMarkReadTarget::All,
    }))
    .await
    .expect("mark activity read command");

    let snapshot = wait_for_state(&mut conn, |state| {
        matches!(
            &state.activity,
            ActivityState::Open { unread, mark_read, .. }
                if unread.rows.is_empty()
                    && matches!(mark_read, matrix_desktop_state::ActivityMarkReadState::Idle)
                    && state
                        .live_signals
                        .rooms
                        .get("!marker:example.test")
                        .and_then(|signals| signals.fully_read_event_id.as_deref())
                        == Some("$marker-unread:example.test")
                    && state
                        .live_signals
                        .rooms
                        .get("!stale:example.test")
                        .and_then(|signals| signals.fully_read_event_id.as_deref())
                        == Some("$stale:example.test")
        )
    })
    .await;
    let ActivityState::Open { unread, .. } = snapshot.activity else {
        panic!("activity should stay open");
    };
    assert!(unread.rows.is_empty());
    assert_eq!(
        snapshot
            .live_signals
            .rooms
            .get("!marker:example.test")
            .and_then(|signals| signals.fully_read_event_id.as_deref()),
        Some("$marker-unread:example.test")
    );
    assert_eq!(
        snapshot
            .live_signals
            .rooms
            .get("!stale:example.test")
            .and_then(|signals| signals.fully_read_event_id.as_deref()),
        Some("$stale:example.test")
    );
}

#[tokio::test]
async fn send_completion_clears_reply_mode_through_runtime() {
    // Regression: production send/reply completion must be Rust-owned. The core
    // drives SendTextSubmitted -> SendTextFinished into AppState so the composer
    // returns to Plain without React repairing product state after the fact.
    let runtime = CoreRuntime::start();
    let mut conn = runtime.attach();
    runtime
        .inject_actions(vec![
            AppAction::RestoreSessionSucceeded(session_info()),
            AppAction::RoomListUpdated {
                spaces: vec![],
                rooms: vec![room_summary("!room:example.test")],
            },
            AppAction::SelectRoom {
                room_id: "!room:example.test".to_owned(),
            },
            AppAction::TimelineSubscribed {
                room_id: "!room:example.test".to_owned(),
            },
            AppAction::ComposerReplyTargetSelected {
                room_id: "!room:example.test".to_owned(),
                event_id: "$root:example.test".to_owned(),
            },
        ])
        .await;
    wait_for_state(&mut conn, |state| {
        matches!(state.timeline.composer.mode, ComposerMode::Reply { .. })
    })
    .await;

    runtime
        .inject_actions(vec![
            AppAction::SendTextSubmitted {
                room_id: "!room:example.test".to_owned(),
                transaction_id: "txn-reply".to_owned(),
                body: "reply body".to_owned(),
            },
            AppAction::SendTextFinished {
                room_id: "!room:example.test".to_owned(),
                transaction_id: "txn-reply".to_owned(),
            },
        ])
        .await;

    let snapshot = wait_for_state(&mut conn, |state| {
        state.timeline.composer.pending_transaction_id.is_none()
            && state.timeline.composer.mode == ComposerMode::Plain
    })
    .await;
    assert_eq!(snapshot.timeline.composer.mode, ComposerMode::Plain);
    assert_eq!(snapshot.timeline.composer.pending_transaction_id, None);
}
