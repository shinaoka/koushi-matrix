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
Upstream commit: `30e9c8bfbb6b6c0e3c2c9fb3793082ad94a5c8da` (vendored submodule; bump this and the parent gitlink together when the fork commit changes)
Source path: `crates/` in the upstream repository
Local path: `vendor/matrix-rust-sdk`
License: Apache-2.0
Copyright: Copyright The Matrix.org Foundation C.I.C.
Notes: Vendored, statically linked into desktop release binaries. Fork changes are documented in `docs/upstream/matrix-rust-sdk-feedback.md`; modified source files carry either an inline `// Matrix desktop fork patch surface:` marker or a `// Modified for the Koushi desktop fork` notice after the upstream header. The upstream Apache-2.0 license text is included at `vendor/matrix-rust-sdk/LICENSE` and reproduced in release artifacts via `LICENSE-APACHE`.

Project: Inter via Fontsource
Repository: https://github.com/fontsource/font-files
Upstream commit: package `@fontsource/inter@5.2.8`
Source path: `fonts/google/inter` package files
Local path: `apps/desktop/node_modules/@fontsource/inter` during build; Vite bundles selected CSS/woff/woff2 assets into desktop release artifacts
License: SIL Open Font License 1.1 (`OFL-1.1`)
Copyright: Copyright 2016 The Inter Project Authors
Notes: Used as the bundled-preferred UI font when the Rust-owned typography profile selects `font = inter`; system UI fonts remain fallback.

Project: Twemoji COLR Font
Repository: https://github.com/mrdrogdrog/twemoji-color-font
Upstream commit: package `twemoji-colr-font@15.0.3`
Source path: package `twemoji.css` and `twemoji.woff2`
Local path: `apps/desktop/node_modules/twemoji-colr-font` during build; Vite bundles selected CSS/woff2 assets into desktop release artifacts
License: package metadata `OFL-1.1`; package CSS header `MIT`; Twemoji visual design/artwork under Creative Commons Attribution 4.0 International (`CC-BY-4.0`)
Copyright: Twemoji font package by Tilman Vatteroth; Twemoji artwork by the Twemoji project
Notes: Used as the bundled-preferred emoji font when the Rust-owned typography profile selects `emoji = twemojiColr`; platform/system emoji fonts remain fallback. npm marks this package deprecated, so upgrades or replacement must revisit the font source and attribution.

Project: KaTeX
Repository: https://github.com/KaTeX/KaTeX
Upstream commit: npm package `katex@0.18.1` (the version pinned in `crates/koushi-core/assets/katex/VERSION`)
Source path: `dist/katex.min.js`, `dist/katex.min.css`, `dist/fonts/*.woff2`
Local path: `crates/koushi-core/assets/katex`
License: MIT
Copyright: Copyright (c) 2013-2020 Khan Academy and other contributors
Notes: Embedded in the desktop binary and copied into every exported history folder (`assets/katex/`) so exported pages render math offline. The CSS keeps only the woff2 font sources. `apps/desktop/src/i18n/katexVendor.test.ts` keeps the vendored version equal to the npm dependency. The license text is `crates/koushi-core/assets/katex/LICENSE`.
