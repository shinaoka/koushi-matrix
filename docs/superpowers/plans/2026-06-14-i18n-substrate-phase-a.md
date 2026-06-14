# 2026-06-14 i18n Substrate Phase A

Goal: complete the headless, React-independent i18n substrate for Issue #4
without mixing locale logic into GUI components.

Canon consulted:

- `REPOSITORY_RULES.md`
- `docs/architecture/overview.md`
- `docs/architecture/state-machine.md`
- `docs/architecture/i18n.md`
- `docs/policies/engineering-rules.md`

## Plan

1. Add RED Rust tests for a Rust-owned locale/display profile resolver.
2. Add RED catalog tests for pseudo-locale expansion, placeholder
   preservation, and RTL/CJK/combining samples.
3. Implement the pure `matrix-desktop-state` resolver and catalog helper.
4. Document the canonical profile and rules in durable architecture docs.
5. Run focused headless verification and leave a GitHub issue work record.

## Phase Boundary

Phase A owns state/profile/catalog substrate only. It intentionally does not
wire root `lang`/`dir` into React, migrate every visible label, convert all CSS
to logical properties, or add browser layout assertions. Those are Phase B.

## Verification

Run at minimum:

```bash
cargo test -p matrix-desktop-state --test locale_display_profile
npm --prefix apps/desktop run test -- --run src/i18n/messages.test.ts
```

Before merging, also run formatting and broader focused gates that cover the
files touched by this plan.
