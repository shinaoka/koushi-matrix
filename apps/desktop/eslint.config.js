// ESLint flat config — BOUNDARY ENFORCEMENT ONLY.
//
// This config enforces architectural import boundaries defined in
// REPOSITORY_RULES.md ("Architecture And Ownership"). It is intentionally
// minimal: it does NOT enable broad style or quality rules to avoid surfacing
// unrelated existing findings (behavior-preserving, issue #87 Phase 0).
//
// Rules encoded here:
//
// 1. src/components/** AND src/App.tsx must not import @tauri-apps/* directly.
//    The transport boundary is apps/desktop/src/backend/*. React components
//    receive the API surface through props/context from App.tsx; they must not
//    reach Tauri IPC themselves. App.tsx has three grandfathered import lines
//    acknowledged with inline eslint-disable-next-line no-restricted-imports
//    comments (tracked for Phase 2 migration). Any NEW direct @tauri-apps import
//    in App.tsx without a disable comment will be caught by this rule.
//    domain/** and test/** are intentionally excluded: convertFileSrc in
//    domain/mediaUrl.ts, notification helpers in domain/desktopNotification.ts,
//    and the @tauri-apps/api/mocks in test/** are correct at those layers.
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

  // Rule 1 (components + App.tsx): Must not directly import @tauri-apps/*.
  // - src/components/**  — zero current violations; any new import is a bug.
  // - src/App.tsx        — the 3 existing @tauri-apps lines are acknowledged
  //   with eslint-disable-next-line no-restricted-imports comments and tracked
  //   for Phase 2 migration. Any NEW import without a disable comment is caught.
  {
    files: [
      "src/components/**/*.ts",
      "src/components/**/*.tsx",
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
                "Do not import @tauri-apps directly here. Route through src/backend/client.ts or use props from App.tsx. Existing App.tsx transport wiring is acknowledged with eslint-disable-next-line; do not add new ones without a tracking comment.",
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
