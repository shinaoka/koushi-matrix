# Koushi desktop release runbook

This is the canonical procedure for publishing Koushi desktop installers. The
release automation lives in
[`release-desktop.yml`](../../.github/workflows/release-desktop.yml).

## Invoke the release skill

Start the agent at the repository root, then invoke the project skill:

| Agent | Invocation |
| --- | --- |
| Codex | `$koushi-release` |
| Claude Code | `/koushi-release` |
| OpenCode | `/koushi-release` |
| Pi | `/skill:koushi-release` |

The shared Agent Skills entry point is
[`../../.agents/skills/koushi-release/SKILL.md`](../../.agents/skills/koushi-release/SKILL.md).
Claude Code and OpenCode have equivalent discovery entry points under
`.claude/skills` and `.opencode/skills`, respectively.

## Release contract

- A release starts with an explicit SemVer target such as `0.2.0` or
  `0.2.0-beta.1`.
- The version must increase and must match in every version file:
  - `apps/desktop/package.json`
  - `apps/desktop/package-lock.json` (the root `version` and `packages[""].version`)
  - `apps/desktop/src-tauri/tauri.conf.json`
  - `apps/desktop/src-tauri/Cargo.toml`
  - `Cargo.lock` (the `koushi-desktop` package entry)
- The lockfiles are part of the release change, not incidental artifacts. A
  stale lockfile entry is rewritten by the next `cargo` command or
  `npm install` on any later branch, so a release that skips it lands its
  version bump in an unrelated PR. v0.11.1 shipped with `Cargo.lock` still on
  `0.11.0`; #955 had to carry the repair. `npm ci` does not validate the npm
  lockfile's root version, so `apps/desktop/package-lock.json` silently lagged
  the manifests through v0.18.0 and v0.19.0 until #1137.
- `node scripts/desktop-release-version.mjs` enforces this on every PR, not
  only release PRs. Its `VERSION_FILES` table is the single list of version
  files: the check, the `--set` rewrite, and this runbook follow it. The script
  also fails when any other tracked file declares the desktop package's version
  (a `koushi-desktop` npm or Cargo manifest or lockfile, or a desktop Tauri
  config with a `version` key) without being listed there, so a newly added
  version-bearing file cannot drift unnoticed.
- Do not create `v<version>` manually. The publish job creates the tag only
  after every required artifact passes its gates.
- The macOS arm64 artifact must be Developer ID signed, notarized, stapled,
  and accepted by Gatekeeper. Koushi v0.1.0 was the final release to include
  an Intel Mac artifact.
- The Windows x64 NSIS installer remains explicitly unsigned until a Windows
  certificate and signing gate are approved.
- The Windows installer requires WebView2 `155.0.4283.45` or newer and invokes
  the Evergreen bootstrapper when the installed runtime is older.
- The Windows signing preparation is tracked in the [draft code signing
  policy](../security/code-signing-policy.md); this draft does not make the
  current unsigned artifact signed or approved for public distribution.
- The Windows build entry point supports `--build-only` for producing the main
  executable and `--bundle-only --signed-input` for rebuilding the NSIS
  installer after an external signing step. The public release workflow does
  not use those signing stages until the provider and protected CI gate are
  approved.
- The Linux x64 AppImage, deb, and RPM packages are unsigned; users verify
  the adjacent SHA-256 files. Credentials use the freedesktop Secret Service
  (GNOME Keyring / KWallet) via the `koushi-desktop` service.
- High or critical npm vulnerabilities stop the release.
- Never expose GitHub Environment secrets or copy signing material into the
  repository, logs, release notes, or artifacts.

The protected `release-macos` Environment also owns the updater trust material:

- `KOUSHI_UPDATER_PUBLIC_KEY` is an Environment variable embedded into the
  macOS binary for signature verification;
- `TAURI_SIGNING_PRIVATE_KEY` and `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` are
  Environment secrets used only while producing the updater archive.

Generate this keypair offline with the Tauri signer. Never commit or print the
private key or password. A normal local build without the public key keeps the
update adapter in `unsupported` and does not make update requests.

Distribution packagers who repackage the Linux artifacts (for example an AUR
package built from the `.deb`) install the empty marker file
`/usr/share/koushi-desktop/package-managed`. With the marker present the update
adapter reports `unsupported` with reason `package_managed`, selects no install
backend, makes no update request, and never spawns `pkexec`, `sudo`, `dpkg`, or
`rpm`; see overview "Desktop Application Updates".

## Prepare the release PR

1. Fetch `origin/main` and confirm the worktree state. Preserve unrelated user
   changes; do not discard or overwrite them.
2. Create a release branch from the current `origin/main`.
3. Rewrite every version file to the exact requested version:

   ```bash
   node scripts/desktop-release-version.mjs --set <version>
   git diff --stat
   ```

   The command edits only the version fields listed above and then runs the
   consistency and discovery checks. Do not change dependency versions unless
   that is separately required.
4. Confirm that neither lockfile carries a dependency change:

   ```bash
   git diff Cargo.lock apps/desktop/package-lock.json
   ```

   The diff must be the single `koushi-desktop` version line in `Cargo.lock`
   and the two root version lines in `package-lock.json`. If anything else
   changed, restore the lockfile and resolve that separately — a release PR
   does not carry a dependency update.
5. Run the local release checks from the repository root:

   ```bash
   npm --prefix apps/desktop run release:version:check
   npm --prefix apps/desktop audit --package-lock-only --audit-level=high
   npm --prefix apps/desktop run typecheck
   npm --prefix apps/desktop run lint
   npm --prefix apps/desktop test -- --run src/scripts/releaseConfiguration.test.ts
   git diff --check
   ```

   These preparation checks do not build installers, access signing
   credentials, or modify the macOS keychain. Packaging and signing happen in
   the protected workflow after merge.

6. Review the diff. The release-only PR should normally contain the three
   synchronized manifest changes plus the lockfile version lines from step 4,
   and no generated installer.
7. Commit, push, create the PR, make it ready for review, or merge only when the
   user has requested the corresponding external action.

## What happens after merge

A version-manifest change reaching `main` starts **Release desktop installers**.
The workflow:

1. validates synchronized SemVer and requires an increase over the previous
   `main` commit;
2. rejects an already-used release tag;
3. runs the lockfile, full, and runtime-only npm vulnerability gates;
4. builds the macOS arm64 DMG and signed updater archive using the protected
   `release-macos` Environment;
5. verifies both apps' signatures, notarization tickets, stapling, and
   Gatekeeper trust;
6. builds the unsigned Windows x64 NSIS trial installer;
7. builds the unsigned Linux x64 AppImage, deb, and RPM packages;
8. creates SHA-256 files for every installer and updater archive;
9. creates `latest.json` from the verified archive signature;
10. creates `latest-beta.json` and updates the moving `latest-beta` Release
    used by clients that include pre-release versions; stable releases keep
    that channel usable until the first pre-release is published;
11. waits for all platform jobs, verifies the downloaded checksums, creates a
    hidden draft Release, uploads every artifact, and finally publishes it.

No public partial release is created when a platform build or verification gate
fails.

## Monitor and verify

Find and follow the run:

```bash
gh run list --workflow release-desktop.yml --branch main --limit 5
gh run watch <run-id> --exit-status
```

After success, verify the release metadata:

```bash
gh release view "v<version>" \
  --json url,isDraft,isPrerelease,tagName,targetCommitish,assets
```

Confirm that the release contains every installer and its `.sha256`
file:

- `Koushi-macos-arm64.dmg`
- `Koushi-macos-arm64.app.tar.gz`
- `Koushi-macos-arm64.app.tar.gz.sig`
- `latest.json`
- `Koushi-windows-x64-unsigned.exe`
- `Koushi-linux-x64.AppImage`
- `Koushi-linux-x64.deb`
- `Koushi-linux-x64.rpm`

Stable download links:

- <https://github.com/shinaoka/koushi-matrix/releases/latest/download/Koushi-macos-arm64.dmg>
- <https://github.com/shinaoka/koushi-matrix/releases/latest/download/latest.json>
- <https://github.com/shinaoka/koushi-matrix/releases/latest/download/Koushi-windows-x64-unsigned.exe>
- <https://github.com/shinaoka/koushi-matrix/releases/latest/download/Koushi-linux-x64.AppImage>
- <https://github.com/shinaoka/koushi-matrix/releases/latest/download/Koushi-linux-x64.deb>
- <https://github.com/shinaoka/koushi-matrix/releases/latest/download/Koushi-linux-x64.rpm>

GitHub's `releases/latest` links select the latest full release, not a
prerelease. The desktop pre-release setting uses the moving `latest-beta`
Release, which is updated for every release. A release is a pre-release for
this purpose when its SemVer prerelease field is non-empty, such as
`0.10.0-alpha.1`, `0.10.0-beta.1`, or `0.10.0-rc.1`. Build metadata alone,
such as `0.10.0+nightly.1`, does not make a release a beta.

## Failure recovery

Inspect the failed jobs first:

```bash
gh run view <run-id> --log-failed
```

- If a build or verification job failed before publication, fix the cause in a
  new PR. If that PR does not change a version manifest, start the corrected
  workflow manually from `main`:

  ```bash
  gh workflow run release-desktop.yml --ref main
  ```

- If draft creation succeeded and only the final publish step failed, rerun the
  failed job from the same workflow run so the successful build jobs and draft
  remain intact.
- Do not delete or recreate a draft, tag, or published Release without explicit
  authorization and a verified target. Those operations can break stable links
  or make an existing version ambiguous.
- Do not bypass a vulnerability, signing, notarization, checksum, or
  all-platform gate merely to obtain an installer.
- Never replace a published artifact in place. Correct a released defect with a
  newer patch version.
