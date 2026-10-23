use super::*;
use matrix_sdk::{ruma::push::Ruleset, test_utils::mocks::MatrixMockServer};
use serde_json::json;
use wiremock::{
    Mock, ResponseTemplate,
    matchers::{body_json, method, path, path_regex},
};

async fn session_for(server: &MatrixMockServer) -> MatrixClientSession {
    let client = server.client_builder().build().await;
    MatrixClientSession::from_client_for_testing(
        client.clone(),
        koushi_state::SessionInfo {
            homeserver: server.uri(),
            user_id: client.user_id().unwrap().to_string(),
            device_id: client.device_id().unwrap().to_string(),
            authentication_method: koushi_state::SessionAuthenticationMethod::Password,
        },
    )
}

/// One `content` rule that another client wrote and disabled, one enabled
/// keyword, one server-default `content` rule, and one rule of each other kind.
fn fixture_ruleset() -> Ruleset {
    serde_json::from_value(json!({
        "override": [
            {"rule_id": ".m.rule.master", "default": true, "enabled": false,
             "conditions": [], "actions": []},
            {"rule_id": "custom.override", "default": false, "enabled": true,
             "conditions": [], "actions": ["notify"]}
        ],
        "content": [
            {"rule_id": ".m.rule.contains_user_name", "default": true, "enabled": true,
             "pattern": "alice", "actions": ["notify", {"set_tweak": "highlight"}]},
            {"rule_id": "client.keyword.alpha", "default": false, "enabled": true,
             "pattern": "alpha", "actions": ["notify", {"set_tweak": "highlight"}]},
            {"rule_id": "client.keyword.beta", "default": false, "enabled": false,
             "pattern": "beta", "actions": ["notify"]}
        ],
        "room": [
            {"rule_id": "!room:example.invalid", "default": false, "enabled": true,
             "actions": ["notify"]}
        ],
        "sender": [
            {"rule_id": "@bob:example.invalid", "default": false, "enabled": true,
             "actions": ["notify"]}
        ],
        "underride": [
            {"rule_id": ".m.rule.message", "default": true, "enabled": true,
             "conditions": [{"kind": "event_match", "key": "type", "pattern": "m.room.message"}],
             "actions": ["notify"]}
        ]
    }))
    .unwrap()
}

/// Every pushrules write endpoint, each expected zero times.
async fn forbid_pushrule_writes(server: &MatrixMockServer) {
    for verb in ["PUT", "DELETE", "POST"] {
        Mock::given(method(verb))
            .and(path_regex(r"^/_matrix/client/v3/pushrules/.*"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
            .expect(0)
            .named(format!("{verb} pushrules"))
            .mount(server.server())
            .await;
    }
}

/// Every pushrules endpoint, each expected zero times.
async fn forbid_pushrule_requests(server: &MatrixMockServer) {
    forbid_pushrule_writes(server).await;
    Mock::given(method("GET"))
        .and(path_regex(r"^/_matrix/client/v3/pushrules/.*"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
        .expect(0)
        .named("GET pushrules")
        .mount(server.server())
        .await;
}

async fn mount_ruleset(server: &MatrixMockServer, rules: serde_json::Value) {
    Mock::given(method("GET"))
        .and(path("/_matrix/client/v3/pushrules/"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"global": rules})))
        .expect(1)
        .named("GET pushrules")
        .mount(server.server())
        .await;
}

// ── Projection ──────────────────────────────────────────────────────────────

#[test]
fn content_rules_project_and_the_other_kinds_are_ignored() {
    let rules = keyword_rules_from_ruleset(&fixture_ruleset());
    assert_eq!(
        rules,
        vec![
            KeywordRule {
                rule_id: "client.keyword.alpha".to_owned(),
                pattern: "alpha".to_owned(),
                enabled: true,
            },
            // A rule another client disabled is still one of the account's
            // content rules, and is still a duplicate of the same pattern.
            KeywordRule {
                rule_id: "client.keyword.beta".to_owned(),
                pattern: "beta".to_owned(),
                enabled: false,
            },
        ],
        "override, room, sender, underride and server-default content rules are not keywords"
    );
}

#[test]
fn the_keyword_list_is_capped_in_server_order() {
    let content: Vec<serde_json::Value> = (0..KEYWORD_RULES_MAX + 3)
        .map(|index| {
            json!({
                "rule_id": format!("keyword.{index}"),
                "default": false,
                "enabled": true,
                "pattern": format!("pattern{index}"),
                "actions": ["notify"]
            })
        })
        .collect();
    let ruleset: Ruleset = serde_json::from_value(json!({"content": content})).unwrap();
    let rules = keyword_rules_from_ruleset(&ruleset);
    assert_eq!(rules.len(), KEYWORD_RULES_MAX);
    assert_eq!(rules[0].pattern, "pattern0");
    assert_eq!(
        rules[KEYWORD_RULES_MAX - 1].pattern,
        format!("pattern{}", KEYWORD_RULES_MAX - 1),
        "the first rules in evaluation order survive the cap"
    );
}

#[test]
fn the_rule_id_is_deterministic_and_outside_the_server_default_namespace() {
    let id = keyword_rule_id("cake");
    assert_eq!(id, keyword_rule_id("cake"));
    assert_ne!(id, keyword_rule_id("cakes"));
    assert!(id.starts_with(KEYWORD_RULE_ID_PREFIX));
    assert!(!id.starts_with('.'));
    assert!(id.chars().count() <= KEYWORD_RULE_ID_MAX_CHARS);
}

// ── Local refusal without IO ────────────────────────────────────────────────

#[test]
fn planning_refuses_an_empty_oversized_or_duplicate_pattern_locally() {
    let ruleset = fixture_ruleset();
    assert_eq!(
        plan_keyword_rule(&ruleset, "   "),
        Err(MatrixKeywordRuleError::EmptyPattern)
    );
    assert_eq!(
        plan_keyword_rule(&ruleset, ""),
        Err(MatrixKeywordRuleError::EmptyPattern)
    );
    let oversized = "a".repeat(KEYWORD_PATTERN_MAX_CHARS + 1);
    assert_eq!(
        plan_keyword_rule(&ruleset, &oversized),
        Err(MatrixKeywordRuleError::PatternTooLong)
    );
    // Matrix matching is case-insensitive, so a different case is the same
    // keyword and is refused instead of being written a second time.
    assert_eq!(
        plan_keyword_rule(&ruleset, "ALPHA"),
        Err(MatrixKeywordRuleError::Duplicate)
    );
    assert_eq!(
        plan_keyword_rule(&ruleset, "beta"),
        Err(MatrixKeywordRuleError::Duplicate)
    );
}

#[test]
fn planning_refuses_non_ascii_case_variants_of_a_keyword() {
    // The homeserver compares lowercased text, so a non-ASCII case variant is
    // the same keyword and must not be written a second time (#1597).
    let ruleset: Ruleset = serde_json::from_value(json!({"content": [
        {"rule_id": "client.keyword.accent", "default": false, "enabled": true,
         "pattern": "café", "actions": ["notify"]},
        {"rule_id": "client.keyword.greek", "default": false, "enabled": false,
         "pattern": "σίγμα", "actions": ["notify"]}
    ]}))
    .unwrap();
    assert_eq!(
        plan_keyword_rule(&ruleset, "CAFÉ"),
        Err(MatrixKeywordRuleError::Duplicate)
    );
    assert_eq!(
        plan_keyword_rule(&ruleset, "ΣΊΓΜΑ"),
        Err(MatrixKeywordRuleError::Duplicate)
    );
    // The comparison is case-insensitive, not compatibility folding.
    let fullwidth: Ruleset = serde_json::from_value(json!({"content": [
        {"rule_id": "client.keyword.fullwidth", "default": false, "enabled": true,
         "pattern": "ｃａｋｅ", "actions": ["notify"]}
    ]}))
    .unwrap();
    assert!(plan_keyword_rule(&fullwidth, "cake").is_ok());
}

#[test]
fn planning_trims_and_returns_the_rule_to_write() {
    let ruleset: Ruleset = serde_json::from_value(json!({"content": []})).unwrap();
    let rule = plan_keyword_rule(&ruleset, "  cake  ").unwrap();
    assert_eq!(rule.pattern, "cake");
    assert_eq!(rule.rule_id, keyword_rule_id("cake"));
    assert!(rule.enabled);
}

#[test]
fn a_rule_id_collision_with_a_different_pattern_fails_closed() {
    // The deterministic suffix makes a retry idempotent; if it ever collided
    // with another keyword, the plan must refuse rather than overwrite it.
    let ruleset: Ruleset = serde_json::from_value(json!({"content": [
        {"rule_id": keyword_rule_id("cake"), "default": false, "enabled": true,
         "pattern": "other", "actions": ["notify"]}
    ]}))
    .unwrap();
    assert_eq!(
        plan_keyword_rule(&ruleset, "cake"),
        Err(MatrixKeywordRuleError::RuleIdConflict)
    );
}

#[tokio::test]
async fn an_empty_or_oversized_pattern_is_refused_without_a_request() {
    let server = MatrixMockServer::new().await;
    let session = session_for(&server).await;
    forbid_pushrule_requests(&server).await;

    assert_eq!(
        add_keyword_rule(&session, "   ").await,
        Err(MatrixKeywordRuleError::EmptyPattern)
    );
    let oversized = "a".repeat(KEYWORD_PATTERN_MAX_CHARS + 1);
    assert_eq!(
        add_keyword_rule(&session, &oversized).await,
        Err(MatrixKeywordRuleError::PatternTooLong)
    );
    server.server().verify().await;
}

#[tokio::test]
async fn removing_a_reserved_or_empty_rule_id_is_refused_without_a_request() {
    let server = MatrixMockServer::new().await;
    let session = session_for(&server).await;
    forbid_pushrule_requests(&server).await;

    let oversized = "k".repeat(KEYWORD_RULE_ID_MAX_CHARS + 1);
    for rule_id in ["", ".m.rule.contains_user_name", oversized.as_str()] {
        assert_eq!(
            remove_keyword_rule(&session, rule_id).await,
            Err(MatrixKeywordRuleError::InvalidRuleId),
            "refused rule id {rule_id:?}"
        );
    }
    server.server().verify().await;
}

// ── Server IO against a mock homeserver ─────────────────────────────────────

#[tokio::test]
async fn reading_keywords_maps_the_server_content_rules() {
    let server = MatrixMockServer::new().await;
    let session = session_for(&server).await;
    forbid_pushrule_writes(&server).await;
    Mock::given(method("GET"))
        .and(path("/_matrix/client/v3/pushrules/"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"global": serde_json::to_value(fixture_ruleset()).unwrap()})),
        )
        .expect(1)
        .named("GET pushrules")
        .mount(server.server())
        .await;

    let rules = fetch_keyword_rules(&session).await.unwrap();
    assert_eq!(rules, keyword_rules_from_ruleset(&fixture_ruleset()));
    server.server().verify().await;
}

#[tokio::test]
async fn adding_a_keyword_writes_a_highlighting_content_rule() {
    let server = MatrixMockServer::new().await;
    let session = session_for(&server).await;
    mount_ruleset(&server, json!({"content": []})).await;
    let expected_id = keyword_rule_id("cake");
    Mock::given(method("PUT"))
        .and(path(format!(
            "/_matrix/client/v3/pushrules/global/content/{expected_id}"
        )))
        .and(body_json(json!({
            "pattern": "cake",
            "actions": ["notify", {"set_tweak": "highlight"}]
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
        .expect(1)
        .named("PUT content keyword")
        .mount(server.server())
        .await;

    let rule = add_keyword_rule(&session, "  cake  ").await.unwrap();
    assert_eq!(
        rule,
        KeywordRule {
            rule_id: expected_id,
            pattern: "cake".to_owned(),
            enabled: true,
        }
    );
    server.server().verify().await;
}

#[tokio::test]
async fn a_duplicate_keyword_is_refused_without_a_rule_write() {
    let server = MatrixMockServer::new().await;
    let session = session_for(&server).await;
    // Only the authoritative read is allowed; no PUT/DELETE/POST.
    forbid_pushrule_writes(&server).await;
    mount_ruleset(
        &server,
        json!({"content": [
            {"rule_id": "client.keyword.cake", "default": false, "enabled": true,
             "pattern": "Cake", "actions": ["notify", {"set_tweak": "highlight"}]}
        ]}),
    )
    .await;

    assert_eq!(
        add_keyword_rule(&session, "cake").await,
        Err(MatrixKeywordRuleError::Duplicate)
    );
    server.server().verify().await;
}

#[tokio::test]
async fn removing_a_keyword_deletes_the_content_rule_by_kind_and_id() {
    let server = MatrixMockServer::new().await;
    let session = session_for(&server).await;
    Mock::given(method("DELETE"))
        .and(path(
            "/_matrix/client/v3/pushrules/global/content/client.keyword.cake",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
        .expect(1)
        .named("DELETE content keyword")
        .mount(server.server())
        .await;
    // Deleting by kind and id needs no prior read.
    Mock::given(method("GET"))
        .and(path_regex(r"^/_matrix/client/v3/pushrules/.*"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
        .expect(0)
        .named("GET pushrules")
        .mount(server.server())
        .await;

    remove_keyword_rule(&session, "client.keyword.cake")
        .await
        .unwrap();
    server.server().verify().await;
}

#[tokio::test]
async fn a_server_error_becomes_the_typed_failure() {
    // A failed read is reported, never projected as an empty keyword list.
    let server = MatrixMockServer::new().await;
    let session = session_for(&server).await;
    Mock::given(method("GET"))
        .and(path("/_matrix/client/v3/pushrules/"))
        .respond_with(ResponseTemplate::new(404).set_body_json(json!({
            "errcode": "M_UNRECOGNIZED",
            "error": "push rules are unavailable"
        })))
        .mount(server.server())
        .await;
    assert_eq!(
        fetch_keyword_rules(&session).await,
        Err(MatrixKeywordRuleError::Failed(
            AccountNotificationsFailureKind::Unsupported
        ))
    );

    // A rejected write is reported, never a silent success.
    let server = MatrixMockServer::new().await;
    let session = session_for(&server).await;
    mount_ruleset(&server, json!({"content": []})).await;
    Mock::given(method("PUT"))
        .and(path_regex(
            r"^/_matrix/client/v3/pushrules/global/content/.*",
        ))
        .respond_with(ResponseTemplate::new(403).set_body_json(json!({
            "errcode": "M_FORBIDDEN",
            "error": "push rules are read only"
        })))
        .mount(server.server())
        .await;
    assert_eq!(
        add_keyword_rule(&session, "cake").await,
        Err(MatrixKeywordRuleError::Failed(
            AccountNotificationsFailureKind::Forbidden
        ))
    );
}
