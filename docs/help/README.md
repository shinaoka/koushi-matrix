# Koushi user guide

> Text-based help for using Koushi, a desktop Matrix client. These pages are
> written for both people and AI assistants answering questions about the app.

## Choose the right version

These instructions describe the code at the **same Git revision as this file**.
On `main`, they may include changes not yet in your installed release. For an
installed version, open its [GitHub Release](https://github.com/shinaoka/koushi-matrix/releases)
and follow the **User guide** link, or select its tag in GitHub's file browser
and open `docs/help/README.md`. Older tags created before this guide was added
will not contain it; do not assume the current guide applies unchanged.

The guide uses English UI labels. The app may show translated labels according
to its language setting. You can ask an AI assistant to explain the steps in
your preferred language.

## Using Koushi

- [Getting started](getting-started.md): Install Koushi, sign in, open your first conversation, and find settings.
- [Settings and help](settings.md): Find every setting by category, keyboard shortcuts, and the GitHub URL for AI assistance.
- [Rooms and Spaces](rooms-and-spaces.md): Join or create rooms, manage invitations, and navigate Spaces and direct messages.
- [Messaging](messaging.md): Send text and files, edit messages, reply, and use threads.
- [Search](search.md): Find past messages and manage local history indexing.
- [Security and recovery](security-and-recovery.md): Verify a session, recover encrypted history, and distinguish key backups from chat exports.
- [User trust model](user-trust-model.md): Understand unverified users, verified identities, identity resets, and device trust.
- [Troubleshooting](troubleshooting.md): Resolve sign-in, missing-history, search, and sending problems, or report a bug.

## Ask an AI assistant

Share the [repository](https://github.com/shinaoka/koushi-matrix), your installed
Koushi version, operating system, and what you want to do. For example:

> I use Koushi version [version] on [operating system]. Read the user guide at
> https://github.com/shinaoka/koushi-matrix/tree/main/docs/help, check the matching
> release tag, and explain how to search past messages in my version. Answer in
> Japanese and link to the pages you used. If the matching guide is unavailable,
> say so before using current instructions.

Keep passwords, recovery keys, key export files, and private messages out of
prompts and public bug reports.

For AI readers: start with the relevant page above. Follow links within the
same revision. Product plans and issues describe proposals, not necessarily
available features. If a detail is absent, identify the gap instead of treating
Element's behavior as Koushi's behavior. The repository's [llms.txt](../../llms.txt)
is generated from this index; it is another route to the same help.
