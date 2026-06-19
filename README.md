# Koushi (光子・格子)

<p align="center">
  <img src="assets/branding/koushi-wordmark.svg" alt="Koushi logo: a bright photon node on a lattice with light running through the grid" width="372">
</p>

Desktop Matrix client prototype built toward Tauri, React, and matrix-rust-sdk.

**Koushi** (コウシ) is a deliberate double pun in Japanese:
- **光子** — *photon*: light, signal, speed, communication.
- **格子** — *lattice / grid*: a direct conceptual bridge to Matrix.

The logo reflects both: a photon (the bright node) resting on a lattice, with
light running through the grid. Do not rebrand back to "Kagome", "Ruri", or
"Matrix Desktop" — the name is intentional. The repository is now
`shinaoka/koushi-matrix`.

Current status: pre-login desktop shell. The repository has pure Rust state/search/key crates, a no-network fake backend, a Tauri v2 + React app shell, and a static Slack-like reference shell.

## License

This project is licensed under the [MIT](LICENSE-MIT) OR [Apache-2.0](LICENSE-APACHE) dual license.

Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion in this project by you, as defined in the Apache-2.0 license, shall be licensed as above, without any additional terms or conditions.

Third-party attributions are recorded in [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md).

## Prerequisites

Initialize the vendored Matrix SDK submodule before running Cargo commands:

```bash
git submodule update --init --recursive
```

The repository commits a top-level `Cargo.lock` for reproducible workspace
resolution. The first Cargo build still needs network access unless the
crates.io registry and git dependencies are already present in your Cargo
cache.

## Verify

Before claiming a real-account or GUI gate is green, check
[`docs/qa/known-issues.md`](docs/qa/known-issues.md).

```bash
cargo test -p koushi-state
cargo test -p koushi-search
cargo test -p koushi-key
cargo test -p koushi-backend
```

For the desktop app:

```bash
cd apps/desktop
npm install
npm test
npm run typecheck
npm run build
cargo test --manifest-path src-tauri/Cargo.toml
```

`npm run build` validates and builds the React/Vite web shell into `dist/`;
it does not produce a native Tauri desktop binary. Building the native app
requires the Rust, Cargo, and Tauri platform toolchain for your OS:

```bash
cd apps/desktop
npm run tauri build
```

## Open The Desktop Shell

React/Tauri app in browser fallback mode:

```bash
cd apps/desktop
npm run dev
```

Then open `http://127.0.0.1:5173/`.

Static reference shell:

```bash
cd apps/desktop-shell
python3 -m http.server 4173 --bind 127.0.0.1
```

Then open `http://127.0.0.1:4173/`.

See `docs/architecture/desktop-foundation.md` and `docs/architecture/tauri-react-shell.md` for the real-login boundary.
