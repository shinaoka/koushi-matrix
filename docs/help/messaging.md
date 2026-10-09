# Messaging

[User guide and version selection](README.md)

## Send a message

1. Open the intended room or direct message.
2. Enter text in the message composer at the bottom of the conversation.
3. Use **Send** or your configured send shortcut.
4. Check the message's status. A failed or pending send is not confirmation that
   the server accepted it.

Choose the send shortcut in **App Settings → Keyboard**. The available modes
are **Enter sends** and the platform modifier plus Enter. The keyboard page
lists the current platform's shortcuts. Confirming an IME candidate is separate
from sending the composed message.

## Schedule a message

Use **Send later** in the message composer to send the message at a chosen
time. The composer shows a time field and buttons that move it by 10 minutes or
an hour; choose **Schedule send** to save the reservation, or **Cancel** to
close without scheduling.

Scheduled messages appear in the room where you created them and behind the
**Scheduled messages** clock button in the header. That panel shows the
reservations for **Home** (the whole account) or for the Space that was selected
when you opened it. Each entry shows its time, message text, destination room,
and a **Thread reply** marker when it sends into a thread. **Edit** changes the
time or text; **Cancel scheduled send** removes the reservation.

Two limits matter:

- Koushi lists only the scheduled messages this device created. Reservations
  made in another Matrix client, or before you reinstalled the app, do not
  appear, so the panel is not a server-wide view of your scheduled sends.
- Delivery depends on your server and the panel labels it: with **Server
  scheduled** the server holds the reservation and still sends it while Koushi
  is closed; with **Local fallback** Koushi holds it and it sends only while the
  app is running.

## Reply or open a thread

Use a message's actions to choose **Reply to message** for a reply in the room,
or **Reply in thread** to open a thread. In a thread, use its separate reply
composer. Check which composer is active before sending.

Select a message's reply count to open its existing thread. Koushi's thread
composer sends replies in that thread; it does not offer nested threads.

## Edit or remove a message

Open the message's context menu. For an editable message you sent, choose
**Edit**, change the text, and choose **Save**. Available actions depend
on ownership, permissions, and whether the message has been sent.

**Redact** on a sent message performs a Matrix redaction, not a guarantee that nobody
previously read or saved it. A pending or failed local send has different actions
from a sent message. Read the confirmation before removing anything.

## Send files and images

1. Choose **Attach file**, or drop a file onto the composer.
2. Review the staged attachment. If an image compression choice appears, choose
   the desired image quality.
3. Add or edit a caption if needed, then send. When no attachment is staged yet,
   adding one file starts its caption with whatever the composer already held, so
   text and file can go out as one message. If that caption is still the composer's
   text when you send the only attachment, the composer is cleared with it;
   otherwise your text stays. Remove an unwanted staged file with **Remove
   attachment** before sending.

An attachment appearing in the composer is not yet an uploaded message. Server
upload limits and network failures can prevent a send. Open an attachment from
the timeline, or use **Room info → Files** to browse room files.

For a failed send, see [sending troubleshooting](troubleshooting.md#message-will-not-send).

## Room changes and unavailable content

Topic and avatar changes appear as notices. Room replacement notices include a
link to the new room. Routine alias, hierarchy and policy updates do not add
conversation rows. Event details remain available through the existing message
source action on visible events.

Unreadable events and messages that cannot yet be displayed have neutral
notices. These are distinct from messages waiting for decryption. Polls, live
location and calls have explanatory placeholders; these placeholders do not
provide voting, location tracking or calling functionality.
