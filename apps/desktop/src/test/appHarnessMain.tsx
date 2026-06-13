/**
 * Playwright harness entry: mounts the FULL <App /> over a recording mock
 * Tauri IPC transport, so the headless-Chromium spec
 * (e2e/basic-operations.spec.ts) can prove that the Task 5 UI drives the
 * right Tauri COMMAND NAMES (create_room, create_space,
 * set_composer_reply_target, send_reply).
 *
 * Loaded only by appHarness.html (never part of the production index.html
 * bundle). No Tauri process, no network: everything flows through
 * TauriIpcMock via Tauri's official `mockIPC` (which installs
 * window.__TAURI_INTERNALS__ so isTauriRuntime() is true and the App selects
 * TauriDesktopApi → invoke()).
 *
 * CRITICAL ORDERING: createDesktopApi() and the tauriTimelineTransport
 * constant both run at App MODULE-LOAD time and snapshot isTauriRuntime().
 * So the mock + window.__harness must be installed BEFORE the App module is
 * imported. We therefore set everything up first and dynamically import the
 * App only afterwards.
 */

import { createRoot } from "react-dom/client";
import { mockIPC, mockWindows } from "@tauri-apps/api/mocks";
import { emit } from "@tauri-apps/api/event";

import type { CoreEventPayload, TimelineItem } from "../domain/coreEvents";
import { roomTimelineKey } from "../domain/coreEvents";
import type {
  ComposerMode,
  DesktopSnapshot
} from "../domain/types";
import { TauriIpcMock, type IpcInvocation } from "./tauriIpcMock";

const CORE_EVENT_NAME = "matrix-desktop://event";

// Identity used across the ready snapshot, timeline key, and CoreEvents.
const HOMESERVER = "https://harness.example.invalid";
const USER_ID = "@harness-user:example.invalid";
const DEVICE_ID = "HARNESSDEVICE";
const SPACE_ID = "!harness-space:example.invalid";
const ROOM_ID = "!harness-room:example.invalid";
const ROOM_NAME = "Harness Room";
const SPACE_NAME = "Harness Space";
const SEED_EVENT_ID = "$seed-event:example.invalid";

// The control surface exposed on window for the Playwright spec. We do NOT
// augment the global Window interface here: src/test/harnessMain.tsx already
// declares a (narrower) window.__harness for the timeline harness, and two
// conflicting global augmentations would break `tsc`. Each harness HTML loads
// exactly one entry module, so a local interface + a single assignment cast is
// sufficient and keeps the two harnesses independent.
interface AppHarnessControl {
  invocations(): readonly IpcInvocation[];
  invocationsOf(command: string): IpcInvocation[];
  clearInvocations(): void;
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  setCommandResponse(command: string, response: any): void;
  pushCoreEvent(event: CoreEventPayload): Promise<void>;
}

// ---------------------------------------------------------------------------
// Ready snapshot factory: a signed-in shell with one space, one selected
// room, and a populated composer mode (Plain by default). The timeline ROWS
// do NOT live here — in Tauri-runtime mode the App renders TimelineView from
// the CoreEvent stream, so the reply target is seeded via pushCoreEvent.
// ---------------------------------------------------------------------------

function readySnapshot(
  overrides: {
    composerMode?: ComposerMode;
    basicOperation?: DesktopSnapshot["state"]["basic_operation"];
    extraSpaces?: DesktopSnapshot["state"]["spaces"];
    extraRailItems?: DesktopSnapshot["sidebar"]["space_rail"];
  } = {}
): DesktopSnapshot {
  const composerMode = overrides.composerMode ?? "Plain";
  const basicOperation = overrides.basicOperation ?? { kind: "idle" };
  const spaces = [
    {
      space_id: SPACE_ID,
      display_name: SPACE_NAME,
      child_room_ids: [ROOM_ID]
    },
    ...(overrides.extraSpaces ?? [])
  ];
  const railItems = [
    {
      space_id: SPACE_ID,
      display_name: SPACE_NAME,
      unread_count: 0,
      is_active: false
    },
    ...(overrides.extraRailItems ?? [])
  ];
  return {
    state: {
      session: {
        kind: "ready",
        homeserver: HOMESERVER,
        user_id: USER_ID,
        device_id: DEVICE_ID
      },
      auth: { kind: "unknown" },
      sync: "running",
      navigation: { active_space_id: null, active_room_id: ROOM_ID },
      spaces,
      rooms: [
        {
          room_id: ROOM_ID,
          display_name: ROOM_NAME,
          is_dm: false,
          unread_count: 0,
          parent_space_ids: []
        }
      ],
      timeline: {
        room_id: ROOM_ID,
        is_subscribed: true,
        is_paginating_backwards: false,
        composer: {
          pending_transaction_id: null,
          draft: "",
          mode: composerMode
        }
      },
      thread: { kind: "closed" },
      search: { kind: "closed" },
      errors: [],
      basic_operation: basicOperation
    },
    sidebar: {
      active_space_id: null,
      account_home: { display_name: "Home", unread_count: 0, is_active: true },
      space_rail: railItems,
      space_rooms: [
        { room_id: ROOM_ID, display_name: ROOM_NAME, unread_count: 0 }
      ],
      global_dms: [],
      space_unread_count: 0,
      dm_unread_count: 0
    },
    timeline: [],
    thread: null
  };
}

// A reply-mode composer snapshot (composer.mode = Reply) used as the
// set_composer_reply_target response so the App's local `sendText` dispatch
// sees reply mode and routes to send_reply.
function replyModeSnapshot(): DesktopSnapshot {
  return readySnapshot({
    composerMode: { Reply: { in_reply_to_event_id: SEED_EVENT_ID } }
  });
}

// A snapshot where a freshly-created room/space is present + active, so the
// create dialog's success path (which closes on success) is exercised.
function afterCreateRoomSnapshot(): DesktopSnapshot {
  const snapshot = readySnapshot();
  const newRoomId = "!created-room:example.invalid";
  snapshot.state.rooms.push({
    room_id: newRoomId,
    display_name: "Created Room",
    is_dm: false,
    unread_count: 0,
    parent_space_ids: []
  });
  snapshot.state.navigation.active_room_id = newRoomId;
  snapshot.state.timeline.room_id = newRoomId;
  snapshot.sidebar.space_rooms.push({
    room_id: newRoomId,
    display_name: "Created Room",
    unread_count: 0
  });
  return snapshot;
}

function afterCreateSpaceSnapshot(): DesktopSnapshot {
  const snapshot = readySnapshot();
  const newSpaceId = "!created-space:example.invalid";
  snapshot.state.spaces.push({
    space_id: newSpaceId,
    display_name: "Created Space",
    child_room_ids: []
  });
  snapshot.state.navigation.active_space_id = newSpaceId;
  snapshot.sidebar.active_space_id = newSpaceId;
  snapshot.sidebar.space_rail.push({
    space_id: newSpaceId,
    display_name: "Created Space",
    unread_count: 0,
    is_active: true
  });
  return snapshot;
}

// ---------------------------------------------------------------------------
// Mock setup (BEFORE importing App)
// ---------------------------------------------------------------------------

const mock = new TauriIpcMock();

// Snapshot-returning commands the App calls. Default snapshot stays ready so
// any unanticipated snapshot read still renders the shell.
mock.setCommandResponse("get_snapshot", readySnapshot());
mock.setCommandResponse("create_room", afterCreateRoomSnapshot());
mock.setCommandResponse("create_space", afterCreateSpaceSnapshot());
// Clicking reply records set_composer_reply_target AND returns a reply-mode
// snapshot, so the subsequent composer submit dispatches send_reply.
mock.setCommandResponse("set_composer_reply_target", replyModeSnapshot());
mock.setCommandResponse("cancel_composer_reply", readySnapshot());
// send_reply / send_text return to a Plain composer snapshot.
mock.setCommandResponse("send_reply", readySnapshot());
mock.setCommandResponse("send_text", readySnapshot());

// Route ALL Tauri IPC through the recording mock. Plugin commands
// (window setTitle/setBadge, notifications, etc.) must NOT throw: they are
// not snapshot commands, so return a harmless value. Event-plugin commands
// (plugin:event|*) are handled internally by mockIPC's shouldMockEvents.
mockIPC(
  (cmd, args) => {
    if (cmd.startsWith("plugin:")) {
      // Window/notification/other plugin calls: return a benign value.
      return null;
    }
    return mock.invoke(cmd, (args ?? {}) as Record<string, unknown>);
  },
  { shouldMockEvents: true }
);

// getCurrentWindow() (used by the title effect) reads __TAURI_INTERNALS__.metadata.
mockWindows("main");

const harnessControl: AppHarnessControl = {
  invocations: () => mock.recordedInvocations(),
  invocationsOf: (command) => mock.invocationsOf(command),
  clearInvocations: () => mock.clearInvocations(),
  setCommandResponse: (command, response) =>
    mock.setCommandResponse(command, response),
  pushCoreEvent: (event) => emit(CORE_EVENT_NAME, event)
};
(window as unknown as { __harness: AppHarnessControl }).__harness =
  harnessControl;

// ---------------------------------------------------------------------------
// Boot: dynamically import App AFTER the mock is installed, then seed the
// timeline (one real event row so a reply target exists) and render.
// ---------------------------------------------------------------------------

async function boot() {
  const { App } = await import("../App");

  const root = document.getElementById("root");
  if (!root) {
    throw new Error("appHarness root element missing");
  }
  createRoot(root).render(<App />);

  // After the App mounts and TimelineView subscribes to matrix-desktop://event,
  // push one InitialItems batch carrying a single event-backed row so the
  // "Reply to message" action renders.
  const seedItem: TimelineItem = {
    id: { Event: { event_id: SEED_EVENT_ID } },
    sender: "@other-user:example.invalid",
    body: "Seed message for reply target",
    timestamp_ms: 1_800_000_000_000,
    in_reply_to_event_id: null
  };
  const payload: CoreEventPayload = {
    kind: "Timeline",
    event: {
      InitialItems: {
        request_id: null,
        key: roomTimelineKey(USER_ID, ROOM_ID),
        generation: 1,
        items: [seedItem]
      }
    }
  };
  // Retry-emit until the App's listener is registered (listen() is async).
  for (let attempt = 0; attempt < 40; attempt += 1) {
    await emit(CORE_EVENT_NAME, payload);
    await new Promise((resolve) => setTimeout(resolve, 25));
    if (document.querySelector('[aria-label="Reply to message"]')) {
      break;
    }
  }
}

void boot();
