# Settings and help

[Back to the user guide](README.md)

## Open user settings

Select **User settings** at the bottom of the left rail. On macOS, you can also
use **Koushi → User Settings** or **Cmd+,**. Settings open in a foreground dialog.
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
| Preferences (環境設定) | Code-block wrapping; URL previews in unencrypted/encrypted rooms; hiding removed messages; close to tray where configurable; automatic update checks and restart to install on macOS; automatic loading of older messages; placement of threaded conversations at their latest reply. |
| Keyboard (キーボード) | Send-message shortcut (Enter or the platform modifier+Enter); reference list of keyboard shortcuts and their availability. |
| Security & Privacy (セキュリティとプライバシー) | Sending read receipts and typing notifications. |
| Encryption (暗号化) | Identity/session verification and trust; secure backup setup and passphrase changes; recovery; encrypted room-key import/export; local-encryption diagnostics and local-data reset. Actions appear according to the current encryption state. |
| Search history (検索履歴) | Crawl speed; indexing of media captions and file names; pause/resume crawler; crawler activity, per-room progress and start/stop actions; rebuild search database. |
| Help & About (ヘルプと情報) | Public GitHub repository URL, copy button, and instructions for asking ChatGPT or another AI assistant. |

The category order follows Element where Koushi has corresponding settings.
Koushi-specific local indexing lives in **Search history**. Koushi does not have
all Element settings; consult this table instead of assuming exact parity.
Room-specific settings remain in the room's own menu and details panel.

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
