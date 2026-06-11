# Tauri React Fake Backend Shell Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Move the pre-login desktop shell into a Tauri v2 + React app that talks to the no-network fake backend through the same command boundary the real Matrix SDK runner will use later.

**Architecture:** Add `apps/desktop` as the first packaged desktop app surface. The React app renders snapshot DTOs and calls a `DesktopApi` interface. In a Tauri runtime, that interface uses `@tauri-apps/api/core.invoke`; in normal Vite/browser development, it uses an in-memory fake API with matching DTOs. The Rust `src-tauri` crate owns a `Mutex<FakeDesktopBackend>` and exposes narrowly scoped commands.

**Tech Stack:** Tauri v2, Vite, React, TypeScript, Vitest, lucide-react, Rust 2024.

---

## Tasks

- [x] Add a Vite React app in `apps/desktop` with TypeScript, Vitest, Tauri CLI scripts, and browser fallback support.
- [x] Write failing TypeScript tests for visible room grouping and exact search behavior against the fake API.
- [x] Implement shared TypeScript DTOs, fallback fixture data, and the `DesktopApi` abstraction.
- [x] Build the React Slack-like shell over snapshot DTOs with Space rail, sidebar, timeline, thread pane, composer, and search results.
- [x] Add `apps/desktop/src-tauri` with Tauri v2 commands wrapping `matrix-desktop-backend`.
- [x] Wire Tauri config to Vite using `devUrl` and `frontendDist`.
- [x] Document app run commands and keep `apps/desktop-shell` as the zero-dependency reference shell.
- [x] Verify with `npm test`, `npm run typecheck`, `npm run build`, `cargo check` for `src-tauri`, and browser layout checks.
