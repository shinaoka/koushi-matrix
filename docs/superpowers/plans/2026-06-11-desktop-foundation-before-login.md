# Desktop Foundation Before Login Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Advance the repository to the point immediately before real Matrix login. The desktop foundation should boot with a deterministic fake backend, exercise the same reducer/search/key boundaries that the real backend will use, and expose a Slack-like desktop shell with Space, global DM, timeline, thread, and search-result surfaces.

**Architecture:** Keep the live Matrix SDK boundary absent in this step. Promote the key-management spike into a production crate, add a fake backend crate that drives `koushi-state` and `koushi-search`, and add a static desktop shell that can be opened locally while the Tauri/React packaging is still pending.

**Security Constraints:** Follow `REPOSITORY_RULES.md`. Decrypted message bodies, attachment names, search queries, access tokens, and derived keys are secrets. Persistent stores for decrypted data are prohibited unless encrypted at rest. Debug output for secret wrappers must be redacted.

## Scope

This plan includes:

1. Production `koushi-key` crate for local unlock-secret storage and key derivation.
2. `koushi-backend` fake runtime that owns the reducer state, fake rooms/messages, and verified search results.
3. Static desktop shell for the pre-login app surface.
4. Architecture notes describing the exact point where real Matrix login will attach.

This plan intentionally excludes:

1. Real Matrix SDK login and sync.
2. Real E2EE store initialization.
3. Video chat.
4. Persistent search index creation.

## Tasks

- [x] Add `koushi-key` to the workspace by moving reusable logic from `spikes/key-management` into `crates/koushi-key`.
- [x] Cover key derivation, redacted debug output, credential ID stability, invalid stored secrets, and missing credential behavior with tests.
- [x] Add `koushi-backend` to the workspace as a no-network fake runtime around the reducer and search adapter.
- [x] Seed a fake ready session with Space rooms, global DMs, main timeline messages, a thread, attachment filename search data, and an intentional ngram false-positive candidate.
- [x] Add tests proving the fake backend boots to a ready session, search is exact-verified, attachment filenames are searchable, and DMs remain global while rooms are Space-filtered.
- [x] Add a Slack-like static desktop shell in `apps/desktop-shell` that renders the pre-login application surface without a landing page.
- [x] Document the pre-login backend boundary and the next Matrix SDK integration steps.
- [x] Run Rust tests and clippy for the touched crates, then verify the desktop shell in the in-app browser.
