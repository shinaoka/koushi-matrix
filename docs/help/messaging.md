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

## Format a message

Select text in a composer and use one of the formatting buttons under it —
**Bold**, **Italic**, **Link**, **List**, **Quote** and **Code** — or press
**Cmd + B** / **Cmd + I** or
**Cmd + Shift + >** (**Ctrl** on Windows and Linux). With nothing selected the
Bold and Italic shortcuts insert the markup pair with placeholder text, exactly as
the button does; **Quote** starts a quoted line, and on a selection it prefixes
every selected line with `> ` the way **List** prefixes `- `. Quoted lines are
sent as a block quote that other Matrix clients render as a quote. Formatting
shortcuts do nothing while an IME composition is in progress or while the
composer is read-only. The full shortcut list is under
**App Settings → Keyboard**.

## Read older messages

Scroll upward to read earlier messages. Older history loads automatically as you
approach it; availability depends on the server, room history policy, and keys
for encrypted messages. New messages do not pull you to the bottom while you are
reading earlier history.

To return to the newest messages, use the down-arrow **Latest** button in the
room header. While you are scrolled up and new messages from other people are
waiting below, a **Jump to bottom, N new messages** bar also appears at the top of
the message area.

A room can also show a point in its history instead of the newest messages, for
example when you open a search result or an Activity entry that is older than the
messages already loaded. That view keeps the **Latest** button in the header and
adds a **Jump to latest message** button inside the timeline; either one returns
to the live conversation.

## Mention someone

Type `@` in a room, direct message, or thread composer and choose a person.
Suggestions include joined and invited members of that room. Invited people
show an **Invited** marker; within the same search match, joined people appear
first. While the member list is loading, known candidates remain available.
People who left, were banned, or are only requesting to join are excluded.

Selecting a person inserts a mention pill. Invited people can see the mention
once they join, subject to the room's history visibility. `@room` appears only
when you have permission to notify the room.

## Resize the emoji picker

Open **Emoji** in a composer or reaction menu and drag any edge or corner of the
picker: an edge changes that dimension, a corner changes both. The opposite edge
or corner stays where it is. You can also focus the marked corner with Tab and
use the arrow keys. The size is shared across pickers on this device and restored
after restarting Koushi. The saved size is drawn wherever the picker opens, and
only a window too small for it narrows it; the emoji grid then scrolls inside the
picker, the search field stays in view, and the grid scrolls with the arrow keys
while it has focus. When the window cannot hold the saved size next to the
message you opened the picker from, the picker covers that message rather than
shrink any further. A corner you drag stays where you put it, so the window may
narrow the picker a little further to keep it there. While the picker is drawn
smaller than your size, a small marker in the header says so on hover, with the
drawn and saved measurements. **App Settings** → **Preferences** shows the saved
size and has a **Reset to default** button.

## Schedule a message

Use **Send later** in the message composer to send the message at a chosen
time. The composer shows a time field and buttons that move it by 10 minutes or
an hour; choose **Schedule send** to save the reservation, or **Cancel** to
close without scheduling.

Scheduled messages appear in the room where you created them and behind the
**Scheduled messages** clock button in the header. That panel shows the
reservations for **Home** (the whole account) or for the Space that was selected
when you opened it. Each entry shows its time, message text, destination room,
its own delivery label, and a **Thread reply** marker when it sends into a
thread. **Edit** changes the time or text; **Cancel scheduled send** removes the
reservation.

Two limits matter:

- Koushi lists only the scheduled messages this device created. Reservations
  made in another Matrix client, or before you reinstalled the app, do not
  appear, so the panel is not a server-wide view of your scheduled sends.
- Delivery depends on your server, and each entry carries its own label: with
  **Server scheduled** the server holds the reservation and still sends it while
  Koushi is closed; with **Local fallback** Koushi holds it and it sends only
  while the app is running. The server decides this per room, so one list can
  mix both paths — the heading then reads **Mixed delivery** and the entries keep
  their own labels.

## Reply or open a thread

Use a message's actions to choose **Reply to message** for a reply in the room,
or **Reply in thread** to open a thread. In a thread, use its separate reply
composer. Choosing a reply target puts focus in that composer, at the end of the
draft, so you can type the reply straight away — unless you are already typing in
it. Each composer keeps its own draft, so check which one you are typing in before
sending.

Select a message's reply count to open its existing thread. A red number shows
unread replies; select it to open the thread. Koushi's thread composer sends
replies in that thread; it does not offer nested threads.

Select the sender line at the top of a reply preview to open the original
message. If the original is already loaded in the room, Koushi scrolls to it and
briefly highlights it there. Otherwise Koushi opens the room at that point in its
history, as described under [Read older messages](#read-older-messages), and
briefly highlights the message there. Links
inside the preview still open their own targets. If the original is unavailable,
Koushi explains that instead of jumping elsewhere.

When a reply quotes an image, sticker, or video, the preview shows the picture
itself under the sender and any caption, at up to 240 × 160 pixels in its
original shape. A very wide or very tall picture, such as a formula, is scaled
down and centered so none of it is cut off. Select the picture to open the original in the media viewer;
select the rest of the preview to jump to the message as before. A pending
picture keeps a neutral placeholder of the same size, and a failed one keeps
that placeholder without an error or retry control. The composer's reply strip
shows the same sender, any caption, and a small thumbnail while you write.

## Edit or remove a message

Open the message's context menu. For an editable message you sent, choose
**Edit**, change the text, and choose **Save**. Available actions depend
on ownership, permissions, and whether the message has been sent.

**Redact** on a sent message performs a Matrix redaction, not a guarantee that nobody
previously read or saved it. A pending or failed local send has different actions
from a sent message. Read the confirmation before removing anything.

## Pin a message

Use a message's actions to choose **Pin message**, and choose **Unpin message**
on a pinned one. Pinning is shared with the room: it needs the permission the
room grants, and every member sees the result. Koushi does not pin a message
for only yourself.

A pinned message keeps a **Pinned message** marker under its text, in both
densities and on a message that continues an earlier one's sender; unpinning
removes it. The **Pinned** row at the top of the room opens the list of pinned
messages, which also unpins from each row while sending is available; while the
message box reports that sending is unavailable, that list keeps its messages
and offers no unpin button, the same way the message actions offer no **Pin**.
The timeline's new notice reads
**pinned a message** or **unpinned a message** for a single change, and **updated
pinned messages** when several messages changed at once.

## Copy or search selected text

Select text inside a message, then open the message's context menu. When a
selection is present, **Copy selected text** and **Search selected text on
DuckDuckGo** appear above the other actions. **Copy selected text** copies
exactly the selected characters. **Search selected text on DuckDuckGo** opens
the selection as a search query in your default browser, using the fixed
DuckDuckGo provider rather than the browser's own default search engine. Both
actions need only the selection, so they also work on a message you cannot edit
and on a not-yet-sent local message.

## Send files and images

1. Choose **Attach file**, or drop a file onto the composer.
2. Review the staged attachment. Drop more files onto the staging dialog to add
   them. If an image compression choice appears, choose the desired image quality.
3. Add or edit a caption if needed, then send. When no attachment is staged yet,
   adding one file starts its caption with whatever the composer already held, so
   text and file can go out as one message. If that caption is still the composer's
   text when you send the only attachment, the composer is cleared with it;
   otherwise your text stays. Remove an unwanted staged file with **Remove
   attachment** before sending.

Editing a message or a scheduled send takes text only; a file dropped on that
editor is ignored rather than silently attached.

An attachment appearing in the composer is not yet an uploaded message. Server
upload limits and network failures can prevent a send. Open an attachment from
the timeline, or use **Room info → Files** to browse room files.

When converting an image to JPEG, transparent areas become white and translucent
shadows blend with white. Choose PNG or WebP to keep transparency, or keep the
original format and size to send the original bytes.
Converted images keep their display orientation and embedded RGB color profile.
If an image's color profile cannot be converted, use the original format and
size to send the file unchanged.

For a failed send, see [sending troubleshooting](troubleshooting.md#message-will-not-send).

## Preview a file without saving it

Text and source files show their first lines in the timeline card, and audio and
video files play in the card once they are downloaded. Selecting a text file's
card opens the whole file in a preview window; the card's download tile still
saves a copy to your computer. Previews read Koushi's own download cache, never
your Downloads folder, and nothing is sent to another service.

Previewing never fetches a large file on its own. A text file above 256 KB, and
every file Koushi cannot display, keeps the plain card and downloads only when
you ask for it. Documents and PDFs are not previewed yet.

## View an image

Select an image in the timeline to open it in the media viewer. The viewer also
shows where the image sits in the room's images, such as `3/12`, and moves to the
previous or next image with the side controls, the left and right arrow keys, or
a horizontal swipe. Navigation stops at the first and last image instead of
wrapping around. The sequence is the room's media list restricted to images, so
an image outside it opens on its own without navigation. Changing the image
resets zoom and pan to the fitted view. The **Room info → Files** gallery opens
the same viewer, which covers files and videos too.

The gallery and the viewer are not limited to the messages loaded in the
timeline. Scrolling to the end of the gallery, or reaching the oldest loaded
image in the viewer, loads older media from the room's history until the start
of the room. If loading fails, the gallery shows **Retry**.

## Room changes and unavailable content

Topic and avatar changes appear as notices. Room replacement notices include a
link to the new room. Routine alias, hierarchy and policy updates do not add
conversation rows. Event details remain available through the existing message
source action on visible events.

A notice is one muted line, not a message: it has no avatar, no separate sender
column and no message body box, and its text follows the sender's name on the
same line. It has no hover action bar. **React**, **Pin** and the read-only
**Message actions** menu sit on the line itself, so **Copy message**, **Copy
link** and **View source** stay available. **Reply**, **Edit**, **Forward** and
the sender alias are not offered, because a room or membership change has no
message to answer, change or pass on. The sender's name still opens their
profile, and the text stays selectable.

Consecutive room and profile changes of the same kind from the same sender arrive
as one line with a count, such as **· 4**: a burst of four profile-picture changes
is one line instead of four. The count opens the group and shows each event as its
own line. A different kind of change, a gap of more than five minutes, a new day,
or new messages breaks the group, so a line never stands for notices you read
already and ones you have not. Join and leave lines, and a notice with work of its
own — an unreadable message that can request keys, a call, a poll — are never
counted this way.

A collapsed notice line shows no timestamp; opening the group shows each event's
time, and **View source** on the line gives the event's time along with its
details.

Unreadable events and messages that cannot yet be displayed have neutral
notices. These are distinct from messages waiting for decryption. Polls, live
location and calls have explanatory placeholders; these placeholders do not
provide voting, location tracking or calling functionality.


Message timestamps show the local time for today and add a localized date for
older messages (yesterday, weekday within the past week, month/day this year,
year for earlier years). Hover or focus a timestamp to read the full date and
time. Date labels update at local midnight and when the window resumes.

When a thread root moves to its latest reply position in the room timeline,
its original date and time appear on a separate **Posted** line above the
message, including the weekday and the year for earlier years. The reply
summary shows **Latest reply** with its own date and time below the reply
button. Hover or focus either timestamp for the full date and time.

Undo/Redo uses the focused message editor's history in room, thread and message
editing composers. Use Cmd+Z / Shift+Cmd+Z on macOS; Ctrl+Z / Ctrl+Y or
Ctrl+Shift+Z on Windows/Linux. **Edit → Undo/Redo** uses the same history.
