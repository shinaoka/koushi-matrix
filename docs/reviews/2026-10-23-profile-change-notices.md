# Profile-change notices and CI advisory follow-up

## Scope and canon

The timeline emitted bare English profile-change text without a localized notice
key. Continued sender rows could therefore show no member identity. Core now
selects typed notice keys; the renderer interpolates the Rust-projected sender
label through all five catalogs. Name, avatar, combined, and generic changes
retain their existing meanings.

Consulted `REPOSITORY_RULES.md` (root-cause fixes, localization, verification,
review/audit, and work records), `docs/architecture/i18n.md`,
`docs/agents/verification.md`, `docs/agents/state-ownership.md`, and
`docs/policies/engineering-rules.md` (localization, test placement, build gates).

Changes touch Core timeline projection, protocol/wire notice keys, the timeline
renderer and catalogs, SDK-event/UI regression tests, and the Tauri wire fixture.
Element Web and Element X Android/iOS identify the affected member or use an
own-user label; Koushi retains generic wording with the projected member name.
Reviewed messaging/settings help: no controls or workflows change.

## Advisory discovered by required CI

The first PR run passed workspace tests but failed cargo-deny because the
newly published RUSTSEC-2026-0385 applies to the pre-existing ctutils 0.4.2 lock.
Its mask-generation defect can affect commonly used platforms; its separate
u128-comparison defect concerns 32-bit targets.

The bounded repair updates only root Cargo.lock to ctutils 0.4.4, the minimum
patched version. SDK source/gitlink, crypto policy, and the deny allowlist do not
change. digest 0.11.3 and universal-hash 0.6.1 already require ctutils ^0.4;
ctutils 0.4.4's Rust 1.87 minimum fits the pinned 1.96 toolchain.

Sources: [RustSec advisory](https://raw.githubusercontent.com/RustSec/advisory-db/main/crates/ctutils/RUSTSEC-2026-0385.md),
[upstream release/security fixes](https://github.com/RustCrypto/utils/pull/1569).

## Independent review

- Design review, before dependency mutation: approved with no blockers. The
  reviewer checked advisory coverage, dependency constraints, MSRV, unchanged
  SDK/deny boundaries, and required gates. No dependency-only gate exemption.
- Final integrated review: approved with no security, design, or correctness
  blockers. Confirmed the sole package delta and archive checksum, unchanged
  SDK/manifests/deny boundaries, and Rust/wire/catalog/renderer consistency.
  Acceptance remains conditional on the post-update merge gates below.

## Verification

- Before the notice fix, the Rust SDK-event projection check and English/Japanese
  UI regressions failed; the same checks passed afterwards. Tests include name
  and avatar removal and a continued sender row.
- Initial local gates passed: 4,439 workspace tests (including Tauri wire
  serialization), 2,270 frontend tests, 41 wire/store checks after regeneration,
  typecheck, lint/build, rustfmt, workspace/QA/release/stack-budget clippy, help
  checks, SDK/boundary guards, and disposable Tuwunel/Synapse QA.
- Local cargo-deny reproduced RUSTSEC-2026-0385 before the lockfile update.
- After the dependency update: cargo-deny passed advisories, bans, licenses,
  and sources; all 4,438 workspace tests passed, as did workspace/QA/release/
  stack-budget clippy and disposable Tuwunel/Synapse QA. rustfmt, SDK and
  boundary guards passed. Required hosted CI checks remain enforced before
  merging.
