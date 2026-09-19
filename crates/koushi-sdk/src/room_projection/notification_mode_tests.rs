use super::room_list_snapshot_from_sdk_rooms;
use koushi_state::RoomNotificationMode;
use matrix_sdk::{
    ruma::{push::Ruleset, room_id},
    test_utils::mocks::MatrixMockServer,
};
use matrix_sdk_test::{JoinedRoomBuilder, event_factory::EventFactory};

#[tokio::test]
async fn server_push_rules_project_mute_and_unmute_without_losing_unread_source() {
    let server = MatrixMockServer::new().await;
    let client = server.client_builder().build().await;
    let room_id = room_id!("!muted:example.invalid");
    server
        .mock_sync()
        .ok_and_run(&client, |b| {
            b.add_joined_room(JoinedRoomBuilder::new(room_id));
        })
        .await;
    let snapshot = room_list_snapshot_from_sdk_rooms(client.joined_rooms()).await;
    assert!(
        snapshot.room_notification_modes.is_empty(),
        "missing account data is not an unmute"
    );
    for rule_id in [
        room_id.to_string(),
        format!("org.matrix.desktop.notify.room.{room_id}"),
    ] {
        let rules: Ruleset = serde_json::from_value(serde_json::json!({
            "override": [{"rule_id":rule_id,"default":false,"enabled":true,
                "conditions":[{"kind":"event_match","key":"room_id","pattern":room_id}],
                "actions":[]}]
        }))
        .unwrap();
        server
            .mock_sync()
            .ok_and_run(&client, |b| {
                b.add_global_account_data(EventFactory::new().push_rules(rules.clone()));
            })
            .await;
        let snapshot = room_list_snapshot_from_sdk_rooms(client.joined_rooms()).await;
        assert_eq!(
            snapshot.room_notification_modes.get(room_id.as_str()),
            Some(&RoomNotificationMode::Mute)
        );
        assert_eq!(
            snapshot.rooms.len(),
            1,
            "mute must not remove the room/history"
        );
    }
    server
        .mock_sync()
        .ok_and_run(&client, |b| {
            b.add_global_account_data(EventFactory::new().push_rules(Ruleset::new()));
        })
        .await;
    let snapshot = room_list_snapshot_from_sdk_rooms(client.joined_rooms()).await;
    assert_eq!(
        snapshot.room_notification_modes.get(room_id.as_str()),
        Some(&RoomNotificationMode::All)
    );
}

#[tokio::test]
async fn authoritative_notification_mode_reads_server_instead_of_cached_rules() {
    use wiremock::{
        Mock, ResponseTemplate,
        matchers::{method, path},
    };
    let server = MatrixMockServer::new().await;
    let client = server.client_builder().build().await;
    let room_id = room_id!("!confirmed:example.invalid");
    server
        .mock_sync()
        .ok_and_run(&client, |b| {
            b.add_joined_room(JoinedRoomBuilder::new(room_id));
            b.add_global_account_data(EventFactory::new().push_rules(Ruleset::new()));
        })
        .await;
    let session = crate::MatrixClientSession {
        info: koushi_state::SessionInfo {
            homeserver: server.uri(),
            user_id: client.user_id().unwrap().to_string(),
            device_id: client.device_id().unwrap().to_string(),
            authentication_method: koushi_state::SessionAuthenticationMethod::Unknown,
        },
        client,
        diagnostic_counters: koushi_diagnostics::DiagnosticCounterContext::registered(),
    };
    Mock::given(method("GET"))
        .and(path("/_matrix/client/v3/pushrules/"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "global":{"override":[{"rule_id":room_id,"default":false,"enabled":true,
            "conditions":[{"kind":"event_match","key":"room_id","pattern":room_id}],"actions":[]}]}
        })))
        .expect(1)
        .mount(server.server())
        .await;
    assert_eq!(
        crate::fetch_room_notification_mode(&session, room_id.as_str())
            .await
            .unwrap(),
        RoomNotificationMode::Mute
    );
    assert_eq!(
        room_list_snapshot_from_sdk_rooms(session.client().joined_rooms())
            .await
            .room_notification_modes[room_id.as_str()],
        RoomNotificationMode::All
    );
}
