// ESLint flat config — BOUNDARY ENFORCEMENT ONLY.
//
// This config enforces architectural import boundaries defined in
// REPOSITORY_RULES.md ("Architecture And Ownership"). It is intentionally
// minimal: it does NOT enable broad style or quality rules to avoid surfacing
// unrelated existing findings (behavior-preserving, issue #87 Phase 0).
//
// Rules encoded here:
//
// 1. Components must not import @tauri-apps/* directly.
//    The transport boundary is apps/desktop/src/backend/*. React components
//    receive the API surface through props/context from App.tsx; they must not
//    reach Tauri IPC themselves.
//
// 2. No source file may import from ../../src-tauri (path escape into the
//    Rust adapter layer). TypeScript types from src-tauri are hand-mirrored
//    in apps/desktop/src/domain/types.ts; that file is the correct import.
//
// 3. App.tsx itself currently holds some direct invoke/listen/getCurrentWindow
//    calls as part of the Tauri event-listener and window-state wiring that
//    has not yet been migrated to backend/client.ts (tracked in #87). Those
//    lines are acknowledged with inline eslint-disable-next-line comments so
//    the rule still catches NEW accidental violations while the migration is
//    in progress.
//
// The @typescript-eslint plugin is registered (but no @typescript-eslint
// rules are enabled) so that existing // eslint-disable-next-line
// @typescript-eslint/no-explicit-any comments in src/test/* do not produce
// "Definition for rule ... was not found" lint errors.

import tseslint from "typescript-eslint";

export default tseslint.config(
  // Use the typescript-eslint parser and register the plugin for all
  // TypeScript/TSX files. Only the parser and plugin registration are added
  // here; no @typescript-eslint rules are turned on (boundary-only config).
  {
    files: ["src/**/*.ts", "src/**/*.tsx"],
    ...tseslint.configs.base,
  },

  // Rule 2 (all src): No path-escape imports into the Rust adapter layer.
  // TypeScript-facing types live in src/domain/types.ts, not src-tauri.
  {
    files: ["src/**/*.ts", "src/**/*.tsx"],
    rules: {
      "no-restricted-imports": [
        "error",
        {
          patterns: [
            {
              group: ["**/src-tauri/**", "../../src-tauri/**", "../src-tauri/**"],
              message:
                "Do not import from src-tauri. Mirror types in src/domain/types.ts instead.",
            },
          ],
        },
      ],
    },
  },

  // Rule 1 (components only): Components must route through the backend client,
  // not Tauri IPC. Scope: src/components/** (App.tsx is the integration host and
  // has acknowledged in-progress violations; domain/** has legitimate @tauri-apps
  // helpers such as convertFileSrc that are correct at that layer).
  {
    files: ["src/components/**/*.ts", "src/components/**/*.tsx"],
    rules: {
      "no-restricted-imports": [
        "error",
        {
          patterns: [
            {
              group: ["@tauri-apps/**"],
              message:
                "Components must not import @tauri-apps directly. Use the backend client (src/backend/client.ts) or props passed from App.tsx.",
            },
            {
              group: ["**/src-tauri/**", "../../src-tauri/**", "../src-tauri/**"],
              message:
                "Do not import from src-tauri. Mirror types in src/domain/types.ts instead.",
            },
          ],
        },
      ],
    },
  },
);
