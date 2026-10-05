# Updater packager opt-out (#1063)

## Scope

Repackaged Linux installs (for example an AUR package that unpacks the upstream
`.deb` into `/usr`) run a binary byte-identical to the `.deb`, so a build-time
switch cannot keep the app from updating itself. This change adds a runtime
marker-file opt-out to the platform-neutral updater engine.

Consulted `REPOSITORY_RULES.md` (Canon-First Change Protocol, User-Facing Text
And Localization, Documentation And Work Records), overview "Desktop
Application Updates", and `docs/architecture/i18n.md`. The updater lifecycle is
an adapter lifecycle documented in the overview, not in `state-machine.md`; the
overview section is amended in the same change.

## Contract

- Marker: `/usr/share/koushi-desktop/package-managed` (Linux only; contents
  ignored; read once when the update adapter is created).
- With the marker, the initial state is `unsupported` with reason
  `package_managed`; `select_backend` never constructs the install backend, so
  no owner, feed request, or installer/privilege escalation (`pkexec`, `sudo`,
  `dpkg`, `rpm`) can start. Every command trigger is refused by the lifecycle.
- `DesktopUpdateState::Unsupported` gained `reason: build | package_managed`;
  React renders `settings.updatePackageManaged` (en, ja) for the new reason.

## Changes

| Surface | Change |
| --- | --- |
| `apps/desktop/src-tauri/src/app_updates.rs` | Reason enum, marker constant, injectable `initial_state_for(marker)`, `select_backend` gate before `PlatformBackend::for_app`. |
| `apps/desktop/src-tauri/src/app_updates/package_managed_tests.rs` | New: marker present → unsupported, backend constructor not called, no work claimed; absent marker unchanged; wire shape; marker path per target. |
| `apps/desktop/src/domain/types.ts`, `components/DesktopUpdates.tsx`, `i18n/messages.ts` | Wire mirror, reason-specific text, en/ja catalog entries. |
| `apps/desktop/src/components/DesktopUpdates.test.tsx`, `e2e/desktop-updates.spec.ts` | Wire fixtures carry the reason; new package-managed render test. |
| `docs/architecture/overview.md`, `docs/help/settings.md`, `docs/releases/desktop-release.md` | Canon, user help, and packager guidance. |

## Verification

| Check | Result |
| --- | --- |
| `cargo test -p koushi-desktop --lib app_updates` | 25 passed |
| `cargo clippy -p koushi-desktop --all-targets -- -D warnings` (macOS) | ok |
| `npx vitest run src/components/DesktopUpdates.test.tsx src/i18n` | ok |
| `npm --prefix apps/desktop run typecheck` / `run lint` | ok |
| `node scripts/user-help.mjs` | ok |

Pending manual check (reporter-run): Arch KDE Wayland with the marker in the AUR
package shows no update offer and `pacman -Qkk` stays clean.
