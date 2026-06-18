# Koushi — UI/UX Design Language (v1)

Status: design reference for implementers. Date: 2026-06-14.

This document defines the visual + interaction design language for
Koushi, the Element-like Matrix desktop client: a **blue** identity, **light + dark**
themes, and an **Element-aligned navigation/menu structure**. It is the design
source other implementation agents follow so the UI is consistent and they do
not have to re-derive look-and-feel per surface.

Mockups (open in a browser or any SVG viewer):
- `docs/design/palette.svg` — semantic tokens, light + dark.
- `docs/design/ui-shell-light.svg` — full shell, light.
- `docs/design/ui-shell-dark.svg` — full shell, dark.

## 0. Relationship to canon (binding)

This is a *design* document; it does not override the rule book.
- Headless-first, Rust-owned product state still holds (REPOSITORY_RULES,
  `docs/architecture/*`, Issue #1). This doc governs **presentation only**:
  CSS tokens, layout, component look, and ephemeral interaction affordances.
  Selection/unread/thread/composer **semantics** remain Rust-owned state, not
  re-invented in CSS/React.
- **No Element assets.** Do not copy Element CSS, icons, sounds, fonts, brand,
  or its green palette. Behaviors may be reimplemented independently; note in
  code when a behavior is inspired by Element UX.
- User-visible text goes through the i18n catalog (`src/i18n/messages.ts`), not
  literals. New labels add catalog entries.

## 1. Principles

1. **One saturated hue.** Blue (`--brand`) is the only saturated color in the
   chrome; everything else is a cool slate ramp so unread/mention/presence and
   the brand read clearly.
2. **Calm chrome, legible content.** Rail/sidebar are quiet; the timeline is the
   brightest surface. Content (messages) has the highest contrast.
3. **Token-driven theming.** Every color is a CSS custom property; light/dark
   differ only in token values, never in component markup.
4. **Element-aligned IA, original identity.** Match Element's menu *structure*
   and information architecture; do not match its colors or assets.
5. **Direction-agnostic, translation-tolerant.** Logical CSS properties, `dir`
   aware, no fixed-width labels.

## 2. Design tokens

Tokens live in `apps/desktop/src/styles.css` as CSS custom properties on
`:root` (light) and `:root[data-theme="dark"]` (dark). See `palette.svg`.

| token | role | light | dark |
| --- | --- | --- | --- |
| `--brand` | primary action / active / link | `#2D6FEF` | `#5C8DF6` |
| `--brand-hover` | hover/pressed brand | `#1F59D0` | `#79A2F8` |
| `--brand-weak` | selected-row / chip bg | `#E7F0FE` | `#1B2942` |
| `--brand-contrast` | text/icon on `--brand` | `#FFFFFF` | `#0A111F` |
| `--rail` | space rail bg | `#16213E` | `#0A111F` |
| `--rail-item` | inactive rail tile | `#27324F` | `#1B2942` |
| `--sidebar` | room-list bg | `#F5F7FB` | `#111A2B` |
| `--surface` | timeline / main bg | `#FFFFFF` | `#0E1726` |
| `--surface-hover` | row hover | `#EEF2F8` | `#1A2740` |
| `--text` | primary text | `#0F1B2D` | `#E6ECF5` |
| `--text-muted` | secondary text | `#5B6B82` | `#93A0B4` |
| `--text-faint` | tertiary / placeholder | `#93A0B4` | `#5B6B82` |
| `--line` | borders / dividers | `#E3E8F0` | `#1E2A40` |
| `--unread` | unread count badge | `#2D6FEF` | `#5C8DF6` |
| `--mention` / `--danger` | mention dot / destructive | `#E5484D` | `#F2575C` |
| `--success` | presence online / success | `#1A9E6C` | `#34B988` |
| `--warning` | warning | `#C98A1B` | `#E0A53A` |

Non-color tokens (theme-independent):
- Radii: `--r-sm 6px`, `--r-md 8px`, `--r-lg 12px`, `--r-pill 999px`.
- Spacing scale (px): 4 / 8 / 12 / 16 / 20 / 24.
- Elevation (dialogs/menus): light `0 12px 40px rgb(15 27 45 / 18%)`,
  dark `0 12px 40px rgb(0 0 0 / 50%)`.
- Type: Inter (bundled later); sizes 11/12/13.5/16/20; weights 400/650/750/800.

## 3. Theming mechanism

- Default light. Dark via `:root[data-theme="dark"]` token overrides.
- Honor system: when no explicit user choice, follow
  `@media (prefers-color-scheme: dark)` by mirroring the dark token set.
- Theme choice is **Rust-owned settings state** (`system | light | dark`)
  surfaced in the snapshot; React applies it by setting `data-theme` on the
  root and `color-scheme`. (Settings persistence is Issue #1 Track 4.)
- `color-scheme: light dark` so native form controls/scrollbars match.

## 4. Layout system

Keep the existing CSS-grid shell (`.app-grid`), four columns:

```
[ rail 72px ] [ room list 318px ] [ timeline minmax(420px,1fr) ] [ right panel 390px ]
```
- Right panel collapses to `0` when closed (`.app-grid.thread-closed`),
  driven by `snapshot.state.thread` / right-panel mode (Rust-owned).
- Min app width target ~1024px; below that the right panel overlays instead of
  splitting (future responsive pass — not v1-blocking).
- Vertical rhythm: 62px header band shared by sidebar/timeline/right panel.

## 5. Navigation & menu structure (Element-aligned)

Mirror Element's left-panel structure; map onto existing components.

**Space rail** (`WorkspaceRail`, col 1):
1. Home (all rooms) — active state = filled `--brand` tile.
2. Space tiles — rounded squares, unread→count badge, mention→red dot.
3. `+` create space (dashed tile).
4. Bottom: account avatar (presence ring) + Settings.

**Room list** (`Sidebar`, col 2), top→bottom:
1. Header: workspace/space name + account avatar (→ account menu/settings).
2. Search field (filter people & rooms).
3. Sections, in this order, each with a section header + count + `+`:
   **Favourites → People → Rooms → Low priority (collapsed)**. DMs live under
   People. (This replaces the current `Rooms` / `People` only split.)
4. Row affordances: unread = `--unread` count badge; mention = `--mention`
   dot; selected = `--brand-weak` bg + 3px `--brand` left bar; hover =
   `--surface-hover` + trailing 3-dot menu (leave / low priority / settings).
5. Footer: Explore public rooms (compass) + Settings.

**Timeline** (col 3): room header (name, topic, members/search/right-panel
toggle) · message list · composer.

**Right panel** (col 4, collapsible): hosts Room info / Members / Thread /
Search results / Settings as modes (Rust-owned `thread` + right-panel mode).

## 6. Components & states

- **Buttons**: `primary` (filled `--brand`, `--brand-contrast` text, hover
  `--brand-hover`), `secondary` (1px `--line`, `--surface`), `ghost`/icon
  (transparent, hover `--surface-hover`), `danger` (`--danger`). 32–36px height,
  `--r-md`. Focus: 2px `--brand` outline offset 2px.
- **Inputs / search**: `--surface`, 1px `--line`, focus border `--brand`;
  placeholder `--text-faint`; logical padding.
- **Room-list item**: avatar (28) · name (`650`) + optional preview/time
  (`--text-muted`) · trailing badge/dot. States: default / hover / selected /
  muted (low-priority = `--text-muted`, no count, mention still shows).
- **Message row**: avatar (36) · sender (`800`) + time (`--text-muted`) · body
  (`--text`); hover reveals action bar (reply/react/edit/redact/more) and
  `--surface-hover`; reactions = pill chips (`--brand-weak`, count); edited tag
  muted; pending/failed send states use `--text-muted` / `--danger`.
- **Composer**: card (`--surface`, 1px `--line`, `--r-lg`); formatting toolbar
  (B/I/link/list/code); textarea (placeholder = `Message {roomName}` via i18n);
  attach/mention/emoji; primary Send. Reply banner above input shows target +
  Cancel (reply mode is Rust-owned; see state-machine.md).
- **Dialogs** (create room/space): centered card, elevation token, overlay
  `rgb(15 27 45 / 45%)`; primary/secondary footer buttons.
- **Menus / context menus**: `--surface`, `--line`, elevation; item hover
  `--surface-hover`; destructive item `--danger`.
- **Badges**: unread = pill, `--unread`, `--brand-contrast` text; mention = 6px
  `--mention` dot; rail count badge bordered by `--rail`.
- **Tabs / segmented** (right panel Info/Members): active = `--brand-weak` bg +
  `--brand` text.

## 7. Iconography, fonts, assets

- Icons: `lucide-react` (already used). No Element icons. Stroke 1.75, 18–20px,
  `currentColor`.
- Font: Inter (bundle locally in a later font/emoji track — Issue #1 Track 3);
  system fallback today. Tabular numerals for counts/timestamps.
- App identity is blue + the `#`/home glyph; no Element logo/brand.

## 8. Accessibility

- WCAG AA contrast for text on `--surface`/`--sidebar` (tokens chosen for this).
- Preserve landmark roles & keyboard focus order already covered by
  `e2e/desktop-shell-a11y.spec.ts`; every interactive element keyboard-reachable
  with a visible `--brand` focus ring.
- Color is never the sole signal: unread has a count, mention has a dot +
  (later) aria, presence has a label.

## 9. i18n / RTL

- Logical CSS (`margin-inline`, `padding-inline`, `inset-inline`, `border-start`)
  — no left/right physical assumptions.
- `lang`/`dir` from the locale profile at the root; `dir="auto"` on
  user/remote text (message bodies, room/display names, reaction keys).
- Labels tolerate ~1.5× expansion; no fixed-width text buttons.

## 10. Implementation map (where this lands)

- Tokens + theme: `apps/desktop/src/styles.css` (`:root` + `[data-theme="dark"]`).
  Replace the current purple ramp (`--rail #34123d`, `--brand #5b236a`, green
  accent) with §2.
- Rail: `WorkspaceRail` (in `App.tsx`); room list: `Sidebar` sections →
  Favourites/People/Rooms/Low-priority; right panel: `ContextualRightPanel` +
  panels in `src/components/`.
- All labels via `src/i18n/messages.ts` (extend catalog for new section names).
- Theme setting: add to Rust settings state + snapshot (Issue #1 Track 4);
  React sets `data-theme`/`color-scheme`.

## 11. Phasing (suggested, not blocking)

1. Token swap + dark mode (`styles.css` + theme application) — highest value,
   lowest risk; everything else inherits it.
2. Room-list sections to Favourites/People/Rooms/Low-priority.
3. Component state polish (message hover bar, reactions, badges).
4. Right-panel modes unification; font/emoji bundling (Track 3).

## Out of scope (v1)

Reactions backend, notifications visual (see Issue #2), responsive/mobile,
density toggle, custom theme editor. These reuse these tokens when built.
