# Search Adapter

The search adapter verifies local search candidates before the UI receives any
snippet or highlight.

`matrix-sdk-search` owns encrypted Tantivy indexes and returns candidate event
IDs. `matrix-desktop-search` owns deterministic verification over resolved
visible event content. The Tauri backend will map Matrix SDK event-cache updates
into the adapter and map verified adapter results into reducer actions.

The adapter does not store data on disk, open OS secrets, call the network, or
own a Matrix SDK client. It keeps only the in-memory resolved document snapshot
needed to verify current search candidates.

Security rules:

- decrypted text is wrapped in redacted debug types;
- ngram candidates are dropped unless an exact visible span is found;
- highlight ranges are UTF-16 offsets relative to the returned snippet;
- attachment filenames use `SearchMatchField::AttachmentFileName`;
- edit events downloaded before their targets are held as pending relations;
- redactions remove base events, applied edits, and pending edits.
