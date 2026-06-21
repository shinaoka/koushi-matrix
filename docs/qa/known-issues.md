# QA Known Issues

This file records currently known product/QA blockers that should be easy to
find before claiming a release gate is green.

## Local Synapse Probed SyncService Compatibility

Status: resolved on 2026-06-20 JST. Local Synapse probed stress now passes.

Date observed: 2026-06-20 JST.
Date resolved: 2026-06-20 JST.

Evidence:

- The local Synapse Docker harness starts successfully after forcing the
  generated listener to container port `8008`.
- Before the fix, forced LegacySync stress passed while probed SyncService stress
  did not:

```bash
npm --prefix apps/desktop run qa:headless-local -- --run --server=synapse --scenario=timeline_stress --core --core-backend=legacy --timeout-ms=240000
```

- Required stress tokens were present:

```text
stress_counts=spaces=2 rooms=4 messages=32
stress_space_scope=ok
stress_no_blank=ok
timeline_stress=ok
restore_cleanup=ok
```

- Probed SyncService selected `SyncService` and then failed during the stress
  path. Redacted failure:

```text
Headless core QA failed: timeline_stress sync receiver after room join: SyncOnce failed: SyncFailed { kind: Http }
```

- Synapse logged repeated `400` responses from its MSC3575 endpoint because
  `extensions.to_device.since` received a normal `/sync` token-like string
  instead of an integer-like token.

Root cause:

- The stress lane mixed manual `SyncOnce` calls with a running SyncService,
  creating a second sync path on the same client.
- Local Synapse can accept the initial SyncService run and later enter an
  HTTP/offline loop on the MSC3575 stream. Leaving that as reconnecting made the
  timeline stop receiving bodies while QA waited.

Resolution:

- `SyncActor` now leaves the SyncService observer on HTTP/offline failure and
  falls back to LegacySync once per actor session.
- `timeline_stress` now waits for live sync projections and timeline diffs
  instead of calling `SyncOnce` while SyncService is running.
- Successful probed Synapse command:

```bash
npm --prefix apps/desktop run qa:headless-local -- --run --server=synapse --scenario=timeline_stress --core --core-backend=probed --timeout-ms=240000
```

- Required stress tokens were present:

```text
sync_backend_a=SyncService
stress_counts=spaces=2 rooms=4 messages=32
stress_space_scope=ok
stress_no_blank=ok
timeline_stress=ok
restore_cleanup=ok
```

## E2EE Recipient Decryptability After Identity Reset

Status: open follow-up for historical undecryptable events. Current encrypted
macOS real-account sends are confirmed decryptable in Element Desktop as of
2026-06-19 21:35 JST.

Date observed: 2026-06-19.

Evidence:

- The macOS real `matrix.org` GUI smoke reached `send=sent` when sending to the
  approved self-DM target `@hiroshi.shinaoka:matrix.org`.
- Element Desktop showed the event from the test account, but the body rendered
  as `Unable to decrypt message`.
- Element Desktop's event source projected the encrypted event as
  `m.bad.encrypted` with:
  `The sender's device has not sent us the keys for this message.`
- Element on iPhone could decrypt the same message body and also showed:
  `Hiroshi Shinaoka (test)'s (@hiroshi.shinaoka.test:matrix.org) identity was
  reset.`
- `headless-core-qa` previously ran `reset_identity_for_qa` at the end of the
  E2EE trust stage. That operation is appropriate only for disposable QA
  identities, not for reusable real-account QA.
- Follow-up macOS real `matrix.org` GUI smoke after adding diagnostics reached
  `send=sent` with `target_dm=encrypted target_selected=true target_members=2`.
  Artifact:

```text
artifacts/mac-gui-smoke/2026-06-19T12-26-29-962Z
```

- A private-data-free `/keys/query` diagnostic from the sender account saw
  `target_device_count=17 target_devices_with_keys=17 target_unsigned=0` for
  `@hiroshi.shinaoka:matrix.org`.
- A later `--keep-session` macOS real `matrix.org` GUI smoke reached
  `send=sent` with the same encrypted DM diagnostics, and Element Desktop could
  decrypt the latest synthetic message:

```text
artifacts/mac-gui-smoke/2026-06-19T12-29-57-087Z
```

- The same Element Desktop room still showed the older 21:12 and 21:26
  synthetic messages as `Unable to decrypt message`.
- Before cleanup, the retained Koushi crypto store for the latest successful
  run contained one outbound group session and no direct withheld-session rows.
  The generated artifact's `data/` directory was deleted after extracting those
  private-data-free counts because it contained real-account session material.

Implication:

- `send=sent` proves that Koushi submitted the event and observed send
  completion. It does **not** prove that every recipient device can decrypt it.
- The iPhone result shows that outbound key sharing is not completely broken.
  The remaining blocker is recipient-device decryptability after identity reset
  and trust changes, especially for Element Desktop.
- The raw event is a normal `m.room.encrypted` Megolm event. The failure is
  missing room-key delivery or acceptance for a specific recipient device, not
  malformed message content.
- The target room was an encrypted 1:1 DM when Koushi sent the follow-up
  synthetic message. Plain-room or wrong-room routing is not the explanation.
- The target user has many signed devices, so the release gate must prove
  per-device decryptability or explicitly define which recipient devices are in
  scope.
- The latest successful send proves that current encrypted DM delivery can
  succeed after recovery/sync settles. The remaining unresolved issue is
  historical undecryptable synthetic messages whose room keys were not delivered
  to Element Desktop when those messages were sent.
- Do not claim historical encrypted-event recovery is green until the QA plan
  distinguishes "submitted", "decrypted by at least one recipient client",
  "decrypted by all expected recipient devices", and "older events recovered
  after an identity reset". Current new-message delivery has been manually
  confirmed in Element Desktop for the artifact above.

Follow-up:

- `headless-core-qa` now requires `KOUSHI_QA_ALLOW_IDENTITY_RESET=1`
  before the E2EE trust stage performs identity reset; otherwise it prints
  `e2ee_identity_reset=skipped`.
- Determine whether Element Desktop needs to re-verify/trust the reset
  identity, request missing keys, or clear stale local sessions.
- Add private-data-free diagnostics to real-account send QA for encrypted
  target DMs: room encrypted, target device count known before send, and whether
  the SDK observed room-key sharing/withheld failures. Do not print event IDs,
  room IDs, sender keys, session IDs, ciphertext, access tokens, or message
  bodies.
- Decide whether the release gate requires Koushi to actively re-share old
  outbound room keys after recipient key requests, or whether manual
  re-verification/key-request handling in the recipient client is sufficient.
- Add a real-account QA assertion for encrypted recipient decryptability, not
  just local send completion. When full recipient-device automation is not
  available, record a manual Element Desktop/iPhone check as part of the release
  gate.

## Tuwunel Local Strict Recipient-Device E2EE

Status: open local-server follow-up. The same strict recipient second-device
lane passes on Conduit.

Date observed: 2026-06-20.

Evidence:

- Passing Conduit command:

```bash
npm --prefix apps/desktop run qa:headless-core -- --server=conduit --scenario=e2ee_trust --core-backend=probed --e2ee-recipient-second-device --timeout-ms=360000
```

- Expected strict token:

```text
e2ee_recipient_second_device_decrypt=ok
```

- Tuwunel command with the same strict option fails after the sender observes a
  recoverable send-queue failure:

```text
send flow failed: local_echo=true local_echo_send_state=NotSent(recoverable) send_completed=false event_id=false
```

- With private Rust SDK diagnostics enabled through `KOUSHI_QA_RUST_LOG`, the
  raw local log shows the SDK send queue disabling itself after:

```text
/_matrix/client/v3/keys/claim ... TimedOut
```

Implication:

- This is not evidence that Koushi intentionally omits room-key sharing. The
  send fails before completion while claiming one-time keys for the recipient
  second device.
- Keep the strict recipient second-device gate green on Conduit. Treat Tuwunel
  strict failure as a local homeserver compatibility issue until the
  `/keys/claim` timeout is resolved or isolated further.

## macOS Real Matrix.org GUI Message Sync

Status: resolved on 2026-06-19.

Date observed: 2026-06-19.

Evidence:

- Real `matrix.org` macOS GUI smoke with
  `--real-login-from-stdin --allow-empty-timeline` passed login, recovery,
  sync, room-list, room selection, timeline subscription, keyboard settings,
  user settings, and logout cleanup.
- That pass did **not** prove message synchronization because
  `--allow-empty-timeline` accepted `timeline_items=0`.
- A follow-up real `matrix.org` macOS GUI smoke with `--send-smoke-message`
  failed to prove message send/sync. Final QA title:

```text
session=ready sync=running rooms=5 spaces=1 active_room=true timeline_room=true timeline_subscribed=true timeline_items=0 errors=1 panel=closed send=pending
```

Artifact:

```text
artifacts/mac-gui-smoke/2026-06-19T11-43-33-888Z
```

Implication:

- Do not claim macOS real-account GUI message synchronization is verified.
- `timeline_subscribed=true` alone is insufficient; require either
  `timeline_items > 0` from real sync or a successful synthetic send
  (`send=sent`) that is reflected by the timeline/CoreEvent path.
- `--allow-empty-timeline` is acceptable only for sparse-account login,
  recovery, room-list, panel, and cleanup validation.

Follow-up:

- The previous active-room send smoke failure is kept above as historical
  evidence. The root QA harness issue was that synthetic sends targeted the
  automatically selected active room, which might not be a joined/writable room.
- The macOS GUI smoke runner now accepts `--send-smoke-user-id=USER_ID` and the
  frontend creates/selects that DM before sending.
- The QA title now includes `error_code=<code>` and the runner has `--verbose`
  plus `qa-diagnostics.log` so one failed run records the relevant
  private-data-free state transitions.

Resolved evidence:

- Real `matrix.org` macOS GUI smoke to the approved self-DM target
  `@hiroshi.shinaoka:matrix.org` reached `send=sent`, opened keyboard settings,
  opened user settings, and completed logout cleanup.
- Artifact:

```text
artifacts/mac-gui-smoke/2026-06-19T12-11-27-680Z
```

- Final relevant QA transitions:

```text
ready session=ready sync=running rooms=5 spaces=1 active_room=true timeline_subscribed=true timeline_items=0 errors=0 error_code=none panel=closed send=pending
send session=ready sync=running rooms=5 spaces=1 active_room=true timeline_subscribed=true timeline_items=0 errors=0 error_code=none panel=closed send=sent
logout session=signedOut sync=stopped rooms=0 spaces=0 active_room=false timeline_subscribed=false timeline_items=0 errors=0 error_code=none panel=userSettings send=sent
```

Recommended retry command shape:

```bash
npm --prefix apps/desktop run qa:mac-gui -- --real-login-from-stdin --allow-empty-timeline --send-smoke-message --verbose --timeout-ms=30000 --send-timeout-ms=20000
```

For a writable real-account send assertion, include:

```bash
--send-smoke-user-id=@hiroshi.shinaoka:matrix.org
```

The macOS GUI smoke runner now writes a private-data-free
`qa-diagnostics.log` into each artifact directory. Use `--verbose` for a
single diagnostic run that mirrors the same QA title state transitions to the
terminal while still keeping credentials, recovery secrets, access tokens, and
message bodies out of output.
