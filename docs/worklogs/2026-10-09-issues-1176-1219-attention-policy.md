# Issues #1176 and #1219 per-account attention policy worklog

## Scope

One Rust-owned per-account attention policy shared by the account-tab badge and
that account's Home rail badge (#1219), plus the remaining cold-start attention
summary top-up (#1176).

## Base and canon

- Base: `20c2cf61` (v0.20.0, `origin/main`).
- Canon consulted: `docs/architecture/state-machine.md`
  ("Unread Source Of Truth", "Account Tabs And Concurrent Sessions",
  "Sidebar Sections And Low Priority", "Native Attention"),
  `docs/agents/verification.md`, `docs/agents/state-ownership.md`.
- The room-list server top-up for #1176 already landed on `main` in
  `4cdf2ebb`; this change closes the standalone attention-summary path and adds
  the shared account policy.

## Changes

- `crates/koushi-state/src/sidebar.rs`: new `AccountAttentionSummary` and
  `account_attention_summary` / `account_attention_summary_for_state`. The Home
  aggregate and the Tauri account tab both read it. Mentions-only rooms without
  a highlight no longer contribute to aggregate badges (Home, Space rail,
  Rooms/DMs section totals), matching the room's visible badge and the
  transient candidate.
- `apps/desktop/src-tauri/src/lib.rs`: `AccountTabSummary.unread_count` is the
  shared policy's `attention_count` for every tab, selected or background.
- `crates/koushi-sdk/src/room_projection.rs`: `room_attention_summary_from_room`
  uses `effective_room_notification_counts`, the same cold-start server top-up
  as the room list.
- `docs/architecture/state-machine.md`: the two bullets above.
- Tests: `crates/koushi-state/tests/account_attention_policy.rs`,
  `crates/koushi-sdk/src/room_projection/tests.rs`,
  `apps/desktop/src-tauri/src/tests.rs`.

## Verification

RED before the fix, GREEN after (see the commit message and PR notes for exact
commands and exit codes):

- `cargo test -p koushi-sdk --lib cold_start_attention_summary_uses_the_topped_up_server_count`
- `cargo test -p koushi-state --test account_attention_policy`

Green gates: full `koushi-state`, full `koushi-sdk`, `koushi-core --lib`,
`koushi-desktop --lib`, `cargo fmt --check`, and clippy `-D warnings` for the
changed crates.

## Known residuals

- #1176's two documented residuals remain: a stored read receipt whose target is
  absent from the cached chunk, and a muted room's raw display count. Both need
  more loaded history, not a different counter, and would break the memory
  budget tracked in #1150.
- The account tab's accessible name still announces account and status, not the
  attention count; that is a frontend presentation follow-up, not policy.
