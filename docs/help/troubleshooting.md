# Troubleshooting

[User guide and version selection](README.md)

## Cannot sign in

Check that **Matrix ID** holds your full Matrix ID, for example
`@alice:example.org`, and that the server named below it is your account's
server. To sign in with only a local username, select **Change server** and enter
the server address in **Homeserver** first. For browser sign-in, complete the
provider's flow and return to Koushi. **Check login methods** appears when the
automatic check fails; select it to retry. See the full
[sign-in steps](getting-started.md#sign-in).

If the server cannot be reached, check your connection and server address.
If Koushi says the server is unsupported, ask the server administrator about
Element X-compatible Simplified Sliding Sync support. A server that works in
another Matrix client may not offer the sync capability Koushi requires.

### The browser does not open for OIDC or Single sign-on

Koushi opens the authorization page in the operating system's default browser.
On Linux, check that the desktop opener exists and has a default browser:

```bash
command -v xdg-open
xdg-open https://example.com
xdg-settings get default-web-browser
```

Install `xdg-utils` when `xdg-open` is missing, then retry **Single sign-on**.
When running Linux under WSLg, start Koushi inside the same `dbus-run-session`
as the desktop session. Keep Koushi running until the provider redirects to
the registered callback scheme. If Koushi shows a browser-launch error, fix
the default-browser configuration and select **Single sign-on** again.

Finish one browser sign-in at a time. When two account tabs are both waiting on
**Single sign-on**, Koushi cannot tell which tab an incoming callback belongs to
and ignores it. Complete or cancel the other unfinished sign-in, then select
**Single sign-on** again in the tab you want.

An expired or revoked session requires signing in again. A verification screen
requires the [session verification flow](security-and-recovery.md#verify-a-session-after-sign-in),
not repeated password attempts.

## Room or message history is missing

Return to **Home** to check whether the room is merely absent from the selected
Space. Check **Invites** if you have not joined it yet. If the room opens but
older history is missing, allow synchronization and backward history loading
to finish. Your membership and room history policy can limit what is available.

For encrypted messages that cannot be read, follow
[Recover missing encrypted history](security-and-recovery.md#recover-missing-encrypted-history).
Do not delete application data or reset identity as a routine way to refresh a
room.

## Search returns no results

Check **All / Space / Room/DM**, try a longer or more distinctive term, and
inspect **Account Settings → Search history** for that account's indexing
status, and **App Settings → Search history** for the shared background-work
speed or pause control. See [Search](search.md). Rebuilding the index does not
recover missing keys.

## Message will not send

If you have already completed session verification and secure backup setup,
you can continue composing during a temporary connection failure. A message
accepted into the local send queue can remain pending while the server is
unreachable; you can compose the next message without waiting for delivery.
Koushi retries a room queue after a recoverable send failure. Keep the app open
until the connection returns, or reopen it to resume the stored queue.

Check the connection status at the top of the window and any verification or
secure backup requirement. Read the status on the affected message. For a failed
send, use its retry action when offered instead of submitting a duplicate
message. Attachments may also fail because of server upload limits.

If the room restricts posting or you are no longer joined, reconnecting will
not grant permission to send. Ask a room administrator if the restriction is
unexpected.

## Notifications are missing

Check **Room info** for **Mute** or **Mentions only**, then check **Account
Settings → Notifications → OS notifications** for desktop notifications,
sounds, and badges. Also check the operating system's
notification permission for Koushi. An unread badge and a desktop notification
are different signals. A red badge counts locally observed unread messages;
server-only notification counts do not create a red number or a desktop alert.
Desktop alerts name the room and its unread count and open the room; they do not
jump to an unverified event. The sender's name, the start of the message, and on
Linux the sender's avatar appear in a banner only with **Show message content in
notifications** on (off by default), and only for a new message Koushi has
decrypted and read itself.

If the notification you are missing is mail or a relay message sent from this
device, see [Notifications from this device do not
arrive](#notifications-from-this-device-do-not-arrive).

## Notifications from this device do not arrive

Koushi sends these itself, so check the destination under **Sent from this
device** before the homeserver:

- The destination's row reads **Configured**. **Not configured** names what is
  missing: a required field, the credential, or a target.
- At least one switch under **Send for** is on, or the room, or a space that
  contains it, is under **Watched rooms and spaces**. A plain group message is
  sent only through the watch list.
- **Wait before sending (seconds)** has elapsed while the room stayed unread.
  Reading the room, muting it, or leaving Koushi focused cancels the wait.
- **Shortest gap between sends (seconds)** has passed since that destination's
  last send.
- With **Only when Koushi is inactive** on, the Koushi window must not be in the
  foreground.
- Koushi is running. The waiting list is kept only in Koushi's memory: nothing
  is sent while Koushi is closed, and a waiting notification is discarded when
  Koushi exits.
- **Status** on the destination's page shows the last attempt. If its row says
  **The last attempt failed**, see [Status](settings.md#status).
- For WeChat, **Accepted by the service** means ServerChan took the request. The
  free plan shows 5 messages a day, so later messages may be accepted without
  appearing in WeChat.

**Homeserver notifications** are separate: mail from your server arriving does
not mean a destination on this device is working. See [the homeserver
setting](#the-homeserver-still-sends-its-own-notifications).

### The message arrives with no text

By default a destination's **Message content** is **Space and room names only**:
room names, kinds, and counts, not message text. Text is added only with a text
mode chosen for that destination and its consent accepted. If you then changed
the service, its endpoint, the **Recipient email address**, or the saved SendKey, the
consent no longer applies and the destination is back to names only until you
agree again. Images, files, undecryptable messages, and deleted messages are
always sent as names only.

### The password field is empty again after changing the server or account

A saved password is used only for the exact server and account it was entered
for. After you change the **Mailbox preset**, **Server**, **Port**, or
**Username**, the page asks for the password again and **Save** waits for it, so
a password entered for one mailbox is never offered to another. Saving the new
server removes the old password; if it was removed that way, the page says **The
saved password was removed because the server or account changed. Enter it
again.** Nothing was lost: enter the password for the new server and choose
**Save**. Changing only the **Recipient email address** keeps the password, but a text
mode asks the plain-text question again before Save.

### The provider says the credential is invalid, or the account is not permitted

**The service rejected the key** means the saved credential was refused. Create a
new one with the provider and save it. The usual causes:

- Gmail issues app passwords only while 2-Step Verification is on, and not for
  every account.
- iCloud Mail needs an app-specific password created for this app.
- QQ Mail invalidates every authorization code when the QQ password changes.
- A Resend API key cannot be read again after it is created.
- A ServerChan SendKey must start with `SCT`; a ServerChan³ key (`sctp…`) is
  refused before it is saved.

**The service refused this sender or this account** means the provider took the
credential but will not send for this **From address**, or the account is not
allowed to send yet. Check the address against the provider's rules; some
services send only from a verified domain. For Resend's testing sender
`onboarding@resend.dev`, the recipient must be the address on your Resend
account.

### Mail lands in the spam folder

Mark the message as not spam and add the sender to your contacts, so later
notices from the same sender reach the inbox. A sender address on a shared or
unverified domain is more likely to be filtered. If the provider asks you to
verify a domain, doing so lets you send from your own domain and improves how
receivers treat the mail; a mailbox you own and send from yourself is the least
surprising sender.

### The homeserver still sends its own notifications

**Sent from this device** and **Homeserver notifications** are separate. The
**Email notifications** switch under **Homeserver notifications** controls your
server's pusher, which applies to every device on the account, and no destination
on this device changes it. Removing a destination leaves the server setting as it
was; turn the server setting off too if you do not want both. See
[Email notifications](settings.md#email-notifications).

### A message looks stale, or repeats a room you already read

The summary is built when it is sent, from the rooms that are unread at that
moment, so a room you read before the send is not included. While a room stays
unread, one send covers it and it is not sent again until it leaves and re-enters
the unread state. Keep in mind that:

- Mail can be delayed on the way, so a room may have been read since the message
  was sent.
- Koushi's timers start again when it restarts, so a room that was already sent
  and is still unread can be sent once more after a restart.
- A send that failed part-way can be retried, and a provider that does not
  recognize the retry as the same message can deliver a second copy.

## Ask for help or report a bug

You can [ask an AI assistant](README.md#ask-an-ai-assistant) using the public
repository and your version. If the issue persists, open a
[GitHub issue](https://github.com/shinaoka/koushi-matrix/issues/new/choose) with:

- Koushi version (**About Koushi** or **Check for Updates…** in the Koushi
  menu), operating system, and whether the problem began after updating.
- The steps you took, what you expected, and what happened instead.
- Whether the affected room is encrypted and whether the issue affects one room
  or several, without disclosing private room IDs or messages.
- A sanitized error description. Review any screenshot before sharing it.

When the problem involves sign-in, startup, sync, encryption, or a crash,
attach the diagnostics report as a file. Select **Open diagnostics** at the top
right of the window, select **Save to file…**, and attach the saved
`koushi-diagnostics.txt`. The report is too long to paste, and the dialog notes
that it omits private data: no message text, room or member names, Matrix IDs,
email addresses, or file paths.

For a missing avatar, save a report while the image is missing, before restarting.
Initials alone do not establish that the server-side profile image was removed.
The report distinguishes the local own-profile thumbnail state and retained-byte
presence from renderer image load/error event totals. Those event totals cover
all avatar slots since the page opened; they do not identify individual images.

For memory growth, save reports before and after the activity that increases
memory. Each report samples event-queue retention and renderer DOM, image,
canvas, Blob URL and timeline-store counts. Image and text byte estimates are
reference sizes, not measured WebKit or GPU allocations; unavailable engine
measurements are explicitly marked. Reports sample the current state and do
not force garbage collection or clear caches.

Never include passwords, recovery keys, access tokens, private messages, or
key export files. If you are unsure what a diagnostics file contains, open it
before attaching it or describe the visible symptom first.
