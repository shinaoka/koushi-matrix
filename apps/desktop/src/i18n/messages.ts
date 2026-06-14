export type Locale = "en" | "ja" | "pseudo";

export type MessageId =
  | "action.add"
  | "action.back"
  | "action.cancel"
  | "action.close"
  | "action.createRoom"
  | "action.createSpace"
  | "action.forward"
  | "action.restartSync"
  | "action.send"
  | "action.sending"
  | "composer.bold"
  | "composer.code"
  | "composer.emoji"
  | "composer.italic"
  | "composer.link"
  | "composer.list"
  | "composer.mention"
  | "composer.messageComposer"
  | "composer.placeholder"
  | "composer.replying"
  | "composer.cancelReply"
  | "dialog.cancelCreate"
  | "dialog.createRoomTitle"
  | "dialog.createSpaceTitle"
  | "dialog.roomName"
  | "dialog.spaceName"
  | "dialog.submitCreateRoom"
  | "dialog.submitCreateSpace"
  | "panel.context"
  | "panel.keyboard"
  | "panel.focusedContext"
  | "panel.recovery"
  | "panel.roomInfo"
  | "panel.search"
  | "panel.spaceInfo"
  | "panel.thread"
  | "panel.userSettings"
  | "room.members"
  | "room.roomInfo"
  | "room.tabs"
  | "room.threadToggle"
  | "settings.accountSwitcher"
  | "settings.current"
  | "settings.device"
  | "settings.general"
  | "settings.homeserver"
  | "settings.keyboard"
  | "settings.localStore"
  | "settings.matrixAccount"
  | "settings.notRestored"
  | "settings.preferences"
  | "settings.searchIndex"
  | "settings.security"
  | "settings.securityPrivacy"
  | "settings.session"
  | "settings.sessionSecret"
  | "settings.switch"
  | "settings.userId"
  | "shortcut.cancelReplyOrEdit"
  | "shortcut.sendMessage"
  | "timeline.conversation"
  | "timeline.conversationStart"
  | "timeline.editedMessage"
  | "timeline.editMessage"
  | "timeline.editBody"
  | "timeline.loading"
  | "timeline.openingThread"
  | "timeline.addReaction"
  | "timeline.reactionSummary"
  | "timeline.reactionPicker"
  | "timeline.reactionOption"
  | "timeline.olderMessages"
  | "timeline.saveEdit"
  | "timeline.cancelEdit"
  | "timeline.redactMessage"
  | "timeline.redactedMessage"
  | "timeline.replyToMessage"
  | "timeline.threadComposer"
  | "timeline.threadPlaceholder"
  | "timeline.threadReplyCountOne"
  | "timeline.threadReplyCountMany"
  | "timeline.threadRoot"
  | "timeline.threadSummaryWithBody"
  | "timeline.threadSummaryWithPreview"
  | "timeline.threadSummaryWithSender"
  | "timeline.openThreadSummary"
  | "timeline.viewReplies"
  | "workspace.createSpace"
  | "workspace.rooms"
  | "workspace.search"
  | "workspace.searchPlaceholder"
  | "workspace.searchScope"
  | "workspace.spaceInfoSettings"
  | "workspace.userSettings"
  | "workspace.workspaces";

type MessageValues = Record<string, string | number>;
type Catalog = Record<MessageId, string>;
export type PseudoLocaleMode = "accented" | "bidi";

const pseudoAccentMap: Record<string, string> = {
  A: "Å",
  B: "ß",
  C: "Ç",
  D: "Ð",
  E: "É",
  F: "Ƒ",
  G: "Ĝ",
  H: "Ħ",
  I: "Î",
  J: "Ĵ",
  K: "Ķ",
  L: "Ŀ",
  M: "Ṁ",
  N: "Ñ",
  O: "Ø",
  P: "Þ",
  Q: "Ǫ",
  R: "Ŕ",
  S: "Š",
  T: "Ŧ",
  U: "Û",
  V: "Ṽ",
  W: "Ŵ",
  X: "Ẋ",
  Y: "Ý",
  Z: "Ž",
  a: "å",
  b: "ƀ",
  c: "ç",
  d: "ð",
  e: "é",
  f: "ƒ",
  g: "ĝ",
  h: "ħ",
  i: "î",
  j: "ĵ",
  k: "ķ",
  l: "ŀ",
  m: "ṁ",
  n: "ñ",
  o: "ø",
  p: "þ",
  q: "ǫ",
  r: "ŕ",
  s: "š",
  t: "ŧ",
  u: "û",
  v: "ṽ",
  w: "ŵ",
  x: "ẋ",
  y: "ý",
  z: "ž"
};

export function pseudoLocalize(input: string, mode: PseudoLocaleMode = "accented"): string {
  const chunks = input.split(/(\{[a-zA-Z0-9_]+\})/g);
  const expanded = chunks
    .map((chunk) => {
      if (/^\{[a-zA-Z0-9_]+\}$/.test(chunk)) {
        return chunk;
      }
      return Array.from(chunk)
        .map((char) => pseudoAccentMap[char] ?? char)
        .join("");
    })
    .join("");

  if (mode === "bidi") {
    return `[!! \u202e${expanded}\u202c !!]`;
  }

  return `[!! ${expanded} !!]`;
}

const en: Catalog = {
  "action.add": "Add",
  "action.back": "Back",
  "action.cancel": "Cancel",
  "action.close": "Close {title}",
  "action.createRoom": "Create room",
  "action.createSpace": "Create space",
  "action.forward": "Forward",
  "action.restartSync": "Restart sync",
  "action.send": "Send",
  "action.sending": "Sending",
  "composer.bold": "Bold",
  "composer.code": "Code",
  "composer.emoji": "Emoji",
  "composer.italic": "Italic",
  "composer.link": "Link",
  "composer.list": "List",
  "composer.mention": "Mention",
  "composer.messageComposer": "Message composer",
  "composer.placeholder": "Message {roomName}",
  "composer.replying": "Replying",
  "composer.cancelReply": "Cancel reply",
  "dialog.cancelCreate": "Cancel create",
  "dialog.createRoomTitle": "Create room",
  "dialog.createSpaceTitle": "Create space",
  "dialog.roomName": "Room name",
  "dialog.spaceName": "Space name",
  "dialog.submitCreateRoom": "Submit create room",
  "dialog.submitCreateSpace": "Submit create space",
  "panel.context": "Context panel",
  "panel.keyboard": "Keyboard",
  "panel.focusedContext": "Focused context",
  "panel.recovery": "Recovery",
  "panel.roomInfo": "Room info",
  "panel.search": "Search",
  "panel.spaceInfo": "Space info",
  "panel.thread": "Thread",
  "panel.userSettings": "User settings",
  "room.members": "Members",
  "room.roomInfo": "Room info",
  "room.tabs": "Room tabs",
  "room.threadToggle": "Toggle thread",
  "settings.accountSwitcher": "Account switcher",
  "settings.current": "Current",
  "settings.device": "Device",
  "settings.general": "General",
  "settings.homeserver": "Homeserver",
  "settings.keyboard": "Keyboard",
  "settings.localStore": "Separate encrypted namespace",
  "settings.matrixAccount": "Matrix account",
  "settings.notRestored": "Not restored",
  "settings.preferences": "Preferences",
  "settings.searchIndex": "Encrypted local index",
  "settings.security": "Security",
  "settings.securityPrivacy": "Security & Privacy",
  "settings.session": "Session",
  "settings.sessionSecret": "OS credential store",
  "settings.switch": "Switch",
  "settings.userId": "User ID",
  "shortcut.cancelReplyOrEdit": "Cancel reply or edit",
  "shortcut.sendMessage": "Send message",
  "timeline.conversation": "Conversation timeline",
  "timeline.conversationStart": "Start of conversation",
  "timeline.editedMessage": "Edited",
  "timeline.editMessage": "Edit message",
  "timeline.editBody": "Edit message body",
  "timeline.loading": "Loading",
  "timeline.openingThread": "Opening thread",
  "timeline.addReaction": "Add reaction",
  "timeline.reactionSummary": "Reaction {key}, count {count}",
  "timeline.reactionPicker": "Choose reaction",
  "timeline.reactionOption": "React with {emoji}",
  "timeline.olderMessages": "Older messages",
  "timeline.saveEdit": "Save edit",
  "timeline.cancelEdit": "Cancel edit",
  "timeline.redactMessage": "Redact message",
  "timeline.redactedMessage": "Message redacted",
  "timeline.replyToMessage": "Reply to message",
  "timeline.threadComposer": "Thread composer",
  "timeline.threadPlaceholder": "Reply",
  "timeline.threadReplyCountOne": "1 reply",
  "timeline.threadReplyCountMany": "{count} replies",
  "timeline.threadRoot": "Thread root {eventId}",
  "timeline.threadSummaryWithBody": "{count} · {preview}",
  "timeline.threadSummaryWithPreview": "{count} · {sender}: {preview}",
  "timeline.threadSummaryWithSender": "{count} · {sender}",
  "timeline.openThreadSummary": "Open thread, {summary}",
  "timeline.viewReplies": "View new replies · {count}",
  "workspace.createSpace": "Create space",
  "workspace.rooms": "Rooms",
  "workspace.search": "Search",
  "workspace.searchPlaceholder": "Search in {spaceName}",
  "workspace.searchScope": "Search scope",
  "workspace.spaceInfoSettings": "Space info and settings",
  "workspace.userSettings": "User settings",
  "workspace.workspaces": "Workspaces"
};

const ja: Catalog = { ...en };

const pseudo: Catalog = Object.fromEntries(
  Object.entries(en).map(([id, value]) => [id, pseudoLocalize(value)])
) as Catalog;

export const catalogs: Record<Locale, Catalog> = { en, ja, pseudo };

export function t(id: MessageId, values: MessageValues = {}, locale: Locale = "en"): string {
  const template = catalogs[locale][id] ?? catalogs.en[id];
  return template.replace(/\{([a-zA-Z0-9_]+)\}/g, (_, key: string) =>
    String(values[key] ?? `{${key}}`)
  );
}
