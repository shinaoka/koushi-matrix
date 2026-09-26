use super::*;
use koushi_state::{NotificationCategory, NotificationCategoryState as S};
use matrix_sdk::{
    ruma::{
        owned_room_id, owned_user_id,
        push::{Action, PushConditionRoomCtx, RuleKind, Ruleset},
        serde::Raw,
        user_id,
    },
    test_utils::mocks::MatrixMockServer,
};
use serde_json::json;
use wiremock::{
    Mock, ResponseTemplate,
    matchers::{body_partial_json, method, path, path_regex},
};

const ALL_CATEGORIES: [NotificationCategory; 4] = [
    NotificationCategory::DirectMessages,
    NotificationCategory::GroupMessages,
    NotificationCategory::MentionsAndReplies,
    NotificationCategory::Invites,
];

fn defaults() -> Ruleset {
    Ruleset::server_default(user_id!("@alice:example.invalid"))
}

fn silence(ruleset: &mut Ruleset, kind: RuleKind, rule_id: &str) {
    ruleset.set_actions(kind, rule_id, Vec::new()).unwrap();
}

fn ruleset_json(ruleset: &Ruleset) -> serde_json::Value {
    serde_json::to_value(ruleset).unwrap()
}

// ── Category projection ─────────────────────────────────────────────────────

#[test]
fn server_defaults_project_every_category_on() {
    let ruleset = defaults();
    let states = summarize_categories(&ruleset);
    assert_eq!(states.direct_messages, S::On);
    assert_eq!(states.group_messages, S::On);
    assert_eq!(states.mentions_and_replies, S::On);
    assert_eq!(states.invites, S::On);
    assert!(account_push_enabled(&ruleset));
}

#[test]
fn encrypted_and_unencrypted_disagreement_projects_mixed() {
    let mut ruleset = defaults();
    silence(&mut ruleset, RuleKind::Underride, ".m.rule.encrypted");
    silence(
        &mut ruleset,
        RuleKind::Underride,
        ".m.rule.encrypted_room_one_to_one",
    );
    silence(&mut ruleset, RuleKind::Underride, ".m.rule.room_one_to_one");
    let states = summarize_categories(&ruleset);
    assert_eq!(states.group_messages, S::Mixed);
    assert_eq!(states.direct_messages, S::Off);
}

#[test]
fn disabled_but_notifying_rule_reads_off() {
    let mut ruleset = defaults();
    ruleset
        .set_enabled(RuleKind::Override, ".m.rule.invite_for_me", false)
        .unwrap();
    assert_eq!(summarize_categories(&ruleset).invites, S::Off);
}

#[test]
fn only_room_mentions_disabled_projects_mixed_mentions() {
    let mut ruleset = defaults();
    ruleset
        .set_enabled(RuleKind::Override, ".m.rule.is_room_mention", false)
        .unwrap();
    assert_eq!(
        summarize_categories(&ruleset).mentions_and_replies,
        S::Mixed
    );
}

#[test]
fn legacy_mention_rules_are_used_when_msc3952_rules_are_absent() {
    let ruleset: Ruleset = serde_json::from_value(json!({
        "override": [
            {"rule_id": ".m.rule.contains_display_name", "default": true, "enabled": true,
             "conditions": [{"kind": "contains_display_name"}],
             "actions": ["notify", {"set_tweak": "highlight"}]},
            {"rule_id": ".m.rule.roomnotif", "default": true, "enabled": false,
             "conditions": [{"kind": "event_match", "key": "content.body", "pattern": "@room"}],
             "actions": ["notify", {"set_tweak": "highlight"}]}
        ]
    }))
    .unwrap();
    assert_eq!(
        summarize_categories(&ruleset).mentions_and_replies,
        S::Mixed
    );
}

#[test]
fn master_rule_marks_account_push_disabled() {
    let mut ruleset = defaults();
    ruleset
        .set_enabled(RuleKind::Override, ".m.rule.master", true)
        .unwrap();
    assert!(!account_push_enabled(&ruleset));
    assert_eq!(
        plan_account_push_writes(&ruleset, true),
        vec![RuleWrite::SetEnabled {
            kind: RuleKind::Override,
            rule_id: ".m.rule.master".to_owned(),
            enabled: false,
        }]
    );
    assert!(plan_account_push_writes(&defaults(), true).is_empty());
}

// ── Write planning ──────────────────────────────────────────────────────────

#[test]
fn reapplying_the_current_value_plans_no_writes() {
    let mut mixed = defaults();
    silence(&mut mixed, RuleKind::Underride, ".m.rule.encrypted");
    for ruleset in [defaults(), mixed] {
        let states = summarize_categories(&ruleset);
        for category in ALL_CATEGORIES {
            match states.get(category) {
                S::On => assert!(plan_category_writes(&ruleset, category, true).is_empty()),
                S::Off => assert!(plan_category_writes(&ruleset, category, false).is_empty()),
                S::Mixed => {}
            }
        }
    }
}

#[test]
fn group_off_silences_only_group_rules() {
    let writes = plan_category_writes(&defaults(), NotificationCategory::GroupMessages, false);
    let ids: Vec<&str> = writes
        .iter()
        .map(|write| match write {
            RuleWrite::SetActions {
                rule_id, actions, ..
            } => {
                assert_eq!(*actions, RuleActions::Silent);
                rule_id.as_str()
            }
            other => panic!("unexpected {other:?}"),
        })
        .collect();
    assert_eq!(
        ids,
        vec![
            ".m.rule.message",
            ".m.rule.encrypted",
            ".org.matrix.msc3930.rule.poll_start"
        ]
    );
}

#[test]
fn turning_a_mixed_category_on_touches_only_the_off_rule() {
    let mut ruleset = defaults();
    silence(&mut ruleset, RuleKind::Underride, ".m.rule.encrypted");
    assert_eq!(
        plan_category_writes(&ruleset, NotificationCategory::GroupMessages, true),
        vec![RuleWrite::SetActions {
            kind: RuleKind::Underride,
            rule_id: ".m.rule.encrypted".to_owned(),
            actions: RuleActions::Notify,
        }]
    );
}

#[test]
fn re_enabling_a_disabled_rule_keeps_its_custom_sound() {
    let mut ruleset: Ruleset = serde_json::from_value(json!({
        "underride": [
            {"rule_id": ".m.rule.room_one_to_one", "default": true, "enabled": false,
             "conditions": [{"kind": "room_member_count", "is": "2"}],
             "actions": ["notify", {"set_tweak": "sound", "value": "custom-ring"}]},
            {"rule_id": ".m.rule.encrypted_room_one_to_one", "default": true, "enabled": true,
             "conditions": [{"kind": "room_member_count", "is": "2"}],
             "actions": ["notify", {"set_tweak": "sound", "value": "custom-ring"}]}
        ]
    }))
    .unwrap();
    let writes = plan_category_writes(&ruleset, NotificationCategory::DirectMessages, true);
    assert_eq!(
        writes,
        vec![RuleWrite::SetEnabled {
            kind: RuleKind::Underride,
            rule_id: ".m.rule.room_one_to_one".to_owned(),
            enabled: true,
        }],
        "only the enabled flag flips; the custom sound tweak is not rewritten"
    );
    // Simulate the write and confirm the sound survived.
    ruleset
        .set_enabled(RuleKind::Underride, ".m.rule.room_one_to_one", true)
        .unwrap();
    let json = ruleset_json(&ruleset).to_string();
    assert_eq!(json.matches("custom-ring").count(), 2);
}

#[test]
fn mentions_off_disables_modern_and_present_legacy_rules() {
    let mut value = ruleset_json(&defaults());
    value["content"] = json!([
        {"rule_id": ".m.rule.contains_user_name", "default": true, "enabled": true,
         "pattern": "alice", "actions": ["notify", {"set_tweak": "highlight"}]}
    ]);
    let ruleset: Ruleset = serde_json::from_value(value).unwrap();
    let writes = plan_category_writes(&ruleset, NotificationCategory::MentionsAndReplies, false);
    let disabled: Vec<&str> = writes
        .iter()
        .map(|write| match write {
            RuleWrite::SetEnabled {
                rule_id,
                enabled: false,
                ..
            } => rule_id.as_str(),
            other => panic!("unexpected {other:?}"),
        })
        .collect();
    assert_eq!(
        disabled,
        vec![
            ".m.rule.is_user_mention",
            ".m.rule.contains_user_name",
            ".m.rule.is_room_mention"
        ]
    );
}

#[test]
fn silent_override_rule_turned_on_restores_spec_actions() {
    let mut ruleset = defaults();
    silence(&mut ruleset, RuleKind::Override, ".m.rule.invite_for_me");
    assert_eq!(
        plan_category_writes(&ruleset, NotificationCategory::Invites, true),
        vec![RuleWrite::SetActions {
            kind: RuleKind::Override,
            rule_id: ".m.rule.invite_for_me".to_owned(),
            actions: RuleActions::NotifyWithSound,
        }]
    );
}

// ── Overlap semantics (standard push-rule precedence) ───────────────────────

async fn evaluate(ruleset: &Ruleset, member_count: u32, mention_alice: bool) -> bool {
    let mut content = json!({"msgtype": "m.text", "body": "synthetic body"});
    if mention_alice {
        content["m.mentions"] = json!({"user_ids": ["@alice:example.invalid"]});
    }
    let event = Raw::new(&json!({
        "type": "m.room.message",
        "event_id": "$event:example.invalid",
        "room_id": "!room:example.invalid",
        "sender": "@bob:example.invalid",
        "origin_server_ts": 1,
        "content": content,
    }))
    .unwrap()
    .cast_unchecked::<matrix_sdk::ruma::events::AnySyncTimelineEvent>();
    let context = PushConditionRoomCtx::new(
        owned_room_id!("!room:example.invalid"),
        member_count.into(),
        owned_user_id!("@alice:example.invalid"),
        "Alice".to_owned(),
    );
    ruleset
        .get_actions(&event, &context)
        .await
        .iter()
        .any(Action::should_notify)
}

fn toggle(ruleset: &mut Ruleset, category: NotificationCategory, enabled: bool) {
    let writes = plan_category_writes(ruleset, category, enabled);
    for write in writes {
        match write {
            RuleWrite::SetEnabled {
                kind,
                rule_id,
                enabled,
            } => ruleset.set_enabled(kind, rule_id, enabled).unwrap(),
            RuleWrite::SetActions {
                kind,
                rule_id,
                actions,
            } => ruleset
                .set_actions(kind, rule_id, actions.to_ruma())
                .unwrap(),
        }
    }
}

#[tokio::test]
async fn group_off_with_mentions_on_still_notifies_mentions() {
    let mut ruleset = defaults();
    toggle(&mut ruleset, NotificationCategory::GroupMessages, false);
    assert!(!evaluate(&ruleset, 5, false).await, "plain group message");
    assert!(evaluate(&ruleset, 5, true).await, "mention in a group");
    // DMs keep notifying: the group switch does not touch one-to-one rules.
    assert!(evaluate(&ruleset, 2, false).await, "plain DM");
}

#[tokio::test]
async fn dm_off_with_mentions_on_still_notifies_mentions_in_dms() {
    let mut ruleset = defaults();
    toggle(&mut ruleset, NotificationCategory::DirectMessages, false);
    assert!(!evaluate(&ruleset, 2, false).await);
    assert!(evaluate(&ruleset, 2, true).await);
    assert!(evaluate(&ruleset, 5, false).await, "group unaffected");
}

#[tokio::test]
async fn mentions_off_with_group_on_still_notifies_as_a_message() {
    let mut ruleset = defaults();
    toggle(
        &mut ruleset,
        NotificationCategory::MentionsAndReplies,
        false,
    );
    assert!(evaluate(&ruleset, 5, true).await);
    toggle(&mut ruleset, NotificationCategory::GroupMessages, false);
    assert!(!evaluate(&ruleset, 5, true).await);
}

// ── Snapshot ────────────────────────────────────────────────────────────────

#[test]
fn snapshot_matches_pushers_to_validated_emails_case_insensitively() {
    let snapshot = build_account_notifications_snapshot(
        &defaults(),
        koushi_state::NotificationEmailManagement::Available,
        &[
            "one@example.invalid".to_owned(),
            "two@example.invalid".to_owned(),
        ],
        &[
            "TWO@example.invalid".to_owned(),
            "stale@example.invalid".to_owned(),
        ],
    );
    assert!(!snapshot.emails[0].notifications_active);
    assert!(snapshot.emails[1].notifications_active);
    assert_eq!(snapshot.unverified_email_pusher_count, 1);
    assert!(snapshot.email_notifications_active());
}

// ── Server IO against a mock homeserver ─────────────────────────────────────

async fn mock_session(server: &MatrixMockServer) -> crate::MatrixClientSession {
    let client = server.client_builder().build().await;
    crate::MatrixClientSession {
        info: koushi_state::SessionInfo {
            homeserver: server.uri(),
            user_id: client.user_id().unwrap().to_string(),
            device_id: client.device_id().unwrap().to_string(),
            authentication_method: koushi_state::SessionAuthenticationMethod::Password,
        },
        client,
        diagnostic_counters: koushi_diagnostics::DiagnosticCounterContext::registered(),
    }
}

/// Every write endpoint this feature could touch, each expected zero times.
async fn forbid_writes(server: &MatrixMockServer) {
    for verb in ["PUT", "DELETE", "POST"] {
        Mock::given(method(verb))
            .and(path_regex(r"^/_matrix/client/v3/pushrules/.*"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
            .expect(0)
            .named(format!("{verb} pushrules"))
            .mount(server.server())
            .await;
    }
    Mock::given(method("POST"))
        .and(path("/_matrix/client/v3/pushers/set"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
        .expect(0)
        .named("POST pushers/set")
        .mount(server.server())
        .await;
    Mock::given(method("POST"))
        .and(path_regex(r"^/_matrix/client/v3/account/3pid.*"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
        .expect(0)
        .named("POST 3pid")
        .mount(server.server())
        .await;
}

async fn mount_reads(
    server: &MatrixMockServer,
    rules: serde_json::Value,
    threepids: serde_json::Value,
    pushers: serde_json::Value,
) {
    Mock::given(method("GET"))
        .and(path("/_matrix/client/v3/pushrules/"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"global": rules})))
        .mount(server.server())
        .await;
    Mock::given(method("GET"))
        .and(path("/_matrix/client/v3/account/3pid"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"threepids": threepids})))
        .mount(server.server())
        .await;
    Mock::given(method("GET"))
        .and(path("/_matrix/client/v3/pushers"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"pushers": pushers})))
        .mount(server.server())
        .await;
    Mock::given(method("GET"))
        .and(path("/_matrix/client/v3/capabilities"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"capabilities": {}})))
        .mount(server.server())
        .await;
}

fn email_pusher(address: &str) -> serde_json::Value {
    json!({"kind": "email", "app_id": "m.email", "pushkey": address,
           "app_display_name": "Email Notifications", "device_display_name": address,
           "lang": "en", "data": {}})
}

#[tokio::test]
async fn opening_the_screen_reads_only_and_preserves_other_clients_rules() {
    let server = MatrixMockServer::new().await;
    let session = mock_session(&server).await;
    // A ruleset another client customised: mixed group rules, a custom sound,
    // a keyword rule, a disabled @room rule, and a room-specific rule.
    let mut ruleset = defaults();
    silence(&mut ruleset, RuleKind::Underride, ".m.rule.encrypted");
    ruleset
        .set_actions(
            RuleKind::Underride,
            ".m.rule.room_one_to_one",
            serde_json::from_value(json!(["notify", {"set_tweak": "sound", "value": "custom"}]))
                .unwrap(),
        )
        .unwrap();
    ruleset
        .set_enabled(RuleKind::Override, ".m.rule.is_room_mention", false)
        .unwrap();
    forbid_writes(&server).await;
    mount_reads(
        &server,
        ruleset_json(&ruleset),
        json!([{"medium": "email", "address": "one@example.invalid",
                "validated_at": 1, "added_at": 1}]),
        json!([email_pusher("one@example.invalid")]),
    )
    .await;

    let snapshot = load_account_notifications(&session).await.unwrap();
    assert_eq!(snapshot.categories.group_messages, S::Mixed);
    assert_eq!(snapshot.categories.mentions_and_replies, S::Mixed);
    assert_eq!(snapshot.categories.direct_messages, S::On);
    assert!(snapshot.emails[0].notifications_active);
    // Loading twice (re-open) is still read-only.
    load_account_notifications(&session).await.unwrap();
    server.server().verify().await;
}

#[tokio::test]
async fn category_toggle_sends_only_the_planned_rule_writes() {
    let server = MatrixMockServer::new().await;
    let session = mock_session(&server).await;
    let mut ruleset = defaults();
    silence(&mut ruleset, RuleKind::Underride, ".m.rule.encrypted");
    mount_reads(&server, ruleset_json(&ruleset), json!([]), json!([])).await;
    Mock::given(method("PUT"))
        .and(path(
            "/_matrix/client/v3/pushrules/global/underride/.m.rule.message/actions",
        ))
        .and(body_partial_json(json!({"actions": []})))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
        .expect(1)
        .mount(server.server())
        .await;
    Mock::given(method("PUT"))
        .and(path(
            "/_matrix/client/v3/pushrules/global/underride/.org.matrix.msc3930.rule.poll_start/actions",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
        .expect(1)
        .mount(server.server())
        .await;
    // `.m.rule.encrypted` is already silent and one-to-one rules are another
    // category: neither may be written.
    Mock::given(method("PUT"))
        .and(path_regex(
            r"^/_matrix/client/v3/pushrules/global/underride/\.m\.rule\.(encrypted|room_one_to_one|encrypted_room_one_to_one)/.*",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
        .expect(0)
        .mount(server.server())
        .await;

    set_notification_category(&session, NotificationCategory::GroupMessages, false)
        .await
        .unwrap();
    server.server().verify().await;
}

#[tokio::test]
async fn changing_the_email_target_adds_the_new_pusher_then_removes_the_old() {
    let server = MatrixMockServer::new().await;
    let session = mock_session(&server).await;
    mount_reads(
        &server,
        ruleset_json(&defaults()),
        json!([
            {"medium": "email", "address": "old@example.invalid", "validated_at": 1, "added_at": 1},
            {"medium": "email", "address": "new@example.invalid", "validated_at": 2, "added_at": 2}
        ]),
        json!([email_pusher("old@example.invalid")]),
    )
    .await;
    Mock::given(method("POST"))
        .and(path("/_matrix/client/v3/pushers/set"))
        .and(body_partial_json(json!({
            "kind": "email", "app_id": "m.email", "pushkey": "new@example.invalid",
            "append": true, "data": {"brand": "Koushi"}
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
        .expect(1)
        .mount(server.server())
        .await;
    Mock::given(method("POST"))
        .and(path("/_matrix/client/v3/pushers/set"))
        .and(body_partial_json(json!({
            "kind": null, "app_id": "m.email", "pushkey": "old@example.invalid"
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
        .expect(1)
        .mount(server.server())
        .await;

    set_email_notification_target(&session, "New@Example.Invalid", "en")
        .await
        .unwrap();
    server.server().verify().await;
}

#[tokio::test]
async fn email_target_must_be_a_validated_3pid() {
    let server = MatrixMockServer::new().await;
    let session = mock_session(&server).await;
    forbid_writes(&server).await;
    mount_reads(&server, ruleset_json(&defaults()), json!([]), json!([])).await;
    assert_eq!(
        set_email_notification_target(&session, "pending@example.invalid", "en").await,
        Err(AccountNotificationsFailureKind::EmailNotRegistered)
    );
    server.server().verify().await;
}

#[tokio::test]
async fn unsupported_email_verification_is_classified() {
    let server = MatrixMockServer::new().await;
    let session = mock_session(&server).await;
    Mock::given(method("POST"))
        .and(path("/_matrix/client/v3/account/3pid/email/requestToken"))
        .respond_with(ResponseTemplate::new(400).set_body_json(json!({
            "errcode": "M_THREEPID_MEDIUM_NOT_SUPPORTED",
            "error": "Adding an email to your account is disabled on this server"
        })))
        .mount(server.server())
        .await;
    let secret = matrix_sdk::ruma::ClientSecret::new();
    assert_eq!(
        request_notification_email_token(&session, &secret, "a@example.invalid", 1).await,
        Err(AccountNotificationsFailureKind::Unsupported)
    );
}

#[tokio::test]
async fn confirm_before_link_and_uiaa_are_distinguished() {
    let server = MatrixMockServer::new().await;
    let session = mock_session(&server).await;
    let secret = matrix_sdk::ruma::ClientSecret::new();
    // First call: interactive auth required.
    Mock::given(method("POST"))
        .and(path("/_matrix/client/v3/account/3pid/add"))
        .respond_with(ResponseTemplate::new(401).set_body_json(json!({
            "session": "uiaa-session",
            "flows": [{"stages": ["m.login.password"]}],
            "params": {}
        })))
        .up_to_n_times(1)
        .mount(server.server())
        .await;
    let first = add_notification_email(&session, &secret, "sid123", None, None).await;
    assert!(matches!(
        first,
        Err(AddNotificationEmailError::UiaaChallenge { session: Some(ref s) }) if s == "uiaa-session"
    ));
    // Second call with auth: link not opened yet.
    Mock::given(method("POST"))
        .and(path("/_matrix/client/v3/account/3pid/add"))
        .respond_with(ResponseTemplate::new(400).set_body_json(json!({
            "errcode": "M_THREEPID_AUTH_FAILED",
            "error": "No validated 3pid session found"
        })))
        .mount(server.server())
        .await;
    let auth = koushi_state::IdentityResetAuthRequest::UiaaPassword {
        password: koushi_state::AuthSecret::new("synthetic-password".to_owned()),
    };
    let second = add_notification_email(
        &session,
        &secret,
        "sid123",
        Some(&auth),
        Some("uiaa-session"),
    )
    .await;
    assert!(matches!(
        second,
        Err(AddNotificationEmailError::Failed(
            AccountNotificationsFailureKind::EmailNotVerified
        ))
    ));
}
