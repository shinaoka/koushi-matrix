# Scheduled-send editing under the sending lock (#1983)

While "Sending is unavailable until Secure Backup is ready." was up, the composer
body was read-only but the scheduled-messages panel still offered an editable body
and time field and a working Save.

## Two layers, one of them already there

`App.tsx` derives `composerReadOnly = !conversationAccess.sendAdmitted` and hands it
to the composer as `canEdit`. The scheduled panel was mounted in the same two places
without any of it: `mediaLists.tsx`'s `ScheduledMessagesList` had no lock input at
all, so `openEdit` opened the shared `Composer` (whose `canEdit` defaults to `true`)
and Save called `onReschedule`.

The write path was already closed: `command_policy.rs` lists
`AppCommand::RescheduleScheduledSend` in `requires_send_admission`, so the runtime
answered a locked reschedule with `OperationFailed { TimelineOperationFailed {
SecureBackupRequired } }` and left the item alone. Nothing pinned that: the policy
test listed `ScheduleSend` but not its edit counterpart, and no runtime test ran a
reschedule with the gate shut. So the panel closed the editor by *accepting a press
that could not land*, and nothing noticed if the guard were dropped.

## Change

UI, matching #1941: the write path is closed where it converges, and the control is
marked unavailable.

- `ScheduledMessagesList` takes a required `canEdit` and passes it on: the Edit
  button is `disabled`, the shared `Composer` gets `canEdit` (so the box itself is
  locked in the render before the editor unmounts), `submitEdit` refuses — every
  write from this list converges there — and an open editor closes when the lock
  arrives, the analogue of #1941 closing an open emoji picker.
- The panel's scheduled branch in `rightPanel.tsx` shows the composer's own notice
  (`composer.sendBlockedSecureBackup`) above the list. It is not rendered inside the
  list: the selected-room pane already shows that sentence beside the same list, and
  `e2e/readable-provisional-shell.spec.ts` counts `main .composer-read-only-notice`
  expecting one.
- `composerReadOnly` lost its `= false` default on `RightPanelProps`, so a mount that
  forgets the lock is a type error rather than an unlocked panel.
- `rightPanel.tsx` passes `canEdit={!composerReadOnly}`; the selected-room pane
  `panes.tsx` passes its existing `canEdit`.

Write path: `RescheduleScheduledSend` joins the policy test's requires-admission
list, and a new runtime test runs the same command against an open and a shut gate.

## Evidence

- UI: the two new panel tests failed against the old component (the editor opened
  with the lock up; `Edit` had no `disabled`). A third test is the unlock regression
  and was green throughout.
- Write path: the pin test passed at once because the guard already existed, so it
  was checked for teeth — removing `RescheduleScheduledSend` from
  `requires_send_admission` made the locked test fail with
  `a refused reschedule changes nothing` (`body` moved to the edited text) and
  stopped the refusal event. Restored.

## The editor closes, and the half-typed edit is lost

The issue asked only that a locked session not open an editor or save; closing an
open editor on the lock is extra, and it discards whatever the user had typed (the
item itself is untouched). It is kept because the alternative — an open box whose
save cannot land — is exactly the shape of this bug, and because #1941 chose the same
response for an open emoji picker. The draft is not preserved for the next unlock;
the editor reopens from the stored item.

## Where the runtime test lives

`crates/koushi-core/src/runtime/tests/scheduled_send_lock.rs` instead of
`koushi-core-testkit/tests/runtime/runtime_scheduled_send.rs`, because the testkit
fixture has no way to shut the Secure Backup gate (no helper sets
`conversation_access().send_admitted`), and the in-crate `src/runtime/tests/` modules
already test pre-dispatch refusals this way (`settings_loading.rs`). Moving it would
mean a new testkit fixture first.

## Not changed

`CancelScheduledSend` is *not* in `requires_send_admission` and does reach the
homeserver through `AccountMessage::CancelServerDelayedSend`, so the panel's Cancel
stays live on a locked surface. Whether a read-only session should still be able to
withdraw a pending send is a product decision (it removes a future write rather than
making one), so it is left as it is and raised on the PR.
