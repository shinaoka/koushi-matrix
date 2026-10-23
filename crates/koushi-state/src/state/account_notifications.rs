//! Account-level notification settings (#981).
//!
//! These are the server-owned settings that every Matrix client on the account
//! shares: the standard push rules behind the four ON/OFF categories, the
//! validated email 3PIDs, and the `kind: email` pushers that deliver the
//! homeserver's unread-notification digest. The device-local "app
//! notifications" switch stays in `SettingsValues.notifications` and is never
//! folded into this slice, so turning app notifications off cannot disable
//! email delivery.
//!
//! Every snapshot is an authoritative server read. The reducer never derives an
//! ON state from a pending or failed operation.

use serde::{Deserialize, Serialize};

use crate::matrix_case_key;

/// One of the shared notification categories shown as a single ON/OFF switch.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum NotificationCategory {
    /// `.m.rule.room_one_to_one` + `.m.rule.encrypted_room_one_to_one`.
    DirectMessages,
    /// `.m.rule.message` + `.m.rule.encrypted`.
    GroupMessages,
    /// `.m.rule.is_user_mention` + `.m.rule.is_room_mention` (legacy
    /// `contains_display_name` / `contains_user_name` / `roomnotif` fallback).
    MentionsAndReplies,
    /// `.m.rule.invite_for_me`.
    Invites,
}

/// Authoritative projection of a category's push rules.
///
/// `Mixed` means the rules behind one switch disagree — for example another
/// client set encrypted and unencrypted group rules differently, or disabled
/// only `@room` mentions. It is preserved until the user toggles the switch.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum NotificationCategoryState {
    On,
    Off,
    Mixed,
    /// The homeserver has none of the category's rules; the switch cannot
    /// change anything and is shown disabled.
    Unavailable,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct NotificationCategoryStates {
    pub direct_messages: NotificationCategoryState,
    pub group_messages: NotificationCategoryState,
    pub mentions_and_replies: NotificationCategoryState,
    pub invites: NotificationCategoryState,
}

impl NotificationCategoryStates {
    pub fn get(&self, category: NotificationCategory) -> NotificationCategoryState {
        match category {
            NotificationCategory::DirectMessages => self.direct_messages,
            NotificationCategory::GroupMessages => self.group_messages,
            NotificationCategory::MentionsAndReplies => self.mentions_and_replies,
            NotificationCategory::Invites => self.invites,
        }
    }
}

/// Whether Koushi may add a notification email for this account.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum NotificationEmailManagement {
    /// Standard 3PID add/validate is available.
    Available,
    /// The homeserver advertises `m.3pid_changes: false`.
    Unsupported,
    /// The session uses OAuth 2.0 / MAS; email is managed at the account
    /// management destination instead of through client-server 3PID calls.
    DelegatedToAccountManagement,
}

/// Whether the homeserver is known to send the mail an email pusher asks for (#1460).
///
/// Matrix exposes no capability for this, so the only facts are verified ones about named
/// servers; every other server is `Unknown` and the UI never claims mail will arrive.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ServerEmailDelivery {
    #[default]
    Unknown,
    /// A verified fact says this server does not send pusher mail (today only `matrix.org`).
    KnownUnavailable,
}

/// A validated email 3PID and whether an email pusher currently targets it.
#[derive(Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct NotificationEmailAddress {
    pub address: String,
    pub notifications_active: bool,
}

impl std::fmt::Debug for NotificationEmailAddress {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("NotificationEmailAddress")
            .field("address", &"<redacted>")
            .field("notifications_active", &self.notifications_active)
            .finish()
    }
}

/// One of the account's own `kind: content` push rules (#1427), as the keyword
/// editor shows it.
///
/// `enabled` is a server fact and not a local preference: another client can
/// turn a rule off, and a rule that is off is still on the account, still
/// refuses a same-pattern add, and still has to be removable, so it is listed
/// as off rather than hidden.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct NotificationKeywordRuleState {
    pub rule_id: String,
    pub pattern: String,
    pub enabled: bool,
}

/// One authoritative read of the account's notification settings.
#[derive(Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct AccountNotificationsSnapshot {
    /// `false` when another client enabled `.m.rule.master`, which silences
    /// every pusher (email included) regardless of the category switches.
    pub account_push_enabled: bool,
    /// MSC4028: `.m.rule.encrypted_event` (or its unstable id) is enabled, so
    /// the server pushes every encrypted event and devices decide locally.
    /// When `false`, the server cannot see mentions inside encrypted events:
    /// with Group messages OFF, mentions in encrypted group rooms reach no
    /// pusher (email or other devices); this app still evaluates them after
    /// decryption. When `true`, Group OFF does not silence encrypted rooms at
    /// the server.
    pub encrypted_event_push: bool,
    pub categories: NotificationCategoryStates,
    pub email_management: NotificationEmailManagement,
    /// Validated email 3PIDs in server order.
    pub emails: Vec<NotificationEmailAddress>,
    /// Email pushers whose address is not a validated 3PID of this account
    /// (left by another client or a removed address). They still count as
    /// active delivery, so the email switch is not shown as OFF while any
    /// remain, and turning email notifications off removes them.
    pub unverified_email_pusher_count: u32,
    /// The account's own `kind: content` rules, in the server's rule order.
    ///
    /// Server state, never a local edit: the list is replaced by each
    /// authoritative read and is never appended to or removed from because the
    /// user asked for it. Loading or opening the screen still writes nothing.
    #[serde(default)]
    pub keywords: Vec<NotificationKeywordRuleState>,
    /// Whether the homeserver is known to send pusher mail; see [`ServerEmailDelivery`].
    #[serde(default)]
    pub server_email_delivery: ServerEmailDelivery,
}

impl AccountNotificationsSnapshot {
    /// Email notifications are ON only when the server reports an email pusher.
    pub fn email_notifications_active(&self) -> bool {
        self.unverified_email_pusher_count > 0
            || self.emails.iter().any(|email| email.notifications_active)
    }
}

impl std::fmt::Debug for AccountNotificationsSnapshot {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("AccountNotificationsSnapshot")
            .field("account_push_enabled", &self.account_push_enabled)
            .field("encrypted_event_push", &self.encrypted_event_push)
            .field("categories", &self.categories)
            .field("email_management", &self.email_management)
            .field("email_count", &self.emails.len())
            .field(
                "active_email_count",
                &self
                    .emails
                    .iter()
                    .filter(|email| email.notifications_active)
                    .count(),
            )
            .field(
                "unverified_email_pusher_count",
                &self.unverified_email_pusher_count,
            )
            .field("server_email_delivery", &self.server_email_delivery)
            .finish()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AccountNotificationsFailureKind {
    /// The homeserver does not offer this operation (no email support,
    /// `M_UNRECOGNIZED`, `M_THREEPID_MEDIUM_NOT_SUPPORTED`, 3PIDs disabled).
    Unsupported,
    /// `M_THREEPID_IN_USE`.
    EmailInUse,
    /// `M_THREEPID_DENIED`.
    EmailDenied,
    /// The address failed local syntax validation or the server rejected it.
    InvalidEmail,
    /// `M_THREEPID_AUTH_FAILED`: the verification link was not opened yet.
    EmailNotVerified,
    /// The target address is not a validated 3PID of this account.
    EmailNotRegistered,
    /// Re-authentication (password step) was rejected.
    AuthRejected,
    /// The server refused the operation (`M_FORBIDDEN` outside the password
    /// step), e.g. a permission or policy restriction.
    Forbidden,
    RateLimited,
    Network,
    Server,
    SessionRequired,
    /// The keyword input was empty after trimming, or carried control
    /// characters that cannot be typed into the single-line field and would
    /// never match a `content.body` (#1427).
    EmptyPattern,
    /// The keyword pattern is longer than [`MAX_NOTIFICATION_KEYWORD_CHARS`].
    PatternTooLong,
    /// A `content` rule for the same pattern already exists on the account; the
    /// comparison is case-insensitive, matching Matrix, and includes a rule
    /// another client turned off. It also covers the astronomically unlikely
    /// rule-id collision with a different pattern, because the account does hold
    /// something at that name.
    Duplicate,
    /// The account already holds [`MAX_NOTIFICATION_KEYWORDS`] keyword rules.
    LimitReached,
    /// The write names a keyword rule the account does not have; a race with
    /// another client removed it between the load and the remove.
    UnknownRule,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum AccountNotificationsLoadState {
    #[default]
    NotLoaded,
    Loading {
        request_id: u64,
    },
    Loaded,
    Failed {
        request_id: u64,
        #[serde(rename = "failureKind")]
        failure_kind: AccountNotificationsFailureKind,
    },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum AccountNotificationsOperation {
    SetCategory {
        category: NotificationCategory,
        enabled: bool,
    },
    SetAccountPush {
        enabled: bool,
    },
    RequestEmailToken,
    ResendEmailToken,
    ConfirmEmail,
    EnableEmailNotifications,
    DisableEmailNotifications,
    /// Write one keyword as a `content` rule that notifies and highlights.
    AddNotificationKeyword {
        pattern: String,
    },
    /// Delete one keyword's `content` rule by the id the server gave it.
    RemoveNotificationKeyword {
        #[serde(rename = "ruleId")]
        rule_id: String,
    },
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum AccountNotificationsOperationState {
    #[default]
    Idle,
    Working {
        request_id: u64,
        operation: AccountNotificationsOperation,
    },
    AwaitingUia {
        request_id: u64,
        flow_id: u64,
        operation: AccountNotificationsOperation,
    },
    Succeeded {
        request_id: u64,
        operation: AccountNotificationsOperation,
    },
    Failed {
        request_id: u64,
        operation: AccountNotificationsOperation,
        #[serde(rename = "failureKind")]
        failure_kind: AccountNotificationsFailureKind,
    },
}

/// A notification email awaiting ownership confirmation.
///
/// The client secret, session id, and send attempt stay in the account actor;
/// the reducer keeps only the address for display and a resend count.
#[derive(Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct PendingNotificationEmail {
    pub address: String,
    pub resend_count: u32,
}

impl std::fmt::Debug for PendingNotificationEmail {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PendingNotificationEmail")
            .field("address", &"<redacted>")
            .field("resend_count", &self.resend_count)
            .finish()
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct AccountNotificationsState {
    pub load: AccountNotificationsLoadState,
    pub snapshot: Option<AccountNotificationsSnapshot>,
    pub pending_email: Option<PendingNotificationEmail>,
    pub operation: AccountNotificationsOperationState,
}

/// Maximum accepted email length (RFC 5321 path limit).
pub const MAX_NOTIFICATION_EMAIL_LEN: usize = 254;

/// Normalize and syntax-check an email address before it leaves the client.
///
/// This is intentionally conservative: one `@`, non-empty local and domain
/// parts, a dot in the domain, no whitespace or control characters. The
/// homeserver remains the authority for deliverability.
pub fn normalize_notification_email(input: &str) -> Option<String> {
    let trimmed = input.trim();
    if trimmed.is_empty() || trimmed.len() > MAX_NOTIFICATION_EMAIL_LEN {
        return None;
    }
    if trimmed
        .chars()
        .any(|character| character.is_whitespace() || character.is_control())
    {
        return None;
    }
    let (local, domain) = trimmed.split_once('@')?;
    if local.is_empty() || domain.is_empty() || domain.contains('@') {
        return None;
    }
    if !domain.contains('.') || domain.starts_with('.') || domain.ends_with('.') {
        return None;
    }
    // Matrix homeservers compare email 3PIDs case-insensitively and store them
    // lower-cased; normalizing here keeps the pusher pushkey identical to the
    // 3PID address the server returns.
    Some(trimmed.to_lowercase())
}

/// Maximum number of keyword rules one account may hold (#1427). Mirrors the
/// SDK's `KEYWORD_RULES_MAX`, which caps the projection; this bound refuses an
/// add that would exceed it rather than silently trimming the newest rule.
pub const MAX_NOTIFICATION_KEYWORDS: usize = 64;

/// Maximum number of characters in one keyword pattern, after trimming
/// (#1427). Mirrors the SDK's `KEYWORD_PATTERN_MAX_CHARS`.
pub const MAX_NOTIFICATION_KEYWORD_CHARS: usize = 64;

/// Normalize and validate a keyword pattern before anything leaves the client.
///
/// The product-facing refusal: trim, reject empty, reject control characters,
/// reject over [`MAX_NOTIFICATION_KEYWORD_CHARS`], reject a pattern that matches
/// an existing rule without regard to case (Matrix matches `content` patterns
/// case-insensitively, so a second rule could never fire), and reject an add
/// that would exceed [`MAX_NOTIFICATION_KEYWORDS`]. `existing` is the last
/// authoritative rule list, disabled rules included, so a local refusal costs no
/// request; the homeserver stays the authority for the write itself, which is
/// why `koushi-sdk`'s `plan_keyword_rule` repeats the pattern and duplicate
/// checks on the write path and maps to the same failure kinds.
pub fn normalize_notification_keyword(
    input: &str,
    existing: &[NotificationKeywordRuleState],
) -> Result<String, AccountNotificationsFailureKind> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err(AccountNotificationsFailureKind::EmptyPattern);
    }
    // A control character cannot be typed into the shared single-line field and
    // would make a pattern that can never match a `content.body`; the frozen
    // failure union has no invalid-character member, so a pattern that is only
    // usable if it is stripped reads as an empty one.
    if trimmed.chars().any(|character| character.is_control()) {
        return Err(AccountNotificationsFailureKind::EmptyPattern);
    }
    if trimmed.chars().count() > MAX_NOTIFICATION_KEYWORD_CHARS {
        return Err(AccountNotificationsFailureKind::PatternTooLong);
    }
    // The homeserver compares lowercased text, so the refusal must fold
    // non-ASCII letters too: `café` and `CAFÉ` are one keyword (#1597).
    let case_key = matrix_case_key(trimmed);
    if existing
        .iter()
        .any(|rule| matrix_case_key(&rule.pattern) == case_key)
    {
        return Err(AccountNotificationsFailureKind::Duplicate);
    }
    // A disabled rule is still on the account, so it counts toward the cap.
    if existing.len() >= MAX_NOTIFICATION_KEYWORDS {
        return Err(AccountNotificationsFailureKind::LimitReached);
    }
    Ok(trimmed.to_owned())
}

/// Whether a removal may name `rule_id`, given the last authoritative list.
///
/// A remove names a rule id that exists in the loaded list; an unknown id is
/// refused without a delete request, because the account would answer with a
/// 404 and the user cannot act on it.
pub fn normalize_notification_keyword_removal(
    rule_id: &str,
    existing: &[NotificationKeywordRuleState],
) -> Result<String, AccountNotificationsFailureKind> {
    let trimmed = rule_id.trim();
    if trimmed.is_empty() || existing.iter().all(|rule| rule.rule_id != trimmed) {
        return Err(AccountNotificationsFailureKind::UnknownRule);
    }
    Ok(trimmed.to_owned())
}

#[cfg(test)]
mod tests;
