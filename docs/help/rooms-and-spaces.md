# Rooms and Spaces

[User guide and version selection](README.md)

## Navigate Home and Spaces

A room contains a conversation. A Space groups rooms; joining a Space does not
necessarily join every room it lists.

Choose **Home** in the left rail to see your account-level navigation. Home's
header row carries **Activity**, **Explore**, and **Invites** at the start and
**Threads**, **Scheduled messages**, and **Info** at the end. A Space replaces
the start of that row with **Members** and keeps the same **Threads**,
**Scheduled messages**, and **Info** actions at the end. Choose a Space to see
its rooms.

A Space lists the rooms it contains, and Home's **Rooms** list holds the rooms
that belong to no Space, so one room is not listed in both. A room that belongs
to several Spaces is listed under each of them. Home always shows all your direct
messages, and its rail badge and **Activity** cover every conversation in the
account. If a room is absent from the Space you are viewing, open the Space that
contains it, or Home for a room in no Space, rather than assuming it has
disappeared. The clock button opens the
[scheduled messages panel](messaging.md#schedule-a-message).

## Read or change a Space description

Select a Space, then choose **Info** in its header. The **Description** card
shows the shared description, including line breaks. **No description** means
the Space has none; **Loading description…** means its settings are still loading.

If your role allows description changes, choose **Edit**, enter the description,
then **Save**. Choose **Clear** to remove it. These changes are shared with
everyone in the Space and appear in other Matrix clients. The local name and
icon in **Names** change only how this device displays the Space.

The card shows saving progress and the result. If a save fails, choose **Edit**
to retry the text you entered. Without permission to change the description,
you can still read it and see updates from other members.

## Join a room

If you received an invitation:

1. Open **Home**, then choose **Invites** in the header.
2. Select the invitation and check the room and inviter.
3. Choose **Accept** to join, or **Decline** to reject it.

If you have a room address:

1. Open **Home**, then choose **Explore** in the header.
2. Enter the room address or Matrix link in the address field and choose
   **Preview**.
3. Review the preview and choose the available join action. Invite-only rooms
   require an invitation; a preview does not grant access.

Explore also lets you search a server's public room directory. This searches
rooms to join, whereas the top search field [searches messages](search.md).

## Suggested rooms in a Space

A Space can mark some of its rooms as suggested. In the Space's **Not joined**
section, suggested rooms come first and show a sparkle icon before the name;
hover over the row to see "Suggested by this Space".

When you join a Space that suggests rooms you can join, Koushi asks once:
**Join the suggested rooms in *Space*?** The dialog lists the rooms. Choose
**Join all** to join them, or **Skip** to join none. The question is not asked
again for that Space. Invite-only rooms are never offered, and nothing is joined
unless you choose **Join all**. If some rooms cannot be joined, the dialog lists
which ones were joined and why the others were not.

If your role allows it, **Space info** shows a **Suggest to *Space* members**
checkbox beside each room in **Rooms**. It changes only whether that room is
suggested; the room's place in the Space and its access are unchanged.

## Create a room or Space

Use **Create room** in the sidebar header, optionally enter a name, and choose
**Who can join**:

- **Members of *Space*** (only when a Space is selected): anyone in that Space
  can find and join the room. You can also allow members of your other Spaces.
- **Ask to join** (only when Koushi reports that the room it will create
  supports knocking): people can send a request to join, and a room admin
  approves or declines each request. Inside a Space this choice reads
  **Members of *Space* can join directly; others can ask to join**: Space
  members join without asking, and everyone else can ask to join.
- **Only people you invite**.
- **Anyone**: a public room. Anyone, including people outside your Spaces, can
  find it in the public room directory and join it. A public room is never
  encrypted.

A room created from **Home** is not part of any Space, so it never offers
**Members of *Space***; it offers **Only people you invite** and **Anyone**,
plus **Ask to join** when knocking is available. When the selected Space is public,
the dialog starts with **Anyone** selected.

Under **Earlier messages**, choose whether new members can read messages sent
before they arrived, or only messages from when they were invited. For a public
room you can also let people read the room without joining. Private rooms are
encrypted unless you turn encryption off. A one-line summary above the buttons
says what your choices mean.

If you leave the room name empty, the room shows a name worked out from its
address or its members instead; a room with only you in it can show as
**Empty Room**. You can set or change the name later in the room's settings.

A public room also has an **address**, such as `#research-group-3fa9c2:example.org`.
The name is what people see in their room list; the address is the link
that opens the room, for example in a message or on a web page. Koushi starts with a random
address that begins with the Space name (or `room` at Home). Choose
**Change** to replace it with one that is easier to read, such as
`#research-group-papers`. The address must be unique on your server, and every
Space on that server shares the same addresses.

Koushi checks the address as you type. If it is already in use, **Create**
stays disabled, the dialog says so, and it offers an alternative you can use
with **Use this address**. If the check cannot be completed, you can still
create the room; the server decides when the room is created. If the address
was taken in the meantime, the dialog keeps everything you entered and moves
you to the address field.

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
3. Review the offered scope, then choose **Invite**.

Inviting never changes room settings. Under **Earlier messages** the dialog
says in one sentence what the invitee will be able to read, based on the room's
current history setting: earlier messages (shared history), messages from when
they were invited, or messages from when they join. In an encrypted room with
shared history it adds a short note that some earlier messages may stay
unreadable when their keys cannot be shared. If you are allowed to change the
history setting, a **Change in Room Info** link opens Room Info and brings you
back to the invite; otherwise no link is shown. Changing the setting does not
revoke events or keys already shared.

For encrypted rooms, Koushi accepts the inviter's historical-key bundle for
30 days after you accept that room's invitation on this client, including
recovery after restarting the app. Imported keys do not expire after 30 days.
This does not request a replacement bundle or restore a pending import that
an older version already discarded. Joining a room through a space without
an individual room invitation does not trigger this invitation-based sharing.

If the action is unavailable, your role or the current room state may not allow
it. Room history visibility and room entry rules are separate settings.

## Conversation list sections

The conversation list shows **Rooms** above **DMs**. Each heading can be
collapsed and sorted from its own menu. The menu closes when you choose an
option, press anywhere outside it, or press Escape, and only one section's menu
is open at a time. The number to the right of a heading is
its unread total for the current Home or Space view; it turns into a red badge
when there is something unread, shows `99+` above 99, and disappears at zero. It
stays on the Rust-reported total for the whole view, so filtering the list or
collapsing the section does not change it.

Long sections show their first five conversations, followed by a **Show N more**
control at the end of the section. That control only expands the section; to
bound it again, use the fold button in the section heading. It sits at the left
of the heading, next to the heading's own collapse chevron, while the create (+)
and section options (⋯) buttons stay to its right. The choice is
remembered per Home or Space and per section, and it is separate from collapsing:
a collapsed section hides every row and both controls, and neither control
changes whether the section is collapsed. While the filter box has text, every
matching conversation is shown and the controls disappear until you clear the
filter. An open conversation past the first five keeps its section expanded so
its row is always visible; while that is what holds the section open it offers
no fold control. A conversation opened from a notification shows its section
even when you had collapsed it, until you collapse that section yourself
(changing the five-row bound does not re-collapse it).

A conversation's own unread badge counts unread replies in its threads as well as
messages in the main timeline, so a room can show unread while the main timeline
has nothing new; reading the thread clears its share. Read markers, the
first-unread position and event navigation stay on the main timeline and ignore
thread replies.

Red numbers count messages the app has observed as unread. The Home and Space
numbers add the eligible room and DM numbers; invitations use a separate green
badge. A muted conversation keeps its red number on its own row but does not add
to Home, Space, or section totals. Hover over a conversation number to see its
main-timeline and thread parts. The room's **Threads** button shows the thread
part, and each counted thread can be opened from the Threads list. The Space
number appears at the upper right of its icon; its access symbol stays at the
lower right. If thread metadata fails to load, already counted threads remain
openable by their known root IDs.

Activity's **Unread conversations** tab lists rooms with a confirmed local
unread count. It opens the room without naming an unverified message. **Recent**
shows observed history; it does not claim those messages are unread. Activity
does not offer a bulk read action because a room count cannot identify the
right event IDs to acknowledge. A failure to load older Recent history does
not hide the confirmed room entries in Unread conversations.

The context-menu action **Mark room timeline as read** applies to the main
timeline only. It appears only when the app has a current main-timeline event
to mark. Open a thread to read its visible replies; a reply already on screen
is marked read without requiring an extra scroll. A manually marked unread room
uses a red dot on its row, not an invented numeric message count.
Leaving a room discards its cached thread counts; rejoining starts a fresh
read of those threads.

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

## See who can join a room or Space

Two separate facts are reported, and neither is used to mean the other: **who
may join** and **whether the conversation is encrypted**. Every conversation
row, the room header, the Space rail, the Space header, and **Room info** show
the participation indicator; the conversation header and **Room info** also
show the encryption indicator.

The participation indicator is one icon with one label. Hover it to read what it
means for that room or Space; the icon, the short label beside it, and the
control's accessible name state the kind without the longer explanation.

- **Public** (globe): anyone can join without an invitation.
- **Invite only** (person with a plus): an invitation is required to join.
- **Space members can join** (two people): the members of the Space named in the
  explanation can join without an invitation.
- **Conditions apply** (two people): members of the listed Spaces or rooms can
  join without an invitation, and everyone else needs one. When Koushi has read
  their names, the explanation lists the Spaces or rooms.
- **Can request** (raised hand): you can ask for an invitation, but joining
  still requires one.
- **Space / request** (raised hand): membership follows the Space named in the
  explanation, and everyone else can ask for an invitation.
- **Not joinable** (question mark): the room keeps a join rule with no ordinary
  way in, and an invitation alone may not be enough.
- **Checking** (spinner) or **Unknown** (question mark): Koushi has not read the
  join conditions yet, or it does not recognize the rule the server reports.

The encryption indicator is separate: **Encrypted** (closed padlock) or **Not
encrypted** (open padlock), with its own explanation. A padlock never means that
an invitation is required, and an invitation is never shown with a padlock.

In the conversation list only the participation icon is shown, as the row's
accessible name, because a list row has room for one. Direct messages show
neither indicator in the list; the header and **Room info** report both facts for
the open conversation.

Neither indicator is a confidentiality or history-visibility signal: an
invite-only room can still hold unencrypted messages, and a public room can be
encrypted. History visibility and the other properties are shown and changed in
**Change room details, access, and history** and in **Space names and access**
below.

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
- **Access and history**: **Join rule** and **History visibility** each show a
  list of choices and, below it, what the current selection means: who can
  join, who can read which history, whether the room is encrypted, and whether
  it is listed in the public directory. The selected choice is marked, and the
  details say whether they describe the saved values
  or unsaved changes. Choose a value and then **Save** (or **Cancel**, or press
  **Esc**) to restore the saved value. Changing the join rule needs permission
  to change join rules, which is separate from permission to rename the room,
  so an account that may change access but not the room name can still change
  it. Choosing **Members of a Space** lists the Spaces you belong to; pick the
  ones whose members may join. A restricted rule is summarised by its
  people-facing access condition (**Conditions apply**, or **Space members can
  join** when exactly one Space is verified) rather than as **Not joinable**, and a
  rule the app cannot rewrite is shown disabled with the reason. The details
  keep server eligibility and key availability separate, note when history
  becomes visible to anyone, and state that a change does not apply to messages
  already sent. The same editor is used while creating a room.

The summary at the top of **Room info**, such as **Encrypted**, **Invite only**
or **Anyone can see history**, moves to the matching card when chosen; the
encryption summary is a statement of fact and does not open a card. A card shows **Saving…** while the change
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

## Request missing historical keys

On a message that cannot be decrypted, choose **Request decryption keys for past
messages**. **Requested** means the encrypted request was sent; it does not mean
all keys have arrived. You can still choose **Retry**. A responding member's
Koushi shares available keys when it comes online and responds. You do not need
to manually verify each other first. Each device must be signed by its own
account, unblocked, and free of a previously verified identity change. Element X
does not handle this Koushi extension; no reply can mean an offline or unsupported client.

This works after joining a room through a Space or public-room link, and after
the invitation's 30-day import window. The room must still allow shared history,
and both accounts must still be joined. Only historical keys the responder's SDK
marks shareable are sent; unavailable keys cannot be recovered by this action.

In rooms with at most 30 joined members including yourself, the default request
goes to the other eligible members. In larger rooms it goes to administrators.
For a wider manual request, open **Room Info → Troubleshooting → Request keys
from everyone**. This section is collapsed initially. Read its warning: shared
keys cannot be revoked, and this request reaches more room members.

At startup Koushi may retry administrators once for a room with recorded
undecryptable messages. A persisted 24-hour cooldown prevents repeated restarts
from sending more automatic requests. Manual retry remains available.

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
