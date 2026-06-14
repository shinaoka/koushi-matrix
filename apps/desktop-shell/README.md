# Desktop Shell

This is the zero-dependency pre-login desktop shell. Use it to inspect the fake ready session before the Tauri/React packaging and real Matrix login runner are wired.

## Run

Python 3 is the recommended way to serve the shell, matching the top-level
README:

```bash
cd apps/desktop-shell
python3 -m http.server 4173 --bind 127.0.0.1
```

Open `http://127.0.0.1:4173/`, then confirm `index.html`, `styles.css`, and
`app.js` load without 404s in the browser's network panel.

Stop the server with `Ctrl+C`. If port `4173` is already in use, pick another
local port and open the matching URL, for example:

```bash
python3 -m http.server 4174 --bind 127.0.0.1
```

Opening `index.html` directly from the filesystem is supported only as a quick
inspection convenience. Prefer the local server path when checking browser
loading behavior.

The fixture data mirrors the Rust fake backend:

- Slack-like workspace rail, sidebar, timeline, and thread pane.
- DMs are global across Spaces.
- Search results use exact highlighting and include attachment filenames.
- The right thread pane is visible on wide viewports and hidden on narrower viewports.
