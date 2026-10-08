# Settings and help

[Back to the user guide](README.md)

## Open settings

Select the account icon at the bottom of the left rail to open **Account
Settings** for that account. Select the gear in the top bar to open **App
Settings**, which apply across account tabs. On macOS, the **Koushi** menu has
these as separate **Account Settings…** and **App Settings…** items; **Cmd+,**
opens **App Settings**. Choosing either item while settings are already open
switches directly to that scope. When an account is selected, use the button at
the bottom of the category list to switch between settings scopes.

Both scopes open in a foreground dialog. Choose a category on the left; its
controls appear on the right. Each side scrolls when the window is small. Close
the dialog with its close button or **Esc**. With focus on a category, use
Up/Down or Home/End to select another category.

Most switches and choices save when selected. Profile and account forms have
their own action buttons. Availability depends on your account, homeserver,
platform, and encryption state; disabled controls are not a promise that the
operation is currently available.

## Where to find each setting

Paths start with **App Settings** or **Account Settings**. Names match the
English UI; Japanese category names are included to help find them in a
translated app.

| Scope and category | Settings and actions |
| --- | --- |
| App Settings → Appearance (外観) | Language (English or 日本語); theme; display density; UI font and emoji style. |
| App Settings → Notifications (通知) | Notification sounds and badge counts. Operating-system permissions also apply. |
| App Settings → Preferences (環境設定) | Code-block wrapping; hiding removed messages; close to tray where configurable; automatic loading of older messages; placement of threaded conversations at their latest reply. |
| App Settings → Keyboard (キーボード) | Send-message shortcut (Enter or the platform modifier+Enter); reference list of keyboard shortcuts and their availability. |
| App Settings → Search history (検索履歴) | Shared background-work speed and pause/resume control. Search crawling and media prefetch use this budget across all accounts. **Off** pauses both. |
| App Settings → Help & About (ヘルプと情報) | Public GitHub repository URL, copy button, and instructions for asking ChatGPT or another AI assistant. |
| Account Settings → Account (アカウント) | Profile display name and avatar; account management page when provided by the server; change password and deactivate account when supported. |
| Account Settings → Sessions (セッション) | Homeserver, user ID, device ID/name, verification, cross-signing, backup and local-store information; Sign out. This is current-session information, not a list of all remote devices. The status shown is the last checked result; Koushi re-checks it automatically when the app or account starts and after a connection outage, and the refresh action checks again immediately. |
| Account Settings → Notifications (通知) | Desktop notifications and **Show message content in notifications** (off by default; notifications show counts only until you turn it on); account-wide notification rules and **Email notifications**. See [Email notifications](#email-notifications). |
| Account Settings → Security & Privacy (セキュリティとプライバシー) | URL previews in unencrypted/encrypted rooms; sending read receipts and typing notifications. |
| Account Settings → Encryption (暗号化) | Identity/session verification and trust; secure backup setup and passphrase changes; recovery; encrypted room-key import/export; local-encryption diagnostics and local-data reset. Actions appear according to the current encryption state. |
| Account Settings → Search history (検索履歴) | Indexing of media captions and file names; crawler activity, per-room progress and start/stop actions; rebuild search database. |

Use **Ctrl + -** / **Ctrl + +** (on macOS
**Cmd + -** / **Cmd + +**) to shrink or enlarge the whole interface, and
**Ctrl + 0** (macOS **Cmd + 0**) to reset it. **Ctrl/Cmd + =** also enlarges
the interface on keyboards with an unshifted equals key.
The **View** menu also offers **Zoom In**, **Zoom Out**, and **Actual Size**.
Koushi's native menu labels follow the app language; in Japanese, **View** is **表示**.
On macOS, **Cmd + Ctrl + F** toggles fullscreen. These window shortcuts
remain available while an in-app dialog is open.

The category order follows Element where Koushi has corresponding settings.
Koushi-specific local indexing lives in **Account Settings → Search history**;
its shared cross-account background-work budget is in **App Settings → Search
history**. Koushi does not have all Element settings; consult this table instead
of assuming exact parity. Room-specific settings remain in the room's own menu
and details panel.

On macOS, choose **Koushi → Check for Updates…** to open **Software update**
directly, including before sign-in. This is separate from account settings;
you do not need to navigate through Preferences or Display.
**Automatically check for updates** checks once when the app starts and then
every 24 hours while enabled (enabled by default). Its switch is in the same
Software update dialog. **Include pre-release versions** also considers
SemVer versions such as `1.2.0-alpha.1`, `1.2.0-beta.1`, and `1.2.0-rc.1`.
Use **Check for updates** for an immediate result; when no newer version is
found, the result says that the current version is up to date. An automatic
discovery opens the same screen. Downloading and restarting are separate,
explicit actions; automatic checks do not automatically install or restart.
Unsupported builds show that in-app updates are unavailable. When Koushi was
installed by a distribution package that manages its files (for example an AUR
package), the dialog instead says that updates are provided by your package
manager; update Koushi through that package manager.
Changing the pre-release setting discards an unapproved candidate and checks the
new channel when automatic checks are enabled. Once you choose Download update,
that release stays selected; the pre-release switch is disabled until the
download/install flow ends. Turning automatic checks off does not remove an
already offered release or stop a manually requested check.

On macOS, dialogs and viewers leave space above their contents for the standard
window buttons. Small windows scroll within the dialog. Escape closes the
topmost dismissible dialog and returns focus to its opener.

For consequences and prerequisites, see [Search](search.md),
[Security and recovery](security-and-recovery.md), and
[User trust model](user-trust-model.md) before changing the corresponding settings.

## Email notifications

**Account Settings → Notifications → Email notifications** has two separate parts:

- The **Email notifications** switch controls delivery. **Off** means your
  homeserver does not email you summaries, even when an address is listed.
  With several addresses, **Send to** chooses the single delivery target.
- **Registered email addresses** lists the email addresses your homeserver has
  registered for your Matrix account. Koushi does not create them. Each shows
  **Email confirmed** (Japanese: メールアドレス確認済み), meaning the homeserver
  records that ownership of the address was confirmed by email; the current
  delivery target shows **Email confirmed · Notification target**. This is
  unrelated to device or contact verification (検証), and a listed address is
  not a guarantee that mail to it is delivered. **Why is this address shown?**
  repeats this explanation.

What you can do with addresses depends on the server:

- When adding is available, **Add email address** (or **Add another email
  address**) sends a confirmation email. Open its link, then press
  **Continue**; a password prompt may appear. The new address is registered
  after confirmation; if email notifications are on, they move to it from
  the current address, which stays registered.
- **This server does not allow adding email addresses here.** means the server
  disallows adding addresses from clients. Addresses already listed stay
  registered, and you can still turn email notifications on or off for them.
- If your account uses an account management page, **Email addresses for this
  account are managed on your account page.** appears with **Manage account &
  devices** when the server provides that page.

## Ask for help

Open the desktop **Help → Koushi Help** menu, including before sign-in, or
**App Settings → Help & About**. Choose **Copy GitHub URL**, then paste it into
ChatGPT or another AI assistant with your Koushi version, operating system, and
question. The button copies only the public repository URL. If copying fails,
select and copy the displayed URL manually.

The [guide index](README.md) explains how to find instructions for your installed
release. The desktop Help menu opens this short assistance dialog; the keyboard
shortcut reference is under **App Settings → Keyboard**.
