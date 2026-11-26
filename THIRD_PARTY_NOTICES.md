# Third Party Notices

This file records third-party source code and assets that are copied,
closely adapted, vendored, or bundled into release artifacts from this
repository.

Reference-only reading of upstream projects does not need an entry here. Direct ports or close adaptations do.

## Entry Template

```text
Project:
Repository:
Upstream commit:
Source path:
Local path:
License:
Copyright:
Notes:
```

## Current Entries

Project: matrix-rust-sdk
Repository: https://github.com/matrix-org/matrix-rust-sdk
Upstream commit: `d315a2fa4fb489f479c7bc85ebea459b408f12ac` (vendored submodule; the Koushi fork `main`, which merges fork PR #24 (#1395 removal of the unused cache-only deep-history restore surface) on top of `e94cddf500064d444e0ac9afd4b3e50c8e3e7c4d` (fork PR #23, #1392 topology-first gap inspection) on top of `a8c2b7cc7d0e39ba99308f809e4e4fb33f394626` (fork PR #22, #1344 operation-scoped search readers) on top of `7572e53c93d08b5ced009da7fd6257853d4a7642`, which merges upstream `matrix-sdk-0.19.1` via the `chore/upstream-0.19.1` branch plus fork PR #21 (#1344 owner-side retention counters); bump this and the parent gitlink together when the fork commit changes)
Source path: `crates/` in the upstream repository
Local path: `vendor/matrix-rust-sdk`
License: Apache-2.0
Copyright: Copyright The Matrix.org Foundation C.I.C.
Notes: Vendored, statically linked into desktop release binaries. Fork changes are documented in `docs/upstream/matrix-rust-sdk-feedback.md`; modified source files carry either an inline `// Matrix desktop fork patch surface:` marker or a `// Modified for the Koushi desktop fork` notice after the upstream header. The upstream Apache-2.0 license text is included at `vendor/matrix-rust-sdk/LICENSE` and reproduced in release artifacts via `LICENSE-APACHE`.

Project: heif-oxide HEIF test fixture
Repository: https://github.com/marcusgrass/heif-oxide
Upstream commit: published crate `heif-oxide` 0.1.0
Source path: the package's generated HEVC still-image test fixture (`opaque.heic`)
Local path: `crates/koushi-media/tests/fixtures/heif/opaque.heic`
License: MIT (chosen from the package's MIT OR Apache-2.0 terms)
Copyright: Copyright (c) 2026 Daniel Phillips
Notes: Redistributed as a test asset. The upstream MIT copyright and permission notice is preserved verbatim at `crates/koushi-media/tests/fixtures/heif/LICENSE-MIT`; the fixture contains no user metadata. Referenced by the media preparation and staging tests.

Project: Inter via Fontsource
Repository: https://github.com/fontsource/font-files
Upstream commit: package `@fontsource/inter@5.2.8`
Source path: `fonts/google/inter` package files
Local path: `apps/desktop/node_modules/@fontsource/inter` during build; Vite bundles selected CSS/woff/woff2 assets into desktop release artifacts
License: SIL Open Font License 1.1 (`OFL-1.1`)
Copyright: Copyright 2016 The Inter Project Authors
Notes: Used as the bundled-preferred UI font when the Rust-owned typography profile selects `font = inter`; system UI fonts remain fallback.

Project: Twemoji COLR Font
Repository: https://github.com/SableClient/twemoji-font
Upstream commit: package `@sableclient/twemoji-font@1.0.4`
Source path: package `dist/index.css` and `dist/files/twemoji.woff2`
Local path: `apps/desktop/node_modules/@sableclient/twemoji-font` during build; Vite bundles selected CSS/woff2 assets into desktop release artifacts
License: Apache-2.0 for code; Twemoji visual design/artwork under Creative Commons Attribution 4.0 International (`CC-BY-4.0`)
Copyright: Copyright 2016-2018, Mozilla Foundation; Twemoji artwork by the Twemoji project
Notes: Used as the bundled-preferred emoji font when the Rust-owned typography profile selects `emoji = twemojiColr`; platform/system emoji fonts remain fallback. The maintained SableClient package replaces the deprecated `twemoji-colr-font`; its `LICENSE.md` preserves the code and artwork attribution.

Project: PDF.js
Repository: https://github.com/mozilla/pdf.js
Upstream commit: npm package `pdfjs-dist@6.5.438`
Source path: `build/pdf.min.mjs` (library) and `build/pdf.worker.min.mjs` (worker)
Local path: `apps/desktop/node_modules/pdfjs-dist` during build; Vite bundles the library into a lazily loaded chunk and the worker as a same-origin asset in desktop release artifacts
License: Apache-2.0
Copyright: Copyright Mozilla Foundation and the PDF.js contributors
Notes: Renders the first-page thumbnail on a PDF attachment card and the in-app PDF viewer (#2096). It parses bytes the Rust media command returns; it never fetches a URL, so no document is sent to a third party. The worker runs under `worker-src 'self' blob:` in the Tauri CSP.

Project: KaTeX
Repository: https://github.com/KaTeX/KaTeX
Upstream commit: npm package `katex@0.18.10` (the version pinned in `crates/koushi-core/assets/katex/VERSION`)
Source path: `dist/katex.min.js`, `dist/katex.min.css`, `dist/fonts/*.woff2`
Local path: `crates/koushi-core/assets/katex`
License: MIT
Copyright: Copyright (c) 2013-2020 Khan Academy and other contributors
Notes: Embedded in the desktop binary and copied into every exported history folder (`assets/katex/`) so exported pages render math offline. The CSS keeps only the woff2 font sources. `apps/desktop/src/i18n/katexVendor.test.ts` keeps the vendored version equal to the npm dependency. The license text is `crates/koushi-core/assets/katex/LICENSE`.
