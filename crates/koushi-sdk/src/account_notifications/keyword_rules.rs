//! Keyword push rules (#1427): the account's `kind: content` rules as the
//! Matrix-facing authority for the device-local keyword editor.
//!
//! Canon (`docs/architecture/state-machine.md`, "Local Notification Delivery",
//! "Keyword rules"): the keyword list is the account's own `content` rules read
//! back from the homeserver rather than mirrored into local settings, adding a
//! keyword writes one `content` rule whose action both notifies and sets the
//! highlight tweak, removing a keyword deletes that rule, matching stays exactly
//! Matrix's whole-word case-insensitive match on `content.body`, the list and
//! each pattern are bounded, a pattern is trimmed, and a duplicate is refused
//! rather than written twice.
//!
//! This module owns only the Matrix-facing primitives: rule ids, patterns, the
//! request shape, and typed failures. It does not decide what the settings
//! surface shows, how a keyword is labelled, or whether the delivery switch is
//! on; the core maps [`KeywordRule`] into that product state.
//!
//! Verified against the vendored SDK and the specification:
//!
//! - A keyword is a `content` rule: the spec's "Content-specific rules" match a
//!   glob `pattern` case-insensitively against `content.body`, starting and
//!   ending at a word boundary, and the vendored SDK treats every non-default
//!   `content` rule as a keyword (`matrix-sdk`,
//!   `src/notification_settings/rules.rs`, `enabled_keywords` / `keyword_rules`).
//! - The highlight action is required. The spec's `highlight` tweak defaults to
//!   `true` when written as `{ "set_tweak": "highlight" }`, and the SDK derives
//!   a mention from `Action::is_highlight()`
//!   (`matrix-sdk/src/event_cache/caches/read_receipts.rs`,
//!   `matrix-sdk-ui/src/notification_client.rs`), which is the authority the
//!   product's highlight count follows. The SDK's own keyword helper,
//!   `Command::SetKeywordPushRule` (`matrix-sdk/src/notification_settings/command.rs`),
//!   writes `notify` plus a default sound and **not** highlight, so it must not
//!   be used here: that rule would notify without ever reaching the mention-kind
//!   authority. This module writes `[Notify, SetTweak(Highlight(Yes))]`, the same
//!   shape the spec gives `.m.rule.is_room_mention` and the retired
//!   `.m.rule.contains_user_name`.
//! - The caller supplies the rule id. `PUT /pushrules/global/{kind}/{ruleId}`
//!   takes it in the path (ruma-client-api `src/push/set_pushrule.rs`,
//!   `RequestBody` carries only `actions` and `pattern` for `content`); it is
//!   unique within the `content` kind and scoped to the account's global
//!   ruleset, not to a device. Spec "Predefined Rules" reserves ids starting
//!   with a dot for server-default rules, so [`keyword_rule_id`] adds a
//!   `.`-free prefix and a deterministic suffix.

use koushi_state::{AccountNotificationsFailureKind, matrix_case_key};
use matrix_sdk::ruma::{
    api::client::push::{delete_pushrule, set_pushrule},
    push::{NewPatternedPushRule, NewPushRule, RuleKind, Ruleset},
};
use thiserror::Error;

use super::{RuleActions, classify_http_error, fetch_push_ruleset};
use crate::MatrixClientSession;

/// Prefix of every keyword rule id Koushi writes, mirroring the room-rule
/// prefix in `room_operations`. It deliberately does not start with a dot,
/// which the spec reserves for server-default rules.
pub const KEYWORD_RULE_ID_PREFIX: &str = "org.matrix.desktop.notify.keyword.";

/// Maximum number of keyword rules [`fetch_keyword_rules`] returns.
pub const KEYWORD_RULES_MAX: usize = 64;

/// Maximum number of characters in one keyword pattern, after trimming.
pub const KEYWORD_PATTERN_MAX_CHARS: usize = 64;

/// Maximum number of characters in a rule id accepted for deletion.
pub const KEYWORD_RULE_ID_MAX_CHARS: usize = 128;

/// One of the account's `content` push rules, projected for the keyword
/// editor. Only these facts cross the boundary; the product owns the rest.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct KeywordRule {
    /// The rule id the server scopes the rule by, unique within `content`.
    pub rule_id: String,
    /// The glob-style pattern Matrix matches against `content.body`.
    pub pattern: String,
    /// Whether the rule is enabled. A disabled rule still exists on the
    /// account and is still a duplicate of the same pattern.
    pub enabled: bool,
}

/// Why a keyword rule read, plan, write, or delete failed.
#[derive(Debug, Error, Eq, PartialEq)]
pub enum MatrixKeywordRuleError {
    /// The pattern is empty after trimming.
    #[error("keyword pattern is empty")]
    EmptyPattern,
    /// The pattern is longer than [`KEYWORD_PATTERN_MAX_CHARS`].
    #[error("keyword pattern is too long")]
    PatternTooLong,
    /// A `content` rule for this pattern already exists on the account.
    #[error("keyword already exists")]
    Duplicate,
    /// The rule id is empty, too long, or in the server-default namespace.
    #[error("keyword rule id is invalid")]
    InvalidRuleId,
    /// The deterministic rule id already identifies a different pattern.
    #[error("keyword rule id is already used by another keyword")]
    RuleIdConflict,
    /// The homeserver read or write failed.
    #[error("Matrix keyword rule operation failed")]
    Failed(AccountNotificationsFailureKind),
}

/// Project the account's keyword rules from an authoritative ruleset.
///
/// Only non-default `content` rules are the account's own keywords. Server
/// order is kept because it is also the evaluation order, and the list is
/// capped at [`KEYWORD_RULES_MAX`]. Pure: it never touches the network, so the
/// core can project a ruleset it already read for the category snapshot.
pub fn keyword_rules_from_ruleset(ruleset: &Ruleset) -> Vec<KeywordRule> {
    ruleset
        .content
        .iter()
        .filter(|rule| !rule.default)
        .take(KEYWORD_RULES_MAX)
        .map(|rule| KeywordRule {
            rule_id: rule.rule_id.clone(),
            pattern: rule.pattern.clone(),
            enabled: rule.enabled,
        })
        .collect()
}

/// Read the account's keyword rules from the homeserver (GET only).
pub async fn fetch_keyword_rules(
    session: &MatrixClientSession,
) -> Result<Vec<KeywordRule>, MatrixKeywordRuleError> {
    let ruleset = fetch_push_ruleset(session)
        .await
        .map_err(MatrixKeywordRuleError::Failed)?;
    Ok(keyword_rules_from_ruleset(&ruleset))
}

/// Plan the one `content` rule to write for `pattern` against the current
/// authoritative `ruleset`.
///
/// Pure: an invalid pattern or a duplicate is refused before any request. The
/// duplicate scan reads the whole `content` list rather than the capped
/// projection, so a keyword beyond the cap still refuses a second write.
pub fn plan_keyword_rule(
    ruleset: &Ruleset,
    pattern: &str,
) -> Result<KeywordRule, MatrixKeywordRuleError> {
    let pattern = normalize_pattern(pattern)?;
    let rule_id = keyword_rule_id(pattern);
    // The homeserver compares lowercased text, so a non-ASCII case variant is
    // the same keyword and must not be written a second time (#1597).
    let case_key = matrix_case_key(pattern);
    if ruleset
        .content
        .iter()
        .any(|rule| !rule.default && matrix_case_key(&rule.pattern) == case_key)
    {
        return Err(MatrixKeywordRuleError::Duplicate);
    }
    // A deterministic id makes a retry idempotent; a collision with a different
    // pattern fails closed instead of overwriting that keyword.
    if ruleset
        .content
        .iter()
        .any(|rule| !rule.default && rule.rule_id == rule_id && rule.pattern != pattern)
    {
        return Err(MatrixKeywordRuleError::RuleIdConflict);
    }
    Ok(KeywordRule {
        rule_id,
        pattern: pattern.to_owned(),
        enabled: true,
    })
}

/// Add one keyword as a `content` rule that notifies and highlights.
///
/// The pattern is trimmed and bounded, a duplicate is refused before the write,
/// and a rejected write is a typed failure rather than a silent success.
pub async fn add_keyword_rule(
    session: &MatrixClientSession,
    pattern: &str,
) -> Result<KeywordRule, MatrixKeywordRuleError> {
    // Refuse an invalid pattern before any request is sent.
    let pattern = normalize_pattern(pattern)?;
    let ruleset = fetch_push_ruleset(session)
        .await
        .map_err(MatrixKeywordRuleError::Failed)?;
    let rule = plan_keyword_rule(&ruleset, pattern)?;
    let new_rule = NewPatternedPushRule::new(
        rule.rule_id.clone(),
        rule.pattern.clone(),
        RuleActions::NotifyHighlight.to_ruma(),
    );
    session
        .client()
        .send(set_pushrule::v3::Request::new(NewPushRule::Content(
            new_rule,
        )))
        .await
        .map_err(|error| MatrixKeywordRuleError::Failed(classify_http_error(&error)))?;
    Ok(rule)
}

/// Remove one keyword by deleting its `content` rule.
pub async fn remove_keyword_rule(
    session: &MatrixClientSession,
    rule_id: &str,
) -> Result<(), MatrixKeywordRuleError> {
    let rule_id = rule_id.trim();
    // A dot prefix is the server-default namespace, so deleting it would remove
    // a spec rule rather than a keyword; an empty or oversized id is refused
    // before it reaches the URL path.
    if rule_id.is_empty()
        || rule_id.starts_with('.')
        || rule_id.chars().count() > KEYWORD_RULE_ID_MAX_CHARS
    {
        return Err(MatrixKeywordRuleError::InvalidRuleId);
    }
    session
        .client()
        .send(delete_pushrule::v3::Request::new(
            RuleKind::Content,
            rule_id.to_owned(),
        ))
        .await
        .map(|_| ())
        .map_err(|error| MatrixKeywordRuleError::Failed(classify_http_error(&error)))
}

fn normalize_pattern(pattern: &str) -> Result<&str, MatrixKeywordRuleError> {
    let pattern = pattern.trim();
    if pattern.is_empty() {
        return Err(MatrixKeywordRuleError::EmptyPattern);
    }
    if pattern.chars().count() > KEYWORD_PATTERN_MAX_CHARS {
        return Err(MatrixKeywordRuleError::PatternTooLong);
    }
    Ok(pattern)
}

/// Deterministic, bounded rule id for a normalized pattern.
fn keyword_rule_id(pattern: &str) -> String {
    format!(
        "{KEYWORD_RULE_ID_PREFIX}{:016x}",
        fnv1a_64(pattern.as_bytes())
    )
}

/// FNV-1a over the pattern's UTF-8 bytes: stable across runs, dependency-free,
/// and fixed-width, so the rule id is bounded regardless of the pattern.
fn fnv1a_64(bytes: &[u8]) -> u64 {
    const OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;
    let mut hash = OFFSET_BASIS;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(PRIME);
    }
    hash
}

#[cfg(test)]
mod tests;
