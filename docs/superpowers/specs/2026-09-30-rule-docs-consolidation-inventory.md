# Agent rule-docs inventory (origin/main @ de06d0ff, 2026-09-30)

Scope: AGENTS.md, REPOSITORY_RULES.md (RR), docs/policies/engineering-rules.md (ER),
docs/agents/{verification,environment,qa-lanes,troubleshooting,history,plans}.md.
Out of scope for edits (only noted as duplicate owners): docs/architecture/* (overview = OV,
state-machine = SM, i18n), docs/agents/state-ownership.md (SO).
All line numbers refer to origin/main.

Total in-scope: 3 798 lines / ~272 KB (ER 1216 + RR 774 + qa-lanes 412 + verification 357 +
environment 332 + troubleshooting 235 + plans 234 + history 171 + AGENTS 66).

---------------------------------------------------------------------------------------------

## 0. Mechanical dependencies a restructure MUST preserve or update

### 0.1 scripts/check-agents-docs.mjs (runs in `npm --prefix apps/desktop run lint`)
- `ROUTER_LINE_BUDGET = 240` for AGENTS.md (currently 66). AGENTS.md:60 restates "240".
- Every `docs/agents/*.md` must appear as the literal substring `docs/agents/<file>` in
  AGENTS.md (reverse check, `router.includes`), and every `(docs/agents/<x>)` link in AGENTS.md
  must resolve (regex strips `#anchor`). Adding/renaming/deleting a topic file requires an
  AGENTS.md table edit. Merging files is allowed as long as both directions hold.
- Retired flags inside fenced code blocks fail in AGENTS.md and every docs/agents/*.md
  except `docs/agents/history.md` (hard-coded quarantine path): `--server=conduit`,
  `--core-backend`, `KOUSHI_QA_FORCE_SYNC_BACKEND`, `--scenario=timeline_legacy`.
  Renaming/moving history.md requires editing `quarantine` (line 29).
- Every `--scenario=<name>` anywhere in AGENTS.md + docs/agents/* (except history) must exist:
  `local-*`/`signed-out` against `scripts/desktop-linux-gui-qa.mjs --list`, others against the
  `"name" => Ok(Self::` arms of `crates/koushi-qa/src/bin/{headless_core_qa/registry.rs,
  real-homeserver-qa.rs, real_homeserver_qa/config.rs}`. Note this now also covers
  `--scenario=startup_latency` (qa-lanes:398) and `--scenario=invites_dm` (verification:116).
- qa-lanes.md is hard-coded (`catalog`, line 151): any table row (`|...`) naming a backticked
  `local-*`/`signed-out` must exist in `--list`. Moving the GUI scenario table out of
  qa-lanes.md silently removes that check unless the path is updated.
- ER and RR are NOT checked by this script (retired flags / scenario names there are unguarded).

### 0.2 Tests that read doc text
- `scripts/build-structure-contract.test.mjs:39-64` — reads ER, requires heading
  `## Build, Dependencies, QA Gates`, then slices from the first `\n1. ` to the next `\n2. `
  inside that section. That item must match `/root workspace Matrix SDK\s+dependencies/` and
  ``/use their exact paths beneath\s+`vendor\/matrix-rust-sdk`/`` and must NOT contain
  "rev-pinned git dependency", "pinned git revision", a github.com matrix-rust-sdk URL, or
  "app crates must not depend on it by local path". => ER Build item 1 (ER:1081-1103) must stay
  the SDK owner, keep numbering `1.` then `2.`, keep that phrasing. Item `0.` before it is fine.
- `apps/desktop/src/scripts/linuxGuiQa.test.ts:363-395` — environment.md must contain the
  Docker lane text token-for-token: `bash -c` (and NOT `bash -lc`), `-u "$(id -u):$(id -g)"`,
  the three `-v /tmp/koushi-desktop-*` mounts, `CARGO_HOME=/tmp/cargo-home`,
  `CARGO_TARGET_DIR=/tmp/koushi-desktop-gui-target`, `NPM_CONFIG_CACHE=/tmp/npm-cache`,
  `koushi-desktop-linux-gui:basic-ops`, `--scenario=local-send`, `--server=tuwunel`,
  `--artifact-dir=/work/artifacts/linux-gui-local-send-docker`, `--timeout-ms=180000`,
  `tuwunel`, `zstd`, the full `PATH=/opt/cargo/bin:...` string, `RUSTC="$(rustup which rustc)"`,
  `RUSTDOC="$(rustup which rustdoc)"`. environment.md:216-227 is effectively frozen (or the test
  path must move with it).
- `apps/desktop/src/scripts/headlessAndRealQa.test.ts:215-224` — qa-lanes.md must contain
  `redact_edit_convergence=ok` and `thread_summary_convergence=ok`.

### 0.3 Numbered-rule citations (break silently if ER items are renumbered)
- "engineering-rules: Secrets rule 2" — `apps/desktop/src-tauri/src/lib.rs:165,181`,
  `apps/desktop/src-tauri/src/tests.rs:170`, `scripts/desktop-release-gate-check.mjs:2,29`.
- "engineering rule 11" (Secure Backup reveal) — `REPOSITORY_RULES.md:341`,
  `docs/agents/state-ownership.md:1268`.
- "engineering-rules Async and Runtime #9/#10/#11" — only dated specs
  (`docs/superpowers/specs/2026-06-22-user-intent-observability-design.md:26`); "Secrets rule 8"
  in a dated plan. Historical, low stakes.
- The many `Async rule N` / `Backpressure rule 11` comments in crates/ and apps/ cite
  **overview.md**'s "Async Design Rules", NOT ER. Do not "fix" them during this work.
- Recommendation: keep ER item numbers 1-11 in Secrets stable (slim in place), or convert those
  7 citations to heading names in the same PR.

### 0.4 Line-number citations (already broken)
- `crates/koushi-core/src/timeline/actor.rs:2764` and `crates/koushi-core/src/timeline/media.rs:569`
  cite "REPOSITORY_RULES L124-128" for reliable delivery. RR:124-128 is now Property Display;
  the rule is RR:305-309 (State-Machine Discipline) / ER Async 9. Replace with a heading cite.
  These are the only line-number citations found (`git grep "RULES.*L[0-9]"`).

### 0.5 Heading names quoted by tools/comments (not tests; messages go misleading if renamed)
- "Architecture And Ownership": `scripts/check-tauri-adapter-boundary.mjs:10,109`,
  `scripts/check-domain-crate-platform-deps.mjs:188` (also says "and Platform Portability",
  which is an OV heading), `apps/desktop/eslint.config.js:4`.
- "REPOSITORY_RULES Key Management" `crates/koushi-core/src/runtime.rs:5418`;
  "REPOSITORY_RULES Security" `crates/koushi-qa/.../compat_flow.rs:65`, `cleanup.rs:34`;
  "State-Machine Discipline" `docs/architecture/state-machine.md:95,4870`;
  "engineering-rules.md -> Documentation" `state-machine.md:96`;
  "Secrets section" `crates/koushi-qa/src/bin/real-homeserver-qa.rs:7`,
  `crates/koushi-core/src/startup_trace.rs:5`.
- `apps/desktop/playwright.config.ts:22` says the worker-contention finding is "in AGENTS.md"
  (it is actually qa-lanes.md:255-259) — already stale.
- `docs/qa/2026-07-20-issue-285-linux-handoff.md:121,189` point to Ubuntu deps /
  `RUMA_UNSTABLE_EXHAUSTIVE_TYPES` "in AGENTS.md" (now environment.md) — dated record, stale.

### 0.6 Anchors referenced from elsewhere (must survive or be updated)
| Anchor | Referenced from |
| --- | --- |
| `REPOSITORY_RULES.md#github-issue-language` | AGENTS.md:9 |
| `REPOSITORY_RULES.md#property-display-and-editing` | `.github/pull_request_template.md:13`, verification.md:250 |
| `REPOSITORY_RULES.md#shared-hot-files` | verification.md:351 |
| `REPOSITORY_RULES.md#review-and-audit` | verification.md:354, ER:1208 |
| `REPOSITORY_RULES.md#macos-native-window-controls-and-overlay-layout` | OV:2096, `docs/superpowers/plans/2026-09-20-macos-dialogs-and-update-navigation.md:7` |
| `engineering-rules.md#build-dependencies-qa-gates` | verification.md:70 (also test-pinned heading) |
| `verification.md#rust-lint-gate` | ER:1108 |
| `verification.md#what-ci-actually-gates` | `.github/workflows/ci.yml:233`, environment.md:82 |
| `environment.md#signed-macos-dmg` | `README.md:225`, troubleshooting.md:18 |
| `environment.md#reusing-a-debug-build` | qa-lanes.md:175, troubleshooting.md:129 |
| `troubleshooting.md#browser-headless-harness` | qa-lanes.md:280, history.md:171 |
| `troubleshooting.md#local-homeserver-core-qa` | history.md:96 |
| `history.md#retired-qa-vocabulary` | AGENTS.md:15 |
| `history.md#login-timeout-investigation-334-375` | troubleshooting.md:208 |
| `qa-lanes.md#output-must-be-private-data-free` | state-ownership.md:124 |
| `state-ownership.md#snapshot-and-wire-contract-mirrors` / `#credential-health` / `#settings-composer-and-scheduled-send` | verification.md:6,222; troubleshooting.md:132; qa-lanes.md:368; plans.md:234 (targets out of scope, keep) |

`docs/help/*.md` links to `troubleshooting.md#cannot-sign-in` etc. target
`docs/help/troubleshooting.md`, a different file — not affected.

### 0.7 Other readers/describers
- `docs/README.md:24-70` restates the role of each doc (RR, ER, AGENTS, each docs/agents file,
  "Durable rules ... promoted", "check-agents-docs guards the routing"). Update if topics merge.
- `.agents/skills/koushi-release/SKILL.md`, `.claude/skills/koushi-release/SKILL.md`: no
  references to these docs (environment.md:326-332 points to the skill, one-way).
- `.github/workflows/build-windows.yml:55`, `rust-toolchain.toml:4`, several crate comments cite
  docs by name only.

---------------------------------------------------------------------------------------------

## 1. Per-file structure

### AGENTS.md — 66 lines (router; loaded every session)
| Lines | Section | Purpose |
| --- | --- | --- |
| 1-5 (5) | header | points to RR as durable rules |
| 6-26 (21) | Essential contracts | 8 one-line contracts (issue language, Rust ownership, sync engine/QA flags, verify-first, private data, SDK source, IME/catalog, deferred calls) |
| 27-50 (24) | Read by task | routing table + 120 s command bound |
| 51-66 (16) | Keeping these notes maintainable | router discipline + checker |

### REPOSITORY_RULES.md — 774 lines
| Lines | Section | Purpose |
| --- | --- | --- |
| 1-10 (10) | header | status, vendored-code licensing, Last amended |
| 11-32 (22) | Read Order And Authority | reading order, doc hierarchy, conflict rule |
| 33-49 (17) | Canon-First Change Protocol | amend canon before code |
| 50-67 (18) | Root-Cause Fix Discipline | no masking fixes |
| 68-93 (26) | Text Input And IME Ownership | IME primitive rules (condensed copy of ER IME) |
| 94-103 (10) | User-Visible Text | short action labels |
| 104-132 (29) | Property Display And Editing | one place per property |
| 133-152 (20) | macOS Native Window Controls And Overlay Layout | safe-area contract |
| 153-283 (131) | Architecture And Ownership | SDK submodule, headless-first (x2), single sync engine, VectorDiff accumulator, Element parity research, Phase A/B, crate ownership (protocol/store/search/media/qa/testkit/sdk), localStorage, browser fakes, React ephemeral state, task lifecycle, Tauri adapter, command registration, UI no SDK types |
| 284-327 (44) | State-Machine Discipline | guarded state machines, reliable delivery, background try_send, thread attention (feature spec) |
| 328-415 (88) | Security Rules | secret classes, Debug privacy, E2EE DTOs, key export format, search-index rules, receipt avatars (feature spec) |
| 416-463 (48) | Key Management | unlock secret, key derivation, persistent-store-first auth, device-ID reuse, journal, Megolm pre-share (feature spec) |
| 464-493 (30) | QA Gates And Cleanup | GUI smoke layer, local homeservers, tokens, cleanup guards, env filtering, FIFO |
| 494-519 (26) | Tests And Fixtures | synthetic data |
| 520-565 (46) | User-Facing Text And Localization | catalog, locale profile, logical CSS, CJK |
| 566-583 (18) | Product Identity And Migration | Koushi identifiers |
| 584-591 (8) | Concurrent Work ... | Wave 2 rationale paragraph |
| 592-620 (29) | Test Placement | tests/ per feature, 200-line inline ceiling |
| 621-650 (30) | Shared Hot Files | hot-file list + append-friendly tips |
| 651-671 (21) | Parallel Implementation Protocol | main/subagent split, integration branches |
| 672-693 (22) | Worktree And Build Artifact Cleanup | worktree hygiene |
| 694-737 (44) | Review And Audit | review ownership, risk-based review, priorities, rule-gap proposals |
| 738-745 (8) | GitHub Issue Language | English issues |
| 746-766 (21) | Documentation And Work Records | user-guide check, plans subordinate, work records, agents tree |
| 767-774 (8) | Licensing | upstream notices |

### docs/policies/engineering-rules.md — 1216 lines
| Lines | Section | Purpose |
| --- | --- | --- |
| 1-11 (11) | header | status, relation to RR/AGENTS |
| 12-20 (9) | Design Simplicity | 3 rules |
| 21-480 (460) | Secrets and Private Data | list + 15 numbered rules. Rule sizes: 1-8 ≈ 27 lines total; **9** native attention 66-109 (44); **10** settings/localStorage/URL previews/schema/composer drafts/revisions/tombstones/scheduled send 110-236 (**127**); **11** E2EE trust/recovery key/backup/Megolm/auth/quarantine/device cleanup 237-359 (**123**); 12 credential health 360-380 (21); 13 media 381-396 (16); **14** profile/avatar/alias/labels 397-466 (70); 15 message actions 467-479 (13) |
| 481-506 (26) | Logging and Diagnostics | 5 rules |
| 507-888 (382) | Async and Runtime | 19 rules. Big ones: **4** scrollback/gap repair/live-edge 533-620 (**88**); **9** reliable delivery/shutdown/waiters 635-671 (37); 11 channel capacity (18); 12 terminal outcomes (21); 14 FIFO correlation (14); **16** verified-session sync ownership/to-device/key-query/multi-stage QA/NoToken 742-862 (**121**); 17-19 (25) |
| 889-1034 (146) | GUI Automation | rule **0** 893-1014 (**122**: headless default, wdio spike, i18n, fakes, room-list/settings/formatted/mention/room-mgmt/activity/security/tooltip/px contracts); rules 1-5 macOS/credential smoke (15) |
| 1035-1073 (39) | Desktop Text Input And IME Safety | 7 rules (primitive names, sync keys, Enter, queues, secrets, tests, gate) |
| 1074-1164 (91) | Build, Dependencies, QA Gates | 0 headless-first, **1 SDK submodule (test-pinned)**, 2 Tuwunel pointer, 3 merge gates, 4 real HS, 5 portability, 6-8 CJK/core convergence, 9 signing, 10 long scenarios |
| 1165-1209 (45) | Documentation | 9 rules restating RR |
| 1210-1216 (7) | `# Current sync contract (Issue #412)` | stray trailing H1 duplicating sync contract |

### docs/agents/verification.md — 357 lines
| Lines | Section | Purpose |
| --- | --- | --- |
| 1-7 | header | |
| 8-32 (25) | Verify first, no human eyes | RED-then-GREEN discipline |
| 33-49 (17) | Minimize human round trips | rich diagnostics |
| 50-59 (10) | Read the gate's own exit status | pipeline exit trap |
| 60-75 (16) | Verification by stage | iteration/review/merge |
| 76-102 (27) | Running focused tests | `--lib`, `--test`, long scenarios, broad Playwright failures |
| 103-119 (17) | What CI actually gates | CI job table |
| 120-214 (95) | ### Rust lint gate | lint commands (120-150) **plus** unrelated CI profile/cache/SDK-mtime/test-consolidation notes (152-213) filed under the lint heading |
| 215-270 (56) | Diff self-review | self-review procedure, priorities, user-guide, property check |
| 271-280 (10) | Design simplicity | pointer + 1 extra sentence |
| 281-320 (40) | Issue #738 flake measurement | probe workflow for a closed issue |
| 321-337 (17) | IME-safe text input checks | commands |
| 338-357 (20) | Cost-controlled agent delegation | cheap-agent scope, main-agent ownership |

### docs/agents/environment.md — 332 lines
| Lines | Section | Purpose |
| --- | --- | --- |
| 1-7 | header | |
| 8-25 (18) | Matrix SDK submodule | init + guard |
| 26-62 (37) | Local homeserver binaries | PATH list, Tuwunel build flags |
| 63-83 (21) | Local gates | hooks + npm gates |
| 84-110 (27) | npm dependency security gate | audit procedure |
| 111-146 (36) | Rust test stack and debug information | RUST_MIN_STACK, line-tables, target cleanup |
| 147-186 (40) | Reusing a debug build | --skip-build + QA title footgun, inotify workaround |
| 187-209 (23) | Linux GUI host packages | apt list, fast checks |
| 210-234 (25) | Linux GUI QA container | **test-pinned docker block** |
| 235-243 (9) | CodeGraph | codegraph usage |
| 244-325 (82) | Signed macOS DMG | CI env + local zsh procedure (30-line shell function) |
| 326-332 (7) | Automated desktop releases | pointer to runbook/skill |

### docs/agents/qa-lanes.md — 412 lines
| Lines | Section | Purpose |
| --- | --- | --- |
| 1-7 | header | |
| 8-21 (14) | Command contract | --server values, retired flags |
| 22-36 (15) | Output must be private-data-free | never-print list, screenshots (anchor used by SO) |
| 37-157 (121) | Headless core lane | command, credential isolation, timeline_stress, scenario table (40 rows, some cells >100 words), key-backup scope, SyncOnce/logout/typing notes |
| 158-247 (90) | Linux virtual-display GUI lane | command, **checker-scanned scenario table**, lane scope notes |
| 248-281 (34) | Browser-headless (Playwright) lane | workers:1, harness CSS, Space roles test |
| 282-298 (17) | Documentation screenshot lane | |
| 299-364 (66) | Real-account lanes | mac smoke viewport tokens, safety rules, prompt order |
| 365-389 (25) | Credential-health tiers | Tier 1-3, disabled Tier 2 workflow |
| 390-412 (23) | Startup latency observability | #123 lane |

### docs/agents/troubleshooting.md — 235 lines
| Lines | Section | Purpose |
| --- | --- | --- |
| 1-7 | header | |
| 8-28 (21) | Local macOS app signing | 2026-09-15 fix narrative |
| 29-97 (69) | Browser-headless harness | 10 symptom bullets |
| 98-135 (38) | Linux GUI (WebDriver) lane | 7 symptom bullets |
| 136-163 (28) | macOS GUI smoke | permissions, process names, 5173, Cmd+Q, PTY stdin |
| 164-182 (19) | Real-account smoke | 5 symptoms |
| 183-235 (53) | Local homeserver core QA | Tuwunel receipts, Synapse 429, login timeout, trust-recheck contract + focused tests, CPU restriction, gate-confirmation history |

### docs/agents/history.md — 171 lines
| Lines | Section | Purpose |
| --- | --- | --- |
| 1-10 | header | quarantine notice |
| 11-26 (16) | Retired QA vocabulary | retired-flag table (AGENTS anchor) |
| 27-91 (65) | Sync backend selection ... (superseded) | probe post-mortem |
| 92-124 (33) | Login timeout investigation (#334, #375) | post-mortem |
| 125-171 (47) | Browser-headless flake history | fixed 2026-06/07 flakes |

### docs/agents/plans.md — 234 lines / 42 KB
| Lines | Section | Purpose |
| --- | --- | --- |
| 1-10 | header | plans are historical |
| 11-19 (9) | Timeline viewport replacement proposal | |
| 20-59 (40) | Runtime and roadmap | 13 plan links, blank-line separated |
| 60-73 (14) | Umbrella #12 | batch plan + reconciliation instruction |
| 74-234 (161) | Feature areas | 153-row table; ~110 rows repeat the identical link in both Phase A and Phase B; plus a licensing rule at 232-234 |

---------------------------------------------------------------------------------------------

## 2. Duplications (same rule in 2+ places) and proposed single owner

Owner key: RR = short durable rule; ER = detailed policy; OP = operational docs/agents topic;
OV/SM/SO = existing architecture/spec owner (out of scope, keep; delete the copy in scope).

| # | Rule | Locations | Proposed owner |
| --- | --- | --- | --- |
| D1 | One Simplified Sliding Sync engine; no backend selection; `--server` values; rejected flags | AGENTS:12-15; RR:169-173; ER:872-878 (rule 18); ER:1210-1216 (stray H1); qa-lanes:10-20; history:13-16; OV:1090 (rule 10) | RR one bullet (product) + qa-lanes Command contract (flags) + history table. Delete ER:1210-1216; keep ER Async 18 only for the admission-check nuance, or move it to OV. AGENTS keeps its one line. |
| D2 | Headless-first / local-server-first; GUI-first prohibited; Phase A/B | RR:164-168 **and** RR:193-201 (twice in one section); ER:1076-1080 (Build 0); AGENTS:16-17; OV:2060; verification:8-32 | RR one bullet (merge 164-168 + 193-201). Delete ER Build 0 (keep numbering: see 0.2 — item `0.` may be removed safely; `1.` must remain). |
| D3 | `vendor/matrix-rust-sdk` path deps, gitlink sole pin, `check-sdk-submodule` | RR:155-163; ER:1081-1089; environment:10-24; AGENTS:20-21 | ER Build 1 (test-pinned). RR → one sentence + link. environment keeps only the two commands + recovery. |
| D4 | Secret classes / never log | RR:330-345; ER:23-35; ER Logging 1-2; RR:385-387 | RR (class list + prohibition). ER keeps only the "allowed in debug/UI" split (ER:33-35) and zeroizing rule. |
| D5 | Public DTO `Debug` privacy contract | RR:355-365; ER:490-499 (Logging 3) | ER Logging 3; RR one sentence. |
| D6 | Real-HS QA output tokenized before persistence | RR:350-354; RR:478-481; ER:501-505 (Logging 5); qa-lanes:22-35 | ER Logging 5 (rule) + qa-lanes (never-print list, anchor used by SO). Delete RR:350-354 and 478-481 (they say the same thing twice within RR). |
| D7 | Synthetic data only in docs/tests/fixtures | RR:346-349; RR:496-513 (5 bullets); ER:30-31; ER:1175 (Doc 4); RR:733-736 | RR Tests And Fixtures (one compact list). Delete ER Doc 4, ER:30-31. |
| D8 | FIFO credentials, never argv/coordinates | RR:490-492; ER:46-48 (Secrets 3); ER:1015-1017 (GUI 1); qa-lanes:322-325 | ER Secrets 3 (keep number). Delete GUI 1 (merge its anecdote-free half), RR:490-492, qa-lanes bullet → link. |
| D9 | Filter parent env before spawning QA children | RR:488-489; ER:49-51 (Secrets 4); qa-lanes:333-335 | ER Secrets 4. |
| D10 | No post-login real-account screenshots; `--allow-private-screenshots` | ER:52-56 (Secrets 5); qa-lanes:33-35; qa-lanes:351-352 | ER Secrets 5 + qa-lanes flag description only. |
| D11 | Logout cleanup unless `--keep-session`; cleanup guards | RR:482-487; ER:629-631 (Async 7); qa-lanes:336-338 | RR QA Gates (rule); delete ER Async 7; qa-lanes → link. |
| D12 | Avoid repeated destructive real-account logins | ER:632-634 (Async 8); qa-lanes:339-341 | qa-lanes (operational). Delete ER Async 8. |
| D13 | Keychain-prompt env vars (`KOUSHI_SKIP_*`, file credential store) | ER:62-65 (Secrets 8); ER:1023-1026 (GUI 4); qa-lanes:326-332, 346-350 | qa-lanes Real-account lanes. ER Secrets 8 keeps one sentence ("a Keychain prompt is an automation failure"); delete GUI 4. |
| D14 | `--allow-empty-timeline` / strict `timeline_items > 0` | ER:1027-1029 (GUI 5); qa-lanes:353-355; troubleshooting:179-181 | qa-lanes. Delete GUI 5; troubleshooting → link. |
| D15 | Port 5173 / process-group cleanup | ER:626-628 (Async 6); troubleshooting:150-153; environment (indirect) | troubleshooting (macOS GUI smoke). Delete ER Async 6 (or reduce to "runners clean their full process group"). |
| D16 | No `Cmd+Q` from automation | ER:1018-1019 (GUI 2); troubleshooting:154-157 | troubleshooting. |
| D17 | AppleScript `first process whose name is` + two process names | ER:1020-1022 (GUI 3); troubleshooting:144-149 | troubleshooting (and fix the name conflict, see C2). |
| D18 | `deadpool-runtime` "no reactor running" | ER:516-518 (Async 2); troubleshooting:173-174; OV Async rule 11 | OV owns; troubleshooting keeps symptom; delete ER Async 2 or keep as one line. |
| D19 | macOS Keychain Tier 2 disabled workflow | ER:373-380 (Secrets 12 tail); qa-lanes:370-385 | qa-lanes Credential-health tiers. Delete from ER. |
| D20 | Key-backup restore scope `JoinedRooms` | ER:242-245 (Secrets 11); qa-lanes:132-136; SO (8 hits) | SO. qa-lanes keeps token meaning only. |
| D21 | No fixed sleeps; one monotonic absolute deadline; final authoritative check | ER:509-515 (Async 1); ER:661-667 (Async 9 tail); qa-lanes:147-151; OV (3 hits); SO | ER Async 1 (merge Async 9 tail into it). qa-lanes keeps the scenario-specific logout note only. |
| D22 | Reliable delivery; no silent `try_send` for state-critical actions | RR:305-309; ER:635-639 (Async 9 head); ER:676-693 (Async 11) | ER Async 9/11; RR one sentence. (See C4 for the try_send inconsistency.) |
| D23 | Production `AppEffect` must not be discarded | RR:301-304; ER:672-675 (Async 10) | ER Async 10 (single statement); delete RR bullet or reduce to a pointer. |
| D24 | Task/subscription ownership; `JoinHandle` drop is detachment; React timers presentation-only | RR:235-243; ER:519-532 (Async 3); OV:778-780 | ER Async 3 (rule), OV (spec). Delete RR:235-243 except one sentence. |
| D25 | Browser fakes must not implement product semantics | RR:225-229; ER:913-919 (GUI 0) | RR (rule). ER GUI 0 keeps only "assert typed command, prove no repair, inject result" test recipe → move to verification. |
| D26 | React ephemeral-only state | RR:230-234; ER:1136-1139 (Build 7); RR:210-217 (renderer-independent core) | RR (merge 210-217 + 230-234). Delete ER Build 7 first half. |
| D27 | WebView `localStorage` legacy-migration only | RR:218-224; ER:129-134 (Secrets 10) | RR (one bullet); detailed migration mechanics → SO. |
| D28 | IME primitives, DOM owns composition, Enter fencing, latest-wins, secrets uncontrolled, lint gate | RR:68-93 (26 lines); ER:1035-1073 (39 lines); verification:321-337; AGENTS:22-23 | ER Desktop Text Input (detailed). RR → 3-line summary + link. verification keeps commands only. |
| D29 | Catalog-only product text; locale profile Rust-owned; logical CSS; CJK fitting | RR:520-564 (46 lines); ER:909-912 (GUI 0); ER:1132, 1140-1150 (Build 6-8); i18n.md:69-99; SO "Japanese / CJK and i18n" | i18n.md owns detail. RR keeps ~8 lines. Delete ER Build 6-8 and the i18n sentence in GUI 0. |
| D30 | `LocaleDisplayProfile` mirror update list | RR:545-548; SO#snapshot-and-wire-contract-mirrors | SO. |
| D31 | Read-receipt reader avatars Rust-owned | RR:388-393; ER:462-466 (Secrets 14) | SO (feature contract); ER Secrets 14 keeps redaction clause only. Delete RR bullet. |
| D32 | Stock Megolm pre-share; no readiness fence/re-share; `discard_room_key` only | RR:446-462 (17 lines); ER:287-299; OV:1868-1914 | OV "Initial outbound Megolm delivery". RR → 2 sentences; delete ER copy. |
| D33 | Persistent-store-first auth; device-ID preflight; journaled fresh-login stores | RR:429-443; ER:300-315; OV Security Model | OV/SO. RR keeps 2-3 sentences ("fail closed; never create replacement crypto"). Delete ER copy. |
| D34 | Secure Backup recovery-key reveal exception | RR:337-342, RR:369-370; ER:246-269; ER:333-338; SO:1268 | ER Secrets 11 (cited by number). Keep but halve it. |
| D35 | State-machine diagrams normative; code/diagram mismatch is a defect | RR:41-43; RR:292-294; ER:1176-1187 (Doc 5); SM:9, 88-96 | SM (already normative) + RR State-Machine Discipline one bullet. Delete ER Doc 5 (SM:96 cites "engineering-rules -> Documentation"; update that cite). |
| D36 | Doc hierarchy: AGENTS routes, durable rules promoted to RR/ER | RR:23-27; RR:762-765; ER:3-8; ER:1167-1174 (Doc 1-3); AGENTS:3-4, 53-57; docs/README.md:56-70 | RR Read Order And Authority (one paragraph). Delete ER header sentence + Doc 1-3, RR:762-765. |
| D37 | Plans subordinate to canon; amend canon first | RR:755-757; ER:1169-1171 (Doc 2); AGENTS:31; plans.md:6-9; docs/README.md | RR Canon-First Change Protocol. |
| D38 | Umbrella/child-issue discoveries synced to canon | ER:1188-1193 (Doc 6); RR:758-761 | RR Documentation And Work Records. |
| D39 | Test placement / no monolithic test files | RR:592-619; ER:1194-1200 (Doc 7) | RR. Delete ER Doc 7. |
| D40 | Worktree + build-artifact cleanup | RR:672-692; ER:1201-1205 (Doc 8); environment:132-145 (target cleanup) | environment.md (operational) — or RR 3 lines. Delete ER Doc 8. |
| D41 | Main agent integrates shared surfaces; subagent output is a draft; ≤2-3 concurrent agents | RR:651-665; verification:340-352; RR:696-700 | RR Parallel Implementation Protocol. verification keeps only the cheap-agent prompt checklist (340-344). |
| D42 | Review priorities list (canon, Rust/Tauri practice, security, contracts) | RR:710-715; verification:230-238 | RR Review And Audit; verification links. |
| D43 | Self-review of full diff incl. untracked files | RR:696-700, 725-728; verification:215-228, 262-263 | verification (procedure); RR one sentence. |
| D44 | Rule-gap findings → canon amendment proposal | RR:716-721; verification:264-265 | RR. |
| D45 | User-guide consistency before PR | RR:748-753; verification:240-247; `.github/pull_request_template.md`; `docs/help-maintenance.md:68` | RR rule (1 sentence) + verification procedure. |
| D46 | Property Display check in review | RR:104-131; verification:249-255; PR template | RR owns; verification → 2 lines. |
| D47 | Long-duration scenarios are integrated gates, not inner-loop probes | ER:1156-1163 (Build 10); verification:88-95 (near-verbatim) | verification Running focused tests. Delete ER Build 10 (or 1 line). |
| D48 | Design simplicity | ER:12-19; verification:271-279 | ER; delete verification section (move its one extra sentence into ER). |
| D49 | Real-HS QA is release/preflight only, after local lanes | RR:472-474; ER:1115-1119 (Build 4) | ER Build 4 (merge). |
| D50 | GUI automation is a smoke layer; headless browser first | RR:466-468; ER:891; ER:893-898 (GUI 0); ER:923-930; AGENTS:16-17 | RR QA Gates one bullet; ER GUI 0 keeps the macOS "unattended agents must not launch GUI" and virtual-display allowance. |
| D51 | Headless helpers must wait for seeded `data-item-id` | ER:920-922; troubleshooting:80-84 | troubleshooting (harness symptom). |
| D52 | Docs-only change gate | ER:1111-1114 (Build 3); verification:71-74 | ER Build 3 (policy) + verification (commands). OK as-is but trim verification to a link. |
| D53 | Crate ownership / platform portability | RR:202-209, 259-280; ER:1120-1131 (Build 5); OV:387-430 Platform Portability | OV owns portability; RR keeps crate-ownership bullets; delete ER Build 5 (it ends with "See Platform Portability"). |
| D54 | Secret scan gate / pre-commit hook | ER:59-61 (Secrets 7); environment:65-71 | ER (rule) + environment (commands). Fine. |
| D55 | QA profile synthetic names / `.local-secrets/qa-profiles` | ER:57-58 (Secrets 6); qa-lanes:342-350; RR:382-384 | qa-lanes. |
| D56 | Single `--lib`/`--test` guidance | verification:78-87; troubleshooting:218-222 uses `--lib` | fine (usage). |

---------------------------------------------------------------------------------------------

## 3. Contradictions / drift

| # | Issue | Side A | Side B |
| --- | --- | --- | --- |
| C1 | AGENTS.md role | ER:6-8 "AGENTS.md remains the operational how-to (permissions, install caveats, recovery steps)"; ER:1172-1174 "promoted from `AGENTS.md`" | AGENTS:3-4, 53-57 (router only; how-to lives in docs/agents); RR:23-27 |
| C2 | Dev-mode process name | ER:1021 "dev process name (`koushi-desktop`)" | troubleshooting:148 "`matrix-desktop-app`" (pre-rename; RR:568-582 says Koushi identifiers only) |
| C3 | Mention candidates source in GUI lane | qa-lanes:199 `local-composer` "mention autocomplete from `ProfileState.users`" | ER:967-977 React "must not ... scan `ProfileState.users`"; ER:426-433 mention eligibility "comes only from the room-keyed `AppState.mention_candidates`" |
| C4 | When `try_send` is allowed | ER:684-686 "`try_send` is permitted **only** for high-frequency data re-projected on the next sync (room-list snapshots)" | RR:310-315 also permits nonblocking `try_send` for background workers with owner-retained latest-wins payloads |
| C5 | Where QA token contracts live | RR:44-45 "QA scenario, token, artifact, or cleanup contracts amend the relevant `docs/qa/` document" | qa-lanes.md is the catalog the checker and `headlessAndRealQa.test.ts` enforce; qa-lanes:3 "tokens that count as evidence" |
| C6 | Review authority vs model branding | RR:46-48 canon amendments approved "by the user or the strongest available model for the agent family" | RR:701 "Independent review follows risk, not model branding"; ER:1209 "Do not maintain a separate model-tier review policy" |
| C7 | Required local merge gates vs CI | ER:1106-1110 crate tests only for `koushi-state`, `koushi-sdk`, `koushi-core` | verification:114 CI runs one feature-unified `--workspace` suite (incl. protocol, testkit, desktop DTO tests); verification:69-70 says "run the local gates in engineering rules" — a local run per ER misses tests CI gates |
| C8 | OS secret store platforms | RR:419-420 "macOS Keychain on macOS, Windows Credential Manager or DPAPI on Windows" | Linux is a shipped/QA'd target (qa-lanes Linux GUI, README Linux build); no Linux secret-store rule anywhere in RR |
| C9 | Headless DOM gate tooling | ER:899-904 "currently `test:ui-headless` ... `@wdio/tauri-service` browser mode may be adopted only after a spike" | verification:112, qa-lanes:248-260 Playwright is the gate; wdio clause is speculative, not a rule |
| C10 | Location of worker-contention rationale | `apps/desktop/playwright.config.ts:22` "AGENTS.md" | qa-lanes:255-259 |
| C11 | Doc structure | ER ends with `# Current sync contract (Issue #412)` — an H1 after ER Documentation 9 (ER:1210) | rest of ER uses `##` under one H1; the block duplicates ER Async 18 and D1 |
| C12 | `Last amended` stamps are stale | RR:9 says 2026-09-26, ER:10 says 2026-09-27; RR:39-40 requires the bump | `git log`: RR changed 2026-09-28 (#927 commits); ER changed 2026-09-28 (#927, #1034) and 2026-09-29 (824a1e15, #1060). The bump is not enforced — drop the requirement or add it to check-agents-docs |
| C13 | Lint heading scope | verification:120 `### Rust lint gate` | lines 152-213 are CI profile/cache/test-consolidation facts, so the anchor `#rust-lint-gate` (cited by ER:1108) lands on a mixed section |

---------------------------------------------------------------------------------------------

## 4. Stale content

Confirmed with `gh`: #738 CLOSED, #1034 CLOSED (migration shipped; tag v0.16.1 contains
commit 147caadf), #369 CLOSED.

| Location | Content | Proposal |
| --- | --- | --- |
| verification:281-320 (40) | "Issue #738 flake measurement" — issue closed; acceptance criteria text ("Acceptance remains pending...") obsolete | Keep 3 lines (probe workflow exists, is non-required, how to run); move the rest to history or drop. Also verification:159 "The scheduled Issue #738 probe uses the same explicit mapping". |
| ER:153-158 | "Version 1 (#1034) resets ... Release notes for the version that ships this migration must tell users..." | Shipped; drop the release-notes instruction; keep the schema-bump rule (and move it to SO). |
| ER:354-355 | "OAuth device naming ... belong to the live-session issue #369" | #369 closed; drop. |
| ER:373-380; qa-lanes:370-385 | disabled `macos-keychain-tier2.yml` — still true (file is under `.github/workflows.disabled/`) | Keep once, in qa-lanes only. |
| ER:899-904 | wdio spike speculation | Drop. |
| ER:1016 | "(a 2026-06-12 run typed a password into the username field)" | Drop anecdote. |
| ER:687-693 | "Silently dropping `SelectRoom` ... was the large-account ... regression; it passed every small-account headless lane." | Rationale → history (or drop); keep rule. |
| ER:706-709 | "#116 blocker was three stacked silent no-ops ..." | → history. |
| ER:738-741 | "#116 stayed invisible because every lane used small accounts ..." | → history. |
| ER:1210-1216 | "Current sync contract (Issue #412)" | Drop (D1). |
| RR:586-590 | "Wave 2 (#38 ... #39) confirmed that parallel Phase A work collides..." | Drop (rationale). |
| RR:179 "(#446)", RR:461 "(#794)" | issue refs inside rules | Drop refs. |
| verification:54-56 | "A 2026-07-25 change claimed a green ..." | Drop anecdote (keep rule). |
| verification:210-213 | "A 2026-07-31 PR waited about 40 minutes ..." | Drop anecdote. |
| verification:267-269 | "#328 diff surfaced a second real bug" | Drop. |
| verification:187-199 | why the standalone `-p` test steps were removed (20/11 crates, 2m10s) | Worklog; drop or history. Keep one sentence "one feature-unified workspace test run; do not add per-package `-p` steps". |
| verification:157-185 | CI cache key/mtime maintenance (29 lines) | Not agent verification; move to a comment in `ci.yml` or environment (CI section). Keep the toolchain-bump checklist (176-181) as 3 lines. |
| troubleshooting:8-28 | 2026-09-15 local DMG ad-hoc signing fix narrative | Keep 3 lines (symptom + "`build:dmg` now signs ad-hoc; ad-hoc ≠ stable identity"); narrative → history. |
| troubleshooting:31-42 | "Until 2026-07-30 that visibility check was its ONLY exit..." | Keep rule "look for paths bypassing `pushCoreEvent`"; drop narrative. |
| troubleshooting:185-193 | `avatar_demand` Tuwunel 1.7.1 receipts — "active investigation ... 2026-09-09 remaining-issues batch worklog" | Check whether still active; if resolved, → history. |
| troubleshooting:209-226 | trust-recheck coalescing contract (a Core spec, not a symptom) | → SO/OV (follow-up); keep the focused test commands. |
| troubleshooting:231-235 | `complete_new_identity_gate_for_qa` "previously returned without observing" | → history. |
| troubleshooting:148 | `matrix-desktop-app` | Fix (C2). |
| history:27-91 (65) | full probe post-mortem with tables | Compress to ~20 lines (lesson: advertised support is not proof; put the deciding token where one run shows it). |
| history:125-171 (47) | fixed 2026-06/07 Playwright flakes, spec line numbers (`basic-operations.spec.ts:81`, `:959`, `:2811`) now meaningless | Compress to ~8 lines or drop; line numbers are stale by construction. |
| history:96, verification:11-13 | Japanese words ("残り", "体制 → 修正") in English docs | Style drift; rewrite in English. |
| plans.md:215-217, 225 | "Bounded index-0 duplicate share (#510)", "Initial Megolm Olm-claim repair (#523)", "New-session Megolm readiness (#577)" listed as governing plans | Directly contradicted by current canon (RR:453-456 forbids readiness fence, duplicate pre-share, repair). Mark superseded or remove. |
| plans.md:20-59, 74-231 | ~150 shipped plans (#551/#552/#634 decomposition wave, Aug 2026) | Collapse to per-umbrella rows (one link per umbrella + "see `docs/superpowers/plans/` for children"); deduplicate identical Phase A/B links. |
| qa-lanes:390-412 | Startup latency "issue #123 Phase A" | Still a valid runnable lane (scenario exists — checker validates); trim to command + 3 bullets, details already in `docs/qa/startup-latency-observability.md`. |
| environment:52-53 | "`/tmp` is swept periodically on this host" | Host-specific; keep as one clause. |
| environment:235-243 | CodeGraph usage | Duplicates user-global CLAUDE.md instruction; keep 2 lines or drop. |

Very long examples:
- environment:291-318 signed-DMG verification shell function (28 lines) — consider moving to a
  script (`scripts/verify-signed-dmg.zsh`) and referencing it; saves ~25 lines.
- environment:193 single 400-char apt line; environment:227 single 700-char docker line
  (test-pinned; leave).
- qa-lanes:97, 98, 106, 122, 123 table cells of 80-150 words each (account_notifications,
  user_verification, directory, search_crawler_catchup, room_history_export). Move prose to
  `docs/qa/` or a footnote list; keep `Proves` to ≤15 words. Tokens column must stay
  (`redact_edit_convergence=ok`, `thread_summary_convergence=ok` are test-pinned).

---------------------------------------------------------------------------------------------

## 5. Architecture/spec content living in rule docs

Spot-read confirms OV already states most of these in detail (not only the identifiers):
OV:1405-1430 composer drafts/revisions, OV:1620-1650 scrollback epochs/`GapRepairReleased`,
OV:1790-1830 verification key-query states/`NoToken`, OV:1868 Megolm, OV:1915 Secure Backup,
OV:1941 gap repair, OV:387-430 portability, SO has per-feature sections for settings,
profiles/aliases, room management, activity, formatted rendering, credential health, CJK.

| Block (in scope) | Lines | Existing owner | Proposal |
| --- | --- | --- | --- |
| ER Secrets 10 settings file contents, density/sidebar/emoji MRU, link-preview defaults, `schema_version` migration, per-room overrides, composer drafts/`ComposerDraftRevision`/tombstones/leases (128/256 quotas), scheduled send | ER:110-236 (127) | OV:1405-1440 (drafts), SO "Settings, composer, and scheduled send" | Keep ~10 lines of privacy rule (what settings files may/may not contain; per-identity prefs need a privacy-reviewed store; drafts are encrypted account-scoped data). Delete draft/lease mechanics (OV owns). `schema_version` rule not found in OV/SO → **move to SO (follow-up)**. |
| ER Secrets 11 E2EE: recovery-key reveal, backup health, Megolm, persistent-store-first auth, quarantine until Verified, SAS handles, provisional-device cleanup | ER:237-359 (123) | OV Security Model 1671-1940; SO "E2EE trust", "Device-to-device verification and device cleanup" | Keep ~25 lines: kind-only diagnostics, recovery-key reveal exception (cited as "rule 11"), `device_cleanup` diagnostic field allowlist, "`local_recovery_key` is never a recovery key". Delete the rest (OV/SO own). |
| ER Secrets 9 native attention mapping, thread-attention fields, sound policy | ER:66-109 (44) | SO "Threads and attention", OV Desktop Attention Surfaces (1253) | Keep 8 lines (notification/badge/title private-data minimization; no permission prompt except explicit action). Delete field-level React rules. |
| ER Secrets 14 profile/avatar cache, aliases, label resolution order, alias dialog | ER:397-466 (70) | SO "Profiles and local aliases" (798-885), i18n.md:91-99 | Keep ~10 lines (redaction of names/MXC/avatars; aliases never leave the device; bounded in-memory thumbnail cache; no plaintext cache). Delete label-resolution and dialog rules. |
| ER Async 4 scrollback epochs, gap repair, live-edge repair, SDK response sequence | ER:533-620 (88) | OV "Timeline Viewport And Scrollback" 1512-1670, OV "Room Timeline Gap Repair" 1941-2019, SM | Replace with 3 lines: "no polling / fixed-delay retries / scroll latches; every blocker-removing transition schedules re-evaluation; see OV". |
| ER Async 16 verified-session sync ownership, to-device pending FIFO (bound 32), key-query claims, multi-stage QA ownership, `NoToken` | ER:742-862 (121) | OV:1790-1830+, SM:4065, 4146 | Keep ~8 lines of rule (one sync owner at a time; manual `SyncOnce` only with no owner; no fixed sleeps/blind resends; redact identifiers). The multi-stage QA participant-ownership paragraph (ER:837-853) is QA-harness policy with no other owner → move to qa-lanes or `docs/qa/` (in-scope move). |
| ER Async 9 shutdown drain / owner-polled futures / media enqueue ordering | ER:640-671 | OV Async Design Rules | Keep first 6 lines + waiter rule; delete the rest. |
| ER Async 14 five-second shutdown deadline | ER:727-734 | OV | Delete (spec). |
| ER Async 19 navigation latest-value slot | ER:879-887 | OV/SO "Room and Space navigation intent" | Delete or 2 lines. |
| ER GUI 0 feature contracts: room-list sections, display prefs (`code_block_wrap`, `hide_redacted`), image compression, formatted HTML, mention GUI, room-management, Activity, Settings/Security, tooltips, px tokens | ER:931-1014 (84) | SO "Rooms, tags, and the sidebar", "Formatted message rendering", "Room management and moderation", "Activity", "Credential health", "GUI presentation contracts" (1472) | Delete from ER; keep only the generic test-method rules (assert typed command; don't let fakes repair; pin discoveries with headless tests). Tooltip/px-token rules → SO "GUI presentation contracts" if absent (follow-up). |
| ER Async 5 `timeline_items` title token semantics | ER:621-625 | — | Move to qa-lanes Linux GUI lane notes. |
| RR:174-185 `VectorDiff` accumulator authority | RR:174-185 (12) | OV (3 `VectorDiff` hits) | Keep as a 2-line rule (it is a durable prohibition), drop the example. |
| RR:316-326 pane-level thread attention | RR:316-326 (11) | SO "Threads and attention" | Delete from RR. |
| RR:388-393 receipt avatars; RR:394-414 search-index rules | RR:388-414 | OV Security Model "Search" | Search-index rules are genuine security prohibitions → keep, but compress 21 → ~10 lines. Delete receipt-avatar bullet. |
| RR:429-462 auth/device-ID/journal/Megolm | RR:429-462 (34) | OV Security Model | Keep 4 lines. |
| RR:249-258 Tauri command registration explanation | RR:249-258 (10) | checker rule `desktop.commands.tauri_command_registration` (`scripts/check-rust-test-structure.mjs`; the former Rust test was removed by #753) | Keep 2 lines. |
| troubleshooting:209-226 trust-recheck coalescing contract | 18 | none found | Move to SO/OV (follow-up); keep test commands. |
| qa-lanes:77-85 timeline_stress oracle vs runtime contract | 9 | — | Fine where it is (QA oracle). |

---------------------------------------------------------------------------------------------

## 6. Top 20 verbose rules by savings (proposed rewording)

Savings are approximate lines removed from in-scope files.

1. **ER Secrets 10 (127 → ~10, −117).** "Device-local settings files hold only typed,
   non-secret preferences; never credentials, keys, session JSON, Matrix identifiers, content,
   queries, or raw errors. Preferences keyed by or containing Matrix identifiers use a
   privacy-reviewed account-scoped encrypted store. A default change that saved files must not
   inherit requires a `schema_version` bump and a `SettingsStore::load` migration. Unsent and
   scheduled message content is account-scoped encrypted data and never crosses to the WebView
   except the active/selected projection; mechanics: OV Composer drafts, SO Settings."
2. **ER Async 16 (121 → ~8, −113).** "Each SDK client has exactly one sync owner at a time
   (restricted, continuous, or a manual `SyncOnce` when no owner exists); enforce at both routing
   boundaries. Verification-only sync never persists its filtered cursor. Verification transport
   is at-least-once with `(sender, flow_id)` identity; idempotence and conflict cancellation live
   in `AccountActor`. Never log request/flow/device identifiers. Detailed state machine: OV
   Security Model, SM." Move the multi-stage QA participant-ownership paragraph (17 lines) to
   qa-lanes.
3. **ER Secrets 11 (123 → ~25, −98).** Keep numbered item; reduce to: kind-only diagnostics;
   recovery-key reveal exception (existing 20 lines → 8); backup health does not block ordinary
   encrypted traffic; `device_cleanup` diagnostic allowlist; pointer to OV for auth/admission/
   Megolm.
4. **ER Async 4 (88 → 3, −85).** "Scrollback and gap repair are event-driven: no polling,
   fixed-delay retries, or scroll latches; every transition that can remove a blocker schedules
   the single evaluator; programmatic scroll echoes are not user demand. Contract: OV Timeline
   Viewport And Scrollback, OV Room Timeline Gap Repair."
5. **ER GUI 0 feature contracts (84 → ~6, −78).** "GUI tests render Rust-shaped snapshots and
   assert the typed command, then inject the Rust-owned result; React never sorts, filters,
   infers, or repairs product state after a click. Per-feature boundaries are in SO."
6. **ER Secrets 14 (70 → ~10, −60).** "Display names, avatar MXC URIs/bytes, thumbnail paths and
   local aliases are account-scoped sensitive data: redacted in Debug/logs/QA/issues. Aliases
   never leave the device as Matrix data. Persistent avatar bytes live only in the encrypted SDK
   media store; Koushi keeps only a bounded in-memory renderable cache; plaintext caches and
   `file://` URLs are prohibited. Label resolution: SO Profiles and local aliases."
7. **plans.md Feature areas (161 → ~60, −100).** One link per plan; collapse the #551 (≈45
   rows), #552 (≈20), #634/#649-651/#641 browser-fake (≈15) waves into one row each pointing
   at the umbrella's first plan; mark #510/#523/#577 superseded.
8. **verification CI internals (152-213: 62 → ~10, −52).** "Required Rust jobs use `[profile.ci]`.
   A toolchain bump edits `rust-toolchain.toml`, every `dtolnay/rust-toolchain@` ref and step
   name in `ci.yml`/`release-desktop.yml`, and the `rust-<version>` SDK cache keys. When the
   cargo command set changes the dependency feature graph, bump the Rust job `shared-key`.
   Workspace tests run once as one feature-unified `--workspace` invocation."
9. **RR Key Management + Megolm (48 → ~14, −34).** Keep unlock-secret/derivation/fail-closed
   bullets; replace RR:429-462 with "Crypto-capable clients are built on their encrypted
   persistent store before authentication; saved device IDs are reused only after a non-creating
   preflight; any mismatch fails closed and never creates replacement crypto. Outbound Megolm uses
   stock Element X pre-share only (OV Initial outbound Megolm delivery)."
10. **ER Documentation section (45 → ~4, −41).** Replace 9 restated rules with: "Documentation,
    concurrency, worktree, and review rules are in RR; state-machine diagrams in SM."
    (Update SM:96 cite.)
11. **history Sync-probe + flake history (112 → ~30, −82).** Keep the retired table and one
    paragraph per post-mortem with the lesson.
12. **verification #738 section (40 → 4, −36).**
13. **RR IME section (26 → 4, −22).** "All user-editable text uses the shared IME-safe
    primitives; the DOM owns active composition and unacknowledged drafts; candidate-confirmation
    Enter is never a product command; secrets stay DOM-owned. Details and the lint gate: ER
    Desktop Text Input And IME Safety."
14. **RR Localization (46 → ~10, −36).** "Product text goes through the message catalog (no
    hardcoded prose in React, Rust errors, Tauri commands, or UI-modelling tests); Core returns
    kinds/codes, not prose. Locale/display behavior and CJK normalization are Rust-owned; React
    consumes `LocaleDisplayProfile`. Layout uses logical CSS properties; CJK fitting is CSS-only.
    New text ships catalog entries plus a pseudo-locale test. Details: i18n.md."
15. **RR Architecture: merge duplicated headless-first (164-168 + 193-201: 14 → 5, −9) and
    compress crate-ownership bullets 202-280 (79 → ~35, −44).**
16. **RR Concurrent Work (107 → ~50, −57).** Drop 586-590; merge Shared Hot Files tips into 3
    lines; Worktree cleanup (22 → 5): "Remove merged worktrees promptly with their unshared
    `target/`, `node_modules/.vite/`, and `node_modules/`; never delete a shared
    `CARGO_TARGET_DIR`; confirm with `git worktree list`."
17. **ER Async 9 (37 → ~12, −25).** "State-critical actor actions are reliable (await, retry
    via owner, or emit a correlated failure). Accepted user intents are owned by a component that
    outlives every replaceable presentation actor; shutdown drains accepted futures under one
    absolute deadline before acknowledging teardown. Snapshot+stream waiters check the predicate
    first and once more at the deadline." (Merge absolute-deadline clause into Async 1.)
18. **ER Secrets 9 (44 → ~8, −36).**
19. **qa-lanes Real-account safety rules (320-355: 36 → ~15, −21)** once D8-D14 point to ER.
20. **troubleshooting narrative trims (signing 21→4, harness bullet 1 12→4, trust-recheck
    contract 18→5, gate history 5→0: −43).**

Also cheap: ER Build 5-8 (−25, OV owns), ER Build 10 (−7), ER GUI rules 1-5 (−15, operational
copies), ER Async 2/5/6/7/8 (−18), ER Async 14/19 (−15), ER header (−4), ER stray H1 (−7).

---------------------------------------------------------------------------------------------

## 7. Size estimates and target structure

### Estimates (ranges; tied to cuts above)
| File | Now | Target | Main cuts |
| --- | --- | --- | --- |
| AGENTS.md | 66 | 60-66 | keep; maybe drop the sync-flag detail to a link |
| REPOSITORY_RULES.md | 774 | 380-450 | D2, D6, D11, D24-D29, D31-D33, D36-D46 owner moves; §6 #9, #13-16; stale RR:586-590 |
| engineering-rules.md | 1216 | 450-550 | §5/§6 #1-6, #10, #17-18; D8-D19, D47-D53; stray H1 |
| verification.md | 357 | 190-220 | #738, CI internals, anecdotes, D42/D48 duplicates |
| environment.md | 332 | 260-285 | docker block frozen; DMG function → script (−25); CodeGraph; debug-info prose |
| qa-lanes.md | 412 | 320-350 | long table cells → docs/qa; real-account safety → links; startup latency trim; gains ~25 lines of QA-harness policy from ER |
| troubleshooting.md | 235 | 170-190 | narratives → history; gains ER GUI 2-3/Async 6 one-liners (net small) |
| history.md | 171 | 90-120 | compress post-mortems; gains a few narratives from ER/verification/troubleshooting |
| plans.md | 234 (42 KB) | 90-130 (~15 KB) | collapse waves, dedupe columns |
| **Total** | **3 797** | **~2 000-2 300** | ~40-45% reduction |

### Proposed ownership (AGENTS.md stays the router; no new topic files needed)
- **AGENTS.md** — router + 8 essential contracts (unchanged role).
- **REPOSITORY_RULES.md** — short durable rules only, one bullet per rule, no mechanisms:
  authority/canon-first/root-cause; layer ownership (Rust owns semantics, Tauri adapter, React
  ephemeral, no browser-fake semantics, crate ownership); state-machine discipline (short);
  security & privacy classes + key management (short); QA/test data rules; UI rules (property
  display, macOS safe area, action labels, IME summary, i18n summary); product identity;
  collaboration (test placement, hot files, parallel protocol, worktree cleanup, review,
  issue language, docs/work records, licensing — absorb plans.md:232-234 THIRD_PARTY_NOTICES rule).
  Keep anchors in 0.6.
- **engineering-rules.md** — detailed *policy* that is not a feature spec: Design Simplicity;
  Secrets 1-15 slimmed in place (numbers stable); Logging; Async & Runtime generic rules
  (1, 3, 9-13, 15, 17, 18 slimmed; 4/16/19 reduced to pointers); GUI Automation (headless
  default, macOS attended-only, virtual display allowed, test-method rules); IME detail;
  Build/QA gates (test-pinned heading + item 1). Drop Documentation section and stray H1.
- **verification.md** — how to prove a change: verify-first, diagnostics, exit status, stages,
  focused tests, long-scenario discipline (single owner, D47), CI job table + lint commands
  (lint heading contains only lint), diff self-review procedure, IME commands, delegation checklist.
- **environment.md** — machine setup + CI maintenance notes (toolchain bump checklist, cache
  keys) + worktree/target cleanup commands.
- **qa-lanes.md** — every lane, command, token (tested strings stay), real-account safety
  (single operational owner of D10-D14, D55), credential tiers, QA harness ownership rules moved
  from ER Async 16.
- **troubleshooting.md** — symptoms → fixes only (owner of D15-D18).
- **history.md** — quarantine + compressed post-mortems + rationale moved out of ER/RR/verification.
- **plans.md** — compact index.
- **Follow-ups outside this edit scope** (need SO/OV owners): settings `schema_version` migration
  rule; trust-recheck coalescing contract; tooltip/px-token GUI rules if absent from SO
  "GUI presentation contracts"; OV already owns scrollback/gap repair/verification/Megolm/drafts.

### Required mechanical updates in the same PR
- Keep ER `## Build, Dependencies, QA Gates` + item `1.` text; keep ER Secrets numbers 2 and 11
  (or update 7 citations in 0.3).
- Fix `REPOSITORY_RULES L124-128` comments (0.4) → "REPOSITORY_RULES State-Machine Discipline".
- Update SM:96 if ER Documentation is removed; update `docs/README.md:24-70` descriptions;
  update `apps/desktop/playwright.config.ts:22` ("AGENTS.md" → qa-lanes.md).
- If the GUI scenario table leaves qa-lanes.md, update `catalog` in check-agents-docs.mjs.
- Run `node scripts/check-agents-docs.mjs`, `node --test scripts/build-structure-contract.test.mjs`,
  the two vitest files (`src/scripts/linuxGuiQa.test.ts`, `src/scripts/headlessAndRealQa.test.ts`),
  `node scripts/user-help.mjs --check` (if help links change), and `git diff --check`.
