# Sidebar Composition Spike

Status: in progress

Goal: prove a desktop-specific sidebar DTO layer over SDK-like room, Space, and DM metadata.

Acceptance:
- `cargo test -p sidebar-composition`
- output model contains Space rail entries, Space-filtered rooms, global DMs, and separated unread counts.
