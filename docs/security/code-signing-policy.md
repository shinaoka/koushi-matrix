# Windows code signing policy

Status: proposed for the SignPath Foundation application. Production signing
is not active until the application and protected CI setup are approved.

Proposed provider attribution, to remain with the policy when signing is
approved:

> Free code signing provided by SignPath.io, certificate by SignPath Foundation.

## Current distribution status

Koushi's Windows x64 NSIS release remains explicitly unsigned until the
project has an approved signing provider and a passing release gate. The
canonical distribution channel is the project's GitHub Releases page.

## Proposed production policy

The project plans to use SignPath Foundation / SignPath.io for free OSS code
signing, subject to approval. The intended scope is limited to Koushi-built
Windows artifacts from `shinaoka/koushi-matrix`:

- the Koushi Windows application executable; and
- the public Windows NSIS installer.

Third-party or upstream executables will not be re-signed as Koushi artifacts.

Production builds will run on GitHub-hosted runners. The signing private key
will remain in SignPath's managed signing infrastructure and will not be stored
in this repository, GitHub Actions secrets, or build artifacts. Every
production signing request will require manual approval by an authorized
approver. Authenticode verification will pass before publication, and the
SHA-256 checksum will be generated from the final signed installer.

## Roles

The project roles are held by the following two maintainers:

- Authors / Maintainers: [Hiroshi Shinaoka](https://github.com/shinaoka),
  [Satoshi Terasaki](https://github.com/terasakisatoshi)
- Reviewers: [Hiroshi Shinaoka](https://github.com/shinaoka),
  [Satoshi Terasaki](https://github.com/terasakisatoshi)
- Signing Approvers: [Hiroshi Shinaoka](https://github.com/shinaoka),
  [Satoshi Terasaki](https://github.com/terasakisatoshi)

Contributors whose changes affect source, dependencies, packaging, release
automation, or signing configuration require review before merge.

## Privacy and network communication

Koushi is a Matrix desktop client. It communicates with Matrix homeservers and
related services selected, configured, or requested by the user. Code signing
does not imply that the application makes no network connections. Any
telemetry or additional third-party data transfer must be documented before it
is enabled in a release.

This section is the project's current public network-communication policy for
the SignPath application. It is linked from the repository README and will be
expanded into a separate privacy document if the project's data-transfer scope
grows beyond Matrix services, user-requested media/link requests, and the
documented update mechanisms.

## System changes and uninstall

The Windows NSIS installer uses the current-user installation mode. Users can
remove Koushi through the normal Windows application-management flow, such as
Settings → Apps → Installed apps → Koushi → Uninstall, or the corresponding
Control Panel uninstall entry. The installer does not require administrator
privileges for the current-user installation.

Application data and credentials may remain in the user's profile or Windows
Credential Manager after uninstall. Users who want a local-state reset should
use Koushi's documented reset flow rather than deleting files while the app is
running.

## Incident response

If a signing account, release workflow, source revision, or published artifact
is suspected to be compromised, production signing will stop while the project
preserves audit data, investigates the affected revision, and publishes a
corrected version or advisory as appropriate. Published artifacts will not be
silently replaced.
