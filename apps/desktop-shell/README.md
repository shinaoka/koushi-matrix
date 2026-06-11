# Desktop Shell

This is the zero-dependency pre-login desktop shell. Open `index.html` directly in a browser to inspect the fake ready session before the Tauri/React packaging and real Matrix login runner are wired.

The fixture data mirrors the Rust fake backend:

- Slack-like workspace rail, sidebar, timeline, and thread pane.
- DMs are global across Spaces.
- Search results use exact highlighting and include attachment filenames.
- The right thread pane is visible on wide viewports and hidden on narrower viewports.
