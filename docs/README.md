# Documentation Map

This directory separates long-term normative documents from dated, short-term
working documents. When documents disagree, the normative documents win, and
the conflict must be resolved by amending one of them explicitly.

## Normative (long-term, kept current)

These documents describe what the product and codebase must look like. They are
not dated snapshots; they are amended in place through review.

- [architecture/overview.md](architecture/overview.md) — the overall
  architecture blueprint: layers, crate boundaries, runtime model, async design
  rules, security model, QA model.
- [policies/engineering-rules.md](policies/engineering-rules.md) — prohibitions
  and unified rules: secrets, logging, QA automation, build gates.

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
  `desktop-foundation.md` and `state-machine.md`. The state-machine contract in
  `state-machine.md` remains authoritative for reducer semantics until folded
  into `overview.md`.
- `spikes/` — spike results.
- `reviews/` — review records.
- `qa/` — QA audit records.
- `upstream/` — upstream SDK feedback notes.

## Operational notes

- [`/AGENTS.md`](../AGENTS.md) (repo root) — environment-specific
  troubleshooting for agents and QA automation (macOS permissions, process
  cleanup, homeserver install caveats). Durable rules that emerge there must be
  promoted to `policies/engineering-rules.md`; AGENTS.md keeps the operational
  how-to.
