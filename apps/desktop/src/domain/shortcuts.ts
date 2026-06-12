export type ShortcutCategory =
  | "Composer"
  | "Room"
  | "Room List"
  | "Navigation"
  | "Autocomplete"
  | "Accessibility";

export type ShortcutParity = "same" | "adapted" | "deferred" | "notApplicable";
export type ShortcutPlatform = "all" | "macos" | "windows" | "linux";
export type NativeMenuArea = "app" | "edit" | "view" | "window" | "help";

export interface KeyboardShortcut {
  id: string;
  category: ShortcutCategory;
  label: string;
  keys: string[];
  parity: ShortcutParity;
  implemented: boolean;
  platforms?: ShortcutPlatform[];
  nativeMenu?: NativeMenuArea;
  accelerator?: string;
  note?: string;
}

export interface KeyboardShortcutGroup {
  category: ShortcutCategory;
  shortcuts: KeyboardShortcut[];
}

export interface NativeMenuAccelerator {
  id: string;
  accelerator: string;
  nativeMenu: NativeMenuArea;
}

export interface ShortcutConflictAudit {
  adaptedWithoutReason: string[];
  duplicateAccelerators: string[];
  duplicateGlobalHandlers: string[];
  nativeMenuWithoutAccelerator: string[];
  acceleratorWithoutNativeMenu: string[];
}

export interface KeyboardEventLike {
  key: string;
  ctrlKey: boolean;
  metaKey: boolean;
  shiftKey: boolean;
  altKey: boolean;
}

const shortcuts: KeyboardShortcut[] = [
  {
    id: "sendMessage",
    category: "Composer",
    label: "Send message",
    keys: ["Enter"],
    parity: "same",
    implemented: true
  },
  {
    id: "newLine",
    category: "Composer",
    label: "New line",
    keys: ["Shift", "Enter"],
    parity: "same",
    implemented: true
  },
  {
    id: "formatBold",
    category: "Composer",
    label: "Bold",
    keys: ["Ctrl/Cmd", "B"],
    parity: "same",
    implemented: false
  },
  {
    id: "formatItalics",
    category: "Composer",
    label: "Italics",
    keys: ["Ctrl/Cmd", "I"],
    parity: "same",
    implemented: false
  },
  {
    id: "formatLink",
    category: "Composer",
    label: "Insert link",
    keys: ["Ctrl/Cmd", "Shift", "L"],
    parity: "same",
    implemented: false
  },
  {
    id: "formatCode",
    category: "Composer",
    label: "Code",
    keys: ["Ctrl/Cmd", "E"],
    parity: "same",
    implemented: false
  },
  {
    id: "cancelReplyOrEdit",
    category: "Composer",
    label: "Cancel reply or edit",
    keys: ["Esc"],
    parity: "same",
    implemented: true
  },
  {
    id: "uploadFile",
    category: "Room",
    label: "Upload file",
    keys: ["Ctrl/Cmd", "Shift", "U"],
    parity: "deferred",
    implemented: false,
    note: "Upload UI is not implemented yet."
  },
  {
    id: "searchInRoom",
    category: "Room",
    label: "Search in room",
    keys: ["Ctrl/Cmd", "F"],
    parity: "same",
    implemented: true
  },
  {
    id: "jumpToOldestUnread",
    category: "Room",
    label: "Jump to oldest unread",
    keys: ["Shift", "PageUp"],
    parity: "same",
    implemented: false
  },
  {
    id: "scrollTimelineUp",
    category: "Room",
    label: "Scroll timeline up",
    keys: ["PageUp"],
    parity: "same",
    implemented: false
  },
  {
    id: "scrollTimelineDown",
    category: "Room",
    label: "Scroll timeline down",
    keys: ["PageDown"],
    parity: "same",
    implemented: false
  },
  {
    id: "jumpToFirstMessage",
    category: "Room",
    label: "Jump to first message",
    keys: ["Ctrl", "Home"],
    parity: "same",
    implemented: false
  },
  {
    id: "jumpToLatestMessage",
    category: "Room",
    label: "Jump to latest message",
    keys: ["Ctrl", "End"],
    parity: "same",
    implemented: false
  },
  {
    id: "selectRoomInRoomList",
    category: "Room List",
    label: "Select room",
    keys: ["Enter"],
    parity: "same",
    implemented: true
  },
  {
    id: "previousRoomInList",
    category: "Room List",
    label: "Previous room in list",
    keys: ["ArrowUp"],
    parity: "same",
    implemented: true
  },
  {
    id: "nextRoomInList",
    category: "Room List",
    label: "Next room in list",
    keys: ["ArrowDown"],
    parity: "same",
    implemented: true
  },
  {
    id: "collapseRoomListSection",
    category: "Room List",
    label: "Collapse section",
    keys: ["ArrowLeft"],
    parity: "same",
    implemented: false
  },
  {
    id: "expandRoomListSection",
    category: "Room List",
    label: "Expand section",
    keys: ["ArrowRight"],
    parity: "same",
    implemented: false
  },
  {
    id: "filterRooms",
    category: "Navigation",
    label: "Find rooms",
    keys: ["Ctrl/Cmd", "K"],
    parity: "same",
    implemented: true
  },
  {
    id: "toggleRightPanel",
    category: "Navigation",
    label: "Toggle right panel",
    keys: ["Ctrl/Cmd", "."],
    parity: "same",
    implemented: true,
    nativeMenu: "view",
    accelerator: "CmdOrCtrl+."
  },
  {
    id: "toggleSpacePanel",
    category: "Navigation",
    label: "Toggle space panel",
    keys: ["Ctrl/Cmd", "Shift", "D"],
    parity: "same",
    implemented: false
  },
  {
    id: "showKeyboardSettings",
    category: "Navigation",
    label: "Keyboard settings",
    keys: ["Ctrl/Cmd", "/"],
    parity: "same",
    implemented: true,
    nativeMenu: "help",
    accelerator: "CmdOrCtrl+/"
  },
  {
    id: "openUserSettings",
    category: "Navigation",
    label: "User settings",
    keys: ["Cmd", ","],
    platforms: ["macos"],
    parity: "same",
    implemented: true,
    nativeMenu: "app",
    accelerator: "CmdOrCtrl+,"
  },
  {
    id: "goHome",
    category: "Navigation",
    label: "Go home",
    keys: ["Ctrl", "Alt", "H"],
    parity: "adapted",
    implemented: true,
    note: "macOS uses Ctrl+Shift+H in Element; this prototype keeps one cross-platform row."
  },
  {
    id: "selectPreviousRoom",
    category: "Navigation",
    label: "Previous room",
    keys: ["Alt", "ArrowUp"],
    parity: "same",
    implemented: false
  },
  {
    id: "selectNextRoom",
    category: "Navigation",
    label: "Next room",
    keys: ["Alt", "ArrowDown"],
    parity: "same",
    implemented: false
  },
  {
    id: "selectPreviousUnreadRoom",
    category: "Navigation",
    label: "Previous unread room",
    keys: ["Alt", "Shift", "ArrowUp"],
    parity: "same",
    implemented: false
  },
  {
    id: "selectNextUnreadRoom",
    category: "Navigation",
    label: "Next unread room",
    keys: ["Alt", "Shift", "ArrowDown"],
    parity: "same",
    implemented: false
  },
  {
    id: "previousVisitedRoomOrSpace",
    category: "Navigation",
    label: "Back",
    keys: ["Cmd", "["],
    platforms: ["macos"],
    parity: "same",
    implemented: false,
    nativeMenu: "view",
    accelerator: "Cmd+["
  },
  {
    id: "nextVisitedRoomOrSpace",
    category: "Navigation",
    label: "Forward",
    keys: ["Cmd", "]"],
    platforms: ["macos"],
    parity: "same",
    implemented: false,
    nativeMenu: "view",
    accelerator: "Cmd+]"
  },
  {
    id: "cancelAutocomplete",
    category: "Autocomplete",
    label: "Cancel autocomplete",
    keys: ["Esc"],
    parity: "same",
    implemented: false
  },
  {
    id: "nextAutocompleteSelection",
    category: "Autocomplete",
    label: "Next autocomplete option",
    keys: ["ArrowDown"],
    parity: "same",
    implemented: false
  },
  {
    id: "previousAutocompleteSelection",
    category: "Autocomplete",
    label: "Previous autocomplete option",
    keys: ["ArrowUp"],
    parity: "same",
    implemented: false
  },
  {
    id: "closeDialogOrMenu",
    category: "Accessibility",
    label: "Close dialog or menu",
    keys: ["Esc"],
    parity: "same",
    implemented: true
  },
  {
    id: "activateButton",
    category: "Accessibility",
    label: "Activate focused control",
    keys: ["Enter"],
    parity: "same",
    implemented: true
  },
  {
    id: "nextLandmark",
    category: "Accessibility",
    label: "Next landmark",
    keys: ["F6"],
    parity: "same",
    implemented: false
  },
  {
    id: "previousLandmark",
    category: "Accessibility",
    label: "Previous landmark",
    keys: ["Shift", "F6"],
    parity: "same",
    implemented: false
  },
  {
    id: "toggleMicrophone",
    category: "Accessibility",
    label: "Toggle microphone in call",
    keys: ["Ctrl/Cmd", "D"],
    parity: "deferred",
    implemented: false,
    note: "Calls are out of scope for this milestone."
  }
];

const categoryOrder: ShortcutCategory[] = [
  "Composer",
  "Room",
  "Room List",
  "Navigation",
  "Autocomplete",
  "Accessibility"
];

const globalKeyboardHandlerIds = [
  "showKeyboardSettings",
  "openUserSettings",
  "searchInRoom",
  "filterRooms",
  "toggleRightPanel"
];

export const keyboardShortcutGroups: KeyboardShortcutGroup[] = categoryOrder.map(
  (category) => ({
    category,
    shortcuts: shortcuts.filter((shortcut) => shortcut.category === category)
  })
);

export function shortcutById(id: string): KeyboardShortcut | undefined {
  return shortcuts.find((shortcut) => shortcut.id === id);
}

export function elementShortcutParity(): KeyboardShortcut[] {
  return [...shortcuts];
}

export function menuAccelerators(): NativeMenuAccelerator[] {
  return shortcuts
    .filter(
      (shortcut): shortcut is KeyboardShortcut & {
        accelerator: string;
        nativeMenu: NativeMenuArea;
      } => Boolean(shortcut.accelerator && shortcut.nativeMenu)
    )
    .map((shortcut) => ({
      id: shortcut.id,
      accelerator: shortcut.accelerator,
      nativeMenu: shortcut.nativeMenu
    }));
}

export function shortcutConflictAudit(): ShortcutConflictAudit {
  return {
    adaptedWithoutReason: shortcuts
      .filter((shortcut) => shortcut.parity === "adapted" && !shortcut.note?.trim())
      .map((shortcut) => shortcut.id),
    duplicateAccelerators: duplicates(
      menuAccelerators().map((shortcut) => shortcut.accelerator)
    ),
    duplicateGlobalHandlers: duplicates(
      globalKeyboardHandlerIds
        .map((id) => shortcutById(id))
        .filter((shortcut): shortcut is KeyboardShortcut => Boolean(shortcut))
        .map((shortcut) => shortcutSignature(shortcut))
    ),
    nativeMenuWithoutAccelerator: shortcuts
      .filter((shortcut) => Boolean(shortcut.nativeMenu) && !shortcut.accelerator)
      .map((shortcut) => shortcut.id),
    acceleratorWithoutNativeMenu: shortcuts
      .filter((shortcut) => Boolean(shortcut.accelerator) && !shortcut.nativeMenu)
      .map((shortcut) => shortcut.id)
  };
}

export function shortcutIdForKeyboardEvent(event: KeyboardEventLike): string | null {
  const key = normalizedKey(event.key);
  const ctrlOrCmd = event.ctrlKey || event.metaKey;

  if (ctrlOrCmd && !event.altKey && !event.shiftKey && key === "/") {
    return "showKeyboardSettings";
  }
  if (event.metaKey && !event.ctrlKey && !event.altKey && !event.shiftKey && key === ",") {
    return "openUserSettings";
  }
  if (ctrlOrCmd && !event.altKey && !event.shiftKey && key === "f") {
    return "searchInRoom";
  }
  if (ctrlOrCmd && !event.altKey && !event.shiftKey && key === "k") {
    return "filterRooms";
  }
  if (ctrlOrCmd && !event.altKey && !event.shiftKey && key === ".") {
    return "toggleRightPanel";
  }

  return null;
}

export function shortcutActionFromMenuPayload(payload: unknown): string | null {
  if (typeof payload !== "string") {
    return null;
  }

  const shortcut = shortcutById(payload);
  return shortcut?.implemented ? shortcut.id : null;
}

function normalizedKey(key: string): string {
  return key.length === 1 ? key.toLowerCase() : key;
}

function shortcutSignature(shortcut: KeyboardShortcut): string {
  const platform = shortcut.platforms?.join(",") ?? "all";
  return `${platform}:${shortcut.keys.join("+").toLowerCase()}`;
}

function duplicates(values: string[]): string[] {
  const seen = new Set<string>();
  const duplicateValues = new Set<string>();
  for (const value of values) {
    if (seen.has(value)) {
      duplicateValues.add(value);
    }
    seen.add(value);
  }
  return [...duplicateValues].sort();
}
