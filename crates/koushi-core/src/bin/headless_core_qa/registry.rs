use super::Duration;

const ENV_HOMESERVER: &str = "KOUSHI_LOCAL_QA_HOMESERVER";

const ENV_SERVER_NAME: &str = "KOUSHI_LOCAL_QA_SERVER_NAME";

const ENV_SERVER_KIND: &str = "KOUSHI_LOCAL_QA_SERVER_KIND";

const ENV_USER_A: &str = "KOUSHI_LOCAL_QA_USER_A";

const ENV_PASSWORD_A: &str = "KOUSHI_LOCAL_QA_PASSWORD_A";

const ENV_USER_B: &str = "KOUSHI_LOCAL_QA_USER_B";

const ENV_PASSWORD_B: &str = "KOUSHI_LOCAL_QA_PASSWORD_B";

const ENV_USER_C: &str = "KOUSHI_LOCAL_QA_USER_C";

const ENV_QA_SCENARIO: &str = "KOUSHI_QA_SCENARIO";

const ENV_ALLOW_IDENTITY_RESET: &str = "KOUSHI_QA_ALLOW_IDENTITY_RESET";

pub(super) const ENV_E2EE_RECIPIENT_SECOND_DEVICE: &str = "KOUSHI_QA_E2EE_RECIPIENT_SECOND_DEVICE";

#[cfg(any(debug_assertions, feature = "qa-bin"))]
pub(super) const ENV_FILE_CREDENTIAL_STORE_DIR: &str = "KOUSHI_QA_FILE_CREDENTIAL_STORE_DIR";

pub(super) const DEVICE_A: &str = "Koushi Core QA A";

pub(super) const DEVICE_B: &str = "Koushi Core QA B";

/// Maximum time to wait for a single event.
pub(super) const EVENT_TIMEOUT: Duration = Duration::from_secs(30);

pub(super) const GATE_RESTORE_READY_BUDGET: Duration = Duration::from_secs(10);

pub(super) const LOGIN_EVENT_TIMEOUT: Duration = Duration::from_secs(180);

pub(super) const ROOM_LIST_EVENT_TIMEOUT: Duration = Duration::from_secs(90);

pub(super) const TIMELINE_INITIAL_EVENT_TIMEOUT: Duration = Duration::from_secs(90);

pub(super) const E2EE_EVENT_TIMEOUT: Duration = Duration::from_secs(90);

pub(super) const SEND_QUEUE_EVENT_TIMEOUT: Duration = Duration::from_secs(300);

pub(super) const TIMELINE_UNSUBSCRIBE_SETTLE_TIMEOUT: Duration = Duration::from_secs(2);

pub(super) const TIMELINE_RECONNECT_EXPECTED_BODY_COUNT: usize = 21;

pub(super) const TIMELINE_RECONNECT_MIN_INITIAL_BODIES: usize = 20;

pub(super) const TIMELINE_RECONNECT_PAGINATE_EVENT_COUNT: u16 = 64;

pub(super) const THREAD_REPLY_BODY: &str = "Phase 11 QA thread reply from B";

pub(super) const E2EE_KEY_BACKUP_SEED_BODY: &str = "Koushi E2EE key backup seed";

pub(super) const E2EE_SECOND_DEVICE_BODY: &str = "Koushi E2EE second-device delivery";

pub(super) const E2EE_MULTI_USER_MULTI_DEVICE_BODY: &str =
    "Koushi E2EE multi-user multi-device delivery";

const DEFAULT_STRESS_SPACE_COUNT: usize = 2;

const DEFAULT_STRESS_ROOMS_PER_SPACE: usize = 2;

const DEFAULT_STRESS_MESSAGES_PER_ROOM: usize = 8;

const MAX_STRESS_SPACE_COUNT: usize = 6;

const MAX_STRESS_ROOMS_PER_SPACE: usize = 8;

const MAX_STRESS_MESSAGES_PER_ROOM: usize = 80;

const ENV_STRESS_SPACE_COUNT: &str = "KOUSHI_QA_STRESS_SPACES";

const ENV_STRESS_ROOMS_PER_SPACE: &str = "KOUSHI_QA_STRESS_ROOMS_PER_SPACE";

const ENV_STRESS_MESSAGES_PER_ROOM: &str = "KOUSHI_QA_STRESS_MESSAGES_PER_ROOM";

const ENV_STRESS_REPLAY_EXISTING: &str = "KOUSHI_QA_STRESS_REPLAY_EXISTING";

pub(super) const QA_WRONG_RECOVERY_SECRET: &str =
    "koushi-desktop-headless-qa-wrong-recovery-secret";

pub(super) const ENV_CACHE_RESTORE_ROOMS: &str = "KOUSHI_QA_CACHE_RESTORE_ROOMS";

pub(super) const ENV_CACHE_RESTORE_DEPTH: &str = "KOUSHI_QA_CACHE_RESTORE_DEPTH";

pub(super) const DEFAULT_CACHE_RESTORE_ROOMS: usize = 3;

pub(super) const DEFAULT_CACHE_RESTORE_DEPTH: usize = 200;

/// Batch size used for backward pagination during the populate (EndReached) pass.
pub(super) const CACHE_RESTORE_PAGINATE_BATCH: u16 = 20;

/// Production-faithful restore parameters, matching the app's live-room constants.
/// Source: apps/desktop/src/components/TimelineView.tsx:406-407
/// (LIVE_ROOM_ANCHOR_RESTORE_MAX_BATCHES=6, EVENT_COUNT=100).
/// These are intentionally small. Room entry should fail fast for stale or
/// very deep persisted anchors and let the UI fall back to live edge; deep
/// event-centered restore belongs to an explicit focused-event timeline.
pub(super) const CACHE_RESTORE_PROD_MAX_BATCHES: u16 = 6;

pub(super) const CACHE_RESTORE_PROD_EVENT_COUNT: u16 = 100;

/// Speed gate: maximum backward-paginate cycles allowed per room during an
/// offline anchor restore. Deep anchors may end as BudgetExhausted, but they
/// must not walk history long enough to block room entry.
pub(super) const CACHE_RESTORE_MAX_CYCLES: u16 = 3;

/// Number of messages in the shallow-anchor room.  Enough to exceed the SDK's
/// initial visible window (~20 items) so that m0 (oldest) is hidden behind a
/// lazy-reveal skip when the session restarts.  All events fit in a single
/// stored chunk (well under 128), so chunks_loaded == 0 during the restore.
/// The anchor (m0) lives in the live in-memory prefix that
/// live_lazy_paginate_backwards reveals (lazy_reveal_batches == 1).
/// The P1 lazy-reveal-fence fix gates on this: without it the settle fence
/// misses the lazy-reveal DiffBatch and may conclude before items settle.
pub(super) const CACHE_RESTORE_SHALLOW_DEPTH: usize = 30;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum QaScenario {
    All,
    Safety,
    LoginSync,
    SessionStatus,
    CredentialHealth,
    NativeAttention,
    EncryptionDebug,
    E2eeTrust,
    E2eeLoginStore,
    DeviceCleanup,
    GateRestore,
    GateNegative,
    GateNoProof,
    InvitesDm,
    RoomSpace,
    Directory,
    RoomManagement,
    RoomPeopleProjection,
    Timeline,
    TimelineReconnect,
    TimelineStress,
    Activity,
    Composer,
    Reply,
    Media,
    LiveSignals,
    Thread,
    EditRedactSearch,
    RedactEditConvergence,
    SearchCrawler,
    ScheduledSend,
    SendQueue,
    RestoreCleanup,
    LinkPreview,
    CacheRestore,
    ReadStateConvergence,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum QaStage {
    Safety,
    LoginSync,
    SessionStatus,
    CredentialHealth,
    NativeAttention,
    EncryptionDebug,
    E2eeTrust,
    E2eeLoginStore,
    DeviceCleanup,
    GateRestore,
    GateNegative,
    GateNoProof,
    InvitesDm,
    RoomSpace,
    Directory,
    RoomManagement,
    RoomPeopleProjection,
    Timeline,
    TimelineReconnect,
    TimelineStress,
    Activity,
    Composer,
    Reply,
    Media,
    LiveSignals,
    Thread,
    EditRedactSearch,
    RedactEditConvergence,
    SearchCrawler,
    ScheduledSend,
    SendQueue,
    RestoreCleanup,
    LinkPreview,
    CacheRestore,
    ReadStateConvergence,
}

/// Refuse to run against the OS keychain. Debug and qa-bin release builds both
/// check the env var and the structurally resolved backend before any login.
pub(super) fn assert_file_credential_store_active() -> Result<(), String> {
    #[cfg(any(debug_assertions, feature = "qa-bin"))]
    {
        if std::env::var_os(ENV_FILE_CREDENTIAL_STORE_DIR).is_none() {
            return Err(format!(
                "core QA refuses to run against the OS keychain: {ENV_FILE_CREDENTIAL_STORE_DIR} is not set"
            ));
        }
        if !koushi_core::store::resolved_credential_backend_is_file_dir() {
            return Err(
                "core QA refuses to run against the OS keychain: resolved credential \
                 store backend is not the file-dir backend"
                    .to_owned(),
            );
        }
        Ok(())
    }

    #[cfg(not(any(debug_assertions, feature = "qa-bin")))]
    {
        Err(
            "core QA refuses to run against the OS keychain: release builds have no \
             file credential store backend"
                .to_owned(),
        )
    }
}

impl QaScenario {
    pub(super) fn from_env() -> Result<Self, String> {
        match std::env::var(ENV_QA_SCENARIO) {
            Ok(value) => Self::from_env_value(&value),
            Err(_) => Ok(Self::All),
        }
    }

    pub(super) fn from_env_value(value: &str) -> Result<Self, String> {
        match value {
            "all" => Ok(Self::All),
            "safety" => Ok(Self::Safety),
            "login_sync" => Ok(Self::LoginSync),
            "session_status" => Ok(Self::SessionStatus),
            "credential_health" => Ok(Self::CredentialHealth),
            "native_attention" => Ok(Self::NativeAttention),
            "encryption_debug" => Ok(Self::EncryptionDebug),
            "e2ee_trust" => Ok(Self::E2eeTrust),
            "e2ee_login_store" => Ok(Self::E2eeLoginStore),
            "device_cleanup" => Ok(Self::DeviceCleanup),
            "gate_restore" => Ok(Self::GateRestore),
            "gate_negative" => Ok(Self::GateNegative),
            "gate_no_proof" => Ok(Self::GateNoProof),
            "invites_dm" => Ok(Self::InvitesDm),
            "room_space" => Ok(Self::RoomSpace),
            "directory" => Ok(Self::Directory),
            "room_management" => Ok(Self::RoomManagement),
            "room_people_projection" => Ok(Self::RoomPeopleProjection),
            "timeline" => Ok(Self::Timeline),
            "timeline_reconnect" => Ok(Self::TimelineReconnect),
            "timeline_stress" => Ok(Self::TimelineStress),
            "activity" => Ok(Self::Activity),
            "composer" => Ok(Self::Composer),
            "reply" => Ok(Self::Reply),
            "media" => Ok(Self::Media),
            "live_signals" => Ok(Self::LiveSignals),
            "thread" => Ok(Self::Thread),
            "edit_redact_search" => Ok(Self::EditRedactSearch),
            "redact_edit_convergence" => Ok(Self::RedactEditConvergence),
            "search_crawler" => Ok(Self::SearchCrawler),
            "scheduled_send" => Ok(Self::ScheduledSend),
            "send_queue" => Ok(Self::SendQueue),
            "restore_cleanup" => Ok(Self::RestoreCleanup),
            "link_preview" => Ok(Self::LinkPreview),
            "cache_restore" => Ok(Self::CacheRestore),
            "read_state_convergence" => Ok(Self::ReadStateConvergence),
            other => Err(format!(
                "{ENV_QA_SCENARIO} must be one of all, safety, login_sync, session_status, credential_health, native_attention, encryption_debug, e2ee_trust, e2ee_login_store, device_cleanup, invites_dm, room_space, directory, room_management, room_people_projection, timeline, timeline_reconnect, timeline_stress, activity, composer, reply, media, live_signals, thread, edit_redact_search, redact_edit_convergence, search_crawler, scheduled_send, restore_cleanup, link_preview, cache_restore, read_state_convergence; got {other}"
            )),
        }
    }

    pub(super) fn should_run_stage(self, stage: QaStage) -> bool {
        match self {
            Self::All => !matches!(
                stage,
                QaStage::TimelineReconnect
                    | QaStage::TimelineStress
                    | QaStage::DeviceCleanup
                    | QaStage::ReadStateConvergence
            ),
            Self::Safety => matches!(stage, QaStage::Safety),
            Self::LoginSync => matches!(stage, QaStage::Safety | QaStage::LoginSync),
            Self::SessionStatus => matches!(
                stage,
                QaStage::Safety | QaStage::LoginSync | QaStage::SessionStatus
            ),
            Self::CredentialHealth => matches!(
                stage,
                QaStage::Safety | QaStage::LoginSync | QaStage::CredentialHealth
            ),
            Self::NativeAttention => matches!(
                stage,
                QaStage::Safety | QaStage::LoginSync | QaStage::NativeAttention
            ),
            Self::EncryptionDebug => matches!(
                stage,
                QaStage::Safety
                    | QaStage::LoginSync
                    | QaStage::RoomSpace
                    | QaStage::EncryptionDebug
            ),
            Self::E2eeTrust => {
                matches!(
                    stage,
                    QaStage::Safety | QaStage::LoginSync | QaStage::E2eeTrust
                )
            }
            Self::E2eeLoginStore => matches!(
                stage,
                QaStage::Safety | QaStage::LoginSync | QaStage::E2eeLoginStore
            ),
            Self::DeviceCleanup => matches!(
                stage,
                QaStage::Safety | QaStage::LoginSync | QaStage::DeviceCleanup
            ),
            Self::GateRestore => matches!(
                stage,
                QaStage::Safety | QaStage::LoginSync | QaStage::GateRestore
            ),
            Self::GateNegative => matches!(
                stage,
                QaStage::Safety | QaStage::LoginSync | QaStage::GateNegative
            ),
            Self::GateNoProof => matches!(stage, QaStage::Safety | QaStage::GateNoProof),
            Self::InvitesDm => matches!(
                stage,
                QaStage::Safety | QaStage::LoginSync | QaStage::InvitesDm
            ),
            Self::RoomSpace => matches!(
                stage,
                QaStage::Safety | QaStage::LoginSync | QaStage::RoomSpace
            ),
            Self::Directory => matches!(
                stage,
                QaStage::Safety | QaStage::LoginSync | QaStage::Directory
            ),
            Self::RoomManagement => matches!(
                stage,
                QaStage::Safety | QaStage::LoginSync | QaStage::RoomSpace | QaStage::RoomManagement
            ),
            Self::RoomPeopleProjection => matches!(
                stage,
                QaStage::Safety
                    | QaStage::LoginSync
                    | QaStage::RoomSpace
                    | QaStage::RoomPeopleProjection
            ),
            Self::Timeline => matches!(
                stage,
                QaStage::Safety | QaStage::LoginSync | QaStage::RoomSpace | QaStage::Timeline
            ),
            Self::TimelineReconnect => {
                matches!(stage, QaStage::Safety | QaStage::TimelineReconnect)
            }
            Self::TimelineStress => matches!(
                stage,
                QaStage::Safety
                    | QaStage::LoginSync
                    | QaStage::RoomSpace
                    | QaStage::Timeline
                    | QaStage::TimelineStress
            ),
            Self::Activity => matches!(
                stage,
                QaStage::Safety
                    | QaStage::LoginSync
                    | QaStage::RoomSpace
                    | QaStage::Timeline
                    | QaStage::Activity
            ),
            Self::Composer => matches!(
                stage,
                QaStage::Safety
                    | QaStage::LoginSync
                    | QaStage::RoomSpace
                    | QaStage::Timeline
                    | QaStage::Composer
            ),
            Self::Reply => matches!(
                stage,
                QaStage::Safety
                    | QaStage::LoginSync
                    | QaStage::RoomSpace
                    | QaStage::Timeline
                    | QaStage::Composer
                    | QaStage::Reply
            ),
            Self::Media => matches!(
                stage,
                QaStage::Safety
                    | QaStage::LoginSync
                    | QaStage::RoomSpace
                    | QaStage::Timeline
                    | QaStage::Media
            ),
            Self::LiveSignals => matches!(
                stage,
                QaStage::Safety
                    | QaStage::LoginSync
                    | QaStage::RoomSpace
                    | QaStage::Timeline
                    | QaStage::LiveSignals
            ),
            Self::Thread => matches!(
                stage,
                QaStage::Safety
                    | QaStage::LoginSync
                    | QaStage::RoomSpace
                    | QaStage::Timeline
                    | QaStage::Reply
                    | QaStage::Thread
            ),
            Self::EditRedactSearch => matches!(
                stage,
                QaStage::Safety
                    | QaStage::LoginSync
                    | QaStage::RoomSpace
                    | QaStage::Timeline
                    | QaStage::EditRedactSearch
            ),
            Self::RedactEditConvergence => matches!(
                stage,
                QaStage::Safety
                    | QaStage::LoginSync
                    | QaStage::RoomSpace
                    | QaStage::Timeline
                    | QaStage::Thread
                    | QaStage::EditRedactSearch
                    | QaStage::RedactEditConvergence
            ),
            Self::SearchCrawler => matches!(
                stage,
                QaStage::Safety
                    | QaStage::LoginSync
                    | QaStage::RoomSpace
                    | QaStage::Timeline
                    | QaStage::EditRedactSearch
                    | QaStage::SearchCrawler
            ),
            Self::ScheduledSend => matches!(
                stage,
                QaStage::Safety
                    | QaStage::LoginSync
                    | QaStage::RoomSpace
                    | QaStage::Timeline
                    | QaStage::ScheduledSend
            ),
            Self::SendQueue => matches!(
                stage,
                QaStage::Safety | QaStage::LoginSync | QaStage::SendQueue
            ),
            Self::RestoreCleanup => matches!(
                stage,
                QaStage::Safety
                    | QaStage::LoginSync
                    | QaStage::RoomSpace
                    | QaStage::Timeline
                    | QaStage::EditRedactSearch
                    | QaStage::RestoreCleanup
            ),
            Self::LinkPreview => matches!(
                stage,
                QaStage::Safety
                    | QaStage::LoginSync
                    | QaStage::RoomSpace
                    | QaStage::Timeline
                    | QaStage::Composer
                    | QaStage::LinkPreview
            ),
            Self::CacheRestore => matches!(stage, QaStage::Safety | QaStage::CacheRestore),
            Self::ReadStateConvergence => {
                matches!(stage, QaStage::Safety | QaStage::ReadStateConvergence)
            }
        }
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub(super) fn suppress_matrix_identifiers(self) -> bool {
        let _ = self;
        true
    }
}

pub(super) fn scenario_preflight_error(scenario: QaScenario) -> Result<(), String> {
    let _ = scenario;
    Ok(())
}

pub(super) fn tokens_for_stage(stage: QaStage) -> &'static [&'static str] {
    match stage {
        QaStage::Safety => &["safety=ok"],
        QaStage::LoginSync => &["login_sync=ok"],
        QaStage::SessionStatus => &[
            "session_status_checking=ok",
            "session_status_ready=ok",
            "session_status_device=ok",
            "session_status=ok",
        ],
        QaStage::CredentialHealth => &["credential_health=ok", "fail_closed=ok"],
        QaStage::NativeAttention => &[
            "notification_candidate=ok",
            "badge_state=ok",
            "suppress_focus=ok",
            "clear_badge=ok",
        ],
        QaStage::EncryptionDebug => &[
            "encryption_debug_cross_signing=ok",
            "encryption_debug_room=ok",
            "encryption_debug_recipient=ok",
            "force_new_outbound_session=ok",
            "share_index0_room_key=ok",
            "index0_not_consumed=ok",
            "encryption_debug_index_advanced=ok",
            "resend_index0_room_key=ok",
            "resend_index_unchanged=ok",
            "encryption_debug=ok",
        ],
        QaStage::E2eeTrust => &[
            "joined_room_restore=ok",
            "e2ee_second_device_decrypt=ok",
            "e2ee_multi_user_multi_device_decrypt=ok",
            "e2ee_unverified_peer_send_nonblocking=ok",
            "e2ee_blocked_device_withheld=ok",
            "e2ee_trust=ok",
        ],
        QaStage::E2eeLoginStore => &[
            "e2ee_login_store_fresh_offline_index0=ok",
            "e2ee_login_store_restore_offline_index0=ok",
            "e2ee_login_store_restart_offline_index0=ok",
            "e2ee_login_store_reauth_offline_index0=ok",
            "e2ee_login_store_online_index0=ok",
            "e2ee_login_store_group_index0=ok",
            "e2ee_login_store_identity_stable=ok",
            "e2ee_login_store=ok",
        ],
        QaStage::DeviceCleanup => &[
            "device_cleanup_remote_first=ok",
            "device_cleanup_relogin_new_device=ok",
        ],
        QaStage::GateRestore => &[
            "gate_restore_bootstrapped=ok",
            "gate_restore_shutdown_complete=ok",
            "gate_restore_runtime_spawned=ok",
            "gate_restore_query_sent=ok",
            "gate_restore_query_result=ok",
            "gate_restore_restore_sent=ok",
            "gate_restore_restore_result=ok",
            "gate_restore_ready=ok",
            "gate_verified_restore=ok",
        ],
        QaStage::GateNegative => &[
            "gate_sas_mismatch_retryable=ok",
            "gate_sas_retry_ready=ok",
            "gate_sas_user_cancel_retryable=ok",
            "gate_sas_user_cancel_retry_ready=ok",
            "gate_sas_timeout_retryable=ok",
            "gate_sas_timeout_retry_ready=ok",
            "gate_recovery_invalid_retryable=ok",
            "gate_recovery_retry_ready=ok",
            "gate_recovery_cancel_retryable=ok",
            "gate_recovery_cancel_retry_ready=ok",
            "gate_trust_loss_locked=ok",
            "gate_trust_loss_commands_blocked=ok",
        ],
        QaStage::GateNoProof => &[
            "gate_no_proof_rejected=ok",
            "gate_no_proof_restart_signed_out=ok",
        ],
        QaStage::InvitesDm => &[
            "invite_recv=ok",
            "invite_accept=ok",
            "invite_decline=ok",
            "member_list=ok",
            "dm_start=ok",
            "dm_space_scope=ok",
        ],
        QaStage::RoomSpace => &["room_space=ok"],
        QaStage::Directory => &["directory_query=ok", "directory_join=ok"],
        QaStage::RoomManagement => &["room_settings=ok", "moderation=ok", "permission_guard=ok"],
        QaStage::RoomPeopleProjection => &[
            "room_people_joined_scope=ok",
            "room_people_alias_search=ok",
            "room_people_surface_isolation=ok",
            "room_people_membership_refresh=ok",
            "room_people_mentions_content=ok",
            "room_people_projection=ok",
        ],
        QaStage::Timeline => &["timeline=ok", "timeline_nav=ok", "hide_redacted=ok"],
        QaStage::TimelineReconnect => &[
            "timeline_reconnect_recv_after_reconnect=ok",
            "live_catchup_checkpoint=ok",
            "live_catchup_gap_repaired=ok",
            "timeline_reconnect=ok",
        ],
        QaStage::TimelineStress => &[
            "timeline_stress=ok",
            "stress_no_blank=ok",
            "stress_space_scope=ok",
        ],
        QaStage::Activity => &[
            "activity_recent=ok",
            "activity_unread=ok",
            "activity_resolution=ok",
            "activity_markread=ok",
        ],
        QaStage::Composer => &[
            "mention_send=ok",
            "markdown_send=ok",
            "slash_command=ok",
            "ime_guard=ok",
        ],
        QaStage::Reply => &[
            "reply=ok",
            "reply_quote=ok",
            "pin_event=ok",
            "pinned_state=ok",
            "unpin_event=ok",
        ],
        QaStage::Media => &[
            "send_media=ok",
            "media_caption=ok",
            "image_compress=ok",
            "upload_staging=ok",
            "media_gallery=ok",
            "recv_media=ok",
            "media_caption_edit=ok",
        ],
        QaStage::LiveSignals => &[
            "read_receipt=ok",
            "fully_read=ok",
            "typing=ok",
            "presence=ok",
            "live_signals=ok",
        ],
        QaStage::Thread => &[
            "thread_canonical=ok",
            "thread_summary=ok",
            "thread_recv=ok",
            "thread_paginate=end_reached",
        ],
        QaStage::EditRedactSearch => &["edit_redact_search=ok"],
        QaStage::RedactEditConvergence => &[
            "redact_edit_convergence=ok",
            "thread_summary_convergence=ok",
        ],
        QaStage::SearchCrawler => &[
            "crawl_backfill=ok",
            "crawl_no_media_bytes=ok",
            "crawl_throttle=ok",
            "crawl_failure=ok",
        ],
        QaStage::ScheduledSend => &[
            "scheduled_capability=local_fallback",
            "scheduled_create=ok",
            "scheduled_reschedule=ok",
            "scheduled_cancel=ok",
            "scheduled_fire=ok",
        ],
        QaStage::SendQueue => &[
            "send_fail=ok",
            "resend=ok",
            "cancel_send=ok",
            "fifo=ok",
            "unsent_restart=ok",
            "display_projection_reset_fallbacks=0",
        ],
        QaStage::RestoreCleanup => &["restore_cleanup=ok"],
        QaStage::LinkPreview => &[
            "link_preview_global=ok",
            "link_preview_room=ok",
            "link_preview_e2ee_default=ok",
            "link_preview_hide=ok",
        ],
        QaStage::CacheRestore => &["cache_restore=ok"],
        QaStage::ReadStateConvergence => &["read_state_convergence=ok"],
    }
}

fn implemented_final_tokens() -> Vec<&'static str> {
    vec![
        "safety=ok",
        "login_sync=ok",
        "session_status_checking=ok",
        "session_status_ready=ok",
        "session_status_device=ok",
        "session_status=ok",
        "credential_health=ok",
        "fail_closed=ok",
        "notification_candidate=ok",
        "badge_state=ok",
        "suppress_focus=ok",
        "clear_badge=ok",
        "invite_recv=ok",
        "invite_accept=ok",
        "invite_decline=ok",
        "member_list=ok",
        "dm_start=ok",
        "dm_space_scope=ok",
        "room_space=ok",
        "directory_query=ok",
        "directory_join=ok",
        "room_settings=ok",
        "moderation=ok",
        "permission_guard=ok",
        "timeline=ok",
        "timeline_nav=ok",
        "hide_redacted=ok",
        "activity_recent=ok",
        "activity_unread=ok",
        "activity_resolution=ok",
        "activity_markread=ok",
        "mention_send=ok",
        "markdown_send=ok",
        "slash_command=ok",
        "ime_guard=ok",
        "reply=ok",
        "reply_quote=ok",
        "pin_event=ok",
        "pinned_state=ok",
        "unpin_event=ok",
        "thread_canonical=ok",
        "thread_summary=ok",
        "thread_recv=ok",
        "thread_paginate=end_reached",
        "send_media=ok",
        "media_caption=ok",
        "image_compress=ok",
        "upload_staging=ok",
        "media_gallery=ok",
        "recv_media=ok",
        "media_caption_edit=ok",
        "read_receipt=ok",
        "fully_read=ok",
        "typing=ok",
        "presence=ok",
        "live_signals=ok",
        "edit_redact_search=ok",
        "crawl_backfill=ok",
        "crawl_no_media_bytes=ok",
        "crawl_throttle=ok",
        "crawl_failure=ok",
        "scheduled_capability=local_fallback",
        "scheduled_create=ok",
        "scheduled_reschedule=ok",
        "scheduled_cancel=ok",
        "scheduled_fire=ok",
        "send_fail=ok",
        "resend=ok",
        "cancel_send=ok",
        "fifo=ok",
        "unsent_restart=ok",
        "display_projection_reset_fallbacks=0",
        "joined_room_restore=ok",
        "e2ee_second_device_decrypt=ok",
        "e2ee_multi_user_multi_device_decrypt=ok",
        "e2ee_unverified_peer_send_nonblocking=ok",
        "e2ee_blocked_device_withheld=ok",
        "e2ee_trust=ok",
        "restore_cleanup=ok",
        "link_preview_global=ok",
        "link_preview_room=ok",
        "link_preview_e2ee_default=ok",
        "link_preview_hide=ok",
    ]
}

pub(super) fn stages_for_scenario(scenario: QaScenario) -> Vec<QaStage> {
    match scenario {
        QaScenario::Safety => vec![QaStage::Safety],
        QaScenario::LoginSync => vec![QaStage::Safety, QaStage::LoginSync],
        QaScenario::SessionStatus => {
            vec![QaStage::Safety, QaStage::LoginSync, QaStage::SessionStatus]
        }
        QaScenario::CredentialHealth => vec![
            QaStage::Safety,
            QaStage::LoginSync,
            QaStage::CredentialHealth,
        ],
        QaScenario::NativeAttention => vec![
            QaStage::Safety,
            QaStage::LoginSync,
            QaStage::NativeAttention,
        ],
        QaScenario::EncryptionDebug => vec![
            QaStage::Safety,
            QaStage::LoginSync,
            QaStage::RoomSpace,
            QaStage::EncryptionDebug,
        ],
        QaScenario::E2eeTrust => {
            vec![QaStage::Safety, QaStage::LoginSync, QaStage::E2eeTrust]
        }
        QaScenario::E2eeLoginStore => {
            vec![QaStage::Safety, QaStage::LoginSync, QaStage::E2eeLoginStore]
        }
        QaScenario::DeviceCleanup => {
            vec![QaStage::Safety, QaStage::LoginSync, QaStage::DeviceCleanup]
        }
        QaScenario::GateRestore => vec![QaStage::Safety, QaStage::LoginSync, QaStage::GateRestore],
        QaScenario::GateNegative => {
            vec![QaStage::Safety, QaStage::LoginSync, QaStage::GateNegative]
        }
        QaScenario::GateNoProof => vec![QaStage::Safety, QaStage::GateNoProof],
        QaScenario::InvitesDm => {
            vec![QaStage::Safety, QaStage::LoginSync, QaStage::InvitesDm]
        }
        QaScenario::RoomSpace => vec![QaStage::Safety, QaStage::LoginSync, QaStage::RoomSpace],
        QaScenario::Directory => vec![QaStage::Safety, QaStage::LoginSync, QaStage::Directory],
        QaScenario::RoomManagement => vec![
            QaStage::Safety,
            QaStage::LoginSync,
            QaStage::RoomSpace,
            QaStage::RoomManagement,
        ],
        QaScenario::RoomPeopleProjection => vec![
            QaStage::Safety,
            QaStage::LoginSync,
            QaStage::RoomSpace,
            QaStage::RoomPeopleProjection,
        ],
        QaScenario::Timeline => vec![
            QaStage::Safety,
            QaStage::LoginSync,
            QaStage::RoomSpace,
            QaStage::Timeline,
        ],
        QaScenario::TimelineReconnect => vec![QaStage::Safety, QaStage::TimelineReconnect],
        QaScenario::TimelineStress => vec![
            QaStage::Safety,
            QaStage::LoginSync,
            QaStage::RoomSpace,
            QaStage::Timeline,
            QaStage::TimelineStress,
        ],
        QaScenario::Activity => vec![
            QaStage::Safety,
            QaStage::LoginSync,
            QaStage::RoomSpace,
            QaStage::Timeline,
            QaStage::Activity,
        ],
        QaScenario::Composer => vec![
            QaStage::Safety,
            QaStage::LoginSync,
            QaStage::RoomSpace,
            QaStage::Timeline,
            QaStage::Composer,
        ],
        QaScenario::Reply => vec![
            QaStage::Safety,
            QaStage::LoginSync,
            QaStage::RoomSpace,
            QaStage::Timeline,
            QaStage::Composer,
            QaStage::Reply,
        ],
        QaScenario::Media => vec![
            QaStage::Safety,
            QaStage::LoginSync,
            QaStage::RoomSpace,
            QaStage::Timeline,
            QaStage::Media,
        ],
        QaScenario::LiveSignals => vec![
            QaStage::Safety,
            QaStage::LoginSync,
            QaStage::RoomSpace,
            QaStage::Timeline,
            QaStage::LiveSignals,
        ],
        QaScenario::Thread => vec![
            QaStage::Safety,
            QaStage::LoginSync,
            QaStage::RoomSpace,
            QaStage::Timeline,
            QaStage::Reply,
            QaStage::Thread,
        ],
        QaScenario::EditRedactSearch => vec![
            QaStage::Safety,
            QaStage::LoginSync,
            QaStage::RoomSpace,
            QaStage::Timeline,
            QaStage::EditRedactSearch,
        ],
        QaScenario::RedactEditConvergence => vec![
            QaStage::Safety,
            QaStage::LoginSync,
            QaStage::RoomSpace,
            QaStage::Timeline,
            QaStage::Thread,
            QaStage::EditRedactSearch,
            QaStage::RedactEditConvergence,
        ],
        QaScenario::SearchCrawler => vec![
            QaStage::Safety,
            QaStage::LoginSync,
            QaStage::RoomSpace,
            QaStage::Timeline,
            QaStage::EditRedactSearch,
            QaStage::SearchCrawler,
        ],
        QaScenario::ScheduledSend => vec![
            QaStage::Safety,
            QaStage::LoginSync,
            QaStage::RoomSpace,
            QaStage::Timeline,
            QaStage::ScheduledSend,
        ],
        QaScenario::SendQueue => vec![QaStage::Safety, QaStage::LoginSync, QaStage::SendQueue],
        QaScenario::RestoreCleanup => vec![
            QaStage::Safety,
            QaStage::LoginSync,
            QaStage::RoomSpace,
            QaStage::Timeline,
            QaStage::EditRedactSearch,
            QaStage::RestoreCleanup,
        ],
        QaScenario::LinkPreview => vec![
            QaStage::Safety,
            QaStage::LoginSync,
            QaStage::RoomSpace,
            QaStage::Timeline,
            QaStage::Composer,
            QaStage::LinkPreview,
        ],
        QaScenario::CacheRestore => vec![QaStage::Safety, QaStage::CacheRestore],
        QaScenario::ReadStateConvergence => {
            vec![QaStage::Safety, QaStage::ReadStateConvergence]
        }
        QaScenario::All => vec![
            QaStage::Safety,
            QaStage::LoginSync,
            QaStage::SessionStatus,
            QaStage::CredentialHealth,
            QaStage::NativeAttention,
            QaStage::InvitesDm,
            QaStage::RoomSpace,
            QaStage::Directory,
            QaStage::RoomManagement,
            QaStage::RoomPeopleProjection,
            QaStage::Timeline,
            QaStage::Activity,
            QaStage::Composer,
            QaStage::Reply,
            QaStage::Media,
            QaStage::LiveSignals,
            QaStage::Thread,
            QaStage::EditRedactSearch,
            QaStage::SearchCrawler,
            QaStage::ScheduledSend,
            QaStage::SendQueue,
            QaStage::E2eeTrust,
            QaStage::RestoreCleanup,
            QaStage::LinkPreview,
        ],
    }
}

pub(super) fn final_tokens_for_scenario(scenario: QaScenario) -> Vec<&'static str> {
    match scenario {
        QaScenario::Safety => vec!["safety=ok"],
        QaScenario::LoginSync => {
            let mut tokens = stages_for_scenario(scenario)
                .into_iter()
                .flat_map(|stage| tokens_for_stage(stage).iter().copied())
                .collect::<Vec<_>>();
            tokens.push("restore_cleanup=ok");
            tokens.dedup();
            tokens
        }
        QaScenario::E2eeLoginStore => {
            let mut tokens = vec!["safety=ok"];
            tokens.extend(tokens_for_stage(QaStage::E2eeLoginStore));
            tokens
        }
        QaScenario::RoomSpace
        | QaScenario::Directory
        | QaScenario::RoomManagement
        | QaScenario::RoomPeopleProjection
        | QaScenario::SessionStatus
        | QaScenario::CredentialHealth
        | QaScenario::NativeAttention
        | QaScenario::EncryptionDebug
        | QaScenario::E2eeTrust
        | QaScenario::InvitesDm
        | QaScenario::Timeline
        | QaScenario::TimelineStress
        | QaScenario::Activity
        | QaScenario::Composer
        | QaScenario::Reply
        | QaScenario::Media
        | QaScenario::LiveSignals
        | QaScenario::Thread
        | QaScenario::EditRedactSearch
        | QaScenario::RedactEditConvergence
        | QaScenario::SearchCrawler
        | QaScenario::ScheduledSend
        | QaScenario::SendQueue
        | QaScenario::RestoreCleanup
        | QaScenario::LinkPreview => {
            let mut tokens = stages_for_scenario(scenario)
                .into_iter()
                .flat_map(|stage| tokens_for_stage(stage).iter().copied())
                .collect::<Vec<_>>();
            tokens.push("restore_cleanup=ok");
            tokens.dedup();
            tokens
        }
        QaScenario::TimelineReconnect
        | QaScenario::CacheRestore
        | QaScenario::DeviceCleanup
        | QaScenario::GateRestore
        | QaScenario::GateNegative
        | QaScenario::GateNoProof
        | QaScenario::ReadStateConvergence => stages_for_scenario(scenario)
            .into_iter()
            .flat_map(|stage| tokens_for_stage(stage).iter().copied())
            .collect(),
        QaScenario::All => implemented_final_tokens(),
    }
}

pub(super) fn scenario_report(server_kind: &str, scenario: QaScenario) -> String {
    format!(
        "server={server_kind}\n{}",
        final_tokens_for_scenario(scenario).join("\n")
    )
}

pub(super) fn should_run_normal_secondary_participant(scenario: QaScenario) -> bool {
    scenario.should_run_stage(QaStage::InvitesDm)
        || scenario.should_run_stage(QaStage::Directory)
        || scenario.should_run_stage(QaStage::RoomSpace)
}

pub(super) fn should_run_focused_send_queue_route(scenario: QaScenario) -> bool {
    scenario == QaScenario::SendQueue
}

pub(super) struct QaConfig {
    pub(super) homeserver: String,
    pub(super) server_name: String,
    pub(super) server_kind: String,
    pub(super) user_a: String,
    pub(super) password_a: String,
    pub(super) user_b: String,
    pub(super) password_b: String,
    pub(super) user_c: Option<String>,
    /// Identity reset changes cross-signing identity for the account. Keep it
    /// opt-in so real-account QA cannot accidentally invalidate other devices.
    pub(super) allow_identity_reset: bool,
}

impl QaConfig {
    pub(super) fn from_env() -> Result<Self, String> {
        Ok(Self {
            homeserver: env_required(ENV_HOMESERVER)?,
            server_name: env_required(ENV_SERVER_NAME)?,
            server_kind: std::env::var(ENV_SERVER_KIND).unwrap_or_else(|_| "local".to_owned()),
            user_a: env_required(ENV_USER_A)?,
            password_a: env_required(ENV_PASSWORD_A)?,
            user_b: env_required(ENV_USER_B)?,
            password_b: env_required(ENV_PASSWORD_B)?,
            user_c: std::env::var(ENV_USER_C).ok(),
            allow_identity_reset: env_flag_enabled(ENV_ALLOW_IDENTITY_RESET)?,
        })
    }

    pub(super) fn dm_scope_control_user_id(&self) -> Result<String, String> {
        let user_c = self.user_c.as_deref().ok_or_else(|| {
            format!("{ENV_USER_C} is required for the invites_dm dm_space_scope check")
        })?;
        Ok(format!("@{}:{}", user_c, self.server_name))
    }

    pub(super) fn with_homeserver(&self, homeserver: String) -> Self {
        Self {
            homeserver,
            server_name: self.server_name.clone(),
            server_kind: self.server_kind.clone(),
            user_a: self.user_a.clone(),
            password_a: self.password_a.clone(),
            user_b: self.user_b.clone(),
            password_b: self.password_b.clone(),
            user_c: self.user_c.clone(),
            allow_identity_reset: self.allow_identity_reset,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct TimelineStressConfig {
    pub(super) space_count: usize,
    pub(super) rooms_per_space: usize,
    pub(super) messages_per_room: usize,
    pub(super) replay_existing: bool,
}

impl TimelineStressConfig {
    pub(super) fn from_env() -> Result<Self, String> {
        Ok(Self {
            space_count: bounded_usize_env(
                ENV_STRESS_SPACE_COUNT,
                DEFAULT_STRESS_SPACE_COUNT,
                MAX_STRESS_SPACE_COUNT,
            )?,
            rooms_per_space: bounded_usize_env(
                ENV_STRESS_ROOMS_PER_SPACE,
                DEFAULT_STRESS_ROOMS_PER_SPACE,
                MAX_STRESS_ROOMS_PER_SPACE,
            )?,
            messages_per_room: bounded_usize_env(
                ENV_STRESS_MESSAGES_PER_ROOM,
                DEFAULT_STRESS_MESSAGES_PER_ROOM,
                MAX_STRESS_MESSAGES_PER_ROOM,
            )?,
            replay_existing: env_flag_enabled(ENV_STRESS_REPLAY_EXISTING)?,
        })
    }

    pub(super) fn total_rooms(self) -> usize {
        self.space_count * self.rooms_per_space
    }

    pub(super) fn total_messages(self) -> usize {
        self.total_rooms() * self.messages_per_room + self.empty_formatted_probe_count()
    }

    fn empty_formatted_probe_count(self) -> usize {
        usize::from(self.total_rooms() > 0)
    }
}

fn bounded_usize_env(name: &str, default: usize, max: usize) -> Result<usize, String> {
    let Ok(value) = std::env::var(name) else {
        return Ok(default);
    };
    let parsed = value
        .parse::<usize>()
        .map_err(|_| format!("{name} must be a positive integer no greater than {max}"))?;
    if parsed == 0 || parsed > max {
        return Err(format!(
            "{name} must be a positive integer no greater than {max}"
        ));
    }
    Ok(parsed)
}

pub(super) fn env_flag_enabled(name: &str) -> Result<bool, String> {
    match std::env::var(name) {
        Ok(value) => parse_env_flag(name, &value),
        Err(_) => Ok(false),
    }
}

fn parse_env_flag(name: &str, value: &str) -> Result<bool, String> {
    if value == "1" || value.eq_ignore_ascii_case("true") {
        return Ok(true);
    }
    if value == "0" || value.eq_ignore_ascii_case("false") || value.is_empty() {
        return Ok(false);
    }
    Err(format!(
        "{name} must be 1, true, 0, false, or unset; got {value}"
    ))
}

fn env_required(name: &str) -> Result<String, String> {
    std::env::var(name).map_err(|_| format!("{name} is required"))
}

#[cfg(test)]
#[path = "registry_tests.rs"]
mod tests;
