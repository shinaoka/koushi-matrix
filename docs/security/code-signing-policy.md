# Windows code signing policy

Status: draft. SignPath Foundation application and production signing setup
are not complete.

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

- Authors / Maintainers: Hiroshi Shinaoka, Satoshi Terasaki
- Reviewers: Hiroshi Shinaoka, Satoshi Terasaki
- Signing Approvers: Hiroshi Shinaoka, Satoshi Terasaki

Contributors whose changes affect source, dependencies, packaging, release
automation, or signing configuration require review before merge.

## Privacy and network communication

Koushi is a Matrix desktop client. It communicates with Matrix homeservers and
related services selected, configured, or requested by the user. Code signing
does not imply that the application makes no network connections. Any
telemetry or additional third-party data transfer must be documented before it
is enabled in a release.

## Incident response

If a signing account, release workflow, source revision, or published artifact
is suspected to be compromised, production signing will stop while the project
preserves audit data, investigates the affected revision, and publishes a
corrected version or advisory as appropriate. Published artifacts will not be
silently replaced.
