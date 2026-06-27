import type { MessageId } from "../i18n/messages";

export type ShortcutCategory =
  | "composer"
  | "room"
  | "roomList"
  | "navigation"
  | "autocomplete"
  | "accessibility";

export type ShortcutParity = "same" | "adapted" | "deferred" | "notApplicable";
export type ShortcutPlatform = "all" | "macos" | "windows" | "linux";
export type NativeMenuArea = "app" | "edit" | "view" | "window" | "help";
export type ShortcutPlatformProfile = "macos" | "windows" | "linux";

export interface KeyboardShortcut {
  id: string;
  category: ShortcutCategory;
  labelMessageId: MessageId;
  keys: string[];
  parity: ShortcutParity;
  implemented: boolean;
  platforms?: ShortcutPlatform[];
  nativeMenu?: NativeMenuArea;
  accelerator?: string;
  noteMessageId?: MessageId;
}

export interface KeyboardShortcutGroup {
  category: ShortcutCategory;
  categoryMessageId: MessageId;
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

export interface ShortcutLabelProfile {
  platform: ShortcutPlatformProfile;
  modLabel: "Cmd" | "Ctrl";
}

const shortcuts: KeyboardShortcut[] = [
  {
    id: "sendMessage",
    category: "composer",
    labelMessageId: "shortcut.sendMessage",
    keys: ["Enter"],
    parity: "same",
    implemented: true
  },
  {
    id: "newLine",
    category: "composer",
    labelMessageId: "shortcut.newLine",
    keys: ["Shift", "Enter"],
    parity: "same",
    implemented: true
  },
  {
    id: "formatBold",
    category: "composer",
    labelMessageId: "shortcut.formatBold",
    keys: ["Ctrl/Cmd", "B"],
    parity: "same",
    implemented: false
  },
  {
    id: "formatItalics",
    category: "composer",
    labelMessageId: "shortcut.formatItalics",
    keys: ["Ctrl/Cmd", "I"],
    parity: "same",
    implemented: false
  },
  {
    id: "formatLink",
    category: "composer",
    labelMessageId: "shortcut.formatLink",
    keys: ["Ctrl/Cmd", "Shift", "L"],
    parity: "same",
    implemented: false
  },
  {
    id: "formatCode",
    category: "composer",
    labelMessageId: "shortcut.formatCode",
    keys: ["Ctrl/Cmd", "E"],
    parity: "same",
    implemented: false
  },
  {
    id: "cancelReplyOrEdit",
    category: "composer",
    labelMessageId: "shortcut.cancelReplyOrEdit",
    keys: ["Esc"],
    parity: "same",
    implemented: true
  },
  {
    id: "uploadFile",
    category: "room",
    labelMessageId: "shortcut.uploadFile",
    keys: ["Ctrl/Cmd", "Shift", "U"],
    parity: "deferred",
    implemented: false,
    noteMessageId: "shortcut.noteUploadUiDeferred"
  },
  {
    id: "searchInRoom",
    category: "room",
    labelMessageId: "shortcut.searchInRoom",
    keys: ["Ctrl/Cmd", "F"],
    parity: "same",
    implemented: true
  },
  {
    id: "jumpToOldestUnread",
    category: "room",
    labelMessageId: "shortcut.jumpToOldestUnread",
    keys: ["Shift", "PageUp"],
    parity: "same",
    implemented: false
  },
  {
    id: "scrollTimelineUp",
    category: "room",
    labelMessageId: "shortcut.scrollTimelineUp",
    keys: ["PageUp"],
    parity: "same",
    implemented: false
  },
  {
    id: "scrollTimelineDown",
    category: "room",
    labelMessageId: "shortcut.scrollTimelineDown",
    keys: ["PageDown"],
    parity: "same",
    implemented: false
  },
  {
    id: "jumpToFirstMessage",
    category: "room",
    labelMessageId: "shortcut.jumpToFirstMessage",
    keys: ["Ctrl", "Home"],
    parity: "same",
    implemented: false
  },
  {
    id: "jumpToLatestMessage",
    category: "room",
    labelMessageId: "shortcut.jumpToLatestMessage",
    keys: ["Ctrl", "End"],
    parity: "same",
    implemented: false
  },
  {
    id: "selectRoomInRoomList",
    category: "roomList",
    labelMessageId: "shortcut.selectRoomInRoomList",
    keys: ["Enter"],
    parity: "same",
    implemented: true
  },
  {
    id: "previousRoomInList",
    category: "roomList",
    labelMessageId: "shortcut.previousRoomInList",
    keys: ["ArrowUp"],
    parity: "same",
    implemented: true
  },
  {
    id: "nextRoomInList",
    category: "roomList",
    labelMessageId: "shortcut.nextRoomInList",
    keys: ["ArrowDown"],
    parity: "same",
    implemented: true
  },
  {
    id: "collapseRoomListSection",
    category: "roomList",
    labelMessageId: "shortcut.collapseRoomListSection",
    keys: ["ArrowLeft"],
    parity: "same",
    implemented: false
  },
  {
    id: "expandRoomListSection",
    category: "roomList",
    labelMessageId: "shortcut.expandRoomListSection",
    keys: ["ArrowRight"],
    parity: "same",
    implemented: false
  },
  {
    id: "filterRooms",
    category: "navigation",
    labelMessageId: "shortcut.filterRooms",
    keys: ["Ctrl/Cmd", "K"],
    parity: "same",
    implemented: true
  },
  {
    id: "toggleRightPanel",
    category: "navigation",
    labelMessageId: "shortcut.toggleRightPanel",
    keys: ["Ctrl/Cmd", "."],
    parity: "same",
    implemented: true,
    nativeMenu: "view",
    accelerator: "CmdOrCtrl+."
  },
  {
    id: "toggleSpacePanel",
    category: "navigation",
    labelMessageId: "shortcut.toggleSpacePanel",
    keys: ["Ctrl/Cmd", "Shift", "D"],
    parity: "same",
    implemented: false
  },
  {
    id: "toggleFullscreen",
    category: "navigation",
    labelMessageId: "shortcut.toggleFullscreen",
    keys: ["Cmd", "Ctrl", "F"],
    platforms: ["macos"],
    parity: "same",
    implemented: true,
    nativeMenu: "window",
    accelerator: "Ctrl+Command+F"
  },
  {
    id: "showKeyboardSettings",
    category: "navigation",
    labelMessageId: "shortcut.showKeyboardSettings",
    keys: ["Ctrl/Cmd", "/"],
    parity: "same",
    implemented: true,
    nativeMenu: "help",
    accelerator: "CmdOrCtrl+/"
  },
  {
    id: "openUserSettings",
    category: "navigation",
    labelMessageId: "shortcut.openUserSettings",
    keys: ["Cmd", ","],
    platforms: ["macos"],
    parity: "same",
    implemented: true,
    nativeMenu: "app",
    accelerator: "CmdOrCtrl+,"
  },
  {
    id: "goHome",
    category: "navigation",
    labelMessageId: "shortcut.goHome",
    keys: ["Ctrl", "Alt", "H"],
    parity: "adapted",
    implemented: true,
    noteMessageId: "shortcut.noteGoHomeAdapted"
  },
  {
    id: "selectPreviousRoom",
    category: "navigation",
    labelMessageId: "shortcut.selectPreviousRoom",
    keys: ["Alt", "ArrowUp"],
    parity: "same",
    implemented: false
  },
  {
    id: "selectNextRoom",
    category: "navigation",
    labelMessageId: "shortcut.selectNextRoom",
    keys: ["Alt", "ArrowDown"],
    parity: "same",
    implemented: false
  },
  {
    id: "selectPreviousUnreadRoom",
    category: "navigation",
    labelMessageId: "shortcut.selectPreviousUnreadRoom",
    keys: ["Alt", "Shift", "ArrowUp"],
    parity: "same",
    implemented: false
  },
  {
    id: "selectNextUnreadRoom",
    category: "navigation",
    labelMessageId: "shortcut.selectNextUnreadRoom",
    keys: ["Alt", "Shift", "ArrowDown"],
    parity: "same",
    implemented: false
  },
  {
    id: "previousVisitedRoomOrSpace",
    category: "navigation",
    labelMessageId: "shortcut.previousVisitedRoomOrSpace",
    keys: ["Cmd", "["],
    platforms: ["macos"],
    parity: "same",
    implemented: false,
    nativeMenu: "view",
    accelerator: "Cmd+["
  },
  {
    id: "nextVisitedRoomOrSpace",
    category: "navigation",
    labelMessageId: "shortcut.nextVisitedRoomOrSpace",
    keys: ["Cmd", "]"],
    platforms: ["macos"],
    parity: "same",
    implemented: false,
    nativeMenu: "view",
    accelerator: "Cmd+]"
  },
  {
    id: "cancelAutocomplete",
    category: "autocomplete",
    labelMessageId: "shortcut.cancelAutocomplete",
    keys: ["Esc"],
    parity: "same",
    implemented: false
  },
  {
    id: "nextAutocompleteSelection",
    category: "autocomplete",
    labelMessageId: "shortcut.nextAutocompleteSelection",
    keys: ["ArrowDown"],
    parity: "same",
    implemented: false
  },
  {
    id: "previousAutocompleteSelection",
    category: "autocomplete",
    labelMessageId: "shortcut.previousAutocompleteSelection",
    keys: ["ArrowUp"],
    parity: "same",
    implemented: false
  },
  {
    id: "closeDialogOrMenu",
    category: "accessibility",
    labelMessageId: "shortcut.closeDialogOrMenu",
    keys: ["Esc"],
    parity: "same",
    implemented: true
  },
  {
    id: "activateButton",
    category: "accessibility",
    labelMessageId: "shortcut.activateButton",
    keys: ["Enter"],
    parity: "same",
    implemented: true
  },
  {
    id: "nextLandmark",
    category: "accessibility",
    labelMessageId: "shortcut.nextLandmark",
    keys: ["F6"],
    parity: "same",
    implemented: false
  },
  {
    id: "previousLandmark",
    category: "accessibility",
    labelMessageId: "shortcut.previousLandmark",
    keys: ["Shift", "F6"],
    parity: "same",
    implemented: false
  },
  {
    id: "toggleMicrophone",
    category: "accessibility",
    labelMessageId: "shortcut.toggleMicrophone",
    keys: ["Ctrl/Cmd", "D"],
    parity: "deferred",
    implemented: false,
    noteMessageId: "shortcut.noteCallsDeferred"
  }
];

const categoryOrder: ShortcutCategory[] = [
  "composer",
  "room",
  "roomList",
  "navigation",
  "autocomplete",
  "accessibility"
];

const categoryMessageIds: Record<ShortcutCategory, MessageId> = {
  composer: "shortcut.categoryComposer",
  room: "shortcut.categoryRoom",
  roomList: "shortcut.categoryRoomList",
  navigation: "shortcut.categoryNavigation",
  autocomplete: "shortcut.categoryAutocomplete",
  accessibility: "shortcut.categoryAccessibility"
};

const globalKeyboardHandlerIds = [
  "showKeyboardSettings",
  "openUserSettings",
  "searchInRoom",
  "filterRooms",
  "toggleRightPanel",
  "toggleFullscreen"
];

export const keyboardShortcutGroups: KeyboardShortcutGroup[] = categoryOrder.map(
  (category) => ({
    category,
    categoryMessageId: categoryMessageIds[category],
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
      .filter((shortcut) => shortcut.parity === "adapted" && !shortcut.noteMessageId)
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

export function shortcutIdForKeyboardEvent(
  event: KeyboardEventLike,
  platform: ShortcutPlatformProfile = defaultShortcutLabelProfile().platform
): string | null {
  const key = normalizedKey(event.key);
  const ctrlOrCmd = event.ctrlKey || event.metaKey;
  // On macOS, Ctrl is reserved for native AppKit Emacs text-editing bindings
  // (Ctrl+F = forward, Ctrl+K = kill to EOL, etc.).  App shortcuts for those
  // keys must use Cmd (metaKey) only so that Ctrl+key reaches the text system.
  const primaryMod =
    platform === "macos" ? event.metaKey && !event.ctrlKey : ctrlOrCmd;

  if (ctrlOrCmd && !event.altKey && !event.shiftKey && key === "/") {
    return "showKeyboardSettings";
  }
  if (event.metaKey && !event.ctrlKey && !event.altKey && !event.shiftKey && key === ",") {
    return "openUserSettings";
  }
  if (event.metaKey && event.ctrlKey && !event.altKey && !event.shiftKey && key === "f") {
    return "toggleFullscreen";
  }
  if (primaryMod && !event.altKey && !event.shiftKey && key === "f") {
    return "searchInRoom";
  }
  if (primaryMod && !event.altKey && !event.shiftKey && key === "k") {
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

export function defaultShortcutLabelProfile(): ShortcutLabelProfile {
  const platform =
    typeof navigator === "undefined"
      ? ""
      : ((navigator as Navigator & { userAgentData?: { platform?: string } })
          .userAgentData?.platform ?? navigator.platform);
  if (/mac/i.test(platform)) {
    return { platform: "macos", modLabel: "Cmd" };
  }
  if (/win/i.test(platform)) {
    return { platform: "windows", modLabel: "Ctrl" };
  }
  return { platform: "linux", modLabel: "Ctrl" };
}

export function formatModShortcut(
  suffix: string,
  profile: ShortcutLabelProfile = defaultShortcutLabelProfile()
): string {
  return `${profile.modLabel}+${suffix}`;
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
