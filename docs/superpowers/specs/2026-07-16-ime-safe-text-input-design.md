# IME-Safe Text Input Design

Status: approved through GitHub issue #272 and the user's instruction to implement the shared design.

## Problem

The desktop frontend has one robust macOS/WKWebView composition lifecycle for the main/thread composer and message editing, but the upload caption and other editable text surfaces bypass it. The upload caption is directly controlled by a Rust-owned snapshot and writes every input event across Tauri before the next `value` arrives. Other fields duplicate ordinary controlled-input and form-submit behavior, so active IME DOM ownership and candidate-confirmation Enter are not repository-wide invariants.

The production TSX inventory contains 42 text-capable controls: 26 uncontrolled by the shared lifecycle while using React `value`, 14 DOM-only secret/recovery controls, and two composition-owned implementations. No `contentEditable` surface exists today.

## Goals

- One implementation owns composition epochs, DOM value/selection protection, external synchronization, and IME Enter fencing.
- Thin input, search, textarea, secure-input, and form adapters reuse that implementation.
- All current composable fields migrate to the adapters.
- Active or locally dirty DOM state cannot be overwritten by stale snapshot values.
- Async text mutation responses cannot apply older full snapshots after a newer request.
- Secret values stay DOM-only.
- Static enforcement prevents raw composable controls and raw forms from reappearing.
- Repository canon states the invariant explicitly.

## Non-Goals

- Do not move ordinary form validation or search semantics into the shared primitive.
- Do not make one universal product field with composer, alias, search, and authentication behavior as props.
- Do not change Matrix command semantics, composer shortcut settings, or Rust-owned message formatting.
- Do not log input values, filenames, Matrix identifiers, or secrets.

## Architecture

### Pure lifecycle and controller

`apps/desktop/src/domain/compositionLifecycle.ts` remains the single lifecycle owner and is generalized from textarea-only DOM synchronization to `HTMLInputElement | HTMLTextAreaElement`.

The controller owns:

- epoch-based `compositionstart` / deferred `compositionend` handling;
- the current DOM node;
- a locally-dirty value marker recorded before consumer callbacks;
- acknowledgement when an external value matches the current DOM value;
- suppression of stale external writes while composing or locally dirty;
- forced synchronization only when `syncKey` changes;
- unified `isComposerImeEnter` evaluation.

### React primitives

`apps/desktop/src/components/ImeTextControl.tsx` exports:

- `ImeTextField` for text/search/url/email/tel inputs;
- `SecureImeTextField` for DOM-only password/secret inputs;
- `ImeTextArea` for ordinary textareas;
- `ImeOwnedTextArea` for composer/edit callers that need the controller lifecycle;
- `ImeSafeForm`, which suppresses product submit caused by candidate-confirmation Enter while leaving the native key default untouched.

Feature callbacks retain domain semantics. The primitives only normalize text-control ownership, composition, and submit behavior.

### Async result ordering

`apps/desktop/src/domain/latestAsyncResult.ts` provides a keyed monotonic result gate and a per-key operation queue. Each submitted request receives a generation; operations for one logical field are serialized, pending generations superseded before dispatch are skipped, and only the current generation may apply its returned full snapshot. This preserves backend write order while coalescing intermediate keystrokes. Upload captions, per-user aliases, and invite queries use the queue; independent logical fields remain concurrent.

### Static enforcement

`scripts/check-ime-text-inputs.mjs` parses TSX with the TypeScript compiler API. Outside `ImeTextControl.tsx` and explicit tests it rejects:

- raw `textarea`;
- raw `input` with omitted type or text/search/password/email/url/tel type;
- `contentEditable`;
- raw `form`.

Non-composable input types remain allowed. The script runs from the desktop `lint` command.

## Data Flow

1. The browser mutates the native text control.
2. The shared primitive records the DOM value as locally dirty before invoking the feature callback.
3. Parent state or an async command may rerender.
4. Matching external values acknowledge the local value; stale values are ignored.
5. A `syncKey` change explicitly ends the old epoch and synchronizes the new field.
6. During composition, Enter is marked as an IME commit and is not passed to product Enter handlers.
7. `ImeSafeForm` consumes that mark if the browser emits submit, prevents only submit, and never prevents the native key event.

## Security

`SecureImeTextField` has no `value` or `defaultValue` prop. Callers read it through a ref only at explicit submit time. Validation state may store booleans or lengths, but not the secret. Diagnostics and tests assert behavior without printing values.

## Testing

- Pure lifecycle tests cover stale end, dirty external suppression, matching acknowledgement, forced key synchronization, and cleanup.
- Parameterized primitive tests cover text, search, textarea, and secure adapters.
- Form tests cover IME Enter suppression and later normal submit.
- Upload staging tests reproduce the stale snapshot rerender while composing and while locally dirty.
- Existing composer/edit tests remain green.
- Static-audit tests prove unsafe and safe JSX classification.
- Full frontend unit tests, typecheck, lint, and build are required before PR.

## Canon

The implementation updates `REPOSITORY_RULES.md`, `docs/policies/engineering-rules.md`, and `docs/architecture/overview.md`. `AGENTS.md` records only the resulting focused operational commands.
