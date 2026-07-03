# Design: 2026-07-03 issue batch (#173-#179)

Status: approved
Date: 2026-07-03
Issues:
- https://github.com/shinaoka/koushi-matrix/issues/173
- https://github.com/shinaoka/koushi-matrix/issues/174
- https://github.com/shinaoka/koushi-matrix/issues/175
- https://github.com/shinaoka/koushi-matrix/issues/176
- https://github.com/shinaoka/koushi-matrix/issues/177
- https://github.com/shinaoka/koushi-matrix/issues/178
- https://github.com/shinaoka/koushi-matrix/issues/179

## Goal

Close all 2026-07-03 product bug/feature issues in one branch and one PR while
removing the search/UI state debt that caused issue #179.

## Batch Design

### Batch 1: small confirmed UX fixes (#173, #174, #175)

- Media save dialogs should default to the operating system Downloads
  directory. Rust owns the platform path lookup and still validates the final
  selected destination before copying.
- One-to-one DM room context menus get a User info action. The action opens the
  existing profile panel for the Rust-projected DM target user; React may route
  the right-panel intent but must not derive profile data.
- The profile panel hides Unban for active room members. Until banned-member
  profiles exist as a Rust-owned model, Unban is not a valid action for the
  active-member view.

### Batch 2: room info and create-room contracts (#176, #177)

- Room info displays Element-style status badges from Rust-owned room and room
  settings data: encryption, public/private status, and history visibility.
- Room info exposes a Rust-projected `matrix.to` share link, preferring a
  canonical alias, then an alternate alias, then a room-id fallback. React only
  renders and copies the provided link.
- Create room uses typed options: private, public, standard room in active
  space, topic, local alias, and encryption where allowed. Public rooms use
  Matrix public room creation settings; standard space rooms carry a parent
  relation and restricted join rule at creation time, with the existing
  `set_space_child` follow-up retained as reconciliation.

### Batch 3: explicit sidebar category and sort controls (#178)

- The DM/Rooms choice becomes an explicit segmented control backed by
  `RoomListFilter::People` and `RoomListFilter::Rooms`.
- The list sort control dispatches the existing Rust-owned
  `SettingsValues.room_list_sort` setting.
- The visible list is rendered from the existing Rust-projected
  `RoomListProjection`; React does not locally duplicate category semantics.

### Batch 4: search source of truth and navigation (#179)

- The search panel renders from Rust-owned `SearchState` and distinguishes
  editing, searching, results, failed, and closed. It must not show a synthetic
  zero-result state while Rust is still searching.
- Closing search is a typed Rust-owned lifecycle transition. It clears active
  search query, highlights, and result state without promoting results into the
  main timeline.
- Search scopes for current space and DMs are room-set scopes, not silent
  global fallbacks.
- Search result clicks select the room and open the main timeline around the
  matched event, while keeping the search results panel available.
- Timeline-visible initial items and live diffs are delivered into the search
  index reliably. Silent `try_send` loss is not allowed for search indexing.

## Ownership

Product state stays Rust-owned. React may own transient dialog drafts, hover
state, menu visibility, and input focus, but it must not invent Matrix room,
member, create-room, search, or navigation semantics.

Search snippets, queries, links, room ids, event ids, and raw SDK errors remain
private-data-sensitive in logs and QA evidence. Tests should use synthetic data
and token-only headless output.

## Verification Strategy

- Add failing focused tests for each issue before implementation.
- Prefer Rust unit or Tauri serialization tests for command/state contracts.
- Use browser-headless/Vitest tests for rendered controls and right-panel
  routing.
- Run focused gates after each batch, then full affected Rust/TypeScript gates
  before PR.

