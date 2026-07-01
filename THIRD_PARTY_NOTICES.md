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
Upstream commit: `f13238024a83d5d6fd03540e023aed1e54fc7393` (vendored submodule)
Source path: `crates/` in the upstream repository
Local path: `vendor/matrix-rust-sdk`
License: Apache-2.0
Copyright: Copyright The Matrix.org Foundation C.I.C.
Notes: Vendored, statically linked into desktop release binaries. Fork changes are documented in `docs/upstream/matrix-rust-sdk-feedback.md`; modified source files carry inline `// Matrix desktop fork patch surface:` markers where applicable. The upstream Apache-2.0 license text is included at `vendor/matrix-rust-sdk/LICENSE` and reproduced in release artifacts via `LICENSE-APACHE`.

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

Project: @matrix-org/emojibase-bindings
Repository: https://github.com/matrix-org/emojibase-bindings
Upstream commit: package `@matrix-org/emojibase-bindings@1.5.0`
Source path: package `build/emoji.js` and type declarations
Local path: `apps/desktop/node_modules/@matrix-org/emojibase-bindings` during build; Vite bundles selected emoji lookup code into desktop release artifacts
License: Apache-2.0
Copyright: Copyright The Matrix.org Foundation C.I.C.
Notes: Used to provide Element-compatible emoji category ordering and lookup data for the desktop emoji picker.

Project: emojibase and emojibase-data
Repository: https://github.com/milesj/emojibase
Upstream commit: packages `emojibase@17.0.0` and `emojibase-data@17.0.0`
Source path: packages `packages/core` and `packages/data`
Local path: `apps/desktop/node_modules/emojibase` and `apps/desktop/node_modules/emojibase-data` during build; Vite bundles selected English compact emoji data into desktop release artifacts
License: MIT
Copyright: Copyright (c) 2017-2019 Miles Johnson
Notes: Transitive runtime dependencies of `@matrix-org/emojibase-bindings`; used for the emoji picker data set, shortcodes, tags, and categories.
