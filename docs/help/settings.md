# Settings and help

[Back to the user guide](README.md)

## Open user settings

Select **User settings** at the bottom of the left rail. On macOS, you can also
use **Koushi → Settings…** or **Cmd+,**. Settings open in a foreground dialog.
Choose a category on the left; its controls appear on the right. Each side
scrolls when the window is small. Close with **Close User settings** or **Esc**.
With focus on a category, use Up/Down or Home/End to select another category.

Most switches and choices save when selected. Profile and account forms have
their own action buttons. Availability depends on your session, homeserver,
platform, and encryption state; disabled controls are not a promise that the
operation is currently available.

## Where to find each setting

Start each path below with **User settings →**. Names match the English UI;
Japanese category names are included to help find them in a translated app.

| Category | Settings and actions |
| --- | --- |
| Account (アカウント) | Language (Default (English), English, Japanese); profile display name and avatar; saved-account switching; account management page when provided by the server; change password and deactivate account when supported. |
| Sessions (セッション) | Homeserver, user ID, device ID/name, verification, cross-signing, backup and local-store information; Sign out. This is current-session information, not a list of all remote devices. |
| Appearance (外観) | Theme; display density; UI font and emoji style. |
| Notifications (通知) | Desktop notifications, notification sounds and badge counts. Operating-system permissions also apply. |
| Preferences (環境設定) | Code-block wrapping; URL previews in unencrypted/encrypted rooms; hiding removed messages; close to tray where configurable; automatic loading of older messages; placement of threaded conversations at their latest reply. |
| Keyboard (キーボード) | Send-message shortcut (Enter or the platform modifier+Enter); reference list of keyboard shortcuts and their availability. |
| Security & Privacy (セキュリティとプライバシー) | Sending read receipts and typing notifications. |
| Encryption (暗号化) | Identity/session verification and trust; secure backup setup and passphrase changes; recovery; encrypted room-key import/export; local-encryption diagnostics and local-data reset. Actions appear according to the current encryption state. |
| Search history (検索履歴) | Crawl speed; indexing of media captions and file names; pause/resume crawler; crawler activity, per-room progress and start/stop actions; rebuild search database. |
| Help & About (ヘルプと情報) | Public GitHub repository URL, copy button, and instructions for asking ChatGPT or another AI assistant. |

Use **Ctrl + -** / **Ctrl + +** (on macOS
**Cmd + -** / **Cmd + +**) to shrink or enlarge the whole interface, and
**Ctrl + 0** (macOS **Cmd + 0**) to reset it. **Ctrl/Cmd + =** also enlarges
the interface on keyboards with an unshifted equals key.
The **View** menu also offers **Zoom In**, **Zoom Out**, and **Actual Size**.
On macOS, **Cmd + Ctrl + F** toggles fullscreen. These window shortcuts
remain available while an in-app dialog is open.

The category order follows Element where Koushi has corresponding settings.
Koushi-specific local indexing lives in **Search history**. Koushi does not have
all Element settings; consult this table instead of assuming exact parity.
Room-specific settings remain in the room's own menu and details panel.

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
Unsupported builds show that in-app updates are unavailable.
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

## Ask for help

Open the desktop **Help → Koushi Help** menu, including before sign-in, or
**User settings → Help & About**. Choose **Copy GitHub URL**, then paste it into
ChatGPT or another AI assistant with your Koushi version, operating system, and
question. The button copies only the public repository URL. If copying fails,
select and copy the displayed URL manually.

The [guide index](README.md) explains how to find instructions for your installed
release. The desktop Help menu opens this short assistance dialog; the keyboard
shortcut reference is under **User settings → Keyboard**.
