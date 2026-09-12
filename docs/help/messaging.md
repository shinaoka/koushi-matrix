# Messaging

[User guide and version selection](README.md)

## Send a message

1. Open the intended room or direct message.
2. Enter text in the message composer at the bottom of the conversation.
3. Use **Send** or your configured send shortcut.
4. Check the message's status. A failed or pending send is not confirmation that
   the server accepted it.

Choose the send shortcut in **User settings → Keyboard**. The available modes
are **Enter sends** and the platform modifier plus Enter. The keyboard page
lists the current platform's shortcuts. Confirming an IME candidate is separate
from sending the composed message.

## Reply or open a thread

Use a message's actions to choose **Reply to message** for a reply in the room,
or **Reply in thread** to open a thread. In a thread, use its separate reply
composer. Check which composer is active before sending.

Select a message's reply count to open its existing thread. Koushi's thread
composer sends replies in that thread; it does not offer nested threads.

## Edit or remove a message

Open the message's context menu. For an editable message you sent, choose
**Edit**, change the text, and choose **Save edit**. Available actions depend
on ownership, permissions, and whether the message has been sent.

**Redact** on a sent message performs a Matrix redaction, not a guarantee that nobody
previously read or saved it. A pending or failed local send has different actions
from a sent message. Read the confirmation before removing anything.

## Send files and images

1. Choose **Attach file**, or drop a file onto the composer.
2. Review the staged attachment. If an image compression choice appears, choose
   the desired image quality.
3. Add a caption if needed, then send. Remove an unwanted staged file with
   **Remove attachment** before sending.

An attachment appearing in the composer is not yet an uploaded message. Server
upload limits and network failures can prevent a send. Open an attachment from
the timeline, or use **Room info → Files** to browse room files.

For a failed send, see [sending troubleshooting](troubleshooting.md#message-will-not-send).
