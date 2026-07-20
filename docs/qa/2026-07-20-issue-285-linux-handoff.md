# Issue #285 Linux Development Handoff

## Objective

Bring GitHub issue [#285](https://github.com/shinaoka/koushi-matrix/issues/285)
to PR-ready state on Linux. The production display-diff projection fix is
implemented. Remaining work is to shorten the SendQueue feedback loop, run the
final evidence lanes once, perform a final review, and stop immediately before
creating the PR.

## Authoritative Git state

- Repository: `shinaoka/koushi-matrix`
- Working branch: `codex/issue-285-display-diff-projection`
- Pre-handoff tip: `58dd02f6c423ee0950cac71e1a6e7c1be5138e25`
- Original merge base with `origin/main`:
  `46c5ecff9aafb3e4b5f10e9307bde21e47929ab5`
- Required Matrix SDK submodule gitlink:
  `55b68cc63de5df70a6a39eed02d1e87343e00269`
- PR: not created

The handoff document is committed after the pre-handoff tip, so the checked-out
branch `HEAD` is authoritative. Do not reset this branch to `origin/main`, and
do not substitute the submodule's remote branch tip for the recorded gitlink.

After the branch has been pushed from the source machine, bootstrap Linux with:

```bash
git clone https://github.com/shinaoka/koushi-matrix.git
cd koushi-matrix
git fetch origin codex/issue-285-display-diff-projection
git switch --track origin/codex/issue-285-display-diff-projection
git submodule sync --recursive
git submodule update --init --recursive
test "$(git -C vendor/matrix-rust-sdk rev-parse HEAD)" = \
  55b68cc63de5df70a6a39eed02d1e87343e00269
git status --short --branch
```

Expected: the submodule assertion succeeds and the worktree is clean.

## What is complete

The branch contains the Core-owned canonical-to-display projection for SDK
timeline batches, bounded display membership/mirror bookkeeping, release-build
index validation and Reset fallback, generation/restore/submission fencing,
desktop TimelineStore contract coverage, and private-data-free diagnostics.

The final independent production review found no blocking issue. Complexity was
accepted as expected projection overhead
`O(W + B log(W+B) + D)`, with `W <= 120` and `D = O(W)`; canonical `Vec`
mutation costs are separate and must not be described as part of that bound.

The QA harness now uses ordered shutdown before every same-data-directory
reopen relevant to this work:

```text
drop all CoreConnection handles
-> await CoreRuntime::shutdown()
-> reopen the same data directory
```

The latest lifecycle commit, `5d99fef`, was independently reviewed with no
findings. Its short verification was:

- ordered-reopen focused regression: 1 passed;
- complete `headless-core-qa` Rust test suite: 71 passed;
- QA binary `cargo check`: passed;
- `cargo fmt --all -- --check`: passed;
- `git diff --check`: passed.

The detailed implementation/evidence log is
`.superpowers/sdd/task-1-report.md`. The original implementation plan is
`docs/superpowers/plans/2026-07-19-issue-285-display-diff-projection.md`.

## Why the last long run did not finish

Two Conduit runs reached all of the following:

```text
send_fail=ok
resend=ok
fifo=ok
cancel_send=ok
unsent_restart=ok
sync_a=stopped
```

They then stopped before restored-session output. The `sync_a=stopped` token
came from `cleanup_after_full_flow`, not from the dedicated SendQueue restart.
That common path still used detached `drop(runtime) + 500ms sleep` before
reopening the same store. Commit `5d99fef` replaced it, the equivalent `All`
path, and cache-restore Connect 1 with ordered shutdown.

No long Conduit run has been executed after `5d99fef`. Therefore the branch is
not yet PR-ready, even though the static and short suites are green.

## Immediate next work: fast SendQueue feedback

The approved design is
`docs/superpowers/specs/2026-07-20-fast-send-queue-qa-design.md`.

Implement it in this order:

1. Add a deterministic fast lane using the production SendQueue state machine
   while controlling only network outcomes, retry timing, and time. It must
   cover offline local echo, retry/FIFO, cancel, ordered shutdown, same-store
   restore, authoritative completion, and duplicate pending/remote-row absence.
   Target: 60 seconds or less; no real homeserver and no blind sleeps.
2. Add a focused Conduit route for `send_queue`. Bootstrap only user A, retain
   its recovery secret, shut down the bootstrap runtime, and invoke the existing
   standalone `run_send_queue_stage`. Do not run user B or the generic
   room/space/two-user timeline/edit/paginate flow. Target: 3-6 minutes.
3. Preserve the full E2E as the PR gate. Do not weaken its cross-stage coverage.

Use strict RED -> GREEN. During implementation run only the failing focused
test and the new fast lane. Do not run a long homeserver lane after every edit.

## Verification sequence on Linux

First install the Ubuntu 24.04 dependencies documented in `AGENTS.md` under
“Linux GUI QA Container” or use the committed Docker image. The repository Rust
toolchain is pinned by `rust-toolchain.toml`; do not override it with a stale
global compiler.

Install the desktop JavaScript dependencies once:

```bash
npm --prefix apps/desktop ci
```

During implementation, run the exact fast SendQueue invocation plus these
short gates:

```bash
cargo test -p koushi-core --test send_queue_fast fast_send_queue_feedback_runs_production_runtime_without_homeserver -- --exact --nocapture
cargo test -p koushi-core --test send_queue_fast
cargo test -p koushi-core --features qa-bin --bin headless-core-qa
cargo test -p koushi-core --lib display_projection
npm --prefix apps/desktop test -- src/domain/timelineStore.test.ts --reporter=dot
cargo check -p koushi-core --features qa-bin --bin headless-core-qa
npm --prefix apps/desktop run typecheck
cargo fmt --all -- --check
git diff --check
```

After implementation and review are complete, run the focused real lane once:

```bash
node scripts/desktop-headless-local-qa.mjs \
  --run \
  --server=conduit \
  --core \
  --core-backend=probed \
  --scenario=send_queue \
  --timeout-ms=600000 \
  --cargo-profile=release
```

Then run the full Conduit E2E once:

```bash
node scripts/desktop-headless-local-qa.mjs \
  --run \
  --server=conduit \
  --core \
  --core-backend=probed \
  --scenario=all \
  --timeout-ms=1200000 \
  --cargo-profile=release
```

Preserve the artifact directory and exact stdout tokens in
`.superpowers/sdd/task-1-report.md`. Confirm that no
`display_projection_reset_fallback` appears in the ordinary lane.

## Linux operational notes

- Preferred local homeserver search path:
  `/tmp/koushi-desktop-local-qa-bin`.
- Export it before local homeserver QA:

  ```bash
  export PATH=/tmp/koushi-desktop-local-qa-bin:$PATH
  conduit --version
  ```

- If building Conduit/Tuwunel from source, set
  `RUMA_UNSTABLE_EXHAUSTIVE_TYPES=1` as required by `AGENTS.md`.
- Use file-backed QA credentials only. Never allow unattended QA to touch the
  desktop OS keychain.
- Do not copy `.local-secrets`, passwords, access tokens, recovery secrets, or
  macOS artifact directories into Git. The QA runner creates disposable local
  credentials and artifacts.
- Keep diagnostics synthetic and private-data-free.
- Follow the repository rule: finish coherent implementation before running
  expensive integration/GUI lanes; do not use a long lane as the debugging
  loop.

## PR-ready exit checklist

- [ ] Fast SendQueue lane completes in 60 seconds or less and is documented.
- [ ] Focused `send_queue` route skips the generic two-user timeline flow.
- [ ] Fast and short suites are green from the current branch HEAD.
- [ ] Focused Conduit SendQueue lane is green after `5d99fef`.
- [ ] Full Conduit E2E is green once after all implementation changes.
- [ ] Ordinary evidence contains no display-projection Reset fallback.
- [ ] Independent final diff review has no unresolved findings.
- [ ] Worktree and submodule are clean and pinned to the recorded revisions.
- [ ] Branch is pushed for review, but no PR is created until explicitly
      requested.
