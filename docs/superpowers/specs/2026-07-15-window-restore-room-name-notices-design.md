# Window Restore And Room-Name Notice Design

**Issues:** #246, #256

## Scope

This PR contains two independent, bounded desktop correctness fixes. It does
not change Matrix room-name state, window-state persistence format, or the
generic fallback for unknown custom events.

## #246: Safe Window Restore

Window geometry remains persisted as physical coordinates. Before applying a
saved state, the desktop adapter obtains every active monitor work area in the
same physical coordinate space and passes those rectangles to a pure geometry
resolver.

The resolver selects the work area with the largest intersection with the
saved rectangle. If none intersects, it uses the primary monitor, falling back
to the first active monitor. It preserves valid in-bounds geometry exactly. It
clamps oversized dimensions to the selected work area and clamps the position
so the entire base rectangle, including the title bar, is usable. If no active
work area can contain the configured minimum window size, the persisted state
is not applied and the configured default window remains in use.

Maximization is applied only after safe base size and position have succeeded.
No state file is deleted. The pure resolver is covered for valid, oversized,
off-screen, disconnected-monitor, negative-origin, and unusably small work
areas.

## #256: Typed Localized Room-Name Notices

Core pattern-matches
`TimelineItemContent::OtherState(AnyOtherStateEventContentChange::RoomName)`
before the generic state-event fallback. A dedicated projection maps typed SDK
content to one of four notice variants:

- `set`: non-empty current name with no non-empty previous name;
- `changed`: distinct non-empty previous and current names;
- `removed`: empty current name;
- `generic`: redacted/unavailable content.

The notice crosses the WebView boundary as a closed Rust-owned localization
key plus optional `old_name` and `new_name` values. The English fallback body
is also projected for non-localized consumers, but it remains non-user content
and `TimelineMessageKind::Notice`. Names are plain strings and never enter the
formatted-HTML renderer.

React selects only the corresponding checked-in English/Japanese catalog
entry and supplies the Rust-projected values to the existing interpolation
function. Existing sender, event identity, timestamp, pagination, focused
view, and thread-root projection paths continue to use the same
`TimelineItem`.

## Failure And Compatibility Rules

- Unknown/custom state events retain the existing generic fallback.
- A redacted room-name event never exposes raw JSON or an event body.
- Empty and whitespace-only names are treated as removal for display.
- Identical previous/current names use the safe `set` wording rather than a
  misleading change.
- Existing timeline fixtures without structured notice values deserialize via
  defaults.

## Verification

- Tauri unit tests drive the pure window geometry resolver and existing
  persistence tests.
- Core unit tests construct typed `StateEventContentChange` values and prove
  all four room-name projections plus the generic fallback.
- TypeScript catalog tests prove both locales interpolate plain CJK, emoji,
  RTL, and HTML-like strings without treating them as markup.
- Timeline component tests prove the typed notice never displays
  `Unsupported event: m.room.name`.
- Focused Rust, TypeScript, typecheck, formatting, and adapter-boundary gates
  run before the PR is opened.
