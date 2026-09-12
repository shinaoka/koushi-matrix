# Security and recovery

[User guide and version selection](README.md)

## Verify a session after sign-in

Account sign-in and access to encryption keys are separate steps. At
**Verify this session**, use an offered verification method:

- With a recovery key, enter it in **Recovery secret** and choose
  **Verify with recovery key**.
- With another device, keep that already verified device online, select
  **Verify with another device**, and follow the confirmation. If matching emoji
  are displayed, compare them on both devices before choosing **They match**.
  Choose **They do not match** if they differ.

The available choices depend on your account. Koushi recommends the recovery
key when available; another device can be offline or missing keys. Wait for
verification to finish rather than repeatedly starting new requests.

## Complete secure backup setup

If **Secure backup required** appears, follow the offered recovery or setup
flow. An existing backup may ask for its recovery key. A new backup asks you
to protect it and choose a destination for the recovery key. Save that key
securely and complete the app's acknowledgement before proceeding.

Do not give recovery keys, backup passphrases, or exported keys to an AI
assistant or include them in an issue. These are credentials for encrypted
history, not diagnostic information.

## Recover missing encrypted history

1. Check whether session verification or secure backup setup is still pending.
2. Open **User settings → Encryption → Open recovery**. In the
   **Encryption Recovery** form, enter the supported recovery key or security
   phrase and choose **Recover**.
3. Let recovery and synchronization finish, then reopen the affected room.
4. If you have a room-key export from another client, select **Import room keys** under **Key management** in the security settings,
   choose the import file, supply its passphrase, and confirm **Import room keys**.

Recovery only helps when the needed keys are available from your backup, key
file, or another eligible device. A verified session does not guarantee that
all historical messages can be decrypted. Avoid resetting identity or erasing
local data as a first troubleshooting step: it can remove access to keys that
exist only on that device.

## Key export and chat export are different

**Key management → Export room keys** saves encryption keys protected by the
chosen passphrase. Select **Export room keys**, choose the destination file,
supply a passphrase, and confirm **Export room keys** in the passphrase dialog.
Import follows the same sequence with **Import room keys** and the passphrase
that protected the original file. Keep both the file and its passphrase secure.

A room-key file is not a readable copy of a conversation. This guide does not
describe a released chat-history download feature; proposed chat export work
is tracked in [issue 59](https://github.com/shinaoka/koushi-matrix/issues/59).
Do not interpret that issue as an available menu item.

## Understand trust labels

User verification, device verification, and whether sending is allowed describe
different things. See the [User trust model](user-trust-model.md) for the meaning
of **Unverified**, **Verified**, and **Identity reset**.
