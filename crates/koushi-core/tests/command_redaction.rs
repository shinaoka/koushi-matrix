use std::path::PathBuf;

use koushi_core::command::{
    AccountCommand, AppCommand, CoreCommand, RoomCommand, RoomKeyExportRequest,
    RoomKeyImportRequest, SearchCommand, SearchScope, SecureBackupPassphraseChangeRequest,
    SecureBackupSetupRequest, SyncCommand, TimelineCommand,
};
use koushi_core::event::{AccountEvent, CoreEvent};
use koushi_core::ids::{AccountKey, TimelineKey};
use koushi_state::{
    AuthSecret, IdentityResetAuthRequest, LoginRequest, MentionIntent, PresenceKind,
    RecoveryRequest, RoomTagKind, TimelineScrollAnchor, TimelineScrollAnchorEdge,
    VerificationCancelReason, VerificationTarget,
};

mod support;
use support::*;

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
        scope: SearchScope::Global,
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
    let timeline_scroll_anchor = CoreCommand::App(AppCommand::TimelineScrollAnchorUpdated {
        request_id: fake_request_id(),
        room_id: "!room:example.test".to_owned(),
        anchor: TimelineScrollAnchor {
            event_id: "$anchor:example.test".to_owned(),
            edge: TimelineScrollAnchorEdge::Top,
            offset_px: 32,
            updated_at_ms: 1_900_000_000_000,
        },
    });
    let restore_timeline_anchor = CoreCommand::Timeline(TimelineCommand::RestoreTimelineAnchor {
        request_id: fake_request_id(),
        key: key.clone(),
        event_id: "$anchor:example.test".to_owned(),
        max_batches: 6,
        event_count: 100,
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
        (
            &timeline_scroll_anchor,
            vec!["!room:example.test", "$anchor:example.test"],
        ),
        (
            &restore_timeline_anchor,
            vec!["!room:example.test", "$anchor:example.test"],
        ),
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
    let callback_url = "koushi-desktop://auth/callback?code=secret-code".to_owned();
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

#[test]
fn oidc_authorization_event_redacts_debug() {
    let url = "https://issuer.example.test/auth?code=secret".to_owned();
    let state = "csrf-secret".to_owned();
    let event = CoreEvent::Account(AccountEvent::OidcAuthorizationCreated {
        request_id: fake_request_id(),
        authorization_url: url.clone(),
        state: state.clone(),
    });

    let debug = format!("{event:?}");

    assert!(debug.contains("OidcAuthorizationCreated"));
    assert!(!debug.contains(&url));
    assert!(!debug.contains(&state));
    assert!(!debug.contains("issuer.example.test"));
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
fn mark_room_as_read_and_unread_commands_are_correlated_and_ready_gated() {
    let request_id = fake_request_id();
    let read = CoreCommand::Room(RoomCommand::MarkRoomAsRead {
        request_id,
        room_id: "!room:example.test".to_owned(),
        event_id: "$event:example.test".to_owned(),
    });
    let unread = CoreCommand::Room(RoomCommand::MarkRoomAsUnread {
        request_id,
        room_id: "!room:example.test".to_owned(),
        unread: true,
    });

    assert_eq!(read.request_id(), request_id);
    assert!(
        read.requires_ready_session(),
        "mark read command requires a ready session"
    );
    let debug = format!("{read:?}");
    assert!(!debug.contains("!room:example.test"));
    assert!(!debug.contains("$event:example.test"));
    assert!(debug.contains("RoomId(..)"));
    assert!(debug.contains("EventId(..)"));

    assert_eq!(unread.request_id(), request_id);
    assert!(
        unread.requires_ready_session(),
        "mark unread command requires a ready session"
    );
    let debug = format!("{unread:?}");
    assert!(!debug.contains("!room:example.test"));
    assert!(debug.contains("RoomId(..)"));
    assert!(debug.contains("unread: true"));
}
