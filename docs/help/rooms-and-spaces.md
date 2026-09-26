# Rooms and Spaces

[User guide and version selection](README.md)

## Navigate Home and Spaces

A room contains a conversation. A Space groups rooms; joining a Space does not
necessarily join every room it lists.

Choose **Home** in the left rail to see your account-level navigation, including
**Invites** and **Explore**. Choose a Space to see its rooms. If a conversation
is absent from a Space view, return to Home before assuming it has disappeared.

## Join a room

If you received an invitation:

1. Open **Home → Invites**.
2. Select the invitation and check the room and inviter.
3. Choose **Accept invite** to join, or **Decline invite** to reject it.

If you have a room address:

1. Open **Home → Explore**.
2. Enter the room address or Matrix link in the address field and choose
   **Preview**.
3. Review the preview and choose the available join action. Invite-only rooms
   require an invitation; a preview does not grant access.

Explore also lets you search a server's public room directory. This searches
rooms to join, whereas the top search field [searches messages](search.md).

## Create a room or Space

Use **Create room** in the sidebar header, enter a name, and review the privacy
and encryption choices before creating it. In the current creation dialog,
choosing a public room turns encryption off. Review the final options rather
than assuming every new room is encrypted.

Use **Create space**, the plus button near the bottom of the left rail, to
create a Space. Room and Space administration actions depend on your role.

## Add an existing room to a Space

1. Select the Space in the left rail.
2. Open the **Rooms** heading's menu (**Options for Rooms**) and choose
   **Add existing room**.
3. Search by room name and choose **Add** next to the room.

The list contains the rooms you have joined, without direct messages. Rooms the
Space already lists show **Added**. A room that appears in the Space only
because the room names the Space as its parent still shows **Add**: other
Matrix clients list a room under a Space only after the Space lists it, so add
it to make the relationship visible everywhere. Adding a room lists it for the
Space's members; it does not change who can join the room.

Adding needs permission to manage the Space's rooms. Without it, the row says
that you do not have permission. If adding fails for another reason, the row
explains why and offers **Retry**. Rooms are added one at a time; rooms you
have not joined and subspaces are not offered here.

If a room you create inside a Space cannot be added to it, Koushi still creates
the room, tells you, and offers **Add existing room** to try again.

## Start a direct message

Select **New DM** in the sidebar header. Enter the person's full Matrix ID in
**Matrix user ID**, check the address, and choose **Start DM**.
A direct message is still a Matrix room; the other person may need to accept
an invitation before participating.

## Invite someone to a room

1. Open the room and select **Room info** in its header.
2. Choose **Invite people**, search for or enter the person's Matrix ID, and
   select the intended person.
3. Review the offered scope and history options, then choose **Send invite**.

If the action is unavailable, your role or the current room state may not allow
it. Room history visibility and room entry rules are separate settings.
**Since invite**, **Since join**, and **Shared history** describe different
history access. In encrypted rooms, reading older history also requires the
appropriate encryption keys. Changing a setting does not revoke events or keys
already shared.

## Conversation list sections

The conversation list shows **Rooms** above **DMs**. Each heading can be
collapsed and sorted from its own menu. The number to the right of a heading is
its unread total for the current Home or Space view; it turns into a red badge
when there is something unread, shows `99+` above 99, and disappears at zero. It
stays on the Rust-reported total for the whole view, so filtering the list or
collapsing the section does not change it.

A **Low priority** section appears below **DMs** when any conversation in the
current view carries the low-priority tag. Low-priority rooms and DMs are listed
there instead of in **Rooms** or **DMs**. Set or clear the tag from a
conversation's context menu; the section is hidden when it is empty and can be
collapsed like the others.

Low priority quiets a conversation without muting or reading it: it raises no
desktop notification or sound, and it is excluded from the Dock or taskbar
badge, the Home and Space rail counts, and the **Rooms** and **DMs** heading
badges, mentions included. The conversation's own row still shows its real
unread count, nothing is marked as read, and removing the tag restores its
contribution without replaying old notifications.

## Room information and notifications

Open **Room info** to find members, files, room notification options, and settings
available to your role. Room notification choices include **All messages**,
**Mentions only**, and **Mute**. Device notification permission and global
notification settings can also affect whether a desktop notification appears.
**Mute** suppresses desktop notifications and sounds and excludes that room from
notification badge totals, even if it still has unread messages or mentions.
Muting does not mark messages as read. Room notification changes made in another
Matrix client are reflected after synchronization. The **Notifications** entry at
the bottom of **Room info** moves to this setting.

## Change room details, access, and history

Open **Room info**. Each property is shown, changed, and confirmed in one card:

- **Details**: **Topic** and **Avatar**. Choose **Edit**, change the value, and
  choose **Save** (or **Cancel**, or press **Esc**). Saving an empty value
  removes the topic or avatar. The avatar card shows the room's picture and its
  `mxc://` address.
- **Access and history**: **Join rule** and **History visibility**. Choose
  **Change**, pick a value, and choose **Save**. The explanation under the choice
  describes the value you are about to save, including when history becomes
  visible to anyone and that a change does not apply to messages already sent.

The badges at the top of **Room info**, such as **Public** or **Anyone can see
history**, move to the matching card. A card shows **Saving…** while the change
is sent and **Saved** once the room reports the new value; if the change fails,
the reason is shown in the same card and the old value stays. If your role cannot
change a property, its card shows the current value and says so. The room name is
edited at the top of the panel.

## Space names and access

Open **Space info** from the Space's menu or header. **Names** shows two separate
values:

- **Matrix name** is the Space's name on Matrix, shared with everyone in it.
- **Local name** and **Local icon** apply only to this device. Choose **Edit** to
  set or change one, then **Save**. **Clear** removes only that value: clearing
  the local name shows the Matrix name again and keeps the local icon.

**Access** shows who can join the Space and, if your role allows it, the
action that changes it; the confirmation and the result appear in the same card.
The **Access** entry at the bottom of **Space info** moves to this section.

## Download room history

Koushi can save a room, or every room of a Space, into a folder you can keep
or move to another place. Open the room and select **Room info**, then select
**Download** in its **Download history** section. For a Space, open **Space
info** and use **Download Space history**: it saves every room of the Space you
have joined, including rooms in its subspaces, and leaves out direct messages.
Rooms you have not joined are listed as skipped.

1. Choose the range:
   - **All available history** saves every event your account can read.
   - **Period** saves the events from the start date through the end date,
     inclusive. The dialog shows the time zone it uses for the dates, which is
     your computer's time zone. Dates before 1970 cannot be chosen. To stay
     fast in long rooms, a period download reads only the history around the
     period, so a message whose timestamp is more than a day out of order with
     the messages around it can be left out.
2. Select **Choose folder and download**, then choose where to save. Koushi
   creates a folder named after the room or Space and the date inside it.
3. The dialog lists every room with its progress: reading messages,
   downloading attachments, writing the page, done, skipped, or failed.
   Closing it does not stop the download; **Room info** or **Space info** keeps
   showing the progress. To stop, select **Stop** in the dialog (select
   **Download** again to reopen it).

The folder holds, for each room:

- `index.html`, a page you can open in a web browser, without a network
  connection. It shows messages with their senders and times, replies,
  threads, reactions, edits, and math. Images appear as small previews that
  open the full picture.
- `files/` with every attachment, and `thumbs/` with the image previews.
- `messages.json`, in the format of Element's chat export, so tools that read
  Element exports can read it, and `events.jsonl`, every event as the server
  sent it, one per line.

The top folder's `index.html` lists the rooms and links to their pages.

Every attachment is downloaded, one file at a time, so a large room or Space
can take a long time and use a lot of disk space. What can be saved depends on
your permission to read the history, on what the server still keeps, and on
which messages this device can decrypt. An attachment that cannot be
downloaded is marked on the page, and the download continues.

**Continue an interrupted download.** If you stop the download, quit Koushi,
or the download fails, the rooms that finished are kept. Download again and
choose the same folder (the one Koushi created): it continues with the rooms
that did not finish, and adds rooms that joined the Space since. After a
download with failed rooms, **Retry failed rooms** does the same in one step.

**Encrypted messages and attachments are saved unencrypted.** Anyone who can
open the folder can read them, so keep it somewhere safe. It is not a backup:
neither Koushi nor Element can import it back.
