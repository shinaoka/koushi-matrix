# Sidebar Composition Spike

Status: in progress

Goal: prove a desktop-specific sidebar DTO layer over SDK-like room, Space, and DM metadata.

Acceptance:
- `cargo test -p sidebar-composition`
- output model contains Space rail entries, Space-filtered rooms, global DMs, and separated unread counts.

## Result

- The desktop sidebar is a composition layer, not a direct SDK model.
- DMs are global even if a DM room appears under a Space.
- Space unread counts exclude DMs; DM unread counts are global.
- The full implementation must replace spike inputs with DTOs derived from `RoomListService`, `SpaceService`, and room metadata.
