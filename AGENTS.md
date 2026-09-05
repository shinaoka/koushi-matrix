# Agent Notes

Operational entry point. Durable rules live in [REPOSITORY_RULES.md](REPOSITORY_RULES.md);
this file routes to the relevant contracts and operational instructions.

## Essential contracts

- Rust owns product state and Matrix semantics. React renders Rust DTOs and
  dispatches typed commands; Tauri is a transport/platform adapter.
- The runtime uses only Element X-compatible Simplified Sliding Sync. Local QA
  accepts `--server=tuwunel`, `--server=synapse`, or `--server=both`; Linux GUI
  QA accepts `--server=tuwunel` only. Retired backend-selection commands are
  rejected; see [history](docs/agents/history.md#retired-qa-vocabulary).
- Reproduce behavior bugs with a failing headless check before fixing them.
  Manual GUI inspection is confirmation, not correctness evidence.
- Never put secrets or real-account private data in logs, fixtures, screenshots,
  or QA artifacts. Use disposable local homeservers for destructive GUI QA.
- Use the checked-out `vendor/matrix-rust-sdk` as the authoritative SDK source.
  Prefer existing Element X behavior and public SDK APIs over fork extensions.
- User-editable text uses the shared IME-safe primitives; product text uses the
  message catalog. Read the relevant rules before changing either surface.
- Voice/video calls and recorded voice messages remain deferred. Do not open
  feature issues for them without explicit user approval to revisit scope.

## Read by task

Before changing behavior, read the root rules and the applicable sections below.
Read relevant sections, not every linked document in full. Resolve conflicting
contracts before changing affected behavior; plans never override the canon.

| Task | Read |
| --- | --- |
| Architecture or layer ownership | [overview](docs/architecture/overview.md) |
| State transitions or guards | [state machines](docs/architecture/state-machine.md) |
| Locale, product text, or layout | [i18n](docs/architecture/i18n.md) |
| Security, runtime, or gate policy | [engineering rules](docs/policies/engineering-rules.md) |
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
