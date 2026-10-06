---
name: koushi-release
description: Prepare, monitor, verify, or troubleshoot a Koushi desktop release. Use when changing the desktop version or operating the macOS, Windows, and Linux GitHub Release workflow.
metadata:
  compatibility: Repository skill for Codex, Claude Code, OpenCode, and Pi. Requires git and Node.js/npm; GitHub operations require gh authentication.
---

# Koushi desktop release

Read [`../../../docs/releases/desktop-release.md`](../../../docs/releases/desktop-release.md) completely before changing a version, operating the release workflow, or advising someone how to release Koushi. Treat that document as the sole source of truth.

Determine which mode the user requested:

- **Explain:** summarize the runbook without changing files or GitHub state.
- **Prepare:** require an explicit target version, update exactly the version files listed in the runbook (the three manifests plus the `Cargo.lock` and `apps/desktop/package-lock.json` root entries, via `node scripts/desktop-release-version.mjs --set <version>`), and run its local gates.
- **Monitor:** inspect the specified or latest release workflow without modifying files.
- **Troubleshoot:** inspect failed jobs and preserve draft releases, tags, artifacts, and credentials unless the user explicitly authorizes a destructive recovery action.

Always preserve these boundaries:

- Never create a release tag manually; the successful publish job owns tag creation.
- Never print, download, or expose signing secrets. Do not weaken signing, notarization, audit, checksum, or platform-completion gates.
- A request to prepare a release does not itself authorize push, PR creation, merge, rerun, draft deletion, or tag deletion. Obtain or confirm the relevant authorization before each external mutation not already requested.
- Do not overwrite an existing published version. Prepare a newer patch release instead.
- Report the target version, commit, workflow run URL, release URL, and verification result when those values exist.
