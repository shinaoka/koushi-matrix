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
Settings → Notifications** for desktop notifications and **App Settings →
Notifications** for sounds and badges. Also check the operating system's
notification permission for Koushi. An unread badge and a desktop notification
are different signals.

## Ask for help or report a bug

You can [ask an AI assistant](README.md#ask-an-ai-assistant) using the public
repository and your version. If the issue persists, open a
[GitHub issue](https://github.com/shinaoka/koushi-matrix/issues/new) with:

- Koushi version, operating system, and whether the problem began after updating.
- The steps you took, what you expected, and what happened instead.
- Whether the affected room is encrypted and whether the issue affects one room
  or several, without disclosing private room IDs or messages.
- A sanitized error description. Review any screenshot or diagnostic attachment
  before sharing it.

Never include passwords, recovery keys, access tokens, private messages, or
key export files. If you are unsure what a diagnostic file contains, describe
the visible symptom first.
