# matrix-desktop

Desktop Matrix client prototype built toward Tauri, React, and matrix-rust-sdk.

Current status: pre-login desktop shell. The repository has pure Rust state/search/key crates, a no-network fake backend, a Tauri v2 + React app shell, and a static Slack-like reference shell.

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
