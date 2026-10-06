// ESLint flat config — BOUNDARY ENFORCEMENT ONLY.
//
// This config enforces architectural import boundaries defined in
// REPOSITORY_RULES.md ("Architecture And Ownership"). It is intentionally
// minimal: it does NOT enable broad style or quality rules to avoid surfacing
// unrelated existing findings (behavior-preserving, issue #87 Phase 0).
//
// Rules encoded here:
//
// 1. src/components/**, src/app/**, and src/App.tsx must not import
//    @tauri-apps/* directly. The transport boundary is
//    apps/desktop/src/backend/*. React components/hooks receive neutral ports;
//    they must not reach Tauri IPC themselves. App.tsx has no direct Tauri
//    import. Any NEW direct import is caught by this rule. Production domain/** is separately
//    guarded below; domain tests and test/** retain only their mock boundaries.
//
// 2. No source file may import from ../../src-tauri (path escape into the
//    Rust adapter layer). TypeScript types from src-tauri are hand-mirrored
//    in apps/desktop/src/domain/types.ts; that file is the correct import.
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

  // Phase 2B domain guard: platform APIs belong under backend adapters. Domain
  // tests may mock Tauri directly; production domain modules may not import it.
  {
    files: ["src/domain/**/*.ts"],
    ignores: ["src/domain/**/*.test.ts"],
    rules: {
      "no-restricted-imports": [
        "error",
        {
          patterns: [
            {
              group: ["@tauri-apps/**"],
              message:
                "Domain modules must not import Tauri. Route through an approved src/backend port.",
            },
          ],
        },
      ],
    },
  },

  // Phase 2B backend guard: concrete Tauri imports are allowed only in the
  // approved adapters. Tests may mock Tauri directly.
  {
    files: ["src/backend/**/*.ts", "src/backend/**/*.tsx"],
    ignores: [
      "src/backend/**/*.test.ts",
      "src/backend/**/*.test.tsx",
      "src/backend/tauri/**/*.ts",
      "src/backend/tauri/**/*.tsx",
      "src/backend/client.ts",
      "src/backend/tauriTimelineTransport.ts",
    ],
    rules: {
      "no-restricted-imports": [
        "error",
        {
          patterns: [
            {
              group: ["@tauri-apps/**"],
              message:
                "Backend composition and neutral contracts must not import Tauri. Use an approved backend adapter.",
            },
          ],
        },
      ],
    },
  },

  // Rule 1 (components + app hooks + App.tsx): Must not directly import
  // @tauri-apps/*. App tests are covered and must mock neutral ports.
  // src/components/**, src/app/**, App.tsx, and appRuntime have zero current
  // violations; any new import is a bug.
  {
    files: [
      "src/components/**/*.ts",
      "src/components/**/*.tsx",
      "src/app/**/*.ts",
      "src/app/**/*.tsx",
      "src/backend/appRuntime.ts",
      "src/App.tsx",
    ],
    rules: {
      "no-restricted-imports": [
        "error",
        {
          patterns: [
            {
              group: ["@tauri-apps/**"],
              message:
                "Do not import @tauri-apps directly here. Route through an approved src/backend adapter/composition root or use props from App.tsx. Existing App.tsx transport wiring is acknowledged with eslint-disable-next-line; do not add new ones without a tracking comment.",
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

  // Rule 3 (all renderer source): no WebView-native dialogs (#1100).
  //
  // wry's WKWebView backend implements no JavaScript dialog handlers, so
  // `window.prompt` returns null at once and `window.confirm` / `window.alert`
  // never appear on macOS. A menu item or destructive guard that depends on
  // one is a silent no-op; use an in-app modal instead.
  {
    files: ["src/**/*.ts", "src/**/*.tsx"],
    rules: {
      "no-alert": "error",
    },
  },
);
