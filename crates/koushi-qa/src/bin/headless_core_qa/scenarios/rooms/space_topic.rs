//! #1565 Phase A: shared Space descriptions use the existing room-topic contract.

use super::*;

pub(super) async fn verify(
    conn_a: &mut CoreConnection,
    conn_b: &mut CoreConnection,
    space_id: &str,
    other_room_id: &str,
    auditor: &koushi_sdk::MatrixClientSession,
) -> Result<(), String> {
    let settings_a = load_room_settings_for_qa(conn_a, space_id, "space_topic A initial").await?;
    let settings_b = load_room_settings_for_qa(conn_b, space_id, "space_topic B initial").await?;
    if settings_a.topic.is_some() || settings_b.topic.is_some() {
        return Err("space_topic: a new Space unexpectedly has a description".to_owned());
    }
    if !settings_a.permissions.can_edit_settings || settings_b.permissions.can_edit_settings {
        return Err("space_topic: creator/member edit permissions disagree".to_owned());
    }

    let forbidden_id = conn_b.next_request_id();
    conn_b
        .command(CoreCommand::Room(RoomCommand::UpdateRoomSetting {
            request_id: forbidden_id,
            room_id: space_id.to_owned(),
            change: RoomSettingChange::Topic(Some("QA forbidden description".to_owned())),
        }))
        .await
        .map_err(|_| "space_topic: forbidden update submission failed".to_owned())?;
    wait_for_room_management_forbidden_operation(
        conn_b,
        forbidden_id,
        RoomManagementOperationKind::Settings,
        "space_topic member edit guard",
    )
    .await?;
    let unchanged =
        load_room_settings_for_qa(conn_b, space_id, "space_topic guard readback").await?;
    if unchanged.topic.is_some() {
        return Err("space_topic: forbidden edit changed the description".to_owned());
    }

    for target in [Some("QA shared description\nSecond line"), None] {
        let update_id = conn_a.next_request_id();
        conn_a
            .command(CoreCommand::Room(RoomCommand::UpdateRoomSetting {
                request_id: update_id,
                room_id: space_id.to_owned(),
                change: RoomSettingChange::Topic(target.map(str::to_owned)),
            }))
            .await
            .map_err(|_| "space_topic: update submission failed".to_owned())?;
        let updated =
            wait_for_room_setting_updated(conn_a, update_id, "space_topic update").await?;
        if updated.topic.as_deref() != target {
            return Err(format!(
                "space_topic: update terminal carries the wrong description \
                 (expected_present={} actual_present={} actual_empty={})",
                target.is_some(),
                updated.topic.is_some(),
                updated.topic.as_deref() == Some(""),
            ));
        }
        // B keeps Space Info open; another member's change must arrive from Core
        // sync without a frontend reload or a competing local topic state.
        wait_for_observed_topic(conn_b, space_id, target).await?;
        wait_for_observed_topic(conn_a, space_id, target).await?;

        // Replace A's selected settings, then reopen the Space through Core's
        // ordinary read command to prove the value survives pane navigation.
        load_room_settings_for_qa(conn_a, other_room_id, "space_topic close Space settings")
            .await?;
        let reopened =
            load_room_settings_for_qa(conn_a, space_id, "space_topic reopen Space settings")
                .await?;
        if reopened.topic.as_deref() != target {
            return Err("space_topic: reopened settings lost the description".to_owned());
        }
    }
    println!("space_topic_basic=ok");
    verify_topic_only_member(conn_a, conn_b, space_id, auditor).await?;
    // Audit only after every write, live observation, reopen, and permission
    // check; an extra HTTP read must not add a barrier between topic writes.
    verify_server_topic(auditor, space_id, Some("QA topic-only member update")).await?;
    println!("space_topic=ok");
    Ok(())
}

async fn verify_server_topic(
    auditor: &koushi_sdk::MatrixClientSession,
    space_id: &str,
    expected: Option<&str>,
) -> Result<(), String> {
    use matrix_sdk::ruma::{api::client::state::get_state_event_for_key, events::StateEventType};
    let room_id = space_id
        .try_into()
        .map_err(|_| "space_topic: invalid fixture Space ID".to_owned())?;
    let response = auditor
        .client()
        .send(get_state_event_for_key::v3::Request::new(
            room_id,
            StateEventType::RoomTopic,
            String::new(),
        ))
        .await
        .map_err(|_| "space_topic: authoritative topic read failed".to_owned())?;
    let content: serde_json::Value = serde_json::from_str(response.event_or_content.get())
        .map_err(|_| "space_topic: authoritative topic content was invalid".to_owned())?;
    let actual = content
        .get("topic")
        .and_then(serde_json::Value::as_str)
        .filter(|value| !value.is_empty());
    if actual != expected {
        return Err(format!(
            "space_topic: authoritative server topic disagrees (expected_present={} actual_present={})",
            expected.is_some(),
            actual.is_some()
        ));
    }
    Ok(())
}

async fn wait_for_observed_topic(
    conn: &mut CoreConnection,
    space_id: &str,
    expected: Option<&str>,
) -> Result<(), String> {
    // #1661: the synced Space projection carries the same confirmed value, so
    // surfaces outside Space info follow another member's edit too.
    let projected_matches = |snapshot: &AppState| {
        snapshot
            .spaces
            .iter()
            .find(|space| space.space_id == space_id)
            .is_some_and(|space| space.topic.as_deref() == expected)
    };
    let deadline = tokio::time::Instant::now() + ROOM_LIST_EVENT_TIMEOUT;
    loop {
        let snapshot = conn.snapshot();
        let observed = projected_matches(&snapshot)
            && snapshot
                .room_management
                .settings
                .as_ref()
                .is_some_and(|settings| {
                    settings.room_id == space_id && settings.topic.as_deref() == expected
                });
        if observed {
            return Ok(());
        }
        match tokio::time::timeout_at(deadline, conn.recv_event()).await {
            Ok(Ok(_)) => continue,
            Ok(Err(lag)) => {
                return Err(format!(
                    "space_topic: event stream lagged (skipped={})",
                    lag.skipped
                ));
            }
            Err(_) => {
                let snapshot = conn.snapshot();
                let settings = snapshot.room_management.settings.as_ref();
                // Failure-only probe: an ordinary Core reload reads the SDK's
                // local cache. Preserve the failed live-observation result.
                let reloaded = load_room_settings_for_qa(
                    conn,
                    space_id,
                    "space_topic failed observation cache probe",
                )
                .await;
                let reload_matches_expected = reloaded
                    .as_ref()
                    .is_ok_and(|value| value.topic.as_deref() == expected);
                let diagnostics = koushi_diagnostics::snapshot();
                let settings_records: Vec<_> = diagnostics
                    .records
                    .iter()
                    .filter(|record| {
                        record.event.source == "core.subscription"
                            && record.event.fields.iter().any(|field| {
                                matches!(field.key, "source" | "trigger")
                                    && field.value
                                        == koushi_diagnostics::DiagnosticValue::Token(
                                            "settings_room",
                                        )
                            })
                    })
                    .collect();
                let settings_intents = settings_records
                    .iter()
                    .filter(|record| record.event.stage == "intent")
                    .count();
                let settings_reconciles = settings_records
                    .iter()
                    .filter(|record| record.event.stage == "reconcile")
                    .count();
                let settings_accepted = settings_records.iter().filter(|record| record.event.fields.iter().any(|field| {
                    field.key == "accepted_bucket" && matches!(field.value, koushi_diagnostics::DiagnosticValue::Count(value) if value > 0)
                })).count();
                let settings_noops = settings_records
                    .iter()
                    .filter(|record| {
                        record.event.fields.iter().any(|field| {
                            field.key == "exact_set_noop"
                                && field.value == koushi_diagnostics::DiagnosticValue::Boolean(true)
                        })
                    })
                    .count();
                return Err(format!(
                    "space_topic: open settings did not observe the shared description \
                     (expected_present={} actual_present={} actual_empty={} \
                     space_projection_match={} \
                     selected_match={} loaded_match={} reload_matches_expected={} \
                     account_epoch={} settings_intents={} settings_reconciles={} settings_accepted={} settings_noops={})",
                    expected.is_some(),
                    settings.is_some_and(|value| value.topic.is_some()),
                    settings.is_some_and(|value| value.topic.as_deref() == Some("")),
                    projected_matches(&snapshot),
                    snapshot.room_management.selected_room_id.as_deref() == Some(space_id),
                    settings.is_some_and(|value| value.room_id == space_id),
                    reload_matches_expected,
                    snapshot.sliding_sync_account_epoch,
                    settings_intents,
                    settings_reconciles,
                    settings_accepted,
                    settings_noops,
                ));
            }
        }
    }
}

/// Configure a real Matrix event-specific permission with the disposable SDK
/// auditor; the behavior under test still uses only production Core commands.
async fn verify_topic_only_member(
    conn_a: &mut CoreConnection,
    conn_b: &mut CoreConnection,
    space_id: &str,
    auditor: &koushi_sdk::MatrixClientSession,
) -> Result<(), String> {
    use matrix_sdk::ruma::{
        OwnedRoomId,
        api::client::state::{get_state_event_for_key, send_state_event},
        events::StateEventType,
        serde::Raw,
    };
    let room_id: OwnedRoomId = space_id
        .try_into()
        .map_err(|_| "space_topic: invalid fixture Space ID".to_owned())?;
    let request = get_state_event_for_key::v3::Request::new(
        room_id.clone(),
        StateEventType::RoomPowerLevels,
        String::new(),
    );
    let response = auditor
        .client()
        .send(request)
        .await
        .map_err(|_| "space_topic: power-level fixture read failed".to_owned())?;
    let mut content: serde_json::Value = serde_json::from_str(response.event_or_content.get())
        .map_err(|_| "space_topic: power-level fixture content was invalid".to_owned())?;
    let events = content
        .as_object_mut()
        .ok_or_else(|| "space_topic: power-level fixture is not an object".to_owned())?
        .entry("events")
        .or_insert_with(|| serde_json::json!({}))
        .as_object_mut()
        .ok_or_else(|| "space_topic: power-level events fixture is not an object".to_owned())?;
    events.insert("m.room.topic".to_owned(), serde_json::json!(0));
    // Keep all other state-event thresholds and membership roles intact.
    let request = send_state_event::v3::Request::new_raw(
        room_id,
        StateEventType::RoomPowerLevels,
        String::new(),
        Raw::from_json(
            serde_json::value::to_raw_value(&content)
                .map_err(|_| "space_topic: power-level fixture encoding failed".to_owned())?,
        ),
    );
    auditor
        .client()
        .send(request)
        .await
        .map_err(|_| "space_topic: power-level fixture update failed".to_owned())?;

    // A subsequent topic event is an observation barrier for B's sync. It also
    // proves the member can read the authoritative shared value before editing.
    let barrier_id = conn_a.next_request_id();
    conn_a
        .command(CoreCommand::Room(RoomCommand::UpdateRoomSetting {
            request_id: barrier_id,
            room_id: space_id.to_owned(),
            change: RoomSettingChange::Topic(Some("QA topic permission barrier".to_owned())),
        }))
        .await
        .map_err(|_| "space_topic: permission barrier submission failed".to_owned())?;
    wait_for_room_setting_updated(conn_a, barrier_id, "space_topic permission barrier").await?;
    wait_for_observed_topic(conn_b, space_id, Some("QA topic permission barrier")).await?;
    if conn_b
        .snapshot()
        .room_management
        .settings
        .as_ref()
        .is_none_or(|settings| settings.permissions.can_edit_settings)
    {
        return Err(
            "space_topic: topic-only member unexpectedly can edit every setting".to_owned(),
        );
    }

    let update_id = conn_b.next_request_id();
    conn_b
        .command(CoreCommand::Room(RoomCommand::UpdateRoomSetting {
            request_id: update_id,
            room_id: space_id.to_owned(),
            change: RoomSettingChange::Topic(Some("QA topic-only member update".to_owned())),
        }))
        .await
        .map_err(|_| "space_topic: topic-only update submission failed".to_owned())?;
    let updated =
        wait_for_room_setting_updated(conn_b, update_id, "space_topic topic-only member update")
            .await?;
    if updated.topic.as_deref() != Some("QA topic-only member update") {
        return Err("space_topic: topic-only member update was not preserved".to_owned());
    }
    wait_for_observed_topic(conn_a, space_id, Some("QA topic-only member update")).await?;

    let forbidden_id = conn_b.next_request_id();
    conn_b
        .command(CoreCommand::Room(RoomCommand::UpdateRoomSetting {
            request_id: forbidden_id,
            room_id: space_id.to_owned(),
            change: RoomSettingChange::Name(Some("QA forbidden rename".to_owned())),
        }))
        .await
        .map_err(|_| "space_topic: topic-only rename submission failed".to_owned())?;
    wait_for_room_management_forbidden_operation(
        conn_b,
        forbidden_id,
        RoomManagementOperationKind::Settings,
        "space_topic topic-only member rename guard",
    )
    .await?;
    Ok(())
}
