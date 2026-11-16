# Agent Notes

Operational entry point and router. Read
[REPOSITORY_RULES.md](REPOSITORY_RULES.md) on every task; read other documents
only through the task table below.

## Essential contracts

- Write GitHub issues in English; see the
  [issue language policy](REPOSITORY_RULES.md#github-issue-language). Close
  issues whose only remaining items are manual checks; see
  [issue closure](REPOSITORY_RULES.md#github-issue-closure).
- Rust owns product state and Matrix semantics. React renders Rust DTOs and
  dispatches typed commands; Tauri is a transport/platform adapter.
- The runtime uses only Element X-compatible Simplified Sliding Sync. Local QA
  accepts `--server=tuwunel`, `--server=synapse`, or `--server=both`; Linux GUI
  QA accepts `--server=tuwunel` only. Retired backend-selection commands are
  rejected; see [history](docs/agents/history.md#retired-qa-vocabulary).
- Reproduce behavior bugs with a failing headless check before fixing them.
  Manual GUI inspection is confirmation, not correctness evidence.
- Never put secrets or real-account private data in logs, fixtures, screenshots,
  or QA artifacts. All automated QA uses disposable local homeservers; external
  homeserver QA is retired.
- Use the checked-out `vendor/matrix-rust-sdk` as the authoritative SDK source.
  Prefer existing Element X behavior and public SDK APIs over fork extensions.
- User-editable text uses the shared IME-safe primitives; product text uses the
  message catalog. Read the relevant rules before changing either surface.
- Voice/video calls and recorded voice messages remain deferred. Do not open
  feature issues for them without explicit user approval to revisit scope.

## Rust builds

- Run Rust build commands through kache with `KACHE_EXPLAIN_MISS=1` so a cache
  miss records dependency-level digests for `kache why-miss`, for example
  `KACHE_EXPLAIN_MISS=1 cargo build`. Keep it scoped to the invocation; see
  [environment](docs/agents/environment.md#kache-miss-diagnostics).
- Never use `env!("CARGO_MANIFEST_DIR")` in test code. It records the absolute
  checkout path in dep-info, so kache rebuilds the crate in every checkout.
  For compile-time fixtures, use `include_bytes!`/`include_str!` with a path
  relative to the source file. For run-time paths, read
  `std::env::var_os("CARGO_MANIFEST_DIR")`; `cargo test` sets it.

## Read by task

Read only the rows that match the task, and only their relevant sections.
Resolve conflicting contracts before changing affected behavior; plans never
override the canon.

| Task | Read |
| --- | --- |
| Architecture or layer ownership | [overview](docs/architecture/overview.md) |
| State transitions or guards | [state machines](docs/architecture/state-machine.md) |
| Locale, product text, or layout | [i18n](docs/architecture/i18n.md) |
| Detailed policy: security, runtime, UI, tests, concurrency, gates | [engineering rules](docs/policies/engineering-rules.md) |
| Setup, SDK checkout, builds, or cleanup | [environment](docs/agents/environment.md) |
| Fixing behavior, testing, or reviewing | [verification](docs/agents/verification.md) |
| Running QA | [QA lanes](docs/agents/qa-lanes.md) |
| DTO mirrors or feature ownership | [state ownership](docs/agents/state-ownership.md) |
| Known failures | [troubleshooting](docs/agents/troubleshooting.md) |
| Retired behavior and commands | [history](docs/agents/history.md) |
| Relevant implementation plans | [plan index](docs/agents/plans.md) |

Bound investigative commands to 120 seconds; documented full gates may use a
longer explicit deadline. On timeout, stop and triage surviving processes and
logs before retrying a narrower command.

## Keeping these notes maintainable

Keep this file a router, not a feature specification or worklog. Put setup and
failure notes in the matching operational topic; durable rules belong in the
root rules or engineering policy. Each fact has one authoritative owner: link
instead of copying. Correct changed instructions in place; quarantine retired
commands in history rather than leaving runnable examples elsewhere.

Every `docs/agents/*.md` file must remain linked here. Add a topic only when no
existing topic fits. The checker enforces a 240-line entry budget, topic-file
links, retired flags in fenced commands, and known QA scenario names; it does
not prove semantic consistency or validate every link in the repository.

```bash
node scripts/check-agents-docs.mjs
```
