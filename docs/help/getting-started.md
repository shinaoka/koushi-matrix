# Getting started

[User guide and version selection](README.md)

## Install Koushi

1. Open [Downloads](../../README.md#downloads) and choose the installer for your
   operating system. Check [Platform support](../../README.md#platform-support)
   before choosing a build; support differs by platform.
2. On macOS, open the DMG and copy Koushi to Applications, then launch it. On
   Windows, run the installer. On Linux, use the package appropriate to your
   distribution or make the AppImage executable and run it.
3. Use the checksum alongside the release asset when checking a download.

Koushi is a client for an existing Matrix account. Have your homeserver address
and the sign-in method supplied by your account provider ready.

## Sign in

1. Enter your server address in **Homeserver** and select **Check login methods**.
2. Use the method offered by your server:
   - **OIDC** or **Single sign-on** opens the browser. Complete the provider's
     sign-in flow and return to Koushi.
   - For **Password** sign-in, enter your username, password, and device name,
     then select **Sign in**. The username is the local part: for the example
     Matrix ID `@alice:example.org`, enter `alice` in **Username**.
3. Complete **Verify this session** and any secure backup setup shown before
   entering the workspace. See [Security and recovery](security-and-recovery.md).
4. Wait for your rooms to load. Older messages may take additional time to
   retrieve and decrypt.

If **Create account** appears, it opens the account provider's registration
page. Availability depends on the server. If the server is unsupported or cannot
be reached, use [sign-in troubleshooting](troubleshooting.md#cannot-sign-in).

## Open a conversation

Select **Home** at the top of the left rail. Select a room or direct message
from the sidebar, or open **Invites** to review an invitation. To start a
conversation or join a room by address, see [Rooms and Spaces](rooms-and-spaces.md).

The center pane shows the conversation. Context such as room information,
search results, or a thread can appear on the right.

## Find settings

Select **User settings** at the bottom of the left rail. A foreground dialog
opens with categories on the left and the selected settings on the right. See
[Settings and help](settings.md) for the location of every setting. Settings for
a particular room are available through **Room info** in that room's header.

For keyboard behavior, open **User settings → Keyboard**. This shows the
shortcuts for your platform and lets you choose how Enter sends a message.
