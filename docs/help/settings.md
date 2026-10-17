# Settings and help

[Back to the user guide](README.md)

## Open settings

Select the account icon at the bottom of the left rail to open **Account
Settings** for that account. Select the gear in the top bar to open **App
Settings**, which apply across account tabs. On macOS, the **Koushi** menu has
these as separate **Account Settings…** and **App Settings…** items; **Cmd+,**
opens **App Settings**. Choosing either item while settings are already open
switches directly to that scope. With **Account Settings** open, use the button
at the bottom of the category list to switch to **App Settings**; to return,
open **Account Settings** from the rail or the **Koushi** menu.

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
| App Settings → Appearance (外観) | Language (English, 日本語, 简体中文, 繁體中文, or 한국어); theme; display density; UI font and emoji style. |
| App Settings → Notifications (通知) | Two tabs: **OS notifications** (notification sounds and badge counts) and **Sent from this device** (the destinations described under [Notifications from this device](#notifications-from-this-device)). Operating-system permissions also apply. |
| App Settings → Preferences (環境設定) | Code-block wrapping; hiding removed messages; close to tray where configurable; automatic loading of older messages; placement of threaded conversations at their latest reply. |
| App Settings → Keyboard (キーボード) | Send-message shortcut (Enter or the platform modifier+Enter); reference list of keyboard shortcuts and their availability. |
| App Settings → Search history (検索履歴) | Shared background-work speed and pause/resume control. Search crawling and media prefetch use this budget across all accounts. **Off** pauses both. |
| App Settings → Help & About (ヘルプと情報) | Public GitHub repository URL, copy button, and instructions for asking ChatGPT or another AI assistant. |
| Account Settings → Account (アカウント) | Profile display name and avatar; account management page when provided by the server; change password when supported. **Deactivate account** sits in a separate **Danger zone** (危険な操作) block and is shown only for password-based accounts; accounts managed by an external provider use the account management page instead. |
| Account Settings → Sessions (セッション) | Homeserver, user ID, device ID/name, verification, cross-signing, backup and local-store information; Sign out. This is current-session information, not a list of all remote devices. The status shown is the last checked result; Koushi re-checks it automatically when the app or account starts and after a connection outage, and the refresh action checks again immediately. |
| Account Settings → Notifications (通知) | Three tabs, **OS notifications** first. **OS notifications** (OS通知): desktop notifications and **Show message content in notifications**. **Sent from this device** (この端末から送信): destinations Koushi sends to from this computer (email or WeChat). **Homeserver notifications** (ホームサーバー通知): account-wide notification rules and **Email notifications**, which apply to every device. See [OS notifications](#os-notifications), [Notifications from this device](#notifications-from-this-device), and [Email notifications](#email-notifications). |
| Account Settings → Security & Privacy (セキュリティとプライバシー) | URL previews in unencrypted/encrypted rooms; sending read receipts and typing notifications. |
| Account Settings → Encryption (暗号化) | Identity/session verification and trust; secure backup setup and passphrase changes; recovery; encrypted room-key import/export; local-encryption diagnostics and local-data reset. Actions appear according to the current encryption state. |
| Account Settings → Search history (検索履歴) | Indexing of media captions and file names; crawler activity, per-room progress and start/stop actions; rebuild search database. |

Use **Ctrl + -** / **Ctrl + +** (on macOS
**Cmd + -** / **Cmd + +**) to shrink or enlarge the whole interface, and
**Ctrl + 0** (macOS **Cmd + 0**) to reset it. **Ctrl/Cmd + =** also enlarges
the interface on keyboards with an unshifted equals key.
The **View** menu also offers **Zoom In**, **Zoom Out**, and **Actual Size**.
Koushi's native menu labels follow the app language; in Japanese, **View** is **表示**.
The language list shows each language in its own script. **简体中文** is Simplified
Chinese, **繁體中文** is Traditional Chinese with Taiwan vocabulary, and **한국어** is
Korean. Desktop notification titles and bodies are currently English in every
language.
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
discovery opens the same screen. Closing the dialog, pressing Escape, or
leaving it unanswered is temporary: the next scheduled or manual check presents
the offer again without reopening immediately. **Ignore this version** stops
automatic reminders for that exact version until Koushi exits; there is no
in-app way to undo it, and signing out or switching accounts does not reset it.
While a version is ignored, the Software update dialog says so and
**Download update** remains available. Background checks continue, a newer
eligible release is still announced, and a manual check still reports the
ignored version. If a later check cannot reach the feed, the existing offer
stays available and the dialog reports the failed check. Downloading and
restarting are separate,
explicit actions; automatic checks do not automatically install or restart.
Unsupported builds show that in-app updates are unavailable. When Koushi was
installed by a distribution package that manages its files (for example an AUR
package), the dialog instead says that updates are provided by your package
manager; update Koushi through that package manager.
Changing the pre-release setting discards an unapproved candidate and checks the
new channel when automatic checks are enabled. Once you choose Download update,
that release stays selected; the pre-release switch is disabled until the
download/install flow ends. Turning automatic checks off does not remove an
already offered release or stop a manually requested check. An undownloaded
offer does not block later checks: when a newer eligible release exists, the
next check replaces the old offer with the newest one.

On Windows, release builds with updater trust material offer separate download
and restart-to-install actions. Downloads are signature-verified before they
become ready. Koushi closes its sessions before launching the installer, which
reopens the updated app. Builds without updater trust material show a
notification and open that specific Release page instead. Windows installers
remain Authenticode-unsigned pending code-signing approval, so Windows may
still show publisher warnings. Linux builds remain unsupported by the in-app updater.

On macOS, dialogs and viewers leave space above their contents for the standard
window buttons. Small windows scroll within the dialog. Escape closes the
topmost dismissible dialog and returns focus to its opener.

For consequences and prerequisites, see [Search](search.md),
[Security and recovery](security-and-recovery.md), and
[User trust model](user-trust-model.md) before changing the corresponding settings.

## Deactivate your account

Deactivating an account is permanent and is **not** the same as signing out.
The account cannot sign in again and its user ID can never be reused. To only
remove this computer's session, use **Sign out** instead.

1. Open **Account Settings → Account** for the account you want to deactivate.
2. In **Danger zone**, choose **Deactivate account**.
3. Check the account shown under **Account to deactivate**, read the list of
   consequences, and optionally turn on **Delete all my messages**.
4. Enter the account password and choose **Continue**.
5. Choose **Deactivate permanently** to confirm, or **Cancel** to stop. Nothing
   is sent to the server before this step.

If the password is wrong, Koushi shows an error and the account stays active.
After a successful deactivation, Koushi signs this computer out of the account.

## OS notifications

The **OS notifications** tab holds the alerts your operating system shows on
this computer. **Desktop notifications** turns them on or off.

On macOS, turning on Desktop notifications requests system permission. If it was
previously denied, allow Koushi in macOS System Settings. Clicking an OS
notification opens the room that raised it; it does not jump to an exact
message.

**Show message content in notifications** is **off** by default, so a banner
names the room and the sender without the message. Turned on, a banner also shows
the start of the message, and your operating system may keep that text in its
notification history after the banner closes. Koushi shows the text only for a
new message it has decrypted and read itself, cuts it to a short single line, and
never writes it to its logs. The switch is saved for this account on this device
only, and it does not change what any destination under **Sent from this device**
receives.

## Email notifications

**Homeserver notifications** sends mail only if your homeserver supports email
notifications, which the tab says next to the switch. matrix.org does not
send notification emails; to be emailed there, add a destination under
[Sent from this device](#notifications-from-this-device) instead.

**Account Settings → Notifications → Homeserver notifications → Email
notifications** has two separate parts:

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

## Notifications from this device

**Sent from this device**, the middle tab of **Account Settings →
Notifications**, lists the destinations Koushi sends to itself, from this
computer, through a service you set up. Because Koushi sends them, they work even
when your homeserver cannot send email. The **Homeserver notifications** tab is
your server's own and independent of these. The destinations are app-wide, so
the **Sent from this device** tab of **App Settings → Notifications** shows the
same list. The app settings have no homeserver tab, because they belong to no
account.

There is no switch to turn a destination on: a destination sends once its fields,
its credential, and at least one target are set, and its row then reads
**Configured**; otherwise it reads **Not configured**. Each row also summarises
what the destination sends for, how much message content it includes, and how
long it waits. With no destination, nothing
leaves this device. Koushi sends only while it is running (keeping it in the tray
is enough): a notification still waiting is held in memory and closing Koushi
discards it.

Choose a destination to open it in a window over the list. **Add a
destination** asks for the service (**Resend (email API)**, **SMTP relay**, or
**WeChat (ServerChan Turbo)**) in the same window and then shows the new
destination there. You can configure up to eight destinations. Each destination
window has five parts, described below.

Nothing in the window is saved as you type. Its bottom edge says **Unsaved
changes** once you edit something, and its single **Save** stores the whole
destination at once: its fields, its targets, its content and timing, a
credential you typed, and your answer to the plain-text question below. A
successful Save closes the window and returns to the list. If Save cannot go
ahead, the bottom edge says what is missing, such as a required field, the
credential of a new destination, or the plain-text checkbox; if the save is
refused, it says **Could not save this destination** and the window stays open
with your edits. **Cancel**, the close button, or Escape closes the window and
drops the edits, including a credential you typed; the window cannot be closed
while a save is in progress. A new destination is added only when Save
succeeds.

### Service and credentials

The destination's service fields (**Recipient email address** for mail; **From address** and
**Name**, the sender name, for an SMTP relay, with **Mailbox preset**,
**Server**, **Port**, **Connection security**, and **Username**) and its
credential: the **Resend API key**, the mailbox's **App password**, or the
ServerChan **SendKey**. A new destination takes its credential in the same window,
and Save sends it together with the rest.

The credential field is write-only: Koushi never shows the saved value again, and
the field is empty again once Save has taken it. It reports **Saved. Enter a new
value to replace it.** or **Not set**; to replace the credential, type the new
one and save. The value is kept in your operating system's credential store, not
in Koushi's settings, and a diagnostics report never carries it. **Delete
destination**, at the bottom of the window beside Cancel and Save, deletes the
destination together with its credential after you confirm. Changing an SMTP relay's
server, port, or username, or a service's endpoint, needs the password again,
because it was entered for the previous server; the window then says so and Save
waits for the new one.

The sender name is optional and starts as **Koushi**; left blank, Koushi uses
**Koushi**. For an SMTP relay the **From address** is required and must be an
address the relay lets you send as. For Resend, **Advanced** holds the **From
address**, the sender **Name**, and the service **Endpoint**. The From address is
optional there and starts as Resend's shared sender, `onboarding@resend.dev`;
see [Use Resend](#use-resend). Leave the endpoint alone unless you are testing
against a local stub.

### Targets

**Send for** has three switches, on by default for a new destination:

- **Direct messages**
- **Mentions and replies**
- **Your keywords**

A plain group message matches none of them. **Watched rooms and spaces** names
rooms and spaces that are always sent: choose **Add a room or space** and pick
from your room list, or **Remove** an entry. A watched room is sent even when its
category is off, but a room you have read, muted, or have open is still skipped.
A watched space covers every room in it, including rooms added later. The list
holds up to 64 rooms and spaces.

Each destination has its own targets, so one mailbox can receive every direct
message while WeChat receives only one watched room.

### Keywords

**Your keywords** delivers for a keyword hit. A keyword is a Matrix `content`
rule on your account, matched against the message body as a whole word and
without regard to case. A hit arrives as a mention, so either this switch or
**Mentions and replies** sends it. The rules live on your homeserver, not in
Koushi's settings, so they are shared by every destination and by your other
Matrix clients, and a rule another client set counts here too.

The words are listed under the switches. Type one into the box and choose
**Add**; each keyword already on the account is listed with a remove control.
Adding writes the rule to your account, which is why your other Matrix clients
highlight the word too; removing deletes that rule for every client. Nothing is
written until you add or remove something, and the list is what the server
reports rather than what Koushi remembers.

A word that another Matrix client turned off stays in the list marked as off. It
is still on your account, which is why adding the same word again is refused
while it is there; remove it, then add it again if you want it back on.

Koushi checks the word before it writes anything: an empty box, a word longer
than 64 characters, a word already in the list, and a list that is already at 64
keywords are each refused with their own message. A refusal means nothing was
changed.

### Message content

Space names, room names, and unread counts are always sent. **Don't send message
text** is the default: a notification names the rooms that need attention, each
room's kind, and its unread and mention counts, and no message text leaves this
device. Two options add text:

- **Send the beginning of each message**, with **Characters to include** (80 by
  default, at most 500).
- **Send the full message text**, up to the service's own limit per room (4000
  characters for mail, 1000 for ServerChan). Longer text is cut and marked as
  truncated; it is never presented as the whole message.

Images, files, messages that could not be decrypted, and deleted messages are
sent as names only whatever the mode.

Choosing a text option shows a warning directly under that option. It states that the text
leaves this device as plain text and passes through the service to the recipient
without end-to-end encryption, and that whether and for how long the service
keeps it depends on that service's own terms. Check **I understand that the text
is sent unencrypted**, then **Save**; Save refuses a text mode while the box is
unchecked. The agreement covers this service, its endpoint, and its recipient
(the **Recipient email address**, or for WeChat the saved SendKey). Changing any of them,
or saving a new SendKey, asks the question again, and a destination whose
agreement no longer matches sends names only until you agree again. Switching between the two text modes keeps
the consent; choosing names only clears it.

This setting is separate from **Show message content in notifications** under
**OS notifications**.

A direct message is headed **Direct message from** (or **Mention from**) and the
other person's name; other rooms are headed with **in** and the room and space
names. **Use the alias you set for a contact, if there is one** is on by default:
a contact you gave an alias is named by that alias, which is then sent to the
service. Without an alias, or with the box unchecked, the full Matrix ID (such as
`@user:server`) is used.

**Preview** shows an example direct message notification with the current
settings: the sender name (**Koushi** unless you changed it), the subject (the
title for WeChat), and the body. It is rendered by the same code that writes real
notifications, from a made-up contact and message.

### Timing

A room is not sent the moment it becomes unread.

- **Wait before sending (seconds)** (300 seconds, 5 minutes, by default) is how
  long a room must stay unread before it can be sent. Reading the room, muting it,
  or, with the option below on, bringing Koushi to the foreground cancels the
  wait, and it starts again the next time the room becomes unread.
- **Shortest gap between sends (seconds)** (900 seconds, 15 minutes, by default)
  is how long further notifications wait after a send, so a busy room cannot fill
  your mailbox. What comes due in the meantime is collected into the next summary.
- **Only when Koushi is inactive** (on by default) holds notifications while the
  Koushi window is in the foreground.

Each destination keeps its own timing, so one destination's send never delays
another's. One summary names a few rooms and then counts the rest, such as "and 3
more". It never contains a room's internal ID.

### Status

**Status** repeats whether setup is complete and shows the last attempt: when it
happened and what the service answered. **Send a test** sends one message through
this destination with the saved credential. A test holds no room, message,
sender, or account data and does not count toward the shortest gap.

- **Accepted by the service.**: The service took the request. This is the
  service's own answer, not proof that the message reached an inbox or a phone.
- **The service rejected the key. Enter it again.**: The saved API key, app
  password, authorization code, or SendKey is wrong, expired, or revoked.
- **The service refused this sender or this account. Check the from address and
  the account status.**: The provider will not send as this **From address**, or
  the account is not allowed to send yet.
- **The service refused the message.**: The provider rejected this message or
  its recipient, not the credential or the account.
- **The service is limiting how often it accepts messages.**: The provider asked
  Koushi to slow down; the next attempt waits for the provider's own delay, which
  the page shows in seconds when the provider gives one, or the shortest gap.
- **Could not reach the service.**: A network, TLS, or server problem. Koushi
  retries within the shortest gap.

Koushi retries only a rate limit and a connection problem. A rejected key,
sender, or message is not retried; fix it and the next notification tries again.
A destination whose last attempt failed says **The last attempt failed** on its
row, so a configured destination is never read as a working one. If nothing
arrives at all, see [Notifications from this device do not
arrive](troubleshooting.md#notifications-from-this-device-do-not-arrive).

## Set up a sending service

The simplest configuration is a mailbox you already own as both the sender and
the destination. It needs no new domain, no DNS records, and no new paid plan:
the mailbox's own app password or authorization code is the only credential, and
the notification is an ordinary email to yourself. A verified sending domain
stays an optional upgrade for deliverability, not a prerequisite.

The steps below are what each provider documents for a third-party client at the
time of writing. Check the provider's page, because a provider can change its
relay or the name of its credential.

### Use Resend

To notify yourself, Resend needs no domain of your own:

1. Create a Resend account with the email address you want notified, then create
   an API key on the
   [API keys page](https://resend.com/docs/dashboard/api-keys/introduction).
   Resend does not show the key again after it is created.
2. In Koushi, choose **Add a destination**, then **Resend (email API)**.
3. Set **Recipient email address** to the email address of your Resend
   account, and paste the API key into **Resend API key**.
4. Choose **Save**, open the destination again from the list, and choose
   **Send a test**.

Under **Advanced**, the **From address** starts as Resend's shared sender
`onboarding@resend.dev`, which needs no domain setup. That sender can send only
to the email address on your Resend account; to send anywhere else, verify a
domain at [resend.com/domains](https://resend.com/domains) and set the **From
address** to an address at that domain. The restriction and the resolution steps
are in Resend's
[403 Error Using resend.dev Domain](https://resend.com/docs/knowledge-base/403-error-resend-dev-domain).
Left blank, **Endpoint** uses the email API endpoint Resend documents.

### Use iCloud Mail

1. Create an app-specific password in your Apple Account, following Apple's
   [app-specific password steps](https://support.apple.com/en-us/102654). Copy
   the password.
2. In Koushi, add a destination of kind **SMTP relay**. Fill in **From
   address** first, then choose the **iCloud Mail** mailbox preset, which fills in
   the server Apple documents
   ([iCloud Mail server settings](https://support.apple.com/en-us/102525)).
3. Paste the app-specific password into **App password**, choose **Save**, then
   send a test.

Apple issues app-specific passwords for accounts that use two-factor
authentication, and the password is only for this kind of app, not your Apple
Account password.

### Use Gmail

1. Turn on 2-Step Verification for your Google Account. Google offers app
   passwords only while it is on.
2. Create an app password following Google's
   [app password steps](https://support.google.com/accounts/answer/185833), and
   copy the 16-character password.
3. In Koushi, add a destination of kind **SMTP relay**. Fill in **From
   address** first, then choose the **Gmail** mailbox preset, which fills in the
   server Google documents
   ([Gmail IMAP, POP, and SMTP](https://developers.google.com/workspace/gmail/imap/imap-smtp)).
4. Paste the app password into **App password**, choose **Save**, then send a
   test.

Google may not offer app passwords for every account, for example a managed
account whose administrator has turned them off, so the setting may be missing or
the password may be refused even with 2-Step Verification on.

### Use QQ Mail

1. Sign in to QQ Mail on the web.
2. Open **Settings**, then **Account and Security**.
3. Turn on the POP3/IMAP/SMTP service and complete the identity check the page
   asks for.
4. Copy the 16-character authorization code. QQ Mail's own page,
   [How to enable POP3/SMTP/IMAP and get an authorization code](https://help.mail.qq.com/detail/0/1087),
   has the full steps; the code is the password for a third-party client, never
   the QQ password. Changing the QQ password invalidates every authorization code
   ([QQ Mail client sign-in guide](https://help.mail.qq.com/detail/0/1088)).
5. In Koushi, add a destination of kind **SMTP relay**. Fill in **From
   address** first, then choose the **QQ Mail** mailbox preset, which fills in the
   relay QQ Mail documents
   ([Mac mail client settings](https://help.mail.qq.com/detail/108/1089/)).
   Paste the authorization code into the password field and choose **Save**.

QQ Mail can also send its own new-mail reminder to WeChat, so the same mailbox
can reach WeChat without a second service. That reminder is part of QQ Mail's own
settings, which Koushi does not configure; see the
[QQ Mail help centre](https://help.mail.qq.com/). It reaches WeChat only while
QQ Mail's reminder is set up and QQ Mail is delivering the message.

### Use WeChat (ServerChan Turbo)

[ServerChan Turbo](https://sct.ftqq.com) (Server酱·Turbo) forwards a message to
your WeChat through its own service account. The destination page shows these
steps with a link to the site:

1. Open [sct.ftqq.com](https://sct.ftqq.com) and log in by scanning the QR code
   with WeChat.
2. Open the **SendKey** page and copy your SendKey. It starts with `SCT`.
3. Nothing else needs setting up: messages arrive through the default 方糖
   service account (方糖服务号).
4. In Koushi, add a destination of kind **WeChat (ServerChan Turbo)**, paste the
   SendKey into **SendKey**, choose **Save**, and send a test.

The SendKey names your WeChat account, so it is the recipient as well as the
credential; it is kept only in the operating system's credential store. A key
from ServerChan³, which starts with `sctp`, is refused, because only ServerChan
Turbo is supported. The free plan shows 5 messages a day; ServerChan may accept
more without showing them, and **Accepted by the service** does not mean the
message was shown. The WeChat card shows only the title, which names the rooms;
message text, if you turn it on, is on the page the card opens.

### Use another SMTP relay

Any SMTP relay can be entered by hand: for an **SMTP relay** destination, choose
**Other server** and fill in **Server**, **Port**, **Connection security**, and
**Username**, then enter the password the relay documents and choose **Save**. This covers a company
relay or a server you run yourself. A self-hosted server or a provider account
needs its host, its port, and its TLS mode; your provider or administrator has
those, and Koushi does not probe a server to guess them. Choose **TLS from the
first byte (usually port 465)** or **STARTTLS after connecting (usually port
587)** exactly as the relay documents, and use a **From address** the relay is
allowed to send as.

## Ask for help

Open the desktop **Help → Koushi Help** menu, including before sign-in, or
**App Settings → Help & About**. Choose **Copy GitHub URL**, then paste it into
ChatGPT or another AI assistant with your Koushi version, operating system, and
question. The button copies only the public repository URL. If copying fails,
select and copy the displayed URL manually.

The [guide index](README.md) explains how to find instructions for your installed
release. The desktop Help menu opens this short assistance dialog; the keyboard
shortcut reference is under **App Settings → Keyboard**.
