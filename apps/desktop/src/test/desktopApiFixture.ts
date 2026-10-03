import { TauriDesktopApi } from "../backend/client";
import type { DesktopApi } from "../backend/desktopApi";
import type { DesktopSnapshot } from "../domain/types";
import { TauriIpcMock, defaultSnapshotResponse } from "./tauriIpcMock";

export interface DesktopApiFixture extends DesktopApi {
  readonly ipc: TauriIpcMock;
}

export function readyDesktopSnapshotFixture(): DesktopSnapshot {
  const snapshot = structuredClone(defaultSnapshotResponse()) as unknown as DesktopSnapshot;
  snapshot.state.domain.session = {
    kind: "ready",
    homeserver: "https://example.invalid",
    user_id: "@user:example.invalid",
    device_id: "DEVICE"
  };
  snapshot.state.domain.secure_backup_gate = { kind: "ready" };
  snapshot.state.domain.sync = "running";
  snapshot.state.domain.space_members = {
    selected_space_id: "!space-alpha:example.invalid",
    generation: 1,
    power_levels_revision: "revision-1",
    can_edit_roles: true,
    space_joined: [{
      user_id: "@joined:example.invalid",
      display_name: "Joined Member",
      display_label: "Joined Member",
      original_display_label: "Joined Member",
      avatar_url: null,
      power_level: 0,
      role: "user",
      membership: "space_joined",
      child_room_ids: [],
      invite_pending: false,
      role_options: [
        { power_level: 50, role: "moderator", requires_confirmation: false },
        { power_level: 100, role: "administrator", requires_confirmation: true }
      ]
    }],
    space_invited: [{
      user_id: "@invited:example.invalid",
      display_name: "Invited Member",
      display_label: "Invited Member",
      original_display_label: "Invited Member",
      avatar_url: null,
      power_level: 0,
      role: "user",
      membership: "space_invited",
      child_room_ids: [],
      invite_pending: false,
      role_options: []
    }],
    child_room_only: [{
      user_id: "@child-only:example.invalid",
      display_name: "Child-only Member",
      display_label: "Child-only Member",
      original_display_label: "Child-only Member",
      avatar_url: null,
      power_level: 0,
      role: "user",
      membership: "child_room_only",
      child_room_ids: ["!room-alpha:example.invalid"],
      invite_pending: false,
      role_options: []
    }],
    child_room_count: 1,
    complete_child_room_count: 1,
    incomplete_child_room_count: 0,
    operation: { kind: "idle" }
  };
  snapshot.state.domain.spaces = [{
    space_id: "!space-alpha:example.invalid",
    raw_name: null,
    display_name: "Synthetic Workspace",
    avatar: null,
    join_rule: null,
    child_room_ids: ["!room-alpha:example.invalid"],
    parent_side_child_room_ids: ["!room-alpha:example.invalid"]
  }];
  snapshot.state.domain.rooms = [
    {
      room_id: "!room-alpha:example.invalid",
      display_name: "synthetic-room",
      display_label: "synthetic-room",
      original_display_label: "synthetic-room",
      avatar: null,
      is_dm: false,
      dm_user_ids: [],
      tags: { favourite: null, low_priority: null },
      unread_count: 0,
      highlight_count: 0,
      parent_space_ids: ["!space-alpha:example.invalid"],
      dm_space_ids: [],
      is_encrypted: false
    },
    {
      room_id: "!dm-hiroshi:example.invalid",
      display_name: "Hiroshi",
      display_label: "Hiroshi",
      original_display_label: "Hiroshi",
      avatar: null,
      is_dm: true,
      dm_user_ids: ["@hiroshi.shinaoka:matrix.org"],
      tags: { favourite: null, low_priority: null },
      unread_count: 0,
      highlight_count: 0,
      parent_space_ids: [],
      dm_space_ids: [],
      is_encrypted: true
    }
  ];
  snapshot.state.ui.navigation.active_space_id = "!space-alpha:example.invalid";
  snapshot.state.ui.navigation.active_room_id = "!room-alpha:example.invalid";
  snapshot.state.ui.navigation.space_order = ["!space-alpha:example.invalid"];
  snapshot.state.ui.navigation.last_room_by_space_id = {
    "!space-alpha:example.invalid": "!room-alpha:example.invalid"
  };
  snapshot.state.ui.room_list = {
    readiness: { kind: "ready", source: "cache", generation: 0 },
    active_filter: { kind: "rooms" },
    sort: { kind: "activity" },
    items: [{ room_id: "!room-alpha:example.invalid", kind: "room" }]
  };
  snapshot.state.ui.timeline.room_id = "!room-alpha:example.invalid";
  snapshot.state.ui.timeline.is_subscribed = true;
  const roomItem = {
    room_id: "!room-alpha:example.invalid",
    display_name: "synthetic-room",
    avatar: null,
    tags: { favourite: null, low_priority: null },
    unread_count: 0,
    highlight_count: 0,
    notification_count: 0,
    display_count: 0,
    has_unread_content: false,
    is_attention_highlighted: false,
    has_unread_mention: false,
    is_muted: false
  };
  const dmItem = { ...roomItem, room_id: "!dm-hiroshi:example.invalid", display_name: "Hiroshi" };
  snapshot.timeline = [{
    room_id: "!room-alpha:example.invalid",
    event_id: "$fixture:example.invalid",
    sender: "@member:example.invalid",
    timestamp_ms: 1,
    body: "Fixture message",
    attachment_filename: null,
    reply_count: 0
  }];
  snapshot.sidebar = {
    ...snapshot.sidebar,
    active_space_id: "!space-alpha:example.invalid",
    account_home: { ...snapshot.sidebar.account_home, is_active: false },
    space_rail: [{
      space_id: "!space-alpha:example.invalid",
      display_name: "Synthetic Workspace",
      local_icon: null,
      avatar: null,
      unread_count: 0,
      highlight_count: 0,
      is_active: true
    }],
    space_rooms: [roomItem],
    global_dms: [dmItem],
    sections: {
      favourites: [],
      rooms: [roomItem],
      people: [dmItem],
      low_priority: [],
      not_joined: []
    }
  };
  return snapshot;
}

export function awaitingVerificationSnapshotFixture(): DesktopSnapshot {
  const snapshot = readyDesktopSnapshotFixture();
  snapshot.state.domain.session = {
    kind: "awaitingVerification",
    homeserver: "https://example.invalid",
    user_id: "@user:example.invalid",
    device_id: "DEVICE",
    gate: {
      methods: ["existingDeviceSas", "recoveryKey"],
      account_kind: "existingIdentity"
    }
  };
  snapshot.state.domain.secure_backup_gate = { kind: "inactive" };
  return snapshot;
}

export function createDesktopApiFixture(
  snapshot: unknown = readyDesktopSnapshotFixture()
): DesktopApiFixture {
  const ipc = new TauriIpcMock();
  for (const command of ["get_snapshot", "settlement_snapshot", "resync_snapshot"]) {
    ipc.setCommandResponse(command, snapshot);
  }
  const api = new TauriDesktopApi(ipc.invoke.bind(ipc)) as unknown as DesktopApiFixture;
  Object.defineProperty(api, "ipc", { value: ipc });
  return api;
}
