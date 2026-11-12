# Security and recovery

[User guide and version selection](README.md)

## Verify a session after sign-in

Account sign-in and access to encryption keys are separate steps. At
**Verify this session**, use an offered verification method:

- With a recovery key, enter it in **Recovery secret** and choose
  **Verify with recovery key**.
- With another device, first open that already verified app or session and keep
  it open. Then select **Verify with another device** and follow the
  confirmation. The request appears in the open session; it may not arrive as a
  notification. If matching emoji are displayed, compare them on both devices
  before choosing **They match**. Choose **They do not match** if they differ.
- On a brand-new account, choose **Create secure backup** (a backup
  passphrase is optional). Your new recovery key is shown on screen; record it
  as described in the next section, then choose **I saved the recovery key**.

The available choices depend on your account. Koushi recommends the recovery
key when available; another device can be offline or missing keys. Wait for
verification to finish rather than repeatedly starting new requests.

## Unverified sessions on your account

When you sign in, and afterwards at most once every 24 hours, Koushi checks
which sessions are signed in to your account. If this session is verified and
another session is not, a popup titled **Unverified sessions on** followed by
the account name appears. It shows the account's user ID and homeserver, how
many sessions are signed in, how many are not verified, and lists the
unverified ones with their last activity.

- **Review sessions** opens **Account Settings → Sessions**, where **Other
  sessions** lists every other session of that account.
- **Later** closes the popup until the next check.

If you do not recognize a session, sign it out with **Manage account &
devices**; Koushi does not sign other sessions out itself. If you do recognize
it, verify it from that device.

## Complete secure backup setup

If **Secure backup required** appears, follow the offered recovery or setup
flow. An existing backup may ask for its recovery key. A new backup shows its
recovery key on screen. Record it somewhere safe: use **Copy**, or optionally
**Save to file…**, then choose **I saved the recovery key** to continue. The
key is not shown again after that confirmation. If the app closes before you
confirm, the unsaved key cannot be recovered; choose **Create new recovery
key** to replace it (the previous key stops working).

If the gate says Koushi is waiting for its first sync response, no Secure
Backup request has been attempted yet. Check your connection and choose
**Retry secure backup** to restart the stalled initial sync attempt.

If the gate keeps reporting a problem after **Retry secure backup**, choose
**Open secure backup diagnostics** to read what actually failed. The dialog
opens while the gate is still in the way and says whether Koushi got no response
at all, how long it waited, or which kind of server response came back, so you
can tell a network problem from a server one.

Do not give recovery keys, backup passphrases, or exported keys to an AI
assistant or include them in an issue. These are credentials for encrypted
history, not diagnostic information.

## Read while secure backup is still being checked

Once your session is verified, Koushi opens your saved conversations even while
it is still checking Secure Backup, unless that check needs recovery or setup
from you. In that state you are reading what this device saved the last time it
synced.

Two notices describe it, and each appears on its own:

- **Saved conversations.** A notice at the top of the conversation list means
  Koushi has not yet received this run's first sync response. Rooms or messages
  that arrived since this device last synced may be missing or out of date, and
  the unread marks and counts are the last values this device saved. The notice
  goes away when that response arrives.
- **Sending is unavailable until Secure Backup is ready.** A notice beside the
  message box means every send is closed, in every room, encrypted or not: the
  message box, the attachment button, thread replies, reactions, edits,
  scheduled sends and resending a queued message. It goes away when Secure
  Backup becomes ready, which can happen before or after the conversation list
  is confirmed.

Opening a saved conversation does not mark it read for you or for your other
clients. Take the offered verification, recovery, or setup action to be able to
send again.

## Recover missing encrypted history

1. Check whether session verification or secure backup setup is still pending.
2. Open **Account Settings → Encryption → Open recovery**. In the
   **Encryption Recovery** form, enter the supported recovery key or security
   phrase and choose **Recover**.
3. Let recovery and synchronization finish, then reopen the affected room.
4. If you have a room-key export from another client, select **Import room keys** under **Key management** in the security settings,
   choose the import file, supply its passphrase, and confirm **Import room keys**.

When a missing key arrives later, from your backup or another device, Koushi
shows the affected messages automatically and makes them searchable; you do not
need to reopen the room.

Recovery only helps when the needed keys are available from your backup, key
file, or another eligible device. A verified session does not guarantee that
all historical messages can be decrypted. **Reset** identity first asks you to
confirm, naming the account whose identity it resets. Avoid resetting identity
or erasing local data as a first troubleshooting step: it can remove access to
keys that exist only on that device.

## Key export and chat export are different

**Key management → Export room keys** saves encryption keys protected by the
chosen passphrase. Select **Export room keys**, choose the destination file,
supply a passphrase, and confirm **Export room keys** in the passphrase dialog.
Import follows the same sequence with **Import room keys** and the passphrase
that protected the original file. Keep both the file and its passphrase secure.

A room-key file is not a readable copy of a conversation. To save a readable
copy of a room's messages, use **Room info → Download history** instead; see
[Download room history](rooms-and-spaces.md#download-room-history). That
folder holds encrypted messages and attachments unencrypted and contains no
room keys.

## Move keys from Element or Element X

Encryption keys stay with the client that received them. Once the old app is
signed out or uninstalled, whatever it did not share or back up is gone, so act
while it is still in place. Every route below also needs the **same account** in
both apps: keys from a different account open nothing here.

There are three routes, in the order that usually works:

1. **Verify from the old device.** This needs Element or Element X still
   installed and signed in, with the history you want still readable in it. In
   Koushi, open **Account Settings → Sessions**, choose **Verify this session**,
   then **Verify with another device**, and confirm the request in the old app.
   It needs no recovery key and no file, so it also works when the account has no
   secure backup. Open an affected room in Koushi and confirm its history
   decrypts before you sign out of the old app.
2. **Enter your recovery key or security phrase.** This needs an existing secure
   backup on the account, or a recovery key you recorded earlier. Open
   **Account Settings → Encryption → Open recovery** and enter it in the
   **Encryption Recovery** form. It works when the old device is already gone.
3. **Import an exported key file.** This needs a file you exported from the old
   client earlier, plus the passphrase that protected it. In Koushi, open
   **Account Settings → Encryption → Key management**, select **Import room
   keys**, choose the file, supply its passphrase, and confirm. It works only for
   keys the file actually contains.

**Element X.** Its local data is encrypted with a key Element X keeps in its own
Keychain, which Koushi cannot read, so a phone that still has your history has to
give the keys up itself: use route 1. If that phone is no longer available, sign
in to Element Web or Element Desktop with the same account, let the keys arrive
there, and then use route 2 or route 3 from that client.

**Before you create a new secure backup.** A new backup starts empty. Once it
replaces the previous one, history that only the old backup held cannot be
restored from the new one, and this is easy to do by accident while trying to fix
a missing-history problem. If the old device or phone still exists, take route 1
first; if a recovery key exists, record it before anything replaces the backup.

**Accounts you only use on the web.** If there is no Element Desktop or Element X
on this computer — a matrix.org account used through `app.element.io`, for
example — open `https://app.element.io/#/settings/security` in a browser, sign in
to the same account, and check **Secure Backup** there. If the account has a
recovery key you no longer hold, that page can reset the backup and show a new
key; a reset abandons history that only the previous key could open, so try
route 1 first and record the new key before returning to Koushi and entering it
under **Open recovery**.

Some history cannot be recovered in Koushi: messages whose keys no remaining
client and no backup holds stay unreadable, and resetting identity or erasing
local data does not bring them back. This is a property of how encrypted history
is shared, not a fault in this app. Setting up secure backup in Koushi keeps
future messages readable on your other devices; for a single message you can also
choose **Request decryption keys for past messages** as described in
[Request missing historical keys](rooms-and-spaces.md#request-missing-historical-keys).

## Understand trust labels

User verification, device verification, and whether sending is allowed describe
different things. See the [User trust model](user-trust-model.md) for the meaning
of **Unverified**, **Verified**, and **Identity reset**.
