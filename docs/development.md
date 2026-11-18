# Develop Koushi

How to build, test, and package Koushi from source. For using Koushi, see the
[website](https://shinaoka.github.io/koushi-matrix/) or the
[user guide](help/README.md). Agent-facing operational detail lives in
[`agents/`](agents/).

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

Before claiming a GUI gate is green, check
[`docs/qa/known-issues.md`](qa/known-issues.md).

For the complete Linux/WSL setup, release build, and WSLg window check, use
[Build on Linux / WSL](#build-on-linux--wsl-ubuntu-2404) below.

```bash
cargo test -p koushi-state
cargo test -p koushi-search
cargo test -p koushi-key
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

### Build a macOS DMG

On macOS, use the checked-in DMG wrapper script through the desktop package:

```bash
npm --prefix apps/desktop run build:dmg
```

The repository-root shell entry point forwards the same options to that build:

```bash
./scripts/desktop-build-dmg.sh
```

The wrapper runs the release preflight check, then builds the native DMG with:

```bash
npm --prefix apps/desktop run tauri -- build --bundles app,dmg
```

The wrapper also supplies the macOS bundle version and updater configuration.
Release builds pass `KOUSHI_UPDATER_PUBLIC_KEY` to Tauri's updater configuration
and require signed updater artifacts. Local builds without an updater key omit
updater artifacts; use the wrapper for this configuration.

Useful variants:

```bash
# Print the underlying Tauri command without building.
npm --prefix apps/desktop run build:dmg -- --print-command

# Skip the local release preflight when iterating on a throwaway local build.
npm --prefix apps/desktop run build:dmg -- --skip-preflight

# Run the macOS signing preflight before building.
npm --prefix apps/desktop run build:dmg:signed

# Equivalent signed build through the shell entry point.
./scripts/desktop-build-dmg.sh --signed
```

The script verifies the app bundle's signature, then prints the generated `.dmg`
artifact path when the build completes.
Installed-app data is stored under
`~/Library/Application Support/koushi-desktop`; credentials use the macOS
Keychain service `koushi-desktop`.

To keep Keychain access across app replacements, use the signed release from
[Downloads](../README.md#downloads), or sign successive local builds with the same signing
identity. The local wrapper selects a Developer ID Application identity when
exactly one is available; without one, it explicitly signs the entire app bundle
ad hoc. An ad-hoc signature identifies one particular build, so replacing it can
require Keychain authorization again. See the
[local signing setup](agents/environment.md#signed-macos-dmg).
Replacing the `.app` does not require signing out or deleting the application
data directory or Keychain entries.

### Build on Linux / WSL (Ubuntu 24.04)

Copy each block below into a Linux **Bash** terminal, in order. On WSL 2,
use WSLg and keep the source in the Linux filesystem. Run Koushi as your
normal user. The standalone release binary has been verified on x86_64
Ubuntu 24.04 / WSL2; native Linux and installer packages remain unverified.

**1. Install build tools (once).** This installs Node.js 24 through
[nvm](https://github.com/nvm-sh/nvm#installing-and-updating) and Rust through
[rustup](https://rustup.rs/). Each block stops if a command fails.

```bash
(
  set -eo pipefail

  sudo apt-get update
  sudo apt-get install -y build-essential ca-certificates curl wget file git \
    dbus-x11 x11-utils xdg-utils \
    pkg-config libwebkit2gtk-4.1-dev libgtk-3-dev \
    libayatana-appindicator3-dev librsvg2-dev libssl-dev libdbus-1-dev \
    libxdo-dev patchelf gnome-keyring seahorse hunspell-en-us

  export NVM_DIR="$HOME/.nvm"
  if [ ! -s "$NVM_DIR/nvm.sh" ]; then
    curl -fsSL https://raw.githubusercontent.com/nvm-sh/nvm/v0.40.8/install.sh | bash
  fi
  . "$NVM_DIR/nvm.sh"
  nvm install 24

  if ! command -v rustup >/dev/null 2>&1; then
    curl --proto '=https' --tlsv1.2 -fsSL https://sh.rustup.rs | \
      sh -s -- -y --profile minimal
  fi
)
```

For other distributions, install the equivalent
[Tauri prerequisites](https://v2.tauri.app/start/prerequisites/#linux), including
D-Bus development headers.

#### Build and run the Linux release

**2. Build and install.** If you already have a checkout, open a terminal at
its repository root. Otherwise, the block clones it into `~/src/koushi-matrix`.
The first build needs network access and may take a while. Close any running
Koushi before replacing its executable.

```bash
(
  set -eo pipefail

  export NVM_DIR="$HOME/.nvm"
  . "$NVM_DIR/nvm.sh"
  nvm use 24
  if [ -f "$HOME/.cargo/env" ]; then
    . "$HOME/.cargo/env"
  fi

  if [ ! -f scripts/check-sdk-submodule.mjs ]; then
    mkdir -p "$HOME/src"
    cd "$HOME/src"
    if [ ! -d koushi-matrix ]; then
      git clone https://github.com/shinaoka/koushi-matrix.git
    fi
    cd koushi-matrix
  fi

  git submodule update --init --recursive vendor/matrix-rust-sdk
  node scripts/check-sdk-submodule.mjs
  rustup show
  npm --prefix apps/desktop audit --package-lock-only --omit=dev --audit-level=high
  npm --prefix apps/desktop ci
  # Recorded, not gating, while #1965 is open.
  npm --prefix apps/desktop audit --audit-level=high || true
  npm --prefix apps/desktop audit --omit=dev --audit-level=high
  node scripts/check-linux-build-deps.mjs

  unset CARGO_TARGET_DIR
  npm --prefix apps/desktop run tauri -- build --no-bundle
  install -Dm755 target/release/koushi-desktop "$HOME/.local/bin/koushi"
)
```

If the runtime audit (`--omit=dev`) reports high/critical vulnerabilities,
resolve them before building; do not bypass that check. Only the full-graph line
is advisory, and only while #1965 is open
([environment](agents/environment.md#npm-dependency-security-gate)).

#### Prepare the Linux keyring and retry SSO

**3. Prepare sign-in storage and launch.** Open the keyring manager:

```bash
seahorse &
```

In **Passwords and Keys**, create a password-protected **Password Keyring** if
needed, select **Set as default**, and unlock it. Enter its password only in
the system dialog. Installing GNOME Keyring alone does not complete this step.

Then launch Koushi from the **same terminal and desktop D-Bus session**:

```bash
"$HOME/.local/bin/koushi"
```

Select **Single sign-on** and keep Koushi running until the browser returns.
If Koushi was running before you prepared the keyring, fully exit and restart
it. No rebuild is needed.

If there is no usable session D-Bus, run `dbus-run-session -- bash` first,
then open Seahorse and launch Koushi inside that shell. A new bus alone does
not create or unlock a keyring.

On WSLg, if the window does not appear, try:

```bash
GDK_BACKEND=x11 \
LIBGL_ALWAYS_SOFTWARE=1 \
WEBKIT_DISABLE_COMPOSITING_MODE=1 \
"$HOME/.local/bin/koushi"
```

App data lives in `~/.local/share/koushi-desktop`; credentials use the
freedesktop Secret Service with service name `koushi-desktop`.

#### Check external browser launch

If SSO does not open a browser, run these in the same desktop session:

```bash
xdg-open https://example.com
xdg-settings get default-web-browser
```

Set a default browser if the second command prints no desktop entry. If SSO
still fails with an unlocked default keyring, report the homeserver and the
approximate time until the error appears.

#### Run the Linux development build

For development, run this at the repository root after setup and keyring
preparation. It starts Vite and the native app together:

```bash
npm --prefix apps/desktop run tauri -- dev
```

For a standalone debug binary:

```bash
(
  set -eo pipefail
  unset CARGO_TARGET_DIR
  npm --prefix apps/desktop run tauri -- build --debug --no-bundle
  ./target/debug/koushi-desktop
)
```

`npm run build` builds only web assets. A plain Cargo build expects a Vite
server; use the Tauri commands above to embed the assets in a standalone app.

#### Build Linux packages

To install Koushi in the desktop application menu on Ubuntu, build a `.deb`.
After installing the build tools above, close Koushi and paste this block at
the repository root:

```bash
(
  set -eo pipefail

  export NVM_DIR="$HOME/.nvm"
  . "$NVM_DIR/nvm.sh"
  nvm use 24
  if [ -f "$HOME/.cargo/env" ]; then
    . "$HOME/.cargo/env"
  fi
  unset CARGO_TARGET_DIR

  git submodule update --init --recursive vendor/matrix-rust-sdk
  node scripts/check-sdk-submodule.mjs
  npm --prefix apps/desktop audit --package-lock-only --omit=dev --audit-level=high
  npm --prefix apps/desktop ci
  # Recorded, not gating, while #1965 is open.
  npm --prefix apps/desktop audit --audit-level=high || true
  npm --prefix apps/desktop audit --omit=dev --audit-level=high
  node scripts/check-linux-build-deps.mjs

  npm --prefix apps/desktop run tauri -- build --bundles deb

  version=$(node -p \
    "JSON.parse(require('fs').readFileSync('apps/desktop/src-tauri/tauri.conf.json','utf8')).version")
  arch=$(dpkg --print-architecture)
  sudo apt install -y \
    "./target/release/bundle/deb/Koushi_${version}_${arch}.deb"
)
```

The `.deb` is created in `target/release/bundle/deb/`. After installation,
search for **Koushi** in your desktop application menu, or launch it from a
terminal:

```bash
/usr/bin/koushi-desktop
```

Sign-in still requires the [unlocked default keyring](#prepare-the-linux-keyring-and-retry-sso).
On WSL, launching the Linux desktop app requires WSLg. These packages remain
unverified.

To build AppImage, deb, and RPM together on x86_64 Linux instead:

```bash
npm --prefix apps/desktop run build:linux
```

Outputs and checksums are printed after the build. Packages are under
`target/x86_64-unknown-linux-gnu/release/bundle/{appimage,deb,rpm}/`.

### Windows users

Close Koushi, then run the following commands from the repository root. The
commands remove previous generated NSIS installers before building and select
the installer matching the checked-out source version. PowerShell must check
each external command's exit code so a failed build cannot install an old file.

```powershell
$ErrorActionPreference = 'Stop'
git submodule update --init --recursive vendor/matrix-rust-sdk
if ($LASTEXITCODE -ne 0) { throw 'SDK submodule update failed' }
node scripts/check-sdk-submodule.mjs
if ($LASTEXITCODE -ne 0) { throw 'SDK submodule check failed' }
npm --prefix apps/desktop audit --package-lock-only --omit=dev --audit-level=high
if ($LASTEXITCODE -ne 0) { throw 'Runtime dependency security audit failed' }
npm --prefix apps/desktop ci
if ($LASTEXITCODE -ne 0) { throw 'Dependency installation failed' }

$nsisDir = Join-Path (Get-Location).Path 'target/x86_64-pc-windows-msvc/release/bundle/nsis'
if (Test-Path -LiteralPath $nsisDir) {
    Get-ChildItem -LiteralPath $nsisDir -Filter *.exe -File |
        ForEach-Object { Remove-Item -LiteralPath $_.FullName }
}
npm --prefix apps/desktop run build:windows
if ($LASTEXITCODE -ne 0) { throw 'Windows build failed' }

$version = (Get-Content apps/desktop/src-tauri/tauri.conf.json -Raw | ConvertFrom-Json).version
$installer = Get-Item -LiteralPath (Join-Path $nsisDir "Koushi_${version}_x64-setup.exe")
$process = Start-Process $installer.FullName -ArgumentList '/S' -Wait -PassThru
if ($process.ExitCode -ne 0) { throw "Installation failed: exit code $($process.ExitCode)" }
```

## Deterministic README screenshot

From the repository root, regenerate the checked-in screenshot in the pinned
Playwright container. The container runs as root so an existing root-owned
`node_modules` or output file cannot block regeneration; the exit trap restores
ownership to the invoking user.

```bash
image=mcr.microsoft.com/playwright:v1.60.0-noble
for run in 1 2; do
  docker run -i --rm --init --shm-size=2g --user 0:0 \
    -e HOST_UID="$(id -u)" -e HOST_GID="$(id -g)" \
    -v "$PWD:/work" -w /work "$image" \
    bash -e -u -o pipefail -s <<'EOF'
trap 'chown -R "$HOST_UID:$HOST_GID" /work/apps/desktop/node_modules /work/apps/desktop/test-results /work/assets/screenshots 2>/dev/null || true' EXIT
npm --prefix apps/desktop ci
npm --prefix apps/desktop run docs:screenshot
EOF
  sha256sum assets/screenshots/koushi-main.png
done
```

Both runs must print the same SHA-256. CI asserts `@playwright/test` is exactly
`1.60.0`, regenerates the image, and rejects any byte or porcelain difference.
When bumping Playwright, update the package manifest and lockfile, the image
version, and the exact version assertion together; then run the pinned command
twice again and require identical hashes before updating this documentation.

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

See `docs/architecture/overview.md`, `docs/architecture/desktop-foundation.md`,
and `docs/architecture/tauri-react-shell.md` for the architecture.
