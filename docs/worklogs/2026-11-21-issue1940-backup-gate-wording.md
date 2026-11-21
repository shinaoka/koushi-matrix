# The disabled-backup gate says the requirement (#1940)

The gate for an account whose secure backup another client switched off read as
an offer: "Confirm only if you want those clients to use the updated backup."
The screen offers exactly two actions — **Re-enable secure backup** and **Sign
out** — so a reader who declined had nowhere to go. #1940 asked for either a
read-only third action or plain wording; the decision was wording, and no
read-only path was added.

## What changed

`gate.secureBackupExplicitDisabledCopy` (all five locales) now states the
requirement and what re-enabling does to the other signed-in clients:

> Koushi requires Secure Backup on this account. Re-enabling creates the
> account's backup, and the other Matrix clients signed in to this account need
> its recovery key before they can read encrypted history.

What went is the "only if you want" clause, which promised a choice the two
buttons do not offer. The other-clients sentence is not cosmetic: in this gate
kind the server holds no backup at all (`Server::Absent` in
`crates/koushi-sdk/src/e2ee.rs:359-363`), so re-enabling bootstraps a new backup
and shows a new recovery key
(`setup_secure_backup_with_confirmation`, `:3014-3042`, which refuses if any
backup exists). The other clients do not adopt it on their own — a client still
uploading to the old version gets `WrongRoomKeysVersion` and the vendored SDK
disables its backup
(`vendor/matrix-rust-sdk/crates/matrix-sdk/src/encryption/backups/mod.rs:797-806`).
Saying they "use the same backup" would have been the same defect the issue
reports, one layer down.

The title (**Secure backup was disabled**) is unchanged — it names the state,
while the requirement sits above it in the gate heading (**Secure backup
required**) and now in the copy as well.

`docs/help/security-and-recovery.md` gained the matching paragraph in
**Complete secure backup setup**: the gate says the backup was disabled while it
shows that, the only ways on are re-enable or sign out, there is no read-only
way past it, and re-enabling creates the account's backup and shows a key the
other signed-in clients need. The scope is on the gate, not on Secure Backup:
`admits_cached_reading()` (`crates/koushi-state/src/state/session.rs:405-424`)
lets a pending or retried check keep reading, which is what the page's
**Read while secure backup is still being checked** section already says, and
only `SetupRequired`, `SecureStorageIncomplete`, `ExistingBackupNeedsRecovery`
and this gate close it.

Terminology follows each catalog's own gate family rather than the newer
Settings page wording: ja uses 安全なバックアップ (`gate.secureBackupTitle`,
`gate.secureBackupReenable`), not セキュアバックアップ, which is
`settings.secureBackup`'s term. Each locale names sign-out with its own
`gate.signOut` string (サインアウト / 로그아웃 / 退出登录 / 登出).

## Why one string and not two

The same id is rendered twice: once as the gate paragraph, once inside the
re-enable confirmation region (`SessionVerificationGate.tsx:599,614`). That reuse
is the canon, not an accident: `docs/architecture/i18n.md:131-134` requires the
confirmation to use the existing catalog IDs for its warning rather than prose
from Core, so deleting the second `<p>` would remove a warning the rules put
there. The wording is therefore descriptive rather than instructional — a
statement about the requirement and the other clients reads correctly as a gate
paragraph and as the confirmation's warning, where "re-enable it, or sign out"
would have described an action already in progress.

## Verification

- RED first: `the disabled-backup gate says the requirement, not a choice
  (#1940)` in `apps/desktop/src/i18n/messages.test.ts` failed on the old English
  copy (`expected 'Re-enabling Secure Backup changes thi…' to match /requires
  Secure Backup/`) before any string changed. It pins the requirement phrase and
  the absence of the old offer idiom (`only if` / 場合のみ / 경우에만 / 仅当 /
  僅在) in every locale. It does not pin sign-out wording — that belongs to
  `gate.signOut`, and the copy no longer names the buttons at all.
- The existing gate test (`SessionVerificationGate.test.tsx:870`, "requires
  explicit confirmation before re-enabling an account-wide disabled backup")
  still passes unchanged: it matches `/other Matrix clients/i`, which the new
  copy keeps.
- `npm run typecheck` clean, `npm run lint` no issues, `npm test -- --run`
  **211 files / 2634 tests passed**.
- `node scripts/user-help.mjs` (11 pages, links, llms.txt) and
  `node scripts/check-agents-docs.mjs` pass; `llms.txt` is unchanged because no
  heading moved.

## Not changed

- No read-only entry into the app. #462's restricted state is what the *other*
  gate kinds do (encrypted sending stays closed while the gate is up); this gate
  is full-screen by design, and adding a third action is a product and
  state-machine change, not a wording one.
- The gate title, the button labels, and `SessionVerificationGate.tsx` itself.
  The re-enable button stays clickable while the confirmation is open, where a
  second click sets the same flag again; that is pre-existing behaviour and not
  a copy defect.
- The site (`site/content/*.json`) does not carry this string.
- `gate.secureBackupSetupCopy` ("before encrypted messaging is available") says
  less than the `setupRequired` gate does: reading is closed there too
  (`session.rs:415-424`). Same defect class as #1940, different gate — it wants
  its own issue, not a ride on this copy change.

## Review round

One `independent-review` pass (GLM) on the finished diff: 3 IMPORTANT, 6 MINOR.

Fixed: the other-clients sentence claimed the other clients "will use the same
backup" when a client on the old version gets `WrongRoomKeysVersion` and disables
its backup; the help paragraph stated a blanket "Secure Backup is required to use
Koushi" that contradicts the page's own pending-check exception; the test pinned
sign-out wording and asserted only `/Matrix/` for four locales, so an inverted
translation would have passed; ja said 再度有効 bare where the gate family pairs
the noun; zh-Hans kept the "account-wide" calque and pointed at "the same backup".

Rejected: deleting the duplicate warning `<p>` in the confirmation region (the
canon requires that warning and the rewritten sentence reads correctly there);
dropping the requirement from the body because the heading already says it (the
reporter read past the heading — that is the whole bug); replacing the five
requirement regexes with one marker (they are what keeps the stance pinned per
locale; a deliberate rewrite should revisit the test, and it says so here).
