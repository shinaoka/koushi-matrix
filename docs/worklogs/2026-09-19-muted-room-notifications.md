# Muted-room notifications and sound

## Contract

Server push rules are projected into Rust room notification policy before unread
counts. Missing or malformed cached account data does not unmute a room. Standard
SDK modes and existing app-owned rules are recognized. Account-data-only changes
reproject the ordered room-list accumulator without replacing it.

Local writes retain optimistic policy through HTTP completion. A bounded direct
server read reconciles the effective mode independently of sync-cache latency.
Request/generation guards reject stale results; fresh push sync releases completed
write fences so other clients can supersede local settings. A failed confirmation
read retains local policy until the next push sync. Logout clears these fences.

Sound now requires an eligible Rust-owned notification candidate. Badge growth
alone cannot play it. Native dispatch checks the same candidate, enabled sound
setting and idle dispatch; the platform adapter retains cooldown/in-flight state.
Raw unread/history data and existing All/default writer semantics are preserved.

## Review

Independent review identified and resolved stale-cache overwrite and an indefinite
matching-echo wait. Final design/diff review approved the request-fenced server
confirmation plus fresh-sync reconciliation. Main's redaction behavior is retained.
The SDK compile-time recursion limit is 256 because the nested projection future
exceeded the default layout-query depth (130 versus 128).

## Validation

Regression checks reproduced stale mode overwrite and candidate-free sound before
fixes. Targeted state, SDK, live observer, native command and frontend tests passed;
the signed macOS application was built and locally installed from origin/main plus
this patch. Actual incoming real-account notification delivery was not exercised.
Merge-gate results are recorded in the PR. The settings location map is unchanged;
the room-notification guide now describes mute behavior.
