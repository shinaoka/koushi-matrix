# #2005: the composer sends `> ` lines as a block quote

## The bug

`format_markdown_subset` in `crates/koushi-state/src/composer_shortcuts.rs` had
rules for inline marks, unordered lists and math blocks, but none for block
quotes. A body of `> quoted line\nreply` therefore produced no formatted body at
all — `build_formatted_message_draft` returned `formatted_body: None` and the
message went out as plain text, so every other client showed the literal `> `
markers. There was also no Quote control in the composer toolbar.

## The rule

A line whose first character is `>` opens a block chunk. Consecutive such lines
form one `<blockquote>`, each line stripped of one `>` and one optional following
space, rendered through the same inline pass as ordinary text and joined with
`<br>` — the separator this formatter already uses for authored line breaks
inside a message. A line that is bare `>` becomes an empty quoted line.

`plain_body` is untouched: it keeps the `> ` text, which is the plain-text
fallback the spec asks for (`m.room.message` `body` beside `formatted_body`).

Interoperability: `blockquote` and `br` are both in the HTML subset the client-server
spec permits in `formatted_body`, and every mainstream client renders a
`<blockquote>` as a quote. Element's own markdown renderer emits
`<blockquote><p>…</p></blockquote>`; this formatter uses `<br>` instead of `<p>`
because no other block in its output wraps its lines in paragraphs, and the
timeline renderer accepts both shapes (`TimelineMessageBody.tsx`'s `blockquote`
node, pinned by `crates/koushi-core/src/timeline/item_projection/tests.rs`).

Deliberate limits, recorded at the recogniser: one `>` level is stripped and only
the inline pass runs inside it, so `> > nested` keeps its inner marker and
`> - item` keeps its list marker as literal text. Both render as text, never as
broken HTML; recursive blocks are the upgrade when a client is seen sending quotes
whose nested structure must round-trip. Leading spaces before `>` are tolerated, so
a pasted `<space><space>> quoted` line is a quote rather than literal markers, as
with an indented `- item`.

## The control

`apps/desktop/src/components/composer.tsx` gains the Quote button: it prefixes
every selected line with `> `, skips a line that already has it, and inserts `> `
at a bare caret. Both block marks share one `applyBlockPrefix` helper, which takes
the mark's own "already marked" test so a line the formatter already reads as
marked is left alone: `- ` or `* ` with any indentation for List (the formatter
accepts both), a bare `>` for Quote. Both tests trim ASCII spaces only, matching
`trim_start_matches(' ')` in the recognisers, so a line indented with a tab or an
ideographic space still gains the marker instead of being wrongly treated as
already marked. The toolbar button sits beside List and Code, shares their
`disabled={!canEdit}` lock, and the list of locked
controls in `composer.test.tsx` now includes it, so the #1941 read-only lock covers
it without a new code path. Like the toolbar and like List, the prefix is
additive rather than a toggle: a selected line that already carries `>` (with or
without the space) is left alone, while pressing again at a caret inside a quoted
line adds a second marker — the formatter then shows the inner one as text.

The contributor asked for Element's keyboard binding, so Quote is also wired to
**Cmd+Shift+>** (**Ctrl** elsewhere) inside the same guard the Bold/Italic
shortcuts use: the platform modifier, no Alt, no Shift for the inline marks, and
no IME composition. The registry row (`formatQuote`) carries the label the
Keyboard settings page renders, in all four locales.

## Verification

```bash
cargo fmt --check                                          # clean
cargo test -p koushi-state --profile ci composer           # 2 + 113 passed
cargo test -p koushi-state --profile ci                    # 133 + 1247 + 3 passed
cargo test -p koushi-core --profile ci                     # 2026 passed, 11 ignored
cargo clippy --profile ci -p koushi-state --all-targets --locked -- -D warnings
cd apps/desktop && npm run typecheck && npm run lint && npm test -- --run
                                                           # 208 files, 2601 tests passed
npx playwright test e2e/composer-format-shortcuts.spec.ts   # 2 passed (chromium)
```

RED, observed before the fix by stashing the formatter change and running the new
tests: all four Rust cases returned `formatted_body: None` (or a literal `&gt; `
line) — `left: None` against `right: Some("<blockquote>quoted line</blockquote>\nreply")`.
On the desktop side, disabling the chord in the keydown guard fails the two
shortcut tests (`prefixes every selected line with a quote marker from the
keyboard`, `inserts the quote marker at a bare caret…`) and the new headless spec
(`Cmd/Ctrl+Shift+> quotes the composer like the toolbar`), which is the one that
proves a real engine delivers `event.key === ">"` for Cmd/Ctrl+Shift+Period.

Not covered: the Linux GUI QA scenario `local-composer` still exercises only Bold
(`gui_local_markdown=ok`); adding a Quote step there needs a real desktop run.

## Canon consulted

`docs/policies/engineering-rules.md` "Logging and Diagnostics"; the received-side
`formatted_body` contract in `docs/architecture/state-machine.md`, which this does
not touch because the sent side is generated rather than projected; and the QA and
user-help pages updated in the same change.
