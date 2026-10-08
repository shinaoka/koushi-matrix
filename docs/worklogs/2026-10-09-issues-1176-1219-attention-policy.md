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
- #1176's room-list half already landed on `main` in `4cdf2ebb`
  (v0.19.1/v0.20.0): `effective_room_notification_counts` tops the
  notification/highlight counters up from `unread_notification_counts` for the
  room badge, the Rooms/Space-rail/Home aggregates and the room-list projection.
  The remaining production gap for this deliverable was therefore the account-tab
  policy in #1219; the #1176 change here only extends the same counter top-up to
  the standalone `room_attention_summary_from_room` helper the issue names.

## Changes

- `crates/koushi-state/src/sidebar.rs`: new `AccountAttentionSummary` and
  `account_attention_summary_for_state`. The Home aggregate and the Tauri account
  tab both read it, so the tab inherits the existing Home muted/low-priority
  policy unchanged. `contributes_attention` keeps exactly its previous
  muted/low-priority exclusions, so unread content from a Mentions-only room
  still contributes to Home, the Space rail, the section totals and the tab.
- `crates/koushi-state/src/lib.rs`: re-export `AccountAttentionSummary` and the
  state-level `account_attention_summary_for_state`.
- `apps/desktop/src-tauri/src/lib.rs`: `AccountTabSummary.unread_count` is the
  shared policy's `attention_count` for every tab, selected or background.
- `crates/koushi-sdk/src/room_projection.rs`: `room_attention_summary_from_room`
  uses `effective_room_notification_counts`, the same cold-start counter top-up
  as the room-list projection, for the same room state.
- `docs/architecture/state-machine.md`: the shared policy and the narrowed
  #1176 top-up rule.
- Tests: `crates/koushi-state/tests/account_attention_policy.rs`,
  `crates/koushi-sdk/src/room_projection/tests.rs`,
  `apps/desktop/src-tauri/src/tests.rs`.

## #1176 parity boundaries

`room_attention_summary_from_room` has no first-party production caller yet (only
the re-export in `crates/koushi-sdk/src/lib.rs` and its unit test). It applies the
same counter top-up as the room-list projection but **not** the room list's
read-marker/stale-count suppression in `matrix_room_list_room_from_counts`, so
the two can still differ when a read marker covers the projected latest event.
The SDK test `stale_server_counts_still_lose_to_a_matching_read_marker` asserts
that known difference explicitly (room list 0, standalone summary 5).

## Verification (observed)

All commands run in this worktree with its own `target/` directory. Exit statuses
were read directly; no filtered-out count is reported as a pass.

| Command | Result | Exit |
| --- | --- | --- |
| `cargo test -p koushi-state --test account_attention_policy` | 8 passed, 0 failed | 0 |
| `cargo test -p koushi-state` (all targets) | all test targets passed | 0 |
| `cargo test -p koushi-sdk --lib room_projection` | 45 passed, 0 failed, 195 filtered | 0 |
| `cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml --lib` | 218 passed, 0 failed | 0 |
| `cargo fmt --check` | no diff | 0 |
| `cargo clippy -p koushi-state -p koushi-sdk --all-targets --locked -- -D warnings` | no warnings | 0 |
| `cargo clippy --manifest-path apps/desktop/src-tauri/Cargo.toml --all-targets --locked -- -D warnings` | no warnings | 0 |
| `node scripts/check-sdk-submodule.mjs` | synced | 0 |
| `node scripts/check-rust-test-structure.mjs` | ok | 0 |
| `node scripts/check-agents-docs.mjs` | ok | 0 |
| `kache doctor --verify` | 19879/19879 entries valid, 0 corrupted | 0 |

Not run: full `cargo test -p koushi-sdk` / `-p koushi-core`, release-config
clippy, QA-bin clippy, any homeserver/QA lane, and any frontend gate (no
TypeScript touched). Hosted CI owns those.

## Known residuals / open items

- #1176's two documented residuals remain: a stored read receipt whose target is
  absent from the cached chunk, and a muted room's raw display count. Both need
  more loaded history, not a different counter, and would break the memory
  budget tracked in #1150.
- `room_attention_summary_from_room` is public consistency API without a
  production caller; it does not suppress stale counts covered by a read marker
  (see above).
- The account tab's accessible name still announces account and status, not the
  attention count; that is a frontend presentation follow-up, not policy.
