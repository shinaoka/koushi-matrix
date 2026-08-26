use super::event_wait::{
    QaEventFuture, QaEventSource, QaSnapshotEventSource, projection_timeline_item,
};
use super::participants::{
    QaE2eeLogoutBarrier, QaOwnedE2eeCleanupOperations,
    ensure_incoming_verification_receiver_sync_not_stopped,
};
use super::registry::{
    QaScenario, QaStage, SEND_QUEUE_EVENT_TIMEOUT, TIMELINE_RECONNECT_EXPECTED_BODY_COUNT,
    should_run_focused_send_queue_route, tokens_for_stage,
};
use super::scenario_timeline::assert_zero_display_projection_reset_fallback_delta;
use super::{
    AccountEvent, AccountKey, AppState, Arc, CoreEvent, CoreFailure, Duration, EventStreamLag,
    Mutex, RequestId, SessionState, SyncEvent, TimelineDiff, TimelineEvent, TimelineItem,
    TimelineItemId, TimelineKey, TimelineMessageActions,
};
use koushi_core::event::ThreadSummaryDto;

pub(super) fn production_part(source: &'static str) -> &'static str {
    source.split("\n#[cfg(test)]").next().unwrap_or(source)
}

pub(super) fn root_source() -> &'static str {
    production_part(include_str!("../headless-core-qa.rs"))
}
pub(super) fn registry_source() -> &'static str {
    production_part(include_str!("registry.rs"))
}
pub(super) fn event_wait_source() -> &'static str {
    production_part(include_str!("event_wait.rs"))
}
pub(super) fn participants_source() -> &'static str {
    production_part(include_str!("participants.rs"))
}
pub(super) fn fixtures_source() -> &'static str {
    production_part(include_str!("fixtures.rs"))
}
pub(super) fn cleanup_source() -> &'static str {
    production_part(include_str!("cleanup.rs"))
}
pub(super) fn diagnostics_source() -> &'static str {
    production_part(include_str!("diagnostics.rs"))
}
pub(super) fn orchestrator_source() -> &'static str {
    production_part(include_str!("orchestrator.rs"))
}
pub(super) fn identity_source() -> &'static str {
    production_part(include_str!("scenarios/identity.rs"))
}
pub(super) fn rooms_source() -> &'static str {
    production_part(include_str!("scenarios/rooms.rs"))
}
pub(super) fn timeline_source() -> &'static str {
    production_part(include_str!("scenarios/timeline.rs"))
}
pub(super) fn search_source() -> &'static str {
    production_part(include_str!("scenarios/search.rs"))
}

pub(super) fn production_source() -> &'static str {
    static SOURCE: std::sync::LazyLock<String> = std::sync::LazyLock::new(|| {
        [
            root_source(),
            registry_source(),
            diagnostics_source(),
            event_wait_source(),
            participants_source(),
            fixtures_source(),
            cleanup_source(),
            orchestrator_source(),
            identity_source(),
            rooms_source(),
            timeline_source(),
            search_source(),
        ]
        .join("\n")
    });
    SOURCE.as_str()
}
pub(super) fn reconnect_test_bodies() -> Vec<String> {
    (0..TIMELINE_RECONNECT_EXPECTED_BODY_COUNT)
        .map(|index| format!("synthetic body {index:02}"))
        .collect()
}

pub(super) fn reconnect_test_items(indices: impl IntoIterator<Item = usize>) -> Vec<TimelineItem> {
    let bodies = reconnect_test_bodies();
    indices
        .into_iter()
        .map(|index| {
            synthetic_timeline_item(
                &format!("$synthetic-{index:02}:example.invalid"),
                Some(&bodies[index]),
                None,
                None,
                None,
            )
        })
        .collect()
}

pub(super) fn reconnect_test_request(sequence: u64) -> RequestId {
    RequestId {
        connection_id: koushi_core::ids::RuntimeConnectionId(1),
        sequence,
    }
}

#[test]
fn active_reconnect_uses_encryption_gate_before_timeline_work() {
    let source = production_source();
    let stage = source
        .split("async fn run_timeline_reconnect_scenario_impl")
        .nth(1)
        .and_then(|rest| rest.split("fn timeline_gap_count_for_qa").next())
        .expect("timeline reconnect stage should exist");
    let active = stage
        .split("let room_id = if restart_with_persisted_gap")
        .nth(1)
        .and_then(|rest| rest.split("// Rebuild both SyncService instances").next())
        .expect("active reconnect room setup should exist")
        .split("} else {")
        .nth(1)
        .expect("active reconnect branch should exist");

    assert!(
        active.contains(
            "create_room_for_qa(\n            &mut conn_a,\n            \"QA Timeline Reconnect Room\",\n            true,"
        ),
        "active reconnect must create an encrypted room"
    );

    let helper = source
        .split("async fn wait_for_encrypted_room_projection_for_qa")
        .nth(1)
        .and_then(|rest| rest.split("async fn wait_for_space_in_space_list").next())
        .expect("encrypted room projection helper should exist");
    assert!(helper.contains("ROOM_LIST_EVENT_TIMEOUT"));
    assert!(helper.contains("room.room_id == expected_room_id && room.is_encrypted"));
    assert!(helper.contains("RoomEvent::RoomListUpdated"));
    assert!(helper.contains("CoreEvent::StateChanged(snapshot)"));
    assert!(helper.contains("tokio::time::timeout_at(deadline, conn.recv_event())"));
    assert!(!helper.contains("tokio::time::sleep"));

    let first_subscribe = stage
        .find("subscribe_and_ack_active_timeline_projection_for_qa(")
        .expect("reconnect must subscribe a timeline");
    let first_send = stage
        .find("TimelineCommand::SendText")
        .expect("reconnect must send a timeline message");
    let gate_a = stage
        .find("wait_for_encrypted_room_projection_for_qa(\n            &mut conn_a")
        .expect("reconnect must gate encryption projection for A");
    let gate_b = stage
        .find("wait_for_encrypted_room_projection_for_qa(\n            &mut conn_b")
        .expect("reconnect must gate encryption projection for B");

    assert!(gate_a < first_subscribe && gate_b < first_subscribe);
    assert!(gate_a < first_send && gate_b < first_send);
}

#[test]
fn invite_timeout_uses_private_safe_observer_diagnostic_summary() {
    let source = production_source();
    let helper = source
        .split("async fn wait_for_invite_in_snapshot")
        .nth(1)
        .expect("invite waiter should exist")
        .split("async fn wait_for_invite_absent")
        .next()
        .expect("invite removal waiter should follow");

    assert!(helper.contains("invite_observer_diagnostic_summary(&koushi_diagnostics::snapshot())"));
    assert!(helper.contains("{observer_diagnostics}"));
    assert!(!helper.contains("expected_room_id:?"));
}

#[test]
fn production_qa_never_overlaps_actor_owned_sync_with_manual_sync_once() {
    let source = production_source();
    let production = source
        .split("#[cfg(test)]\nmod tests")
        .next()
        .expect("production source should precede tests");

    assert!(
        !production.contains("SyncCommand::SyncOnce"),
        "production QA must wait on actor-owned typed events instead of issuing manual SyncOnce"
    );
    assert!(
        !production.contains("sync_once_for_qa("),
        "production QA must not retain manual SyncOnce helpers or callers"
    );
}

#[test]
fn owner_driven_e2ee_body_waiter_keeps_the_extended_deadline() {
    let source = production_source();
    let helper = source
        .split("async fn wait_for_item_with_body_or_decryption_failure")
        .nth(1)
        .expect("owner-driven E2EE body waiter should exist")
        .split("async fn wait_for_bodies_and_pagination_settle")
        .next()
        .expect("pagination waiter should follow the E2EE body waiter");

    assert!(helper.contains("E2EE_EVENT_TIMEOUT"));
    assert!(helper.contains("tokio::time::timeout_at(deadline, conn.recv_event())"));
    assert!(!helper.contains("SyncCommand::SyncOnce"));
}

#[test]
fn unverified_peer_refreshes_device_keys_before_behavioral_checkpoints() {
    let source = identity_source();
    let stage = source
        .split("async fn verify_multi_user_multi_device_room_key_delivery_for_qa")
        .nth(1)
        .expect("multi-device delivery stage should exist")
        .split("enum QaParticipantLoginGate")
        .next()
        .expect("participant gate should follow multi-device delivery");

    let refresh = stage
        .find("refresh_device_keys_and_assert_known_for_qa(")
        .expect("unverified-peer stage must refresh and assert the exact device");
    let send = stage
        .find("TimelineCommand::SendText")
        .expect("unverified-peer stage must retain its behavioral send checkpoint");
    assert!(refresh < send);
    assert!(stage.contains("wait_for_send_flow_completion_with_timeout("));
    assert!(stage.contains("E2EE_EVENT_TIMEOUT"));
    assert!(stage.contains("e2ee multi-device A2 receive"));
    assert!(stage.contains("e2ee multi-device B receive"));
    assert!(stage.contains("blocked QA blacklist ack timeout"));
    assert!(stage.contains("wait_for_withheld_event_projection_from_source("));
    assert!(stage.contains("room_id: room_id.clone()"));
    let promote = stage
        .find("blocked QA promote B3")
        .expect("B3 must be promoted before the withheld probe");
    let blacklist = stage
        .find("let blacklist_id")
        .expect("the withheld probe must blacklist B3");
    let blocked_send = stage
        .find("let blocked_send")
        .expect("the withheld probe must send after blacklisting B3");
    assert!(promote < blacklist);
    assert!(blacklist < blocked_send);
    assert!(!stage.contains("AccountCommand::RequestVerification"));
    assert!(!stage.contains("SyncCommand::SyncOnce"));

    let helper = participants_source()
        .split("async fn refresh_device_keys_and_assert_known_for_qa")
        .nth(1)
        .expect("device-key refresh checkpoint helper should exist")
        .split("pub(super) enum QaParticipantLoginGate")
        .next()
        .expect("participant login gate should follow the checkpoint helper");
    assert!(helper.contains("AccountCommand::QaRefreshDeviceKeysAndAssertKnown"));
    assert!(helper.contains("tokio::time::timeout(E2EE_EVENT_TIMEOUT, ack)"));
    assert!(!helper.contains("AccountCommand::RequestVerification"));
    assert!(!helper.contains("tokio::time::sleep"));
}

#[test]
fn e2ee_key_delivery_preestablishes_invite_before_optional_b_login() {
    let source = identity_source();
    let stage = source
        .split("async fn verify_multi_user_multi_device_room_key_delivery_for_qa")
        .nth(1)
        .expect("multi-device delivery stage should exist")
        .split("async fn refresh_device_keys_and_assert_known_for_qa")
        .next()
        .expect("device-key refresh helper should follow multi-device delivery");

    let create = stage
        .find("let room_id = create_room_for_qa(")
        .expect("E2EE room should be created");
    let invite = stage
        .find("invite_user_for_qa(")
        .expect("B should be invited to the E2EE room");
    let owned_login = stage
        .find("login_synced_participant_for_qa(")
        .expect("focused E2EE should bootstrap and start normal actor-owned sync");
    let observe = stage
        .find("wait_for_invite_in_snapshot(")
        .expect("B should observe the pre-existing invite snapshot");
    let cleanup = stage
        .rfind("cleanup_e2ee_multi_device_participants")
        .expect("owned B should retain ordered cleanup after key-delivery checks");

    assert!(create < invite);
    assert!(invite < owned_login);
    assert!(owned_login < observe);
    assert!(observe < cleanup);
    assert_eq!(stage.matches("login_synced_participant_for_qa(").count(), 1);
    assert_eq!(
        stage
            .matches("cleanup_e2ee_multi_device_participants")
            .count(),
        1
    );
    assert!(!stage.contains("SyncCommand::SyncOnce"));
    assert!(!stage.contains("sync_once_for_qa("));
    assert!(!stage.contains("tokio::time::sleep"));
}

pub(super) fn synthetic_timeline_item(
    event_id: &str,
    body: Option<&str>,
    in_reply_to_event_id: Option<&str>,
    thread_root: Option<&str>,
    thread_summary: Option<ThreadSummaryDto>,
) -> TimelineItem {
    TimelineItem {
        request_state: None,
        id: TimelineItemId::Event {
            event_id: event_id.to_owned(),
        },
        sender: Some("@member:test".to_owned()),
        sender_label: None,
        sender_avatar: None,
        body: body.map(str::to_owned),
        notice_i18n: None,
        message_kind: Default::default(),
        spoiler_spans: Vec::new(),
        timestamp_ms: None,
        in_reply_to_event_id: in_reply_to_event_id.map(str::to_owned),
        formatted: None,
        reply_quote: None,
        thread_root: thread_root.map(str::to_owned),
        thread_summary,
        media: None,
        link_previews: None,
        link_ranges: Vec::new(),
        reactions: Vec::new(),
        can_react: false,
        is_redacted: false,
        is_hidden: false,
        can_redact: false,
        is_edited: false,
        can_edit: false,
        actions: TimelineMessageActions::default(),
        send_state: None,
        unable_to_decrypt: None,
    }
}

#[test]
fn headless_qa_binary_initializes_rust_log_tracing() {
    let production_source = production_source();
    assert!(production_source.contains("init_headless_qa_tracing_from_env();"));
    assert!(production_source.contains("tracing_subscriber::EnvFilter"));
}

#[test]
fn e2ee_strict_qa_keeps_actor_owned_sync_running_for_multi_device_send() {
    let production_source = production_source();

    assert!(!production_source.contains("ENV_E2EE_PAUSE_SYNC_BEFORE_MULTI_DEVICE_SEND"));
    assert!(!production_source.contains("pause sync A before multi-device send"));
    assert!(!production_source.contains("pause sync B2 before multi-device send"));
    assert!(production_source.contains("wait_for_item_with_body_or_decryption_failure("));
}

#[test]
fn e2ee_strict_qa_uses_typed_causal_checks_after_recipient_device_verification() {
    let production_source = production_source();

    assert!(!production_source.contains("settle_e2ee_device_list_propagation_for_qa"));
    assert!(!production_source.contains("DEVICE_LIST_SETTLE_SYNC_TIMEOUT"));
    assert!(production_source.contains("e2ee recipient verification B/B2"));
    assert!(production_source.contains("e2ee multi-device B2 room list"));
    assert!(production_source.contains("e2ee multi-device B2 receive"));
}

#[test]
fn same_user_secondary_device_runtimes_isolate_saved_credentials() {
    let production_source = production_source();

    for label in [
        "gate-negative-a2",
        "gate-negative-a3",
        "gate-negative-a4",
        "gate-negative-a5",
        "gate-negative-a6",
        "a2",
        "encryption-debug-a2",
        "e2ee-b2",
        "e2ee-b3-unverified",
    ] {
        assert!(
            production_source.contains(&format!("start_isolated_qa_runtime(\"{label}\")")),
            "secondary-device runtime {label} must not restore the primary device credential",
        );
    }
}

#[test]
fn e2ee_device_verification_labels_distinguish_recipient_second_device() {
    let production_source = production_source();

    assert!(production_source.contains("e2ee gated self verification A/A2"));
    assert!(production_source.contains("e2ee recipient verification B/B2"));
    assert!(production_source.contains("primary incoming request"));
    assert!(!production_source.contains("request secondary to primary"));
}

#[test]
fn send_queue_display_projection_fallback_gate_requires_zero_counter_delta() {
    assert_eq!(
        assert_zero_display_projection_reset_fallback_delta(41, 41),
        Ok(())
    );
    assert!(assert_zero_display_projection_reset_fallback_delta(41, 42).is_err());

    let source = production_source();
    let stage = source
        .split("async fn run_send_queue_stage")
        .nth(1)
        .expect("send queue stage")
        .split("async fn unsubscribe_timeline_for_qa")
        .next()
        .expect("send queue stage boundary");
    assert!(stage.contains("display_projection_reset_fallback_count()"));
    assert!(stage.contains("assert_zero_display_projection_reset_fallback_delta"));
    assert!(stage.contains("println!(\"display_projection_reset_fallbacks=0\")"));
}

#[test]
fn send_queue_alone_uses_the_focused_early_route() {
    assert!(should_run_focused_send_queue_route(QaScenario::SendQueue));

    for scenario in [
        QaScenario::All,
        QaScenario::LoginSync,
        QaScenario::RoomSpace,
        QaScenario::Timeline,
        QaScenario::E2eeTrust,
    ] {
        assert!(
            !should_run_focused_send_queue_route(scenario),
            "{scenario:?} must retain its existing route"
        );
    }

    let source = production_source();
    let run_async_before_generic_fixture = source
        .split("async fn run_async")
        .nth(1)
        .and_then(|rest| rest.split("// One CoreRuntime per synthetic user").next())
        .expect("run_async before the generic two-user fixture");
    let focused_dispatch = run_async_before_generic_fixture
        .find("if should_run_focused_send_queue_route(scenario)")
        .expect("run_async dispatches SendQueue through its focused route");
    let focused_call = run_async_before_generic_fixture
        .find("run_focused_send_queue_scenario(&config).await?")
        .expect("run_async invokes the focused SendQueue scenario");
    let focused_return = run_async_before_generic_fixture[focused_call..]
        .find("return Ok(scenario_report(&config.server_kind, scenario))")
        .map(|offset| focused_call + offset)
        .expect("focused SendQueue dispatch returns before generic fixture setup");
    assert!(focused_dispatch < focused_call);
    assert!(focused_call < focused_return);

    let route = source
        .split("async fn run_focused_send_queue_scenario")
        .nth(1)
        .and_then(|rest| rest.split("async fn run_send_queue_stage").next())
        .expect("focused SendQueue route");
    let drop_connection = route
        .find("drop(conn)")
        .expect("focused route drops its bootstrap connection");
    let ordered_shutdown = route
        .find("runtime.shutdown()")
        .expect("focused route awaits ordered runtime shutdown");
    let standalone_stage = route
        .find("run_send_queue_stage(config, &recovery_secret).await")
        .expect("focused route invokes the standalone SendQueue stage");

    assert!(route.contains("QaParticipantLoginGate::BootstrapNewIdentity"));
    assert!(route.contains("bootstrap_recovery_secret"));
    assert!(drop_connection < ordered_shutdown);
    assert!(ordered_shutdown < standalone_stage);
    assert!(!route.contains("user_b"));
    assert!(!route.contains("password_b"));
}

#[test]
fn focused_send_queue_bootstrap_logs_out_before_ordered_shutdown() {
    let source = production_source();
    let route = source
        .split("async fn run_focused_send_queue_scenario")
        .nth(1)
        .and_then(|rest| rest.split("async fn run_send_queue_stage").next())
        .expect("focused SendQueue route");

    let sync_stop = route
        .find("SyncCommand::Stop")
        .expect("focused route submits sync stop");
    let sync_stopped = route
        .find("wait_for_sync_stopped")
        .expect("focused route waits for correlated sync stop");
    let logout = route
        .find("AccountCommand::Logout")
        .expect("focused route submits logout");
    let logged_out = route
        .find("wait_for_logged_out")
        .expect("focused route waits for correlated logout");
    let drop_connection = route
        .find("drop(conn)")
        .expect("focused route drops its bootstrap connection");
    let ordered_shutdown = route
        .find("runtime.shutdown()")
        .expect("focused route awaits ordered runtime shutdown");
    let missing_secret = route
        .find("send_queue bootstrap recovery secret unavailable")
        .expect("focused route reports a missing recovery secret");
    let standalone_stage = route
        .find("run_send_queue_stage(config, &recovery_secret).await")
        .expect("focused route invokes the standalone SendQueue stage");

    assert!(!route.contains("account_key: _"));
    assert!(route.contains("&account_key"));
    assert!(sync_stop < sync_stopped);
    assert!(sync_stopped < logout);
    assert!(logout < logged_out);
    assert!(logged_out < drop_connection);
    assert!(drop_connection < ordered_shutdown);
    assert!(ordered_shutdown < missing_secret);
    assert!(missing_secret < standalone_stage);
}

#[test]
fn shared_primary_login_always_completes_the_new_identity_gate() {
    // Primary A is always a freshly registered user, so it always parks in
    // the verification gate and `LoggedIn` stays pending until promotion.
    // An allowlist of scenarios here silently broke every scenario missing
    // from it; the gate completion must be unconditional.
    let source = production_source();
    let shared_login = source
        .split("--- Login A (persistent store selected before authentication) ---")
        .nth(1)
        .expect("shared primary login route")
        .split("wait_for_logged_in(&mut conn_a, login_a_id")
        .next()
        .expect("shared primary login waits for LoggedIn");
    assert!(
        shared_login.contains("complete_new_identity_gate_for_qa(&mut conn_a"),
        "the shared login route must complete the gate before waiting for LoggedIn"
    );
    assert!(
        !shared_login.contains("should_bootstrap_new_identity_before_logged_in"),
        "gate completion must not be gated on the scenario"
    );
}

#[test]
fn new_identity_gate_settles_its_bootstrap_confirmation_before_returning() {
    // #375: the helper used to submit ConfirmSessionBootstrapSaved and
    // return immediately, so a failed confirmation left the session
    // unpromoted and the run reported only `login A: timed out waiting for
    // LoggedIn event` — after this helper had printed its success token.
    let source = production_source();
    let helper = source
        .split("async fn complete_new_identity_gate_for_qa")
        .nth(1)
        .expect("new identity gate helper")
        .split("async fn wait_for_existing_identity_gate")
        .next()
        .expect("helper ends before the existing-identity gate");

    let confirm_submit = helper
        .find("ConfirmSessionBootstrapSaved")
        .expect("helper submits the saved confirmation");
    let observes_failure = helper
        .find("failed == confirm_id")
        .expect("helper observes the confirmation's correlated failure");
    let returns = helper
        .rfind("Ok(Some(recovery_secret))")
        .expect("helper returns the disposable recovery secret");
    assert!(
        confirm_submit < observes_failure && observes_failure < returns,
        "the confirmation outcome must be observed between submit and return"
    );
    assert!(
        helper.contains("timed out settling bootstrap confirmation; phase="),
        "a stuck confirmation must name the session phase, not fall through \
         to a login-wait timeout"
    );
}

#[test]
fn login_wait_timeout_names_the_session_phase() {
    // The message is the only artifact a failed CI run leaves behind, so it
    // has to distinguish "never promoted" from "promotion in flight" (#375).
    let source = production_source();
    let waiter = source
        .split("async fn wait_for_logged_in")
        .nth(1)
        .expect("login waiter")
        .split("async fn ")
        .next()
        .expect("waiter body");
    assert!(
        waiter.contains("timed out waiting for LoggedIn event; phase="),
        "the login-wait timeout must report the session phase"
    );
    assert!(
        waiter.contains("gate_session_phase(&conn.snapshot().session)"),
        "the phase must come from the authoritative snapshot"
    );
}

#[test]
fn scenarios_that_must_not_bootstrap_return_before_the_shared_login() {
    // This is what makes the unconditional gate completion safe: a scenario
    // that owns its own login never reaches the shared route.
    let source = production_source();
    let before_shared_login = source
        .split("async fn run_async")
        .nth(1)
        .expect("run_async body")
        .split("--- Login A (persistent store selected before authentication) ---")
        .next()
        .expect("shared primary login follows the early returns");
    for marker in [
        "QaScenario::GateNoProof",
        "QaScenario::TimelineReconnect",
        "QaScenario::CacheRestore",
        "should_run_focused_send_queue_route(scenario)",
    ] {
        assert!(
            before_shared_login.contains(marker),
            "{marker} must return before the shared primary login"
        );
    }
}

#[test]
fn run_async_centrally_owns_one_normal_secondary_login() {
    let source = production_source();
    let before_room_space = source
        .split("async fn run_async")
        .nth(1)
        .expect("run_async should exist")
        .split("// --- Phase 4: Room operations")
        .next()
        .expect("RoomSpace should follow shared stage setup");

    assert!(before_room_space.contains(
        "let mut normal_secondary = if should_run_normal_secondary_participant(scenario)"
    ));
    assert_eq!(
        before_room_space
            .matches("login_synced_participant_for_qa(")
            .count(),
        1,
        "run_async must own exactly one normal B login"
    );
    assert!(before_room_space.contains("QaParticipantLoginGate::BootstrapNewIdentity"));
    assert_eq!(
        before_room_space
            .matches("cleanup_normal_secondary_participant_for_qa(")
            .count(),
        2,
        "focused InvitesDm and pre-RoomSpace exits each need one ordered cleanup path"
    );
}

#[test]
fn invites_dm_and_directory_borrow_b_without_owning_its_lifecycle() {
    let source = production_source();
    let invites = source
        .split("async fn run_invites_dm_stage")
        .nth(1)
        .expect("InvitesDm stage should exist")
        .split("async fn run_directory_stage")
        .next()
        .expect("directory stage should follow InvitesDm");
    let directory = source
        .split("async fn run_directory_stage")
        .nth(1)
        .expect("directory stage should exist")
        .split("async fn join_directory_room_for_qa")
        .next()
        .expect("directory join helper should follow directory stage");

    for (label, stage) in [("InvitesDm", invites), ("Directory", directory)] {
        assert!(
            stage.contains("conn_b: &mut CoreConnection"),
            "{label} must borrow the centrally owned B connection"
        );
        for forbidden in [
            "CoreRuntime::",
            "AccountCommand::LoginPassword",
            "wait_for_logged_in",
            "login_synced_participant_for_qa(",
            "cleanup_logged_in_runtime",
        ] {
            assert!(
                !stage.contains(forbidden),
                "{label} must not own B lifecycle operation {forbidden}"
            );
        }
    }
}

#[test]
fn room_space_reuses_and_consumes_the_central_secondary_owner() {
    let source = production_source();
    let room_space = source
        .split("// --- Phase 4: Room operations")
        .nth(1)
        .expect("RoomSpace stage should exist")
        .split("// --- Phase 5: Timeline subscribe")
        .next()
        .expect("Timeline should follow RoomSpace");

    assert!(room_space.contains("normal_secondary.take()"));
    assert!(room_space.contains("let QaParticipantLoginOutcome"));
    for forbidden in [
        "CoreRuntime::start_with_data_dir(data_dir_b)",
        "AccountCommand::LoginPassword",
        "wait_for_logged_in",
        "login_synced_participant_for_qa(",
    ] {
        assert!(
            !room_space.contains(forbidden),
            "RoomSpace must reuse B instead of performing {forbidden}"
        );
    }
}

#[test]
fn normal_secondary_cleanup_paths_use_one_ordered_runtime_shutdown() {
    let source = cleanup_source();
    let focused_cleanup = source
        .split("async fn cleanup_logged_in_runtime")
        .nth(1)
        .expect("logged-in runtime cleanup should exist")
        .split("async fn cleanup_normal_secondary_participant_for_qa")
        .next()
        .expect("normal secondary cleanup should follow runtime cleanup");
    assert!(focused_cleanup.contains("runtime.shutdown().await"));
    assert!(!focused_cleanup.contains("drop(runtime)"));
    assert!(!focused_cleanup.contains("tokio::time::sleep"));

    let all_cleanup = orchestrator_source()
        .split("// --- Logout B ---")
        .nth(1)
        .expect("All should own a B cleanup section")
        .split("Ok(scenario_report(&config.server_kind, scenario))")
        .next()
        .expect("All B cleanup should end before run_async returns");
    assert_eq!(all_cleanup.matches("AccountCommand::Logout").count(), 1);
    assert_eq!(all_cleanup.matches("runtime_b.shutdown().await").count(), 1);
    assert!(!all_cleanup.contains("cleanup_normal_secondary_participant_for_qa"));
}

#[test]
fn all_flow_retains_the_primary_recovery_secret_for_its_send_queue_stage() {
    assert!(QaScenario::All.should_run_stage(QaStage::SendQueue));
    // The primary recovery secret is now produced unconditionally by the
    // shared login route, so All keeps it without a scenario allowlist —
    // `shared_primary_login_always_completes_the_new_identity_gate` pins that.
    let source = production_source();
    let all_send_queue_route = source
        .split("if scenario.should_run_stage(QaStage::SendQueue)")
        .nth(1)
        .expect("All route should retain the SendQueue stage")
        .split("if !scenario.should_run_stage(QaStage::EditRedactSearch)")
        .next()
        .expect("EditRedactSearch route should follow SendQueue");
    assert!(all_send_queue_route.contains("bootstrap_recovery_secret_a"));
    assert!(all_send_queue_route.contains("run_send_queue_stage(&config, recovery_secret)"));

    let standalone_send_queue = source
        .split("async fn run_send_queue_stage")
        .nth(1)
        .expect("standalone SendQueue stage")
        .split("async fn unsubscribe_timeline_for_qa")
        .next()
        .expect("standalone SendQueue stage end");
    assert!(
        standalone_send_queue
            .contains("QaParticipantLoginGate::RecoverExistingIdentity(recovery_secret)")
    );
}

#[test]
fn standalone_send_queue_login_requires_primary_recovery_secret() {
    let source = production_source();
    let stage = source
        .split("async fn run_send_queue_stage")
        .nth(1)
        .expect("standalone SendQueue stage")
        .split("async fn unsubscribe_timeline_for_qa")
        .next()
        .expect("standalone SendQueue stage end");

    assert!(stage.contains("login_synced_participant_for_qa("));
    assert!(stage.contains("proxy.homeserver_url()"));
    assert!(stage.contains("recovery_secret: &AuthSecret"));
    assert!(stage.contains("QaParticipantLoginGate::RecoverExistingIdentity(recovery_secret)"));
    assert!(!stage.contains("\n        true,"));
    assert!(!stage.contains("AccountCommand::LoginPassword"));
    assert!(!stage.contains("wait_for_logged_in"));
}

#[test]
fn participant_login_gate_policy_distinguishes_bootstrap_from_recovery() {
    let source = production_source();
    let before_helper = source
        .split("async fn login_synced_participant_for_qa")
        .next()
        .expect("source before centralized participant login helper");
    let helper = source
        .split("async fn login_synced_participant_for_qa")
        .nth(1)
        .expect("centralized participant login helper")
        .split("async fn subscribe_timeline_for_qa")
        .next()
        .expect("centralized participant login helper end");

    assert!(before_helper.contains("enum QaParticipantLoginGate<'a>"));
    assert!(before_helper.contains("BootstrapNewIdentity"));
    assert!(before_helper.contains("RecoverExistingIdentity(&'a AuthSecret)"));
    assert!(helper.contains("gate: QaParticipantLoginGate<'_>"));
    assert!(!helper.contains("bootstrap_new_identity: bool"));
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum RecordedOwnedE2eeCleanupOperation {
    StopSync,
    Logout(QaE2eeLogoutBarrier),
    AuthoritativeLogoutBarrier(QaE2eeLogoutBarrier),
    DropConnection,
    ShutdownRuntime,
}

pub(super) struct RecordingOwnedE2eeCleanupOperations {
    pub(super) participant: &'static str,
    pub(super) operations:
        std::sync::Arc<std::sync::Mutex<Vec<(&'static str, RecordedOwnedE2eeCleanupOperation)>>>,
    pub(super) fail_authoritative_barrier: bool,
}

impl RecordingOwnedE2eeCleanupOperations {
    pub(super) fn record(&self, operation: RecordedOwnedE2eeCleanupOperation) {
        self.operations
            .lock()
            .expect("cleanup observation lock")
            .push((self.participant, operation));
    }
}

impl QaOwnedE2eeCleanupOperations for RecordingOwnedE2eeCleanupOperations {
    async fn stop_sync(&mut self, _label: &str) -> Result<(), String> {
        self.record(RecordedOwnedE2eeCleanupOperation::StopSync);
        Ok(())
    }

    async fn submit_logout(
        &mut self,
        barrier: &QaE2eeLogoutBarrier,
        _label: &str,
    ) -> Result<(), String> {
        self.record(RecordedOwnedE2eeCleanupOperation::Logout(barrier.clone()));
        Ok(())
    }

    async fn wait_for_authoritative_logout(
        &mut self,
        barrier: &QaE2eeLogoutBarrier,
        _label: &str,
    ) -> Result<(), String> {
        self.record(RecordedOwnedE2eeCleanupOperation::AuthoritativeLogoutBarrier(barrier.clone()));
        if self.fail_authoritative_barrier {
            Err("injected authoritative logout barrier failure".to_owned())
        } else {
            Ok(())
        }
    }

    fn drop_connection(&mut self) {
        self.record(RecordedOwnedE2eeCleanupOperation::DropConnection);
    }

    async fn shutdown_runtime(&mut self) {
        self.record(RecordedOwnedE2eeCleanupOperation::ShutdownRuntime);
    }
}

pub(super) fn recording_owned_e2ee_cleanup_operations(
    participant: &'static str,
    fail_authoritative_barrier: bool,
    operations: &std::sync::Arc<
        std::sync::Mutex<Vec<(&'static str, RecordedOwnedE2eeCleanupOperation)>>,
    >,
) -> RecordingOwnedE2eeCleanupOperations {
    RecordingOwnedE2eeCleanupOperations {
        participant,
        operations: operations.clone(),
        fail_authoritative_barrier,
    }
}

pub(super) struct ScriptedQaEventSource {
    pub(super) events: std::collections::VecDeque<CoreEvent>,
}

impl QaEventSource for ScriptedQaEventSource {
    fn recv_event(
        &mut self,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<
                    Output = Result<CoreEvent, koushi_core::runtime::EventStreamLag>,
                > + Send
                + '_,
        >,
    > {
        Box::pin(async move {
            match self.events.pop_front() {
                Some(event) => Ok(event),
                None => std::future::pending().await,
            }
        })
    }
}

pub(super) fn withheld_projection_test_item(event_id: &str, body: &str) -> TimelineItem {
    let mut item = projection_timeline_item(event_id, false);
    item.body = Some(body.to_owned());
    item
}

pub(super) fn withheld_projection_items_updated(key: TimelineKey, item: TimelineItem) -> CoreEvent {
    CoreEvent::Timeline(TimelineEvent::ItemsUpdated {
        key,
        generation: koushi_core::ids::TimelineGeneration(0),
        batch_id: koushi_core::ids::TimelineBatchId(1),
        diffs: vec![TimelineDiff::PushBack { item }],
    })
}

pub(super) struct ScriptedQaSnapshotEventSource {
    pub(super) events: std::collections::VecDeque<(CoreEvent, SessionState)>,
    pub(super) snapshot: AppState,
    pub(super) received: usize,
}

impl QaEventSource for ScriptedQaSnapshotEventSource {
    fn recv_event(
        &mut self,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<
                    Output = Result<CoreEvent, koushi_core::runtime::EventStreamLag>,
                > + Send
                + '_,
        >,
    > {
        Box::pin(async move {
            match self.events.pop_front() {
                Some((event, session)) => {
                    self.snapshot.session = session;
                    self.received += 1;
                    Ok(event)
                }
                None => std::future::pending().await,
            }
        })
    }
}

impl QaSnapshotEventSource for ScriptedQaSnapshotEventSource {
    fn snapshot(&self) -> AppState {
        self.snapshot.clone()
    }
}

pub(super) struct IntervalQaEventSource {
    pub(super) interval: tokio::time::Interval,
}

impl QaEventSource for IntervalQaEventSource {
    fn recv_event(
        &mut self,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<
                    Output = Result<CoreEvent, koushi_core::runtime::EventStreamLag>,
                > + Send
                + '_,
        >,
    > {
        Box::pin(async move {
            self.interval.tick().await;
            Ok(CoreEvent::Sync(SyncEvent::Running))
        })
    }
}

pub(super) struct IntervalQaSnapshotEventSource {
    pub(super) interval: tokio::time::Interval,
    pub(super) snapshot: AppState,
    pub(super) first_event: Option<CoreEvent>,
}

impl QaEventSource for IntervalQaSnapshotEventSource {
    fn recv_event(
        &mut self,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<
                    Output = Result<CoreEvent, koushi_core::runtime::EventStreamLag>,
                > + Send
                + '_,
        >,
    > {
        Box::pin(async move {
            if let Some(event) = self.first_event.take() {
                return Ok(event);
            }
            self.interval.tick().await;
            Ok(CoreEvent::Sync(SyncEvent::Running))
        })
    }
}

impl QaSnapshotEventSource for IntervalQaSnapshotEventSource {
    fn snapshot(&self) -> AppState {
        self.snapshot.clone()
    }
}

pub(super) struct SharedSnapshotPendingEventSource {
    pub(super) snapshot: Arc<Mutex<AppState>>,
}

impl QaEventSource for SharedSnapshotPendingEventSource {
    fn recv_event(&mut self) -> QaEventFuture<'_> {
        Box::pin(std::future::pending())
    }
}

impl QaSnapshotEventSource for SharedSnapshotPendingEventSource {
    fn snapshot(&self) -> AppState {
        self.snapshot
            .lock()
            .expect("shared QA snapshot lock should not be poisoned")
            .clone()
    }
}

pub(super) struct FirstEventSharedSnapshotPendingSource {
    pub(super) first_event: Option<CoreEvent>,
    pub(super) snapshot: Arc<Mutex<AppState>>,
}

impl QaEventSource for FirstEventSharedSnapshotPendingSource {
    fn recv_event(&mut self) -> QaEventFuture<'_> {
        if let Some(event) = self.first_event.take() {
            return Box::pin(async move { Ok(event) });
        }
        Box::pin(std::future::pending())
    }
}

impl QaSnapshotEventSource for FirstEventSharedSnapshotPendingSource {
    fn snapshot(&self) -> AppState {
        self.snapshot
            .lock()
            .expect("shared QA snapshot lock should not be poisoned")
            .clone()
    }
}

pub(super) struct FirstEventThenTerminalLagSource {
    pub(super) first_event: Option<CoreEvent>,
    pub(super) snapshot: AppState,
    pub(super) skipped: u64,
}

impl QaEventSource for FirstEventThenTerminalLagSource {
    fn recv_event(&mut self) -> QaEventFuture<'_> {
        Box::pin(async move {
            if let Some(event) = self.first_event.take() {
                return Ok(event);
            }
            self.snapshot.session = SessionState::SignedOut;
            Err(EventStreamLag {
                skipped: self.skipped,
            })
        })
    }
}

impl QaSnapshotEventSource for FirstEventThenTerminalLagSource {
    fn snapshot(&self) -> AppState {
        self.snapshot.clone()
    }
}

pub(super) fn qa_state_with_session(session: SessionState) -> AppState {
    AppState {
        session,
        ..AppState::default()
    }
}

pub(super) fn qa_logged_out_event(request_id: RequestId, account_key: AccountKey) -> CoreEvent {
    CoreEvent::Account(AccountEvent::LoggedOut {
        request_id,
        account_key,
    })
}

pub(super) fn qa_operation_failed_event(request_id: RequestId) -> CoreEvent {
    CoreEvent::OperationFailed {
        request_id,
        failure: CoreFailure::SessionNotFound,
    }
}

pub(super) fn strict_e2ee_waiter_inventory() -> &'static [(&'static str, &'static str)] {
    &[
        (
            "wait_for_existing_identity_gate",
            "\nasync fn wait_for_recovery_gate",
        ),
        (
            "wait_for_room_in_room_list",
            "\nasync fn wait_for_space_in_space_list",
        ),
        (
            "wait_for_sync_started_and_running",
            "\nasync fn wait_for_sync_started",
        ),
        ("wait_for_ready_snapshot", "\nasync fn wait_for_logged_in"),
        ("wait_for_logged_in", "\nasync fn wait_for_session_restored"),
        (
            "subscribe_and_ack_active_timeline_projection_for_qa",
            "\nfn thread_initial_items_need_paginate_backfill",
        ),
        (
            "wait_for_verification_requested_event_only",
            "\nfn requested_verification_flow_id",
        ),
        (
            "wait_for_verification_accepted",
            "\nfn verification_state_is_at_least_accepted",
        ),
        (
            "wait_for_initial_items_from_source",
            "\n#[derive(Default)]\nstruct InitialItemsWaitDiagnostics",
        ),
        (
            "wait_for_send_flow_completion_with_timeout",
            "\nasync fn send_text_expect_local_echo",
        ),
        (
            "wait_for_item_with_body_or_decryption_failure",
            "\nasync fn wait_for_withheld_event_projection_from_source",
        ),
        (
            "wait_for_withheld_event_projection_from_source",
            "\n/// Wait until all `expected_bodies` are found",
        ),
    ]
}

pub(super) fn strict_e2ee_waiter_body(source: &str, waiter: &str, end_declaration: &str) -> String {
    let source = source.replace("pub(super) ", "");
    source
        .split(&format!("async fn {waiter}"))
        .nth(1)
        .unwrap_or_else(|| panic!("missing strict E2EE waiter {waiter}"))
        .split(end_declaration)
        .next()
        .unwrap_or_else(|| panic!("missing end declaration for strict E2EE waiter {waiter}"))
        .to_owned()
}

pub(super) fn strict_e2ee_waiter_source(waiter: &str) -> &'static str {
    match waiter {
        "wait_for_existing_identity_gate" => participants_source(),
        "wait_for_room_in_room_list" => event_wait_source(),
        "subscribe_and_ack_active_timeline_projection_for_qa" => timeline_source(),
        "wait_for_verification_requested_event_only" | "wait_for_verification_accepted" => {
            participants_source()
        }
        _ => event_wait_source(),
    }
}

pub(super) fn strict_e2ee_rolling_waiters_with_override(
    override_source: Option<(&str, &str)>,
) -> Vec<&'static str> {
    strict_e2ee_waiter_inventory()
        .iter()
        .filter_map(|&(waiter, end_declaration)| {
            let source = override_source
                .filter(|(override_waiter, _)| *override_waiter == waiter)
                .map(|(_, source)| source)
                .unwrap_or_else(|| strict_e2ee_waiter_source(waiter));
            strict_e2ee_waiter_body(source, waiter, end_declaration)
                .contains("tokio::time::timeout(")
                .then_some(waiter)
        })
        .collect()
}

pub(super) fn strict_e2ee_rolling_waiters() -> Vec<&'static str> {
    strict_e2ee_rolling_waiters_with_override(None)
}

#[test]
fn strict_e2ee_guard_extracts_each_complete_waiter_body() {
    for &(waiter, end_declaration) in strict_e2ee_waiter_inventory() {
        let body =
            strict_e2ee_waiter_body(strict_e2ee_waiter_source(waiter), waiter, end_declaration);
        assert!(
            body.contains(".recv(") || body.contains("recv_event()"),
            "{waiter} extraction must reach its event receive loop"
        );
    }
}

#[test]
fn strict_e2ee_guard_detects_a_rolling_timeout_in_every_inventory_body() {
    for &(waiter, _) in strict_e2ee_waiter_inventory() {
        let declaration = format!("async fn {waiter}");
        let injected_declaration =
            format!("{declaration}\n    tokio::time::timeout(EVENT_TIMEOUT, conn.recv_event())");
        let injected =
            strict_e2ee_waiter_source(waiter).replacen(&declaration, &injected_declaration, 1);

        assert_eq!(
            strict_e2ee_rolling_waiters_with_override(Some((waiter, &injected))),
            vec![waiter],
            "the structural guard must detect a rolling timeout in {waiter}"
        );
    }
}

#[test]
fn strict_e2ee_event_waiters_do_not_restart_timeouts_per_event() {
    let rolling_waiters = strict_e2ee_rolling_waiters();
    assert!(
        rolling_waiters.is_empty(),
        "strict E2EE waiters must use one absolute deadline; rolling={rolling_waiters:?}"
    );
}

#[test]
fn active_room_thread_refresh_uses_the_exact_causal_waiter() {
    let source = production_source();
    let refresh = source
        .split("let refresh_room_a_id = conn_a.next_request_id();")
        .nth(1)
        .expect("thread stage should refresh the active room timeline")
        .split("wait_for_room_timeline_thread_summary")
        .next()
        .expect("thread summary wait should follow the room refresh");

    assert!(refresh.contains("wait_for_initial_items("));
}

#[test]
fn e2ee_trust_stage_does_not_overlap_normal_sync_with_manual_sync_once() {
    let source = production_source();
    let stage = source
        .split("async fn run_e2ee_trust_stage(")
        .nth(1)
        .expect("E2EE trust stage should exist")
        .split("async fn cleanup_logged_in_runtime(")
        .next()
        .expect("secondary-device cleanup should follow E2EE trust");

    assert!(
        !stage.contains("sync_once_for_qa("),
        "E2EE trust must use the authoritative bootstrap and typed gate readiness while SyncService owns the client"
    );
    assert!(
        !stage.contains("publish primary cross-signing facts before gated second-device login")
    );
}

#[test]
fn device_cleanup_scenario_has_a_dedicated_remote_first_proof() {
    let source = production_source();
    let route = source
        .split("if scenario == QaScenario::DeviceCleanup")
        .nth(1)
        .expect("device cleanup scenario route should exist")
        .split("if scenario == QaScenario::E2eeTrust")
        .next()
        .expect("E2EE trust route should follow device cleanup");
    let proof = source
        .split("async fn run_provisional_device_cleanup_qa")
        .nth(1)
        .expect("device cleanup proof should exist")
        .split("async fn login_until_device_cleanup_offered")
        .next()
        .expect("device cleanup login helper should follow proof");

    assert!(route.contains("run_provisional_device_cleanup_qa(&config).await?"));
    assert!(
        proof.contains("audit_removed_device_absent_from_server"),
        "the remote-first token requires an independent server device-list audit"
    );
    assert!(tokens_for_stage(QaStage::DeviceCleanup).contains(&"device_cleanup_remote_first=ok"));
    assert!(
        tokens_for_stage(QaStage::DeviceCleanup).contains(&"device_cleanup_relogin_new_device=ok")
    );
}

#[test]
fn encrypted_backup_seed_uses_live_room_discovery_and_exact_causal_waiter() {
    let source = production_source();
    let seed = source
        .split("async fn seed_encrypted_room_key_for_qa(")
        .nth(1)
        .expect("encrypted backup seed helper should exist")
        .split("async fn enable_key_backup_for_qa(")
        .next()
        .expect("key backup enable helper should follow seed helper");

    assert!(
        !seed.contains("sync_once_for_qa("),
        "backup seed room discovery must not overlap the running SyncService"
    );
    assert!(seed.contains("wait_for_room_in_room_list("));
    assert!(seed.contains("wait_for_initial_items("));
    assert!(seed.contains("subscribe encrypted backup seed"));
}

#[test]
fn second_device_encrypted_room_resubscribe_uses_exact_causal_waiter() {
    let source = production_source();
    let delivery = source
        .split("async fn verify_second_device_room_key_delivery_for_qa(")
        .nth(1)
        .expect("second-device encrypted delivery helper should exist")
        .split("async fn verify_multi_user_multi_device_room_key_delivery_for_qa(")
        .next()
        .expect("multi-device delivery helper should follow second-device delivery");

    assert!(delivery.contains("wait_for_initial_items("));
}

#[test]
fn generic_secondary_timeline_subscribe_uses_exact_causal_waiter() {
    let source = production_source();
    let secondary_subscribe = source
        .split("// B subscribes and receives both messages")
        .nth(1)
        .expect("generic B timeline subscribe block")
        .split("// Paginate backward on B")
        .next()
        .expect("generic B timeline subscribe block end");

    assert!(secondary_subscribe.contains("wait_for_initial_items("));
}

#[test]
fn timeline_stress_uses_event_waiters_not_manual_sync_once() {
    let source = production_source();
    let body = source
        .split("async fn run_timeline_stress_stage")
        .nth(1)
        .and_then(|rest| {
            rest.split("async fn run_timeline_stress_room_messages")
                .next()
        })
        .expect("timeline stress stage body");

    assert!(
        !body.contains("sync_once_for_qa"),
        "timeline stress must not mix manual /sync with the running SyncService path"
    );
    assert!(
        body.contains("wait_for_invite_in_snapshot"),
        "timeline stress should wait for invite projection through the live sync path"
    );
}

#[test]
fn login_wait_uses_dedicated_timeout_for_loaded_local_homeservers() {
    let source = production_source();
    let ready_helper = source
        .split("fn ready_account_key")
        .nth(1)
        .and_then(|rest| rest.split("async fn wait_for_logged_in").next())
        .expect("ready account-key helper body");
    let wait_body = source
        .split("async fn wait_for_logged_in")
        .nth(1)
        .and_then(|rest| {
            rest.split("/// Wait for `AccountEvent::SessionRestored`")
                .next()
        })
        .expect("wait_for_logged_in body");

    assert!(
        source.contains("const LOGIN_EVENT_TIMEOUT: Duration = Duration::from_secs(180);"),
        "login waits need their own timeout because local homeservers can finish /login slowly under full QA load"
    );
    assert!(
        wait_body.contains("QaEventDeadline::after(LOGIN_EVENT_TIMEOUT)")
            && wait_body.contains(".recv(conn)"),
        "wait_for_logged_in must use one absolute dedicated login deadline"
    );
    assert!(
        ready_helper.contains("SessionState::Ready(info)")
            && ready_helper.contains("Some(AccountKey(info.user_id))")
            && wait_body.matches("ready_account_key(conn)").count() >= 3,
        "the identity-gate helper may consume LoggedIn, so the waiter must accept the authoritative Ready snapshot"
    );
}

#[test]
fn all_directory_stage_runs_before_room_space_operations() {
    let source = production_source();
    let run_async_body = source
        .split("async fn run_async")
        .nth(1)
        .and_then(|rest| rest.split("async fn cleanup_after_full_flow").next())
        .expect("run_async body");
    let directory_call = "run_directory_stage(&config, &mut conn_a, conn_b).await?";
    let directory_index = run_async_body
        .find(directory_call)
        .expect("directory stage call in run_async");
    let room_space_index = run_async_body
        .find("// --- Phase 4: Room operations")
        .expect("room-space stage marker");

    assert!(
        directory_index < room_space_index,
        "All flow must run directory QA before RoomSpace operations"
    );
    assert!(
        !run_async_body[room_space_index..].contains(directory_call),
        "directory QA must not be re-run after RoomSpace has started B sync"
    );
}

#[test]
fn send_queue_fifo_wait_uses_dedicated_reconnect_timeout() {
    let source = production_source();
    let body = source
        .split("async fn wait_for_send_completions_in_order")
        .nth(1)
        .and_then(|rest| {
            rest.split("async fn wait_for_cancelled_or_removed_send")
                .next()
        })
        .expect("send queue FIFO wait body");

    assert_eq!(
        SEND_QUEUE_EVENT_TIMEOUT,
        Duration::from_secs(300),
        "SendQueue reconnect timeout must be 300 seconds, independently of the generic event timeout"
    );
    assert!(
        body.contains("tokio::time::timeout(SEND_QUEUE_EVENT_TIMEOUT, conn.recv_event())"),
        "FIFO retry waiter must use the send-queue reconnect timeout"
    );
    assert!(
        body.contains("first_completed={first_completed}"),
        "FIFO retry timeout should report whether the first queued send completed"
    );
}

#[test]
fn send_queue_unsubscribes_timeline_before_runtime_shutdown() {
    let source = production_source();
    let body = source
        .split("async fn run_send_queue_stage")
        .nth(1)
        .and_then(|rest| {
            rest.split("async fn run_timeline_reconnect_scenario")
                .next()
        })
        .expect("send queue stage body");
    let restart_slice = body
        .split("stop_sync_for_qa(&mut conn, \"send_queue stop before restart\")")
        .nth(1)
        .and_then(|rest| rest.split("let mut conn = runtime.attach();").next())
        .expect("send queue restart lifecycle slice");

    assert!(
        body.contains("send_queue unsubscribe before restart shutdown"),
        "send_queue should drop its subscribed timeline before restart shutdown"
    );
    assert!(
        body.contains("send_queue unsubscribe before cleanup"),
        "send_queue should drop its restored timeline before final cleanup"
    );
    assert!(
        source.contains(
            "const TIMELINE_UNSUBSCRIBE_SETTLE_TIMEOUT: Duration = Duration::from_secs(2);"
        ),
        "send_queue needs a bounded settle window because Unsubscribe has no completion event"
    );
    assert!(
        body.contains("TIMELINE_UNSUBSCRIBE_SETTLE_TIMEOUT"),
        "send_queue unsubscribe helper should wait for timeline actor shutdown before runtime drop"
    );
    let shutdown = restart_slice
        .find("runtime.shutdown().await")
        .expect("restart must await the ordered runtime shutdown barrier");
    let reopen = restart_slice
        .find("CoreRuntime::start_with_data_dir(data_dir)")
        .expect("restart must reopen the same persisted data directory");
    assert!(
        shutdown < reopen,
        "runtime shutdown must complete before reopen"
    );
    assert!(!restart_slice.contains("drop(runtime)"));
    assert!(!restart_slice.contains("Duration::from_millis(500)"));
}

#[test]
fn same_data_dir_reopen_paths_use_ordered_runtime_shutdown() {
    fn assert_ordered_reopen(
        label: &str,
        restart_slice: &str,
        drop_connection: &str,
        shutdown_runtime: &str,
        reopen_runtime: &str,
    ) {
        let drop_connection = restart_slice
            .find(drop_connection)
            .unwrap_or_else(|| panic!("{label}: connection must be dropped before shutdown"));
        let shutdown = restart_slice
            .find(shutdown_runtime)
            .unwrap_or_else(|| panic!("{label}: ordered runtime shutdown is required"));
        let reopen = restart_slice
            .find(reopen_runtime)
            .unwrap_or_else(|| panic!("{label}: same data directory must be reopened"));

        assert!(drop_connection < shutdown, "{label}: drop connection first");
        assert!(shutdown < reopen, "{label}: shutdown must precede reopen");
        assert!(
            !restart_slice.contains("drop(runtime"),
            "{label}: dropping a runtime is not a shutdown barrier"
        );
        assert!(
            !restart_slice.contains("Duration::from_millis(500)"),
            "{label}: blind store-lock sleeps are forbidden"
        );
    }

    let source = production_source();
    let cleanup = source
        .split("async fn cleanup_after_full_flow")
        .nth(1)
        .and_then(|rest| rest.split("let mut conn_a2 = runtime_a2.attach();").next())
        .expect("full-flow cleanup restart slice");
    assert_ordered_reopen(
        "cleanup_after_full_flow",
        cleanup,
        "drop(conn_a)",
        "runtime_a.shutdown().await",
        "CoreRuntime::start_with_data_dir(data_dir_a)",
    );

    let all_flow = source
        .split("// --- Sync stop A + store-backed restore A + logout A ---")
        .nth(1)
        .and_then(|rest| rest.split("let mut conn_a2 = runtime_a2.attach();").next())
        .expect("All-scenario restore restart slice");
    assert_ordered_reopen(
        "run_async All restore",
        all_flow,
        "drop(conn_a)",
        "runtime_a.shutdown().await",
        "CoreRuntime::start_with_data_dir(data_dir_a)",
    );

    let cache_restore = source
        .split("async fn run_cache_restore_scenario")
        .nth(1)
        .and_then(|rest| rest.split("let mut conn2 = runtime2.attach();").next())
        .expect("cache restore restart slice");
    assert_ordered_reopen(
        "cache restore",
        cache_restore,
        "drop(conn)",
        "runtime.shutdown().await",
        "CoreRuntime::start_with_data_dir(data_dir)",
    );
}

#[test]
fn timeline_stress_backfill_only_advances_current_paginate_request() {
    let source = production_source();
    let body = source
        .split("async fn wait_for_stress_bodies_and_no_blank_rows")
        .nth(1)
        .and_then(|rest| {
            rest.split("async fn submit_stress_backfill_paginate")
                .next()
        })
        .expect("stress body wait helper");
    let pagination_arm = body
        .split("CoreEvent::Timeline(TimelineEvent::PaginationStateChanged")
        .nth(1)
        .and_then(|rest| rest.split("CoreEvent::OperationFailed").next())
        .expect("pagination state arm");

    assert!(
        pagination_arm.contains("request_id: ev_id")
            && pagination_arm.contains("ev_id == &Some(current_paginate_request_id)"),
        "stress backfill must ignore stale pagination state from older requests on the same timeline"
    );
}

#[test]
fn timeline_stress_replay_existing_is_read_only() {
    let source = production_source();
    let run_async_body = source
        .split("async fn run_async")
        .nth(1)
        .and_then(|rest| rest.split("// --- Phase 4: Room operations").next())
        .expect("run_async pre-room-create body");
    assert!(
        run_async_body.contains("run_timeline_stress_replay_stage"),
        "timeline stress replay must branch before the normal room creation flow"
    );

    let replay_body = source
        .split("async fn run_timeline_stress_replay_stage")
        .nth(1)
        .and_then(|rest| rest.split("struct StressRoomCoordinates").next())
        .expect("timeline stress replay body");
    for forbidden in ["CreateRoom", "CreateSpace", "SendText"] {
        assert!(
            !replay_body.contains(forbidden),
            "timeline stress replay must not perform mutating operation {forbidden}"
        );
    }
    assert!(replay_body.contains("Subscribe"));
    assert!(replay_body.contains("submit_stress_backfill_paginate"));
}

#[test]
fn e2ee_trust_stage_prints_joined_room_restore_scope_token() {
    let source = production_source();
    let legacy_token = concat!("e2ee_key_backup_restore_", "success=ok");

    assert!(source.contains("println!(\"joined_room_restore=ok\")"));
    assert!(!source.contains(legacy_token));
}

#[test]
fn e2ee_trust_stage_reports_second_device_decrypt_token() {
    let source = production_source();

    assert!(tokens_for_stage(QaStage::E2eeTrust).contains(&"e2ee_second_device_decrypt=ok"));
    assert!(source.contains("println!(\"e2ee_second_device_decrypt=ok\")"));
}

#[test]
fn e2ee_trust_stage_reports_multi_user_multi_device_decrypt_token() {
    let source = production_source();

    assert!(
        tokens_for_stage(QaStage::E2eeTrust).contains(&"e2ee_multi_user_multi_device_decrypt=ok")
    );
    assert!(source.contains("println!(\"e2ee_multi_user_multi_device_decrypt=ok\")"));
}

#[test]
fn e2ee_trust_stage_makes_identity_reset_explicitly_opt_in() {
    let source = production_source();

    assert!(source.contains("KOUSHI_QA_ALLOW_IDENTITY_RESET"));
    assert!(source.contains("if config.allow_identity_reset"));
    assert!(source.contains("println!(\"e2ee_identity_reset=skipped\")"));
}

#[test]
fn core_qa_stdout_does_not_format_matrix_identifiers() {
    let source = production_source();

    for forbidden in [
        concat!("println!(\"", "room_", "id={"),
        concat!("println!(\"", "space_", "id={"),
        concat!("println!(\"", "event_", "id={"),
        concat!("println!(\"", "sdk_", "txn={"),
        concat!("println!(\"", "transaction_", "id={"),
    ] {
        assert!(
            !source.contains(forbidden),
            "core QA stdout must not format {forbidden}"
        );
    }
}

#[test]
fn provisional_self_verification_keeps_primary_normal_sync_running() {
    let source = production_source();
    let helper = source
        .split("async fn verify_provisional_second_device_for_qa")
        .nth(1)
        .expect("provisional self-verification helper should exist")
        .split("fn verification_closed_summary")
        .next()
        .expect("verification summary helper should follow provisional verification");

    assert!(helper.contains("AccountCommand::StartOwnUserSas"));
    let refresh = helper
        .find("refresh_device_keys_and_assert_known_for_qa(")
        .expect("primary must causally discover the exact provisional device");
    let start = helper
        .find("AccountCommand::StartOwnUserSas")
        .expect("provisional device should start own-user SAS");
    assert!(refresh < start);
    assert!(helper.contains("target_a2.clone()"));
    assert!(helper.contains("primary incoming request"));
    assert!(helper.contains("SasQaOutcome::Timeout"));
    assert!(helper.contains("SasQaOutcome::Mismatch"));
    assert!(helper.contains("AccountCommand::CancelVerification"));
    assert!(helper.contains("AccountCommand::ConfirmSasVerification"));
    assert!(helper.contains("timed out waiting for authoritative Ready"));

    for forbidden in [
        "stop_sync_for_qa(conn_a",
        "start_sync_for_qa(conn_a",
        "sync_once_for_qa(conn_a",
    ] {
        assert!(
            !helper.contains(forbidden),
            "primary normal sync must remain continuously owned during SAS: {forbidden}"
        );
    }

    assert!(!helper.contains("stop_sync_for_qa(conn_a2"));
    assert!(!helper.contains("start_sync_for_qa(conn_a2"));
}

#[test]
fn incoming_verification_waiter_rejects_stopped_receiver_sync_at_entry() {
    let label = "incoming verification receiver";
    assert_eq!(
        ensure_incoming_verification_receiver_sync_not_stopped(
            &koushi_state::SyncState::Stopped,
            label,
        ),
        Err(format!(
            "{label}: receiver sync is stopped; cannot await an incoming verification request"
        ))
    );
    for sync in [
        koushi_state::SyncState::Running,
        koushi_state::SyncState::Starting,
        koushi_state::SyncState::Failed {
            reason: "synthetic failure detail".to_owned(),
        },
        koushi_state::SyncState::Reconnecting {
            reason: "synthetic reconnect detail".to_owned(),
        },
    ] {
        assert_eq!(
            ensure_incoming_verification_receiver_sync_not_stopped(&sync, label),
            Ok(())
        );
    }

    let source = production_source();
    let guard = source
        .split("fn ensure_incoming_verification_receiver_sync_not_stopped")
        .nth(1)
        .expect("incoming verification sync guard should exist")
        .split("async fn wait_for_verification_requested_event_only")
        .next()
        .expect("incoming verification waiter should follow its sync guard");
    assert!(guard.contains("koushi_state::SyncState::Stopped"));
    assert!(
        guard.contains("receiver sync is stopped; cannot await an incoming verification request")
    );
    assert!(!guard.contains("{sync:?}"));

    let waiter = source
        .split("async fn wait_for_verification_requested_event_only")
        .nth(1)
        .expect("incoming verification waiter should exist")
        .split("fn requested_verification_flow_id")
        .next()
        .expect("verification flow classifier should follow incoming waiter");
    let sync_guard = waiter
        .find(
            "ensure_incoming_verification_receiver_sync_not_stopped(&conn.snapshot().sync, label)?",
        )
        .expect("incoming waiter should fail fast on stopped receiver sync");
    let deadline = waiter
        .find("let deadline")
        .expect("incoming waiter should retain its bounded deadline");
    assert!(sync_guard < deadline);
}

#[test]
fn unused_manual_second_device_verification_cascade_is_absent() {
    let source = production_source();
    let production = source
        .split("#[cfg(test)]\nmod tests")
        .next()
        .expect("production source should precede tests");
    for unused in [
        "async fn verify_second_device_for_qa",
        "enum VerificationRequestAttempt",
        "async fn request_device_verification_for_qa",
        "async fn wait_for_verification_requested_or_failed",
        "async fn wait_for_verification_accepted_with_sync_once",
        "async fn drive_until_both_verification_sas",
        "async fn wait_for_verification_done",
        "fn verification_state_done",
    ] {
        assert!(
            !production.contains(unused),
            "obsolete zero-caller verification orchestration must be deleted: {unused}"
        );
    }
}
