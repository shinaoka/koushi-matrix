# matrix-desktop

Desktop Matrix client prototype built toward Tauri, React, and matrix-rust-sdk.

Current status: pre-login foundation. The repository has pure Rust state/search/key crates, a no-network fake backend, and a static Slack-like desktop shell for exercising the layout before real Matrix login.

## Verify

```bash
cargo test -p matrix-desktop-state
cargo test -p matrix-desktop-search
cargo test -p matrix-desktop-key
cargo test -p matrix-desktop-backend
```

## Open The Desktop Shell

```bash
cd apps/desktop-shell
python3 -m http.server 4173 --bind 127.0.0.1
```

Then open `http://127.0.0.1:4173/`.

See `docs/architecture/desktop-foundation.md` for the real-login boundary.
