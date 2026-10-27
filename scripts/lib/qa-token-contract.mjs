// Shared QA token / private-data assertions for real and headless QA wrappers.
//
// QA scripts must assert scenario-specific success tokens (not just process exit
// code), and real-account/real-homeserver QA output must never carry private
// Matrix identifiers. These helpers enforce both contracts directly in the Node
// wrappers so a passing exit code can never mask a missing token or a leaked id.

/**
 * Parse the set of `name=value` success tokens from QA output. Only the closed
 * vocabulary of status values is accepted so prose can never be mistaken for a
 * token.
 */
const ADDRESS_TOKENS = [
  "room_address_preview_create_share=ok",
  "room_address_collision=ok",
  "room_address_availability=ok",
  "room_address_space_prefix=ok",
  "room_public_space_default=ok",
  "room_unnamed_public_space=ok",
  "room_unnamed_alias_name=ok",
  "room_unnamed_alias_conflict=ok",
  "room_unnamed_rename=ok",
  "room_public_space_private_choice=ok"
];
const IGNORE_RECOVERY_TOKEN = "ignored_user_history_recovery=ok";

export const HEADLESS_LOCAL_QA_SCENARIO_TOKENS = Object.freeze({
  history_key_recovery_window: Object.freeze([
    "history_window_deferred=ok",
    "history_window_29_days=ok",
    "history_window_30_days_expired=ok",
    "history_recovery_offline_pending=ok",
    "history_recovery_startup_admin_retry=ok",
    "history_recovery_restart_cooldown=ok",
    "history_recovery_join_without_invite=ok",
    "history_recovery_without_manual_verification=ok",
    "history_recovery_31_members_admin=ok",
    "history_recovery_force_all=ok",
    "history_recovery_after_window_expiry=ok",
    "history_recovery_visibility_guard=ok",
    "history_recovery_membership_guard=ok",
    "history_recovery_blocked_device_guard=ok",
    "history_recovery_crash_replay=ok",
    "history_recovery_policy_race_guard=ok",
    "history_recovery_identity_race_guard=ok",
    "history_window_cleanup=ok",
    "history_key_recovery_window=ok"
  ]),
  avatar_demand: Object.freeze(["avatar_window_requests=ok"]),
  directory: Object.freeze(["directory_query=ok", "directory_join=ok", ...ADDRESS_TOKENS]),
  live_signals: Object.freeze(["live_signals=ok", IGNORE_RECOVERY_TOKEN]),
  room_people_projection: Object.freeze(["room_people_invited_scope=ok", "room_people_projection=ok"]),
  all: Object.freeze([...ADDRESS_TOKENS, IGNORE_RECOVERY_TOKEN, "room_people_invited_scope=ok"]),
  redact_edit_convergence: Object.freeze([
    "redact_edit_convergence=ok",
    "thread_summary_convergence=ok"
  ]),
  read_state_convergence: Object.freeze(["read_state_convergence=ok"]),
  hidden_state_acl: Object.freeze(["hidden_state_acl=ok"]),
  search_crawler_catchup: Object.freeze([
    "crawl_catchup_live=ok",
    "crawl_catchup_restart=ok",
    "search_crawler_catchup=ok"
  ]),
  user_verification: Object.freeze([
    "user_verification_offered=ok",
    "user_verification_a_observed_join=ok",
    "user_verification_request_sent=ok",
    "user_verification_incoming_request=ok",
    "user_verification_accepted=ok",
    "user_verification_sas_match=ok",
    "user_verification_done=ok",
    "user_verification_identity_verified=ok",
    "user_verification=ok"
  ]),
  account_notifications: Object.freeze([
    "account_notifications_load=ok",
    "account_notifications_no_write_on_open=ok",
    "account_notifications_category_write=ok",
    "account_notifications_overlap=ok",
    "account_notifications_category_restore=ok",
    "account_notifications_email_unsupported=ok",
    "account_notifications_email_requires_verified=ok",
    "account_notifications_keyword_actions=ok",
    "account_notifications_keyword_projection=ok",
    "account_notifications_keyword_duplicate=ok",
    "account_notifications_keyword_disabled=ok",
    "account_notifications_keyword_remove=ok",
    "account_notifications=ok"
  ]),
  thread_late_joiner: Object.freeze([
    "thread_late_joiner_root_not_visible=ok",
    "thread_late_joiner_thread_panel=ok",
    "thread_late_joiner_withheld_open_settled=ok",
    "thread_late_joiner_withheld_retry_close=ok",
    "thread_late_joiner=ok"
  ]),
  mcp_message_actions: Object.freeze(["mcp_message_actions=ok"]),
  notification_delivery: Object.freeze([
    "notification_delivery_file_credential_store=ok",
    "notification_delivery_unfocused=ok",
    "notification_delivery_channel_off=ok",
    "notification_delivery_dwell=ok",
    "notification_delivery_request=ok",
    "notification_delivery_body=ok",
    "notification_delivery_read_cancels=ok",
    "notification_delivery_rearm=ok",
    "notification_delivery_min_interval=ok",
    "notification_delivery_watched_room=ok",
    "notification_delivery_watched_space=ok",
    "notification_delivery_secret_hygiene=ok",
    "notification_delivery_secret_binding=ok",
    "notification_delivery_prefix=ok",
    "notification_delivery_prefix_no_consent=ok",
    "notification_delivery_prefix_option_off=ok",
    "notification_delivery_prefix_no_text=ok",
    "notification_delivery_prefix_consent_cleared=ok",
    "notification_delivery_prefix_endpoint_cleared=ok",
    "notification_delivery_prefix_hygiene=ok",
    "notification_delivery_serverchan_configured=ok",
    "notification_delivery_serverchan_sendkey_drops_consent=ok",
    "notification_delivery_serverchan_request=ok",
    "notification_delivery_serverchan_no_text=ok",
    "notification_delivery_serverchan_secret_hygiene=ok",
    "notification_delivery=ok"
  ]),
  room_history_export: Object.freeze([
    "history_export_full=ok",
    "history_export_period=ok",
    "history_export_period_fallback=ok",
    "history_export_utd_counted=ok",
    "history_export_cancel=ok",
    "history_export_space=ok",
    "history_export_attachments=ok",
    "history_export_resume=ok",
    "room_history_export=ok"
  ]),
  room_management: Object.freeze([
    "space_topic=ok",
    "knock_room_created=ok",
    "knock_join_request=ok"
  ])
});

export function tokensFromOutput(output) {
  return new Set(
    String(output)
      .split(/\s+/)
      .filter((token) =>
        /^[a-z0-9_]+=(ok|running|created|not_found|completed|partial)$/.test(token)
      )
  );
}

export function requiredTokensForHeadlessScenario(scenario) {
  return HEADLESS_LOCAL_QA_SCENARIO_TOKENS[scenario] ?? [];
}

/**
 * Throw unless every required token is present in the output. Asserts the
 * scenario actually reached each documented checkpoint.
 */
export function assertRequiredTokens(output, requiredTokens, label) {
  const tokens = tokensFromOutput(output);
  const missing = requiredTokens.filter((token) => !tokens.has(token));
  if (missing.length > 0) {
    throw new Error(`${label}: missing required QA tokens: ${missing.join(", ")}`);
  }
}

/**
 * Throw if any Matrix identifier (@user:server, !room:server, $event:server)
 * appears in the output. Real QA must surface only private-data-free tokens.
 */
export function assertNoMatrixIdentifiers(output, label) {
  const text = String(output);
  // Boundary includes `=` so the net also catches ids attached to a token key
  // (e.g. `user_id=@user:server`) — the exact format the old binary leaked — not
  // only whitespace-delimited ids.
  const matrixIdPattern = /(?:^|[\s=])([@!$][A-Za-z0-9._=\-]+:[A-Za-z0-9.\-]+)(?:\s|$)/;
  const match = text.match(matrixIdPattern);
  if (match) {
    throw new Error(`${label}: private Matrix identifier leaked into QA output`);
  }
}

/**
 * Throw if a local filesystem path appears in output that may become an issue
 * artifact. Keep the diagnostic generic so the rejection itself is shareable.
 */
export function assertNoLocalPaths(output, label) {
  const text = String(output);
  const localPathPattern =
    /(?:^|[\s"'(=])((?:\/(?:home|Users|tmp|private\/tmp|var\/folders|workspace|runner|Volumes)\/[^\s"'<>]*)|(?:[A-Za-z]:\\[^\s"'<>]+))/;
  if (localPathPattern.test(text)) {
    throw new Error(`${label}: local filesystem path leaked into QA output`);
  }
}

/**
 * Throw on common raw SDK/runtime diagnostic shapes. Public QA output should
 * contain stable product tokens and coarse failure kinds, not library internals.
 */
export function assertNoRawSdkErrors(output, label) {
  const text = String(output);
  const rawSdkPattern =
    /\b(?:matrix_sdk(?:_\w+)*::|ruma::|reqwest::|hyper::|SdkError|HttpError|ClientApiError|StoreError|ServerError|M_[A-Z0-9_]+)\b/;
  if (rawSdkPattern.test(text)) {
    // Never echo the offending source line: it may carry private payloads.
    // Report only the stable coarse kind so the rejection itself is shareable.
    throw new Error(`${label}: raw SDK diagnostic leaked into QA output`);
  }
}
