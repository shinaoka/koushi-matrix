import type { MessageId } from "../i18n/messages";

// Keep the user-facing map in docs/help/settings.md aligned with these categories.
export const settingsCategories = [
  { id: "account", label: "settings.categoryAccount" },
  { id: "sessions", label: "settings.categorySessions" },
  { id: "appearance", label: "settings.appearance" },
  { id: "notifications", label: "settings.notifications" },
  { id: "preferences", label: "settings.categoryPreferences" },
  { id: "keyboard", label: "settings.keyboard" },
  { id: "privacy", label: "settings.securityPrivacy" },
  { id: "encryption", label: "trust.encryption" },
  { id: "search", label: "settings.searchHistory" },
  { id: "help", label: "settings.categoryHelp" }
] as const satisfies ReadonlyArray<{ id: string; label: MessageId }>;

export type SettingsCategoryId = (typeof settingsCategories)[number]["id"];
