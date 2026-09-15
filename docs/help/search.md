# Search past messages

[User guide and version selection](README.md)

## Search a conversation or several rooms

1. Use the search field at the top of the window.
2. Choose its scope: **All**, **Space**, or **Room/DM**. Select the relevant Space
   or room first when using a narrower scope.
3. Type a search term. Results appear in the Search panel; wait for an active
   search to finish.
4. Select a result to open the matching message in context.

If the app reports **Search term is too short**, enter a longer term. Search
supports Japanese and other CJK text as well as Latin text. **Explore** is a
separate public room directory, not message search.

## Why older messages may be missing

Message search uses a local index. The app must retrieve and, for encrypted
rooms, decrypt history before it becomes searchable. Results can grow while
history indexing is running. An empty result does not prove the message is
absent from the server.

Check the scope, use a distinctive phrase from the message, and inspect
**User settings → Search history**. If the message cannot be decrypted, address
[recovery](security-and-recovery.md#recover-missing-encrypted-history) first.

## Manage history indexing

1. Open **User settings** at the bottom of the left rail.
2. Choose **Search history**.
3. Check the activity summary and **Room index status**.
4. Use **Resume crawler** if paused. Choose **Standard**, **Fast**, or **Slow**
   for the crawl speed; room rows offer **Start** and **Stop** when available.

**Processed** counts timeline events scanned; **indexed** counts searchable
messages. They need not be equal. **Index media captions** and **Index file
names** control those additional searchable fields; this is not a promise to
search text inside attached documents.

**Rebuild search database** clears the local search index and retrieves history
again after confirmation. Use it when intentionally rebuilding the index;
results may be incomplete while it runs. It cannot restore encryption keys or
make inaccessible server history available.
