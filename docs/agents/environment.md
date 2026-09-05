# Agent Environment Setup

Machine-level setup for this repository: toolchains, submodule, local homeserver
binaries, build reuse, containers, and signed release builds. Read
[AGENTS.md](../../AGENTS.md) first for the current QA contract; failure symptoms
live in [troubleshooting.md](troubleshooting.md).

## Matrix SDK submodule

The root workspace compiles all Matrix SDK crates directly from
`vendor/matrix-rust-sdk`. Do not replace those path dependencies with a Git URL
or fixed `rev`: the submodule gitlink is the only SDK revision pin. After
updating or switching worktrees, initialize the exact gitlink and run the guard
before compiling:

```bash
git submodule update --init --recursive vendor/matrix-rust-sdk
node scripts/check-sdk-submodule.mjs
```

If the guard rejects `Cargo.toml`, restore the five exact submodule path
dependencies. If it rejects submodule status, update the checkout to the
recorded gitlink; do not work around the failure by adding a remote SDK
revision.

## Local homeserver binaries

Local homeserver QA runners resolve `tuwunel` from the child process `PATH`;
they do not maintain a separate absolute-path probe list. Prepend local QA
binary directories before running headless or local GUI lanes.

The canonical durable search-path list lives in
[docs/qa/headless-basic-operations.md](../qa/headless-basic-operations.md#local-homeserver-binary-search-path);
keep this operational note in sync when changing it.

Search path list for local homeserver binaries:

- Host fast lane, preferred: `/tmp/koushi-desktop-local-qa-bin`
- Host fallback/test binaries: `/tmp/koushi-desktop-local-qa-bin-test`
- Docker lane: `/usr/local/bin` inside the committed Linux GUI image
- Windows/manual equivalent: `%TEMP%\koushi-desktop-local-qa-bin` or another
  synthetic, ignored QA bin directory prepended to `PATH`
- Existing user/system `PATH` entries after the QA bin directories

POSIX host example and verification:

```bash
export PATH=/tmp/koushi-desktop-local-qa-bin:/tmp/koushi-desktop-local-qa-bin-test:$PATH
tuwunel --version
```

`/tmp` is swept periodically on this host, so re-stage the binaries after a
sweep rather than assuming the directory survives.

Installing Tuwunel from source with `cargo install --git` must set
`RUMA_UNSTABLE_EXHAUSTIVE_TYPES=1`. Without it, Ruma marks many public API
structs as non-exhaustive and the homeserver fails to compile with `E0639:
cannot create non-exhaustive struct using struct expression`. On macOS, install
Tuwunel with `--no-default-features` unless a Linux-oriented build profile is
intentional: the default feature set includes deployment features such as
`systemd`/`io_uring` that are not useful for local desktop QA.

## Local gates

Enable the repo pre-commit hook once per clone: `git config core.hooksPath
.githooks`. It runs the secret scan on staged files
(`scripts/desktop-secret-scan.mjs --staged`).

Gate commands (from `apps/desktop`):

- `npm run qa:secret-scan`
- `npm run qa:wasm-check` (requires `rustup target add wasm32-unknown-unknown`)
- `npm run qa:release-gates` — structural credential-gate check plus `cargo
  check --release`. The compile step is slow on a cold target dir; use `node
  ../../scripts/desktop-release-gate-check.mjs --no-compile` for the quick
  structural pass.
- `npm run lint` — includes the IME text-input inventory gate.

Hosted CI runs via `.github/workflows/ci.yml` on every pull request; these same
gates also run locally and in `release:preflight`. Keep the local gates green
before pushing so CI confirms rather than discovers. See
[verification.md](verification.md#what-ci-actually-gates) for the job list.

## npm dependency security gate

Before running any npm-backed build, validate the checked-in lockfile against the
current npm advisory database:

```bash
npm --prefix apps/desktop audit --package-lock-only --audit-level=high
```

If npm reports any `high` or `critical` vulnerability, stop before running `npm
run build`, Tauri, a DMG build, or any other packaging command. Do not waive the
finding and do not use `npm audit fix --force`. Resolve the vulnerable dependency
to a compatible fixed version in `apps/desktop/package-lock.json`, then recreate
the local dependency tree and verify both the complete and runtime-only graphs:

```bash
npm --prefix apps/desktop ci
npm --prefix apps/desktop audit --audit-level=high
npm --prefix apps/desktop audit --omit=dev --audit-level=high
```

Proceed with the build only when all three audit commands exit successfully. The
lockfile is the reproducible security boundary: changing only an existing
`node_modules` tree is not a fix. Commit dependency-security changes on an
isolated branch so other developers and `origin/main` are unaffected until the
change is reviewed and merged.

## Rust test stack and debug information

The repository Cargo config defaults Cargo-launched tests to `RUST_MIN_STACK=4194304` with `force=false`. This avoids debug-profile libtest stack cliffs while allowing developers and CI to override it; it does not affect packaged desktop processes launched outside Cargo.

Full debug symbols are not needed for ordinary Rust tests, headless QA, or local
iteration. Prefer line tables so backtraces retain file/line locations without
retaining full DWARF data; reserve full symbols for LLDB/GDB or other debugger
work. Distribution builds should strip debug information.

When a profile is being tuned, record the selected profile in the worklog. A
large `target/` directory is disposable: after changing toolchains, profiles, or
large feature matrices, remove the stale root target rather than preserving
incompatible incremental artifacts:

```bash
rm -rf -- target
```

Do not use that cleanup for source files, `Cargo.lock`, `node_modules`, QA
artifacts, or uncommitted work. Rebuild the needed target after cleanup.

## Reusing a debug build

Build the debug app once, then reuse it with `--skip-build` (optionally
`--app-binary=PATH`) so each scenario trial skips the full Tauri rebuild:

```bash
npm --prefix apps/desktop run tauri build -- --debug --no-bundle
```

`--skip-build` reuses an existing debug binary, but the QA window-title tokens
(`koushi-desktop qa session=...`) are baked into the frontend at build time
behind `VITE_KOUSHI_QA_TITLE=1`. A binary built without that env shows the
normal product title (e.g. `Koushi · 1 unread`) instead, so the lane's
`waitForLocalLoginReady` times out with "local GUI login did not reach a ready
state. Last title: Koushi · 1 unread". The runner's own build sets this env;
when pre-building manually, also set `VITE_KOUSHI_QA_TITLE=1`, or run one lane
without `--skip-build` first to produce a QA-title binary the remaining
`--skip-build` lanes can reuse.

After changing frontend render code, run one lane without `--skip-build` before
returning to the fast loop. A stale binary can miss new DOM contracts and fail
in ways that look like product bugs.

## Linux GUI host packages

One-time package install needs `sudo`/root, but tests and smoke then run as a
normal user; no `su` or root shell is needed for the fast loop. On Ubuntu 24.04:

```bash
sudo apt-get update && sudo apt-get install -y --no-install-recommends build-essential ca-certificates curl dbus-x11 file fontconfig fonts-dejavu-core fonts-noto-color-emoji fonts-noto-core git libayatana-appindicator3-dev libnss-wrapper libssl-dev libwebkit2gtk-4.1-dev libxdo-dev librsvg2-dev pkg-config webkit2gtk-driver xvfb
cargo install tauri-driver --locked
```

Fast checks after install:

```bash
cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml
node scripts/desktop-linux-gui-qa.mjs --check-tools
node scripts/desktop-linux-gui-qa.mjs --list
```

Reuse the existing Cargo, npm, and GUI target caches during the inner loop.

## Linux GUI QA container

Docker is the reproducible release/CI recipe, not the default fast iteration
path. Run it when you need the committed lane or want to prove the recipe end to
end.

- Build the committed lane image with `docker build -f
  docker/linux-gui.Dockerfile -t koushi-desktop-linux-gui:basic-ops .`
- The committed image includes `tuwunel` (pinned `v1.7.1`) and `zstd`, so the
  local homeserver lanes run entirely inside the container.
- The Docker recipe pins Rust toolchain `1.96.0` for reproducibility.
- The lane image includes `libnss-wrapper` so the numeric container UID can be
  given a temporary passwd/group entry during DBus-authenticated GUI smoke.

Run the lane from the repo root with the workspace mounted at `/work`:

```bash
docker run --rm -it --shm-size=2g -u "$(id -u):$(id -g)" -v "$PWD:/work" -v /tmp/koushi-desktop-cargo-home:/tmp/cargo-home -v /tmp/koushi-desktop-gui-target:/tmp/koushi-desktop-gui-target -v /tmp/koushi-desktop-npm-cache:/tmp/npm-cache -w /work -e HOME=/tmp -e RUSTUP_HOME=/opt/rustup -e CARGO_HOME=/tmp/cargo-home -e CARGO_TARGET_DIR=/tmp/koushi-desktop-gui-target -e NPM_CONFIG_CACHE=/tmp/npm-cache -e PATH=/opt/cargo/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin koushi-desktop-linux-gui:basic-ops bash -c 'export RUSTC="$(rustup which rustc)"; export RUSTDOC="$(rustup which rustdoc)"; npm --prefix apps/desktop run qa:linux-gui -- --scenario=local-send --server=tuwunel --artifact-dir=/work/artifacts/linux-gui-local-send-docker --timeout-ms=180000'
```

The runner writes artifacts to `artifacts/linux-gui-local-send-docker/` inside
the mounted repo. Keep that directory ignored and inspect the run log and
screenshots there when a lane fails. Keep artifact directories
scenario-specific so retries do not blur results between lanes.

## CodeGraph

When this worktree has a `.codegraph/` directory, use CodeGraph before `rg`,
`grep`, `find`, or manual file reads for codebase-orientation questions. Prefer
`codegraph explore "<question or symbols>"` for architectural or flow questions
and `codegraph node <symbol-or-file>` for exact symbol/file source with call
context. If a new worktree lacks `.codegraph/`, initialize it with `codegraph
init .` before broad code investigation.

## Signed macOS DMG

Signed, notarized macOS artifacts have two supported paths: the protected
`release-macos` GitHub Environment used by `.github/workflows/release-desktop.yml`,
and an attended local `zsh` session on a Mac with the project's Developer ID
Application certificate and private key installed. Never commit, paste into
logs, or store in a repository file any certificate, private key, export
password, Apple credential, or notarization token.

The CI Environment holds `APPLE_CERTIFICATE`, `APPLE_CERTIFICATE_PASSWORD`,
`KEYCHAIN_PASSWORD`, `APPLE_API_ISSUER`, `APPLE_API_KEY`, and
`APPLE_API_PRIVATE_KEY`; `APPLE_SIGNING_IDENTITY` is an Environment variable.
Only the macOS jobs receive that Environment. They create an ephemeral keychain
and API-key file, run Tauri signing/notarization, require `codesign`, `stapler`,
and Gatekeeper verification, and delete temporary key material in an `always()`
step. The Environment deployment policy permits `main` only.

For a local build, list the available signing identities, then enter the
matching identity and Apple-ID notarization credentials into session-only
environment variables. `APPLE_PASSWORD` is an app-specific password, not the
normal Apple Account password:

```zsh
security find-identity -v -p codesigning
read -r "APPLE_SIGNING_IDENTITY?Developer ID Application identity: "
read -r "APPLE_ID?Apple ID: "
read -r "APPLE_TEAM_ID?Apple Team ID: "
read -rs "APPLE_PASSWORD?App-specific password: "; echo
export APPLE_SIGNING_IDENTITY APPLE_ID APPLE_TEAM_ID APPLE_PASSWORD
```

Run the signed repository DMG entry point. It invokes the macOS signing
credential preflight itself and stops before packaging if that gate fails. With
the variables present, Tauri signs the app and installer, submits them for
Apple notarization, and staples the notarization result during the build:

```zsh
npm --prefix apps/desktop run build:dmg:signed
```

Do not treat a zero build exit as sufficient release evidence. Resolve the
generated artifacts and require `codesign`, `stapler`, and Gatekeeper to accept
both the application and DMG before opening the installer. Remove only the two
generated bundle output directories before building, then validate exactly one
newly generated application and DMG. Run the whole sequence in a function so
every failed command returns before `open`:

```zsh
build_and_verify_signed_dmg() {
  local bundle_root app dmg
  local -a apps dmgs
  bundle_root="target/release/bundle"

  rm -rf -- "$bundle_root/macos" "$bundle_root/dmg" || return
  npm --prefix apps/desktop run build:dmg:signed || return
  apps=("${(@f)$(find "$bundle_root/macos" -maxdepth 1 -name '*.app' -print)}")
  dmgs=("${(@f)$(find "$bundle_root/dmg" -maxdepth 1 -name '*.dmg' -print)}")
  (( ${#apps[@]} == 1 && ${#dmgs[@]} == 1 )) &&
    [[ -n "$apps[1]" && -n "$dmgs[1]" ]] || {
    print -u2 "Expected exactly one current app and DMG artifact"
    return 1
  }
  app="$apps[1]"
  dmg="$dmgs[1]"

  codesign --verify --deep --strict --verbose=2 "$app" &&
    xcrun stapler validate "$app" &&
    spctl --assess --type execute --verbose=4 "$app" &&
    codesign --verify --verbose=2 "$dmg" &&
    xcrun stapler validate "$dmg" &&
    spctl --assess --type open --context context:primary-signature --verbose=4 "$dmg" &&
    open "$dmg"
}
build_and_verify_signed_dmg
```

After verification, remove the credentials from the current shell:

```zsh
unset APPLE_SIGNING_IDENTITY APPLE_ID APPLE_TEAM_ID APPLE_PASSWORD
```

## Automated desktop releases

The canonical preparation, monitoring, publication, and failure-recovery
procedure is [`docs/releases/desktop-release.md`](../releases/desktop-release.md).
Use the repository's `koushi-release` skill to execute that runbook. This file
owns only the signing-environment setup above; do not duplicate the release
procedure here.
