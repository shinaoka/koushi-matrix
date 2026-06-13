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
  | "dialog.submitCreateRoom"
  | "dialog.submitCreateSpace"
  | "panel.context"
  | "panel.keyboard"
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
  | "timeline.editMessage"
  | "timeline.redactMessage"
  | "timeline.replyToMessage"
  | "timeline.viewReplies"
  | "workspace.createSpace"
  | "workspace.rooms"
  | "workspace.search"
  | "workspace.searchScope"
  | "workspace.spaceInfoSettings"
  | "workspace.userSettings"
  | "workspace.workspaces";

type MessageValues = Record<string, string | number>;
type Catalog = Record<MessageId, string>;

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
  "dialog.submitCreateRoom": "Submit create room",
  "dialog.submitCreateSpace": "Submit create space",
  "panel.context": "Context panel",
  "panel.keyboard": "Keyboard",
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
  "timeline.editMessage": "Edit message",
  "timeline.redactMessage": "Redact message",
  "timeline.replyToMessage": "Reply to message",
  "timeline.viewReplies": "View new replies · {count}",
  "workspace.createSpace": "Create space",
  "workspace.rooms": "Rooms",
  "workspace.search": "Search",
  "workspace.searchScope": "Search scope",
  "workspace.spaceInfoSettings": "Space info and settings",
  "workspace.userSettings": "User settings",
  "workspace.workspaces": "Workspaces"
};

const ja: Catalog = {
  ...en,
  "composer.placeholder": "{roomName} へのメッセージ",
  "composer.replying": "返信中",
  "timeline.viewReplies": "新しい返信を確認する · {count}"
};

const pseudo: Catalog = Object.fromEntries(
  Object.entries(en).map(([id, value]) => [id, `[!! ${value} !!]`])
) as Catalog;

export const catalogs: Record<Locale, Catalog> = { en, ja, pseudo };

export function t(id: MessageId, values: MessageValues = {}, locale: Locale = "en"): string {
  const template = catalogs[locale][id] ?? catalogs.en[id];
  return template.replace(/\{([a-zA-Z0-9_]+)\}/g, (_, key: string) =>
    String(values[key] ?? `{${key}}`)
  );
}
