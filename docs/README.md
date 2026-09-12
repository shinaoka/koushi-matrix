# Documentation Map

This directory separates long-term normative documents from dated, short-term
working documents. When documents disagree, the normative documents win, and
the conflict must be resolved by amending one of them explicitly.

## User help

- [User guide](help/README.md) — usage instructions for people and AI assistants,
  maintained alongside the product code. Use the matching release tag for
  installed versions.
- [Help maintenance](help-maintenance.md) — edit the shared Markdown source,
  regenerate llms.txt, and check links. Design proposals and worklogs do not
  establish which features a released version supports.

## Normative (long-term, kept current)

These documents describe what the product and codebase must look like. They are
not dated snapshots; they are amended in place through review.

- [architecture/overview.md](architecture/overview.md) — the overall
  architecture blueprint: layers, crate boundaries, runtime model, async design
  rules, security model, QA model.
- [`../REPOSITORY_RULES.md`](../REPOSITORY_RULES.md) — root durable repository
  rules: authority order, architecture boundaries, state-machine discipline,
  security/privacy prohibitions, QA cleanup, tests, and documentation rules.
- [architecture/state-machine.md](architecture/state-machine.md) — normative
  reducer state-machine diagrams and guard notes.
- [architecture/i18n.md](architecture/i18n.md) — Rust-owned locale/display
  profile, catalog, pseudo-locale, RTL, and i18n headless gate rules.
- [policies/engineering-rules.md](policies/engineering-rules.md) — prohibitions
  and detailed policy rules: secrets, logging, QA automation, build gates.

## Working documents (dated, short-term)

These are implementation guides, plans, and snapshots. They are valid for the
work they describe and become historical once that work lands. They must not
contradict the normative documents; if an implementation discovery requires a
design change, amend `architecture/overview.md` first.

- `superpowers/specs/` — dated design specs for a specific implementation
  effort (e.g. `2026-06-12-headless-core-runtime-design.md` is the migration
  guide toward the runtime described in `architecture/overview.md`).
- `superpowers/plans/` — dated execution plans.
- `architecture/` (dated files) — point-in-time architecture snapshots such as
  `desktop-foundation.md`.
- `spikes/` — spike results.
- `reviews/` — review records.
- `qa/` — QA audit records, including
  [`qa/known-issues.md`](qa/known-issues.md) for open QA/product blockers that
  must be checked before claiming release readiness.
- `upstream/` — upstream SDK feedback notes.

## Operational notes

- [`/AGENTS.md`](../AGENTS.md) (repo root) — the operational entry file: current
  runtime/QA contract plus an index into `agents/`. Kept small because every
  agent session loads it.
- [`agents/`](agents/) — the operational detail, one topic per file:
  [`environment.md`](agents/environment.md) (setup, toolchains, containers),
  [`verification.md`](agents/verification.md) (how to prove a change),
  [`qa-lanes.md`](agents/qa-lanes.md) (every lane, scenario, and evidence token),
  [`state-ownership.md`](agents/state-ownership.md) (who owns which state, per
  area), [`troubleshooting.md`](agents/troubleshooting.md) (known symptoms),
  [`history.md`](agents/history.md) (superseded contracts), and
  [`plans.md`](agents/plans.md) (dated plan index).

  Durable rules that emerge there must be promoted to `../REPOSITORY_RULES.md` or
  `policies/engineering-rules.md`; the tree keeps the operational how-to.
  `scripts/check-agents-docs.mjs` guards the routing.
