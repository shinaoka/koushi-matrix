# Issue #775 — native, observable OIDC browser launch

## Scope and acceptance

- `start_oidc_login` remains one Tauri command that obtains the Rust-owned OIDC authorization and launches its HTTP(S) URL through the native opener before returning.
- Windows success/failure tests prove the native launch seam; Linux/macOS compile and existing generic link behavior remain unchanged.
- Authorization creation failure, invalid URL, and native launch failure map to distinct localized messages in App's existing React-local `loginTransportError` alert. The alert clears at each new start and remains clear on success; Rust-owned authentication errors remain separate.
- Diagnostics contain only fixed source/stage tokens: `authorization_created`, `url_rejected`, `browser_launch_requested`, `browser_launch_failed`, `browser_launch_succeeded`. They never contain URL, query, state, code, homeserver, or native error text.
- Repeated starts for the same pending OIDC attempt reuse the existing authorization and never create/replace a competing SDK attempt.
- Callback/deep-link completion continues consuming the original Rust-owned pending attempt.

## Ownership and boundary

Today the Tauri command returns the authorization URL/state to React, which calls the generic link/media port. That port catches native opener failure and performs an unobservable asynchronous `window.open` fallback. `App.startOidcLogin` has no catch, so authorization and launch failures look identical to no action.

Move only the system-browser launch across the existing Tauri boundary. AccountActor continues owning PKCE/state/pending SDK flow; Tauri owns native URL validation and OS opener invocation; React receives only a typed coarse outcome plus settlement and renders it. Timeline links keep the generic port and fallback behavior.

## Native response

Replace the frontend-facing `OidcAuthorization` shape with:

```text
OidcBrowserLaunchResponse {
  outcome: launched | invalid_authorization_url | browser_launch_failed,
  settlement: CommandSettlement
}
```

No authorization URL or state crosses into the WebView through either the command response or the broadcast Core-event stream. Authorization creation still uses command rejection; React catches that rejection without inspecting/logging it and sets the existing `loginTransportError` alert to `auth.ssoAuthorizationFailed`. It sets that state to null before every start and leaves it null on `launched`. The two typed launch failures set dedicated catalog messages after applying settlement.

## Native launch helper

Add a small Tauri-layer helper that parses with `url::Url`, accepts only exact `http`/`https` schemes with no username/password userinfo, and invokes an injected `FnOnce(&str) -> Result<(), E>` with the original URL bytes. Production supplies `app.opener().open_url(url, None::<&str>)`. The helper returns only `InvalidAuthorizationUrl` or `BrowserLaunchFailed`; neither error retains the URL/native error.

Command ordering:

1. await correlated `OidcAuthorizationCreated` exactly as today;
2. record `authorization_created`;
3. validate URL; on rejection record `url_rejected`, return typed failure with settlement;
4. record `browser_launch_requested`, invoke native opener;
5. return/record typed success or failure.

A native opener success proves OS dispatch was requested successfully; no impossible claim is made that a third-party browser painted a window.

## Pending-attempt idempotence

Replace AccountActor's pending OIDC tuple with a private struct containing the original start request, SDK flow, authorization URL, and state. On a repeated `StartOidcLogin` for the same normalized homeserver, emit `OidcAuthorizationCreated` for the new request ID using the retained authorization, without calling the SDK or replacing the pending flow. A start for a different normalized homeserver is rejected with a correlated coarse cancellation failure and leaves the original pending attempt intact; they never coexist. Tauri surfaces that command rejection through the same caught `auth.ssoAuthorizationFailed` alert (it is not a native-launch outcome and needs no fourth catalog key). Completion still takes the one pending struct and consumes its original flow.

This also makes a user retry after native launch failure relaunch the same valid pending authorization rather than create competition. Frontend `isBusy` prevents overlap while one command is in flight; Core idempotence covers later repeated clicks. A callback attempt consumes the pending flow even if provider completion fails, after which the next start is fresh. Add one private AccountActor retirement helper used by logout/change-homeserver/account reset: it takes and drops the SDK flow and, for a fresh-login allocation, invokes the existing pending-login cleanup with `BrowserCancellation` evidence before proceeding. Restart also drops the in-memory flow and startup journal reconciliation handles interrupted cleanup. No timer-based OAuth expiry is invented locally.

## Verify first

Before production changes:

1. App/browser test: signed-out SSO click with command rejection must show authorization error; typed invalid URL and launch failure must show distinct visible errors; success must clear error. Existing implementation is RED (unhandled rejection / URL sent to generic link port).
2. Tauri helper tests: HTTP and HTTPS call injected launcher with exact URL; ftp/malformed/userinfo reject without calling; launcher rejection maps coarsely and Debug/diagnostics contain no URL/query/native text. Add `cargo test -p koushi-desktop oidc_browser --lib` to the `windows-overlay-acl` job in `.github/workflows/ci.yml` (retaining its overlay test) and use a `koushi-windows-tauri-native` cache key; this injected-launcher test is the deterministic Windows seam proof while the production call compiles against `OpenerExt`.
3. AccountActor tests: configure one synthetic pending flow, repeat same-homeserver start, receive new correlated authorization event while original completion still succeeds; different-homeserver start fails while the original still completes; change-homeserver/logout retirement makes a later completion fail as cancelled and covers pending cleanup.
4. Static/browser boundary assertion: SSO path no longer calls `openExternalHttpUrl`/`window.open`; generic timeline links remain unchanged.
5. Core-event forwarder test: `OidcAuthorizationCreated` reaches the WebView with request correlation only; URL/state are absent, while the runtime waiter still receives the full internal event.

Then run identical tests GREEN.

## Canon and mirrors

- Update `DesktopApi`, `TauriDesktopApi`, test IPC normalization, browser fakes, and TypeScript types.
- Special-case `OidcAuthorizationCreated` in `core_event_forwarder::serialize_core_event` to emit only `{ request_id }` to the WebView. Keep the full internal Core event for the Tauri request waiter. Update `coreEvents.ts` and regenerate `coreEvents.generated.json` through its Rust contract test; do not hand-edit the artifact.
- Add exactly three English/Japanese/pseudo-derived message IDs and catalog parity tests: `auth.ssoAuthorizationFailed`, `auth.ssoInvalidAuthorizationUrl`, and `auth.ssoBrowserLaunchFailed`.
- Update `docs/architecture/state-machine.md`: authorization artifacts remain Core/Tauri-only; repeated starts replay one pending attempt; Tauri validates/launches and projects coarse outcome.
- Update `docs/agents/state-ownership.md` with the native OIDC launch boundary and redaction rule.
- Register no new Tauri command; keep `start_oidc_login` registration unchanged.

## Cross-platform and fallback decision

OIDC uses the registered native opener on Windows, macOS, and Linux; only generic timeline links retain the React link/media port and `window.open` fallback. SSO deliberately has no WebView fallback because an asynchronous popup is unreliable and unobservable. Native launch failure tells the user to configure a default browser and retry; retry relaunches the same pending authorization without creating competition.

## Gates

Focused Rust/Tauri/actor/frontend/browser tests; Windows runner execution; callback/deep-link tests; full Core/SDK/desktop/state tests, Vitest and relevant Playwright; typecheck, lint, build, format/diff, generated/protocol/SDK-submodule/secret/boundary/docs checks; independent implementation review; GitHub required CI, merge, Issue close, main CI.

## Review record

- Design review Round 1: one Critical WebView-event leak, two Important test/error-surface gaps, and two Minor lifecycle/URL findings. The plan now redacts the broadcast event projection, pins Windows CI execution, uses/clears the existing local transport alert, rejects URL userinfo, and documents pending-attempt retirement/replay semantics.
- Design re-review found one blocking contradiction plus four Minor clarifications. The plan now consistently rejects a different homeserver while retaining the original pending flow, names all three catalog keys, explicitly removes the SSO fallback on every platform, and gives the expanded Windows job a dedicated cache key.
- Final design re-review: `reviewer-flash` **Correct-to-merge** after verifying explicit `BrowserCancellation` retirement. Its final Minor was incorporated: different-homeserver rejection uses the caught authorization-failure alert, not a fourth launch outcome.
- Implementation review: `reviewer-flash` **Correct-to-merge**. Follow-up review confirmed parsed and empty-userinfo rejection plus account-reset retirement. The final defense-in-depth Minor was incorporated by rejecting backslash-normalized authorization URLs; the optional duplicate reset-path test was not required because the shared helper and direct call were both reviewed.
