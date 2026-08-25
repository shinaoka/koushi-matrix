# Agent Notes

Operational entry file for agents and QA automation in this local environment.
This file is loaded into every session, so it stays small: it holds the contracts
you must know before acting, and an index to the detail. Durable repository rules
do not live here.

**Load only what your task needs.** The detail lives in [docs/agents/](docs/agents/)
and is meant to be opened one file at a time, not read whole.

## Current runtime and QA contract

The runtime is one Element X-compatible Simplified Sliding Sync engine. Legacy
`/sync`, backend probing, backend forcing, and fallback selection are removed
(#412, #417). Therefore:

- Local QA accepts `--server=tuwunel`, `--server=synapse`, or `--server=both`.
  Linux GUI lanes accept `--server=tuwunel` only.
- `--core-backend`, `KOUSHI_QA_FORCE_SYNC_BACKEND`, `conduit`, and
  `timeline_legacy_*` scenarios are **rejected by the runners**. Older plans,
  issues, and transcripts still contain them; they are artifacts, not commands.
  See [docs/agents/history.md](docs/agents/history.md#retired-qa-vocabulary).

## Read order

1. [REPOSITORY_RULES.md](REPOSITORY_RULES.md) — root durable rules for this
   repository.
2. [docs/architecture/overview.md](docs/architecture/overview.md) — long-term
   architecture, layer ownership, runtime, security, and QA model.
3. [docs/architecture/state-machine.md](docs/architecture/state-machine.md) —
   normative reducer state-machine diagrams and guard notes.
4. [docs/architecture/i18n.md](docs/architecture/i18n.md) — Rust-owned
   locale/display profile, catalog, pseudo-locale, RTL, and i18n gates.
5. [docs/policies/engineering-rules.md](docs/policies/engineering-rules.md) —
   detailed policy extension for secrets, logging, QA automation, and gates.
6. The relevant dated implementation plan — indexed in
   [docs/agents/plans.md](docs/agents/plans.md).

## Where the detail lives

| Open this | When |
| --- | --- |
| [docs/agents/environment.md](docs/agents/environment.md) | Setting up: SDK submodule, local homeserver binaries and `PATH`, local gates, debug-profile policy/reuse, target cleanup, Linux GUI container, CodeGraph, signed macOS DMG |
| [docs/agents/verification.md](docs/agents/verification.md) | Before fixing anything: verify-first discipline, focused-test invocation, exit-status rules, what CI gates, diff self-review, agent delegation, IME input checks |
| [docs/agents/qa-lanes.md](docs/agents/qa-lanes.md) | Running a lane: command shapes, every core and GUI scenario with its evidence tokens, browser-headless, real-account, startup latency |
| [docs/agents/state-ownership.md](docs/agents/state-ownership.md) | Touching a feature area: who owns which state, the snapshot/DTO mirror checklist, and the per-area boundary rules |
| [docs/agents/troubleshooting.md](docs/agents/troubleshooting.md) | A lane or harness is failing and you want the known cause |
| [docs/agents/history.md](docs/agents/history.md) | Understanding a superseded contract, or decoding an old plan |
| [docs/agents/plans.md](docs/agents/plans.md) | Finding the dated plan that governs an area |

Two rules from that tree are load-bearing often enough to state here:

- **Verify first.** Build the reproducible headless check (体制) BEFORE the fix
  and let the same check turn green as the proof: 体制 → 修正, never the reverse.
  Manual or visual GUI inspection is a confirmation, never the gate.
- **Rust owns product state.** React only renders Rust DTOs and dispatches typed
  commands; it never owns or repairs Matrix semantics. Avoid unnecessary SDK
  extensions: prefer an equivalent existing Element X approach and SDK path.

## Quick start

```bash
# after clone or worktree switch
git config core.hooksPath .githooks
git submodule update --init --recursive vendor/matrix-rust-sdk
node scripts/check-sdk-submodule.mjs

# local homeserver binaries on PATH
export PATH=/tmp/koushi-desktop-local-qa-bin:$PATH && tuwunel --version

# fast gates
npm --prefix apps/desktop run typecheck
npm --prefix apps/desktop run lint
cargo test -p koushi-state --lib
```

Full Rust debug information is exceptional: prefer line tables for local tests/QA, reserve full symbols for debugger work, and use stripped builds for distribution. The profile and disposable-`target/` cleanup procedure is in [docs/agents/environment.md](docs/agents/environment.md#rust-debug-information-and-target-cleanup).

## Out of scope (deferred)

Real-time and recorded audio/video are deferred and intentionally absent from the
product roadmap:

- Voice / video calls — MatrixRTC / Element Call (MSC4143, MSC3401), including
  1:1 and group calling.
- Voice messages — recorded audio clips with waveform record/playback UI
  (MSC3245).

This is a conscious "not yet" decision, not a permanent exclusion; revisit before
GA. Do not open feature issues for these without re-deciding scope here.

## Keeping these notes maintainable

This tree replaced a single 2300-line file that no longer fit in a session
budget. Preserve the hierarchy when you add to it.

**Where a new note goes:**

| The note is | Put it in |
| --- | --- |
| A machine setup step, toolchain pin, or container recipe | `docs/agents/environment.md` |
| A rule about how to prove something | `docs/agents/verification.md` |
| A new scenario, command shape, or evidence token | `docs/agents/qa-lanes.md` |
| "X is Rust-owned, React may only …" for a feature area | `docs/agents/state-ownership.md` |
| A symptom and its cause, for a lane that failed | `docs/agents/troubleshooting.md` |
| A record of something now removed or superseded | `docs/agents/history.md` |
| A durable rule that binds all future work | `REPOSITORY_RULES.md` or `docs/policies/engineering-rules.md` — not this tree |

**Rules for the tree:**

1. **Do not grow this file.** It is capped at 240 lines and every session pays
   for it. A new operational note belongs in the matching topic file above. If
   the note genuinely applies to every task before any file is opened, it may go
   here — and then something else must leave.
2. **Add a new topic file only when an existing one does not fit**, and link it
   from the table above in the same change. An unlinked file is invisible.
3. **Correct in place; do not append a contradiction.** When behavior changes,
   edit the affected note. If a command or contract is retired, move it to
   `history.md` with what replaced it, so the retired form stays findable but
   never looks runnable. A stale command that still reads as valid is worse than
   no note: 27 unrunnable command examples accumulated in the old file this way.
4. **One owner per fact.** Cross-link instead of restating. The same DTO-mirror
   rule was duplicated across eight sections of the old file and drifted apart.
5. **When an operational note hardens into a durable rule**, promote it to
   `REPOSITORY_RULES.md` or `docs/policies/engineering-rules.md` and keep only
   the local how-to detail here.

The structure is enforced, not just requested. `npm --prefix apps/desktop run
lint` runs `scripts/check-agents-docs.mjs`, which fails when this file exceeds
its budget, when a `docs/agents/` file is not linked here, or when a retired flag
or unknown `--scenario=` name appears outside `history.md`:

```bash
node scripts/check-agents-docs.mjs
```
