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

/// One authoritative read of the account's notification settings.
#[derive(Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct AccountNotificationsSnapshot {
    /// `false` when another client enabled `.m.rule.master`, which silences
    /// every pusher (email included) regardless of the category switches.
    pub account_push_enabled: bool,
    pub categories: NotificationCategoryStates,
    pub email_management: NotificationEmailManagement,
    /// Validated email 3PIDs in server order.
    pub emails: Vec<NotificationEmailAddress>,
    /// Email pushers whose address is not a validated 3PID of this account
    /// (left by another client or a removed address). They still count as
    /// active delivery, so the email switch is not shown as OFF while any
    /// remain, and turning email notifications off removes them.
    pub unverified_email_pusher_count: u32,
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
    /// Re-authentication was rejected.
    AuthRejected,
    RateLimited,
    Network,
    Server,
    SessionRequired,
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

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
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

#[cfg(test)]
mod tests {
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
        };
        let pending = PendingNotificationEmail {
            address: "secret@example.invalid".to_owned(),
            resend_count: 1,
        };
        let rendered = format!("{snapshot:?} {pending:?}");
        assert!(!rendered.contains("secret@"), "{rendered}");
        assert!(rendered.contains("active_email_count: 1"));
    }
}
