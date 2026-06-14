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
