# Disabled Puppeteer browser downloader

Koushi's Linux GUI QA connects WebdriverIO to an already running
`tauri-driver`. It never downloads Chrome or Firefox through Puppeteer.

The upstream `@puppeteer/browsers` dependency currently pulls in
`extract-zip`, for which every published version is affected by
GHSA-jmr9-qjv8-65gv. This local package preserves the import contract but
fails closed if browser download or discovery is attempted.

The root-level `puppeteer-browsers-2.13.3.tgz` is generated from the adjacent
`safe-stubs/puppeteer-browsers/` directory with `npm pack`. The tarball is used so
`npm ci` can deduplicate the transitive WebdriverIO dependency reliably.
