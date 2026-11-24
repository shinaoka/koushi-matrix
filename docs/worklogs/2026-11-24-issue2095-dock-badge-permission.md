# The Dock badge needs the badge authorization option (#2095)

Reported: on macOS 26 the Dock icon never shows the unread badge, and System
Settings → Notifications → Koushi has no **Badge application icon** switch at
all. Koushi's own side was provably fine — `desktop.native_badge
apply_settled outcome=applied` with the label read back from `NSDockTile` — so
the badge was set and macOS declined to draw it.

## Cause

The notification authorization request asked for the wrong options.
`request_native_attention_notification_permission` called
`mac_usernotifications::request_auth()`, and that crate (0.3.1) hardcodes

```rust
UNAuthorizationOptions::Alert | UNAuthorizationOptions::Sound
```

in `src/auth.rs:27`. macOS lists the *Badge application icon* switch, and honours
application icon badges, only for an app that requested `UNAuthorizationOptionBadge`.
It never had, so there was nothing for the switch to control. The Dock badge path
(`setShowsApplicationBadge(true)` plus `setBadgeLabel`) was never the problem.

## What changed

- The request is made directly through `objc2-user-notifications`
  (`requestAuthorizationWithOptions:completionHandler:`), which is the same
  binding `mac-usernotifications` links, so the option set is now ours:
  `Alert | Sound | Badge`. `block2` builds the completion handler and a tokio
  oneshot carries the answer back, so the command keeps its "returns whether the
  OS granted banners" contract and stays `Send`.
- The requested bits are plain constants (`UNAUTHORIZATION_OPTION_*`,
  `REQUESTED_NOTIFICATION_OPTIONS`) rather than an `objc2` type, so the option
  set is testable on Linux and Windows too. A macOS-only test pins those bits to
  the binding's own values and to the combined option set, which is what makes
  the constant safe rather than a guess.
- Grants that predate the badge option: macOS adds newly requested options to an
  existing grant, but only when asked again, and it never re-decides a grant. The
  webview already asked once per launch, but only while the OS was still
  `not_determined`. It now also asks once per launch for an already `authorized`
  install, which is the only way an existing install can gain the badge option
  without the user touching the switch. A denial is still never retried, and the
  launch claim still makes it one ask per launch rather than one per render,
  mount, or account switch.
- Diagnostics: a request records `permission_settled` and a grant read records
  `grant_read`, both with `alert_requested`, `sound_requested`, `badge_requested`,
  `authorization` and `badge_setting` (`enabled`, `disabled`, `not_supported`,
  `unknown`); the request also carries `granted`. That is what tells a missing
  Dock badge apart from a missing badge grant on a real machine, and the
  `grant_read` line appears for installs that are already authorized without
  anything needing to be re-requested first.

`mac-usernotifications` is kept: the delegate (body clicks), send, cancel,
close-delivered and `getNotificationSettings` all still come from it, and reading
`badge_setting` through its settings wrapper is that same `getNotificationSettings`
call. Only the authorization request moved.

## Behaviour to verify on a real machine

None of this can be observed off macOS: `cargo test` has no bundle, so
`UNUserNotificationCenter` cannot answer there, and the issue's second claim (that
a grant without the badge option suppresses the Dock badge) is inferred from the
behaviour, not from Apple documentation.

1. Existing install, notifications already allowed: launch this build, leave
   Settings alone, and check that System Settings → Notifications → Koushi now
   has **Badge application icon** (on).
2. Unread messages in an unmuted room, window unfocused: the Dock icon shows the
   count.
3. Diagnostics show `desktop.native_notification permission_settled` with
   `badge_requested=true` and `badge_setting=enabled`, and
   `desktop.native_badge apply_settled outcome=applied`.
4. A fresh install still prompts once and the prompt still covers banners; the
   Badge switch is present and on after allowing.
5. A denied install is not prompted again, and Settings still offers System
   Settings.

## Verification run here

- `cargo fmt --all -- --check`, `cargo check -p koushi-desktop --locked`.
- `cargo test -p koushi-desktop --lib -- native_attention`: 37 passed, including
  the option-set test on every platform, the macOS bit and mapping test, and a
  test that a grant read records `badge_setting` and `badge_requested` without
  claiming a grant answer.
- `cargo clippy --profile ci -p koushi-desktop --all-targets --locked -- -D
  warnings`.
- Frontend: `npm run typecheck`, `npm run lint`, `npx vitest run` (2730 passed),
  including the new launch-time re-ask test and the updated startup tests.
- Not run here: a Linux build of `koushi-desktop` (no Linux sysroot on this
  host). The new macOS-only items carry `allow(dead_code)` off macOS in the same
  style as the surrounding code, so the shared definitions cannot fail that
  crate's Linux lint job; Linux CI owns that proof.
