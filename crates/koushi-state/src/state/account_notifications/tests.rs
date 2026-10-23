//! Tests for the account notification settings state (#981, #1427).

use super::*;

#[test]
fn normalize_accepts_plain_addresses_and_lowercases() {
    assert_eq!(
        normalize_notification_email("  Alice@Example.Invalid "),
        Some("alice@example.invalid".to_owned())
    );
}

#[test]
fn normalize_rejects_malformed_addresses() {
    for input in [
        "",
        "alice",
        "@example.invalid",
        "alice@",
        "alice@localhost",
        "a b@example.invalid",
        "alice@@example.invalid",
        "alice@.example",
        "alice@example.",
    ] {
        assert_eq!(normalize_notification_email(input), None, "{input:?}");
    }
    let too_long = format!("{}@example.invalid", "a".repeat(260));
    assert_eq!(normalize_notification_email(&too_long), None);
}

#[test]
fn debug_redacts_addresses() {
    let snapshot = AccountNotificationsSnapshot {
        account_push_enabled: true,
        encrypted_event_push: false,
        categories: NotificationCategoryStates {
            direct_messages: NotificationCategoryState::On,
            group_messages: NotificationCategoryState::Off,
            mentions_and_replies: NotificationCategoryState::Mixed,
            invites: NotificationCategoryState::On,
        },
        email_management: NotificationEmailManagement::Available,
        emails: vec![NotificationEmailAddress {
            address: "secret@example.invalid".to_owned(),
            notifications_active: true,
        }],
        unverified_email_pusher_count: 0,
        keywords: vec![NotificationKeywordRuleState {
            rule_id: "client.keyword.secret".to_owned(),
            pattern: "secret".to_owned(),
            enabled: false,
        }],
        server_email_delivery: ServerEmailDelivery::Unknown,
    };
    let pending = PendingNotificationEmail {
        address: "secret@example.invalid".to_owned(),
        resend_count: 1,
    };
    let rendered = format!("{snapshot:?} {pending:?}");
    assert!(!rendered.contains("secret@"), "{rendered}");
    assert!(rendered.contains("active_email_count: 1"));
}

fn rule(rule_id: &str, pattern: &str, enabled: bool) -> NotificationKeywordRuleState {
    NotificationKeywordRuleState {
        rule_id: rule_id.to_owned(),
        pattern: pattern.to_owned(),
        enabled,
    }
}

#[test]
fn keyword_normalization_trims_and_accepts() {
    assert_eq!(
        normalize_notification_keyword("  cake  ", &[]),
        Ok("cake".to_owned())
    );
}

#[test]
fn keyword_normalization_refuses_the_local_failures() {
    let existing = vec![
        rule("client.keyword.alpha", "alpha", true),
        // A rule another client turned off is still a duplicate and still
        // counts toward the cap.
        rule("client.keyword.beta", "beta", false),
    ];
    for (input, expected) in [
        ("", AccountNotificationsFailureKind::EmptyPattern),
        ("   ", AccountNotificationsFailureKind::EmptyPattern),
        ("\u{7}", AccountNotificationsFailureKind::EmptyPattern),
        ("ca\u{7}ke", AccountNotificationsFailureKind::EmptyPattern),
    ] {
        assert_eq!(
            normalize_notification_keyword(input, &existing),
            Err(expected),
            "{input:?}"
        );
    }
    let too_long = "a".repeat(MAX_NOTIFICATION_KEYWORD_CHARS + 1);
    assert_eq!(
        normalize_notification_keyword(&too_long, &existing),
        Err(AccountNotificationsFailureKind::PatternTooLong)
    );
    assert_eq!(
        normalize_notification_keyword("ALPHA", &existing),
        Err(AccountNotificationsFailureKind::Duplicate)
    );
    assert_eq!(
        normalize_notification_keyword("beta", &existing),
        Err(AccountNotificationsFailureKind::Duplicate)
    );
}

#[test]
fn keyword_normalization_refuses_non_ascii_case_variants() {
    // Matrix matching lowercases with full Unicode mappings, so a non-ASCII
    // letter's case variants are one keyword (#1597).
    for (existing_pattern, input) in [
        ("café", "CAFÉ"),
        ("CAFÉ", "café"),
        ("σίγμα", "ΣΊΓΜΑ"),
        ("дом", "ДОМ"),
    ] {
        let existing = vec![rule("client.keyword.one", existing_pattern, true)];
        assert_eq!(
            normalize_notification_keyword(input, &existing),
            Err(AccountNotificationsFailureKind::Duplicate),
            "{existing_pattern:?} vs {input:?}"
        );
    }
    // A width difference is not a case difference: the homeserver keeps those
    // patterns distinct, so the local refusal must not over-refuse them.
    let existing = vec![rule("client.keyword.fullwidth", "ｃａｋｅ", true)];
    assert_eq!(
        normalize_notification_keyword("cake", &existing),
        Ok("cake".to_owned())
    );
}

#[test]
fn keyword_normalization_refuses_an_add_beyond_the_cap() {
    let full: Vec<NotificationKeywordRuleState> = (0..MAX_NOTIFICATION_KEYWORDS)
        .map(|index| {
            rule(
                &format!("client.keyword.{index}"),
                &format!("k{index}"),
                true,
            )
        })
        .collect();
    assert_eq!(
        normalize_notification_keyword("one-more", &full),
        Err(AccountNotificationsFailureKind::LimitReached)
    );
    // The cap counts rules, so removing one makes room again.
    assert_eq!(
        normalize_notification_keyword("one-more", &full[1..]),
        Ok("one-more".to_owned())
    );
}

#[test]
fn keyword_removal_only_names_a_loaded_rule() {
    let existing = vec![rule("client.keyword.alpha", "alpha", false)];
    assert_eq!(
        normalize_notification_keyword_removal("client.keyword.alpha", &existing),
        Ok("client.keyword.alpha".to_owned())
    );
    for unknown in [
        "",
        "   ",
        "client.keyword.absent",
        ".m.rule.contains_user_name",
    ] {
        assert_eq!(
            normalize_notification_keyword_removal(unknown, &existing),
            Err(AccountNotificationsFailureKind::UnknownRule),
            "{unknown:?}"
        );
    }
}

#[test]
fn keyword_wire_shapes_match_the_typescript_mirror() {
    let snapshot = AccountNotificationsSnapshot {
        account_push_enabled: true,
        encrypted_event_push: false,
        categories: NotificationCategoryStates {
            direct_messages: NotificationCategoryState::On,
            group_messages: NotificationCategoryState::Off,
            mentions_and_replies: NotificationCategoryState::Mixed,
            invites: NotificationCategoryState::On,
        },
        email_management: NotificationEmailManagement::Available,
        emails: Vec::new(),
        unverified_email_pusher_count: 0,
        keywords: vec![rule("client.keyword.alpha", "alpha", false)],
        server_email_delivery: ServerEmailDelivery::KnownUnavailable,
    };
    let value = serde_json::to_value(&snapshot).unwrap();
    assert_eq!(value["server_email_delivery"], "knownUnavailable");
    // A snapshot from before #1460 carries no verdict and reads as unknown.
    let mut legacy = value.clone();
    legacy
        .as_object_mut()
        .unwrap()
        .remove("server_email_delivery");
    let legacy: AccountNotificationsSnapshot = serde_json::from_value(legacy).unwrap();
    assert_eq!(legacy.server_email_delivery, ServerEmailDelivery::Unknown);
    assert_eq!(
        value["keywords"],
        serde_json::json!([{
            "rule_id": "client.keyword.alpha",
            "pattern": "alpha",
            "enabled": false
        }])
    );
    // The operation enum is internally tagged and camelCase, but its fields
    // keep snake_case except where the React mirror says otherwise.
    assert_eq!(
        serde_json::to_value(AccountNotificationsOperation::AddNotificationKeyword {
            pattern: "alpha".to_owned(),
        })
        .unwrap(),
        serde_json::json!({"kind": "addNotificationKeyword", "pattern": "alpha"})
    );
    assert_eq!(
        serde_json::to_value(AccountNotificationsOperation::RemoveNotificationKeyword {
            rule_id: "client.keyword.alpha".to_owned(),
        })
        .unwrap(),
        serde_json::json!({
            "kind": "removeNotificationKeyword",
            "ruleId": "client.keyword.alpha"
        })
    );
    for (kind, expected) in [
        (
            AccountNotificationsFailureKind::EmptyPattern,
            "emptyPattern",
        ),
        (
            AccountNotificationsFailureKind::PatternTooLong,
            "patternTooLong",
        ),
        (AccountNotificationsFailureKind::Duplicate, "duplicate"),
        (
            AccountNotificationsFailureKind::LimitReached,
            "limitReached",
        ),
        (AccountNotificationsFailureKind::UnknownRule, "unknownRule"),
    ] {
        assert_eq!(serde_json::to_value(kind).unwrap(), expected);
    }
}
