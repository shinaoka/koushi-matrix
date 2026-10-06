use std::{path::PathBuf, sync::Arc};

use koushi_core::{
    AccountCommand, CoreCommand, CoreCommandPolicy, CoreRuntime, NativeArtifactKind,
    NativeArtifactRegistry, RoomKeyExportRequest,
};
use koushi_protocol::command::{HistoryExportLabels, HistoryExportRequest};
use koushi_state::{AuthSecret, HistoryExportRange, HistoryExportScope};

#[tokio::test]
async fn rejected_command_releases_its_exact_native_artifact_registration() {
    let data_dir = tempfile::tempdir().expect("data dir");
    let registry = Arc::new(NativeArtifactRegistry::new());
    let runtime = CoreRuntime::start_with_data_dir_and_native_artifact_port(
        data_dir.path().to_path_buf(),
        registry.clone(),
    );
    let connection = runtime.attach();
    let request_id = connection.next_request_id();
    connection
        .register_native_artifact(
            request_id,
            NativeArtifactKind::RoomKeyExportDestination,
            PathBuf::from("synthetic-export-path"),
        )
        .expect("register path");

    connection
        .command_with_admission(CoreCommand::Account(AccountCommand::ExportRoomKeys {
            request_id,
            request: RoomKeyExportRequest {
                passphrase: AuthSecret::new("synthetic-passphrase"),
            },
        }))
        .await
        .expect("local command admission");

    assert!(registry.is_empty());
    drop(connection);
    runtime.shutdown().await;
}

#[tokio::test]
async fn rejected_recovery_key_save_releases_its_destination() {
    // #1049: the identity bootstrap reveals its key on screen; only the
    // optional save (here rejected without a session) owns a destination.
    let data_dir = tempfile::tempdir().expect("data dir");
    let registry = Arc::new(NativeArtifactRegistry::new());
    let runtime = CoreRuntime::start_with_data_dir_and_native_artifact_port(
        data_dir.path().to_path_buf(),
        registry.clone(),
    );
    let connection = runtime.attach();
    let request_id = connection.next_request_id();
    connection
        .register_native_artifact(
            request_id,
            NativeArtifactKind::RecoveryKeyDestination,
            PathBuf::from("synthetic-recovery-key-path"),
        )
        .expect("register path");

    connection
        .command_with_admission(CoreCommand::Account(
            AccountCommand::SaveSecureBackupRecoveryKey {
                request_id,
                reveal_request_id: 1,
            },
        ))
        .await
        .expect("local command admission");

    assert!(registry.is_empty());
    drop(connection);
    runtime.shutdown().await;
}

const ROOM: &str = "!history:example.invalid";

fn export_command(request_id: koushi_protocol::ids::RequestId) -> CoreCommand {
    CoreCommand::Account(AccountCommand::ExportHistory {
        request_id,
        request: HistoryExportRequest {
            scope: HistoryExportScope::Room {
                room_id: ROOM.to_owned(),
            },
            range: HistoryExportRange::Period {
                start_ms: 1_700_000_000_000,
                end_exclusive_ms: 1_700_086_400_000,
                time_zone: "Asia/Tokyo".to_owned(),
            },
            display_time_zone: "Asia/Tokyo".to_owned(),
            export_date_utc_offset_minutes: 540,
            folder_name_stem: "Private Folder Stem".to_owned(),
            labels: HistoryExportLabels::default(),
        },
    })
}

#[test]
fn export_commands_are_correlated_ready_gated_and_redacted() {
    let request_id = koushi_protocol::ids::RequestId {
        connection_id: koushi_protocol::ids::RuntimeConnectionId(3),
        sequence: 17,
    };
    let target_request_id = koushi_protocol::ids::RequestId {
        connection_id: koushi_protocol::ids::RuntimeConnectionId(3),
        sequence: 16,
    };
    let stop = CoreCommand::Account(AccountCommand::StopHistoryExport {
        request_id,
        target_request_id,
    });
    let retry = CoreCommand::Account(AccountCommand::RetryHistoryExport {
        request_id,
        target_request_id,
    });
    for command in [export_command(request_id), stop, retry] {
        assert_eq!(command.request_id(), request_id);
        assert!(command.requires_ready_session());
        let debug = format!("{command:?}");
        assert!(!debug.contains(ROOM), "{debug}");
        assert!(!debug.contains("Asia/Tokyo"), "{debug}");
        assert!(!debug.contains("path"), "{debug}");
        assert!(!debug.contains("Private Folder Stem"), "{debug}");
    }
}

#[tokio::test]
async fn a_rejected_export_releases_its_destination_registration() {
    let data_dir = tempfile::tempdir().expect("data dir");
    let registry = Arc::new(NativeArtifactRegistry::new());
    let runtime = CoreRuntime::start_with_data_dir_and_native_artifact_port(
        data_dir.path().to_path_buf(),
        registry.clone(),
    );
    let connection = runtime.attach();
    let request_id = connection.next_request_id();
    connection
        .register_native_artifact(
            request_id,
            NativeArtifactKind::HistoryExportDirectory,
            PathBuf::from("synthetic-history-export-path"),
        )
        .expect("register path");

    connection
        .command_with_admission(export_command(request_id))
        .await
        .expect("local command admission");

    assert!(registry.is_empty());
    assert_eq!(
        connection.snapshot().history_export,
        koushi_state::HistoryExportState::Idle
    );
    drop(connection);
    runtime.shutdown().await;
}
