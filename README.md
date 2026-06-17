# Ruri (瑠璃)

Desktop Matrix client prototype built toward Tauri, React, and matrix-rust-sdk.

Ruri (瑠璃, deep azure / lapis-lazuli blue) is the shipped product name for
this desktop Matrix client. The repository codename remains `matrix-desktop`.

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

```bash
cargo test -p matrix-desktop-state
cargo test -p matrix-desktop-search
cargo test -p matrix-desktop-key
cargo test -p matrix-desktop-backend
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
