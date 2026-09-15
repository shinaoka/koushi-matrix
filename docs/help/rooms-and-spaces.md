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

## Room information and notifications

Open **Room info** to find members, files, room notification options, and settings
available to your role. Room notification choices include **All messages**,
**Mentions only**, and **Mute**. Device notification permission and global
notification settings can also affect whether a desktop notification appears.
