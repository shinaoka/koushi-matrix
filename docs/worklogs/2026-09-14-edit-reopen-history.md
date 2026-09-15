# Reopening an edited message: pre-upgrade source comparison

Source inspection and deterministic SDK/Core reproduction on 2026-09-14.

Compared Core immediately before the SDK upgrade (`fd52c723^`) and the old SDK
pin `a04792c7a` with the current checkout. Both the old and current Core build
`actions.editable_document` from `original_json_for_event_item`, which calls
SDK `original_json()`. TimelineItemRow prefers that document over displayed
`item.body` when opening the edit form. Therefore formatted messages can offer
old text for re-editing even when the effective displayed body is current.

The old SDK explicitly documents original_json as immutable after editing and
exposes latest_edit_json()/latest_json() for the effective revision. This API
contract predates the September upstream merge. Source blame shows this Core
selection at extraction commit b43de6dc (August 21); that extraction is not proof
of the original introduction date. This path is not a lost fork patch.

A second pre-existing issue is Core effective_message_content: for m.replace it
looks for m.new_content inside m.relates_to. The replacement content is a sibling
inside content, not nested inside m.relates_to. Fixing latest-event selection
alone would still read an edit's fallback body. This code is also present before
the upgrade. Original JSON must remain available for source/crypto diagnostics;
change only effective editable-content projection after a RED regression test.

This evidence explains a re-edit draft rollback path. It does not prove that a
server failed to save a submitted edit. No real message contents are included.

## Reproduction and correction

`reopening_edit_uses_latest_sdk_revision` and
`reopening_thread_root_edit_uses_latest_sdk_revision` sync an original rich
message followed by two replacements through a real SDK timeline. The second
case includes a bundled thread summary and reply, then updates the root
projection service. Before the fix both fail at revision 1: the SDK displays
`first edited text @Project`, but the editable document is
`original text @Project` (synthetic fixture text).

Core now uses `latest_json()` only for editable-document and mentions
projection, and reads replacement content from `content.m.new_content`.
`edited_thread_root_keeps_latest_document_through_service_and_display` also
verifies that the root service and relocated display retain each revision
rather than a stale fallback. The previous mentions replacement fixture put
m.new_content in the wrong location; it now uses the actual Matrix shape.

All four added edit checks and the entire Core suite pass: 1,082 passed,
9 ignored. This fixes a proven editor-prefill rollback, not a demonstrated
server-side failure to save. No extra SDK patch or gitlink update is involved.
