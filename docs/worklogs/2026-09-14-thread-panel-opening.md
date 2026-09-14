# Thread panel opening after cached pagination

The supplied diagnostic shows repeated thread subscription builds completing,
followed by a zero-millisecond initial page and Opening returning to Closed,
without actor spawn. It cannot distinguish SDK error from premature projection
inspection. Do not copy account logs or message contents into this worklog.

Core inspected a second SDK snapshot immediately after paginate_backwards.
SDK thread pagination sends cache diffs to a separate relay before returning;
its return does not guarantee the display projection is updated. A synthetic
readiness test failed before the fix because it completed with failure before
the delayed projection arrived.

Core now keeps the pre-pagination subscription and applies only its diffs to
its own vector. Successful non-end pagination awaits nonempty content under one
10-second deadline. End-reached remains authoritative, including empty. This
is content readiness, not proof the exact page has fully committed; normal
actor subscription receives subsequent changes. No fixed sleep, retry, SDK
patch, or detached waiter was added. EOF, deadline and SDK failure have distinct
closed diagnostic tokens. The existing SDK-error path remains recoverable.

Four focused async tests pass: delayed publication, end/EOF, non-extending
deadline, and cancellation releasing the stream. Independent review approved
behavior and canon; its missing diagnostic allowlist finding was fixed and
covered by a token test. This unit evidence does not by itself prove the user's
particular failed open had no other cause. Installed-app validation follows.
