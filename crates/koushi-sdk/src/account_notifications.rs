//! Account-level notification settings (#981): shared push-rule categories,
//! email 3PIDs, and `kind: email` pushers.
//!
//! Element compatibility contract (docs/architecture/state-machine.md,
//! "Account Notification Settings"):
//!
//! - Categories are projections of the standard predefined push rules, read
//!   from `GET /pushrules/` (not the sync cache). Reading never writes.
//! - A write happens only for an explicit user toggle, and only touches the
//!   rules of that category that are not already in the requested state.
//!   Rules outside the category, custom rules, keywords, and room rules are
//!   left untouched; re-enabling a disabled rule keeps its actions. Turning an
//!   underride category OFF writes `actions: []` (dropping any custom tweak on
//!   those rules), and ON restores the spec default actions.
//! - Email pushers use Element Web's shape: `kind: email`, `app_id: m.email`,
//!   `pushkey: <validated 3PID address>`, `append: true`.

use koushi_state::{
    AccountNotificationsFailureKind, AccountNotificationsSnapshot, IdentityResetAuthRequest,
    NotificationCategory, NotificationCategoryState, NotificationCategoryStates,
    NotificationEmailAddress, NotificationEmailManagement,
};
use matrix_sdk::ruma::{
    api::client::push::{
        EmailPusherData, PusherIds, PusherInit, PusherKind, get_pushers, get_pushrules_all,
        set_pushrule_actions, set_pushrule_enabled,
    },
    push::{Action, RuleKind, Ruleset, SoundTweakValue, Tweak},
    thirdparty::Medium,
};

use crate::MatrixClientSession;

/// Element Web's app id for email pushers.
pub const EMAIL_PUSHER_APP_ID: &str = "m.email";
const EMAIL_PUSHER_APP_DISPLAY_NAME: &str = "Email Notifications";
const EMAIL_PUSHER_BRAND: &str = "Koushi";

// ── Pure rule model ─────────────────────────────────────────────────────────

/// App-owned description of the actions Koushi writes when turning a rule ON
/// or OFF. Using a closed enum keeps write plans comparable in tests.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RuleActions {
    /// `[]` — the rule matches but does not notify.
    Silent,
    /// `["notify"]`.
    Notify,
    /// `["notify", {"set_tweak": "sound", "value": "default"}]`.
    NotifyWithSound,
    /// `["notify", {"set_tweak": "highlight"}]`.
    NotifyHighlight,
    /// `["notify", sound default, highlight]`.
    NotifyHighlightWithSound,
}

impl RuleActions {
    fn to_ruma(self) -> Vec<Action> {
        let sound = || Action::SetTweak(Tweak::Sound(SoundTweakValue::Default));
        let highlight = || Action::SetTweak(Tweak::Highlight(true.into()));
        match self {
            Self::Silent => Vec::new(),
            Self::Notify => vec![Action::Notify],
            Self::NotifyWithSound => vec![Action::Notify, sound()],
            Self::NotifyHighlight => vec![Action::Notify, highlight()],
            Self::NotifyHighlightWithSound => vec![Action::Notify, sound(), highlight()],
        }
    }
}

/// One push-rule mutation planned for a category toggle.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RuleWrite {
    SetEnabled {
        kind: RuleKind,
        rule_id: String,
        enabled: bool,
    },
    SetActions {
        kind: RuleKind,
        rule_id: String,
        actions: RuleActions,
    },
}

/// How a rule is switched OFF.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum OffStrategy {
    /// Underride message rules: keep the rule enabled with empty actions,
    /// matching the SDK's `MentionsAndKeywordsOnly` default mode and Element.
    SilentActions,
    /// Override/content rules: disable the rule, matching Element X's
    /// `set_*_enabled(false)`; its actions (and sound) survive for re-enable.
    Disable,
}

#[derive(Clone, Debug)]
struct RuleSpec {
    kind: RuleKind,
    rule_id: &'static str,
    /// Spec default actions restored when a silent rule is turned back ON.
    on_actions: RuleActions,
    off: OffStrategy,
}

impl RuleSpec {
    const fn underride(rule_id: &'static str, on_actions: RuleActions) -> Self {
        Self {
            kind: RuleKind::Underride,
            rule_id,
            on_actions,
            off: OffStrategy::SilentActions,
        }
    }

    const fn toggled(kind: RuleKind, rule_id: &'static str, on_actions: RuleActions) -> Self {
        Self {
            kind,
            rule_id,
            on_actions,
            off: OffStrategy::Disable,
        }
    }
}

fn dm_rules() -> [RuleSpec; 2] {
    [
        RuleSpec::underride(".m.rule.room_one_to_one", RuleActions::NotifyWithSound),
        RuleSpec::underride(
            ".m.rule.encrypted_room_one_to_one",
            RuleActions::NotifyWithSound,
        ),
    ]
}

fn group_rules() -> [RuleSpec; 2] {
    [
        RuleSpec::underride(".m.rule.message", RuleActions::Notify),
        RuleSpec::underride(".m.rule.encrypted", RuleActions::Notify),
    ]
}

/// Poll-start rules follow their category on write (as the SDK's
/// `set_default_room_notification_mode` does) but never make a category read
/// as mixed: they are unstable and absent on some homeservers.
fn dm_companion_rules() -> [RuleSpec; 1] {
    [RuleSpec::underride(
        ".org.matrix.msc3930.rule.poll_start_one_to_one",
        RuleActions::NotifyWithSound,
    )]
}

fn group_companion_rules() -> [RuleSpec; 1] {
    [RuleSpec::underride(
        ".org.matrix.msc3930.rule.poll_start",
        RuleActions::Notify,
    )]
}

fn user_mention_modern() -> RuleSpec {
    RuleSpec::toggled(
        RuleKind::Override,
        ".m.rule.is_user_mention",
        RuleActions::NotifyHighlightWithSound,
    )
}

fn user_mention_legacy() -> [RuleSpec; 2] {
    [
        RuleSpec::toggled(
            RuleKind::Override,
            ".m.rule.contains_display_name",
            RuleActions::NotifyHighlightWithSound,
        ),
        RuleSpec::toggled(
            RuleKind::Content,
            ".m.rule.contains_user_name",
            RuleActions::NotifyHighlightWithSound,
        ),
    ]
}

fn room_mention_modern() -> RuleSpec {
    RuleSpec::toggled(
        RuleKind::Override,
        ".m.rule.is_room_mention",
        RuleActions::NotifyHighlight,
    )
}

fn room_mention_legacy() -> RuleSpec {
    RuleSpec::toggled(
        RuleKind::Override,
        ".m.rule.roomnotif",
        RuleActions::NotifyHighlight,
    )
}

fn invite_rule() -> RuleSpec {
    RuleSpec::toggled(
        RuleKind::Override,
        ".m.rule.invite_for_me",
        RuleActions::NotifyWithSound,
    )
}

const MASTER_RULE_ID: &str = ".m.rule.master";

/// `Some(true)` when the rule exists, is enabled, and notifies.
fn rule_notifies(ruleset: &Ruleset, spec: &RuleSpec) -> Option<bool> {
    ruleset
        .get(spec.kind.clone(), spec.rule_id)
        .map(|rule| rule.enabled() && rule.triggers_notification())
}

fn combine(states: impl IntoIterator<Item = Option<bool>>) -> NotificationCategoryState {
    let mut any_on = false;
    let mut any_off = false;
    for state in states.into_iter().flatten() {
        if state {
            any_on = true;
        } else {
            any_off = true;
        }
    }
    match (any_on, any_off) {
        (true, true) => NotificationCategoryState::Mixed,
        (true, false) => NotificationCategoryState::On,
        _ => NotificationCategoryState::Off,
    }
}

/// Mirrors the SDK's `is_user_mention_enabled`: the MSC3952 rule wins when the
/// server has it; otherwise the deprecated display-name/user-name rules.
fn user_mentions_notify(ruleset: &Ruleset) -> Option<bool> {
    rule_notifies(ruleset, &user_mention_modern()).or_else(|| {
        let legacy: Vec<bool> = user_mention_legacy()
            .iter()
            .filter_map(|spec| rule_notifies(ruleset, spec))
            .collect();
        (!legacy.is_empty()).then(|| legacy.into_iter().any(|notifies| notifies))
    })
}

fn room_mentions_notify(ruleset: &Ruleset) -> Option<bool> {
    rule_notifies(ruleset, &room_mention_modern())
        .or_else(|| rule_notifies(ruleset, &room_mention_legacy()))
}

/// Project the category switches from an authoritative ruleset.
pub fn summarize_categories(ruleset: &Ruleset) -> NotificationCategoryStates {
    NotificationCategoryStates {
        direct_messages: combine(dm_rules().iter().map(|spec| rule_notifies(ruleset, spec))),
        group_messages: combine(
            group_rules()
                .iter()
                .map(|spec| rule_notifies(ruleset, spec)),
        ),
        mentions_and_replies: combine([
            user_mentions_notify(ruleset),
            room_mentions_notify(ruleset),
        ]),
        invites: combine([rule_notifies(ruleset, &invite_rule())]),
    }
}

/// `false` when `.m.rule.master` is enabled (all notifications suppressed).
pub fn account_push_enabled(ruleset: &Ruleset) -> bool {
    !ruleset
        .get(RuleKind::Override, MASTER_RULE_ID)
        .is_some_and(|rule| rule.enabled())
}

fn plan_rule(ruleset: &Ruleset, spec: &RuleSpec, enabled: bool, writes: &mut Vec<RuleWrite>) {
    let Some(rule) = ruleset.get(spec.kind.clone(), spec.rule_id) else {
        // Absent on this homeserver: nothing to write, nothing to create.
        return;
    };
    let rule_enabled = rule.enabled();
    let rule_triggers = rule.triggers_notification();
    let notifies = rule_enabled && rule_triggers;
    if notifies == enabled {
        return;
    }
    let rule_id = spec.rule_id.to_owned();
    if enabled {
        if !rule_triggers {
            writes.push(RuleWrite::SetActions {
                kind: spec.kind.clone(),
                rule_id: rule_id.clone(),
                actions: spec.on_actions,
            });
        }
        if !rule_enabled {
            writes.push(RuleWrite::SetEnabled {
                kind: spec.kind.clone(),
                rule_id,
                enabled: true,
            });
        }
    } else {
        match spec.off {
            OffStrategy::SilentActions => writes.push(RuleWrite::SetActions {
                kind: spec.kind.clone(),
                rule_id,
                actions: RuleActions::Silent,
            }),
            OffStrategy::Disable => writes.push(RuleWrite::SetEnabled {
                kind: spec.kind.clone(),
                rule_id,
                enabled: false,
            }),
        }
    }
}

/// Plan the minimal writes that put `category` in the requested state.
///
/// Returns an empty plan when every rule of the category already matches, so
/// re-applying the current value never rewrites the server.
pub fn plan_category_writes(
    ruleset: &Ruleset,
    category: NotificationCategory,
    enabled: bool,
) -> Vec<RuleWrite> {
    let mut specs: Vec<RuleSpec> = Vec::new();
    match category {
        NotificationCategory::DirectMessages => {
            specs.extend(dm_rules());
            specs.extend(dm_companion_rules());
        }
        NotificationCategory::GroupMessages => {
            specs.extend(group_rules());
            specs.extend(group_companion_rules());
        }
        NotificationCategory::MentionsAndReplies => {
            // Same rule set the SDK touches for user/room mention toggles:
            // the MSC3952 rules plus the deprecated fallbacks when present.
            specs.push(user_mention_modern());
            specs.extend(user_mention_legacy());
            specs.push(room_mention_modern());
            specs.push(room_mention_legacy());
        }
        NotificationCategory::Invites => specs.push(invite_rule()),
    }
    let mut writes = Vec::new();
    for spec in &specs {
        plan_rule(ruleset, spec, enabled, &mut writes);
    }
    writes
}

/// Plan the `.m.rule.master` write for the account-wide notification switch.
pub fn plan_account_push_writes(ruleset: &Ruleset, enabled: bool) -> Vec<RuleWrite> {
    if account_push_enabled(ruleset) == enabled {
        return Vec::new();
    }
    if ruleset.get(RuleKind::Override, MASTER_RULE_ID).is_none() {
        return Vec::new();
    }
    vec![RuleWrite::SetEnabled {
        kind: RuleKind::Override,
        rule_id: MASTER_RULE_ID.to_owned(),
        enabled: !enabled,
    }]
}

/// Build a snapshot from authoritative server reads.
pub fn build_account_notifications_snapshot(
    ruleset: &Ruleset,
    email_management: NotificationEmailManagement,
    validated_emails: &[String],
    email_pusher_keys: &[String],
) -> AccountNotificationsSnapshot {
    let emails: Vec<NotificationEmailAddress> = validated_emails
        .iter()
        .map(|address| NotificationEmailAddress {
            address: address.clone(),
            notifications_active: email_pusher_keys
                .iter()
                .any(|pushkey| pushkey.eq_ignore_ascii_case(address)),
        })
        .collect();
    let unverified_email_pusher_count = email_pusher_keys
        .iter()
        .filter(|pushkey| {
            !validated_emails
                .iter()
                .any(|address| address.eq_ignore_ascii_case(pushkey))
        })
        .count()
        .try_into()
        .unwrap_or(u32::MAX);
    AccountNotificationsSnapshot {
        account_push_enabled: account_push_enabled(ruleset),
        categories: summarize_categories(ruleset),
        email_management,
        emails,
        unverified_email_pusher_count,
    }
}

// ── Errors ──────────────────────────────────────────────────────────────────

/// Outcome of confirming a pending email (`POST /account/3pid/add`).
#[derive(thiserror::Error)]
pub enum AddNotificationEmailError {
    #[error("interactive authentication required")]
    UiaaChallenge { session: Option<String> },
    #[error("add notification email failed")]
    Failed(AccountNotificationsFailureKind),
}

impl std::fmt::Debug for AddNotificationEmailError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UiaaChallenge { session } => formatter
                .debug_struct("UiaaChallenge")
                .field("session", &session.as_ref().map(|_| "SessionId(..)"))
                .finish(),
            Self::Failed(kind) => formatter.debug_tuple("Failed").field(kind).finish(),
        }
    }
}

/// Coarse, private-data-free classification of a homeserver error.
pub fn classify_http_error(error: &matrix_sdk::HttpError) -> AccountNotificationsFailureKind {
    use matrix_sdk::ruma::api::error::ErrorKind;

    match error.client_api_error_kind() {
        Some(ErrorKind::ThreepidInUse) => return AccountNotificationsFailureKind::EmailInUse,
        Some(ErrorKind::ThreepidDenied) => return AccountNotificationsFailureKind::EmailDenied,
        Some(ErrorKind::ThreepidMediumNotSupported) | Some(ErrorKind::Unrecognized) => {
            return AccountNotificationsFailureKind::Unsupported;
        }
        Some(ErrorKind::ThreepidAuthFailed) => {
            return AccountNotificationsFailureKind::EmailNotVerified;
        }
        Some(ErrorKind::ThreepidNotFound) => {
            return AccountNotificationsFailureKind::EmailNotRegistered;
        }
        Some(ErrorKind::LimitExceeded(_)) => return AccountNotificationsFailureKind::RateLimited,
        Some(ErrorKind::Forbidden) => return AccountNotificationsFailureKind::AuthRejected,
        Some(ErrorKind::InvalidParam) | Some(ErrorKind::BadJson) => {
            return AccountNotificationsFailureKind::InvalidEmail;
        }
        _ => {}
    }
    match error {
        matrix_sdk::HttpError::Reqwest(_) => AccountNotificationsFailureKind::Network,
        matrix_sdk::HttpError::Cached(inner) => classify_http_error(inner),
        _ => {
            if error
                .as_client_api_error()
                .is_some_and(|error| matches!(error.status_code.as_u16(), 404 | 405))
            {
                AccountNotificationsFailureKind::Unsupported
            } else {
                AccountNotificationsFailureKind::Server
            }
        }
    }
}

fn classify_sdk_error(error: &matrix_sdk::Error) -> AccountNotificationsFailureKind {
    match error {
        matrix_sdk::Error::Http(http) => classify_http_error(http),
        _ => AccountNotificationsFailureKind::Server,
    }
}

// ── Server IO ───────────────────────────────────────────────────────────────

/// Fetch the authoritative push ruleset (bypasses the sync cache).
pub async fn fetch_push_ruleset(
    session: &MatrixClientSession,
) -> Result<Ruleset, AccountNotificationsFailureKind> {
    session
        .client()
        .send(get_pushrules_all::v3::Request::new())
        .await
        .map(|response| response.global)
        .map_err(|error| classify_http_error(&error))
}

async fn fetch_validated_emails(
    session: &MatrixClientSession,
) -> Result<Vec<String>, AccountNotificationsFailureKind> {
    let response = session
        .client()
        .account()
        .get_3pids()
        .await
        .map_err(|error| classify_sdk_error(&error))?;
    Ok(response
        .threepids
        .into_iter()
        .filter(|threepid| threepid.medium == Medium::Email)
        .map(|threepid| threepid.address)
        .collect())
}

/// Pushkeys of the account's `kind: email` pushers.
async fn fetch_email_pusher_ids(
    session: &MatrixClientSession,
) -> Result<Vec<PusherIds>, AccountNotificationsFailureKind> {
    let response = session
        .client()
        .send(get_pushers::v3::Request::new())
        .await
        .map_err(|error| classify_http_error(&error))?;
    Ok(response
        .pushers
        .into_iter()
        .filter(|pusher| matches!(pusher.kind, PusherKind::Email(_)))
        .map(|pusher| pusher.ids)
        .collect())
}

async fn email_management(session: &MatrixClientSession) -> NotificationEmailManagement {
    if session.info.authentication_method == koushi_state::SessionAuthenticationMethod::OAuth {
        return NotificationEmailManagement::DelegatedToAccountManagement;
    }
    // `m.3pid_changes` defaults to true when absent (spec); a failed
    // capabilities read keeps the standard flow available and lets the
    // actual request report the server's answer.
    let can_change = session
        .client()
        .homeserver_capabilities()
        .can_change_thirdparty_ids()
        .await
        .unwrap_or(true);
    if can_change {
        NotificationEmailManagement::Available
    } else {
        NotificationEmailManagement::Unsupported
    }
}

/// Read-only load. Performs GET requests only; never writes rules or pushers.
pub async fn load_account_notifications(
    session: &MatrixClientSession,
) -> Result<AccountNotificationsSnapshot, AccountNotificationsFailureKind> {
    let ruleset = fetch_push_ruleset(session).await?;
    let management = email_management(session).await;
    let emails = fetch_validated_emails(session).await?;
    let pushers = fetch_email_pusher_ids(session).await?;
    let pushkeys: Vec<String> = pushers.into_iter().map(|ids| ids.pushkey).collect();
    Ok(build_account_notifications_snapshot(
        &ruleset, management, &emails, &pushkeys,
    ))
}

async fn apply_rule_writes(
    session: &MatrixClientSession,
    writes: Vec<RuleWrite>,
) -> Result<(), AccountNotificationsFailureKind> {
    let client = session.client();
    for write in writes {
        let result = match write {
            RuleWrite::SetEnabled {
                kind,
                rule_id,
                enabled,
            } => client
                .send(set_pushrule_enabled::v3::Request::new(
                    kind, rule_id, enabled,
                ))
                .await
                .map(|_| ()),
            RuleWrite::SetActions {
                kind,
                rule_id,
                actions,
            } => client
                .send(set_pushrule_actions::v3::Request::new(
                    kind,
                    rule_id,
                    actions.to_ruma(),
                ))
                .await
                .map(|_| ()),
        };
        result.map_err(|error| classify_http_error(&error))?;
    }
    Ok(())
}

/// Apply a user toggle to one category against the current server ruleset.
pub async fn set_notification_category(
    session: &MatrixClientSession,
    category: NotificationCategory,
    enabled: bool,
) -> Result<(), AccountNotificationsFailureKind> {
    let ruleset = fetch_push_ruleset(session).await?;
    apply_rule_writes(session, plan_category_writes(&ruleset, category, enabled)).await
}

/// Apply the account-wide notification switch (`.m.rule.master`).
pub async fn set_account_push_enabled(
    session: &MatrixClientSession,
    enabled: bool,
) -> Result<(), AccountNotificationsFailureKind> {
    let ruleset = fetch_push_ruleset(session).await?;
    apply_rule_writes(session, plan_account_push_writes(&ruleset, enabled)).await
}

/// Ask the homeserver to send a verification email for `address`.
/// Returns the 3PID validation session id.
pub async fn request_notification_email_token(
    session: &MatrixClientSession,
    client_secret: &matrix_sdk::ruma::ClientSecret,
    address: &str,
    send_attempt: u32,
) -> Result<String, AccountNotificationsFailureKind> {
    session
        .client()
        .account()
        .request_3pid_email_token(client_secret, address, send_attempt.into())
        .await
        .map(|response| response.sid.to_string())
        .map_err(|error| classify_sdk_error(&error))
}

/// Bind a verified email to the account (`POST /account/3pid/add`).
pub async fn add_notification_email(
    session: &MatrixClientSession,
    client_secret: &matrix_sdk::ruma::ClientSecret,
    sid: &str,
    auth: Option<&IdentityResetAuthRequest>,
    uiaa_session: Option<&str>,
) -> Result<(), AddNotificationEmailError> {
    let sid = <&matrix_sdk::ruma::SessionId>::try_from(sid)
        .map_err(|_| AddNotificationEmailError::Failed(AccountNotificationsFailureKind::Server))?;
    let auth_data = crate::e2ee::account_management_auth_data(session, auth, uiaa_session);
    match session
        .client()
        .account()
        .add_3pid(client_secret, sid, auth_data)
        .await
    {
        Ok(_) => Ok(()),
        Err(error) => {
            if let Some(uiaa) = error.as_uiaa_response() {
                // A rejected password comes back as a fresh UIAA challenge
                // carrying an auth error; a not-yet-verified email may too.
                if let Some(auth_error) = &uiaa.auth_error {
                    use matrix_sdk::ruma::api::error::ErrorKind;
                    let kind = match auth_error.kind {
                        ErrorKind::ThreepidAuthFailed => {
                            AccountNotificationsFailureKind::EmailNotVerified
                        }
                        _ => AccountNotificationsFailureKind::AuthRejected,
                    };
                    return Err(AddNotificationEmailError::Failed(kind));
                }
                return Err(AddNotificationEmailError::UiaaChallenge {
                    session: uiaa.session.clone(),
                });
            }
            Err(AddNotificationEmailError::Failed(classify_sdk_error(
                &error,
            )))
        }
    }
}

/// Make `address` the single email notification target.
///
/// Order is add-then-remove so a failure never leaves the account without the
/// previous target; every other email pusher (including ones another client
/// created for a different address) is removed afterwards so the displayed
/// target always equals the active pusher set.
pub async fn set_email_notification_target(
    session: &MatrixClientSession,
    address: &str,
    lang: &str,
) -> Result<(), AccountNotificationsFailureKind> {
    let emails = fetch_validated_emails(session).await?;
    let Some(address) = emails
        .into_iter()
        .find(|candidate| candidate.eq_ignore_ascii_case(address))
    else {
        return Err(AccountNotificationsFailureKind::EmailNotRegistered);
    };
    let existing = fetch_email_pusher_ids(session).await?;
    let already_active = existing
        .iter()
        .any(|ids| ids.app_id == EMAIL_PUSHER_APP_ID && ids.pushkey.eq_ignore_ascii_case(&address));
    if !already_active {
        let mut data = EmailPusherData::new();
        data.data.insert(
            "brand".to_owned(),
            serde_json::Value::String(EMAIL_PUSHER_BRAND.to_owned()),
        );
        let pusher = PusherInit {
            ids: PusherIds::new(address.clone(), EMAIL_PUSHER_APP_ID.to_owned()),
            kind: PusherKind::Email(data),
            app_display_name: EMAIL_PUSHER_APP_DISPLAY_NAME.to_owned(),
            device_display_name: address.clone(),
            profile_tag: None,
            lang: lang.to_owned(),
        };
        session
            .client()
            .pusher()
            .set(pusher.into(), true)
            .await
            .map_err(|error| classify_sdk_error(&error))?;
    }
    for ids in existing {
        if ids.app_id == EMAIL_PUSHER_APP_ID && ids.pushkey.eq_ignore_ascii_case(&address) {
            continue;
        }
        session
            .client()
            .pusher()
            .delete(ids)
            .await
            .map_err(|error| classify_sdk_error(&error))?;
    }
    Ok(())
}

/// Remove every email pusher on the account.
pub async fn disable_email_notifications(
    session: &MatrixClientSession,
) -> Result<(), AccountNotificationsFailureKind> {
    for ids in fetch_email_pusher_ids(session).await? {
        session
            .client()
            .pusher()
            .delete(ids)
            .await
            .map_err(|error| classify_sdk_error(&error))?;
    }
    Ok(())
}

/// Whether any email pusher is currently active (used to carry an active
/// target over to a newly verified address during "Change").
pub async fn email_notifications_active(
    session: &MatrixClientSession,
) -> Result<bool, AccountNotificationsFailureKind> {
    Ok(!fetch_email_pusher_ids(session).await?.is_empty())
}

#[cfg(test)]
mod tests;
