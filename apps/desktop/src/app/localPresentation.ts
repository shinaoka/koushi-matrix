export type DisplayDensity = "compact" | "default" | "comfortable";

export interface SpaceLocalOverride {
  name?: string;
  icon?: string;
}

export type SpaceLocalOverrides = Record<string, SpaceLocalOverride>;

const SPACE_OVERRIDES_STORAGE_KEY = "koushi.spaceLocalOverrides.v1";
const DISPLAY_DENSITY_STORAGE_KEY = "koushi.displayDensity.v1";

export const SPACE_OVERRIDES_CHANGED_EVENT = "koushi-space-local-overrides-changed";

function browserStorage(): Storage | null {
  return typeof window === "undefined" ? null : window.localStorage;
}

export function readSpaceLocalOverrides(): SpaceLocalOverrides {
  const storage = browserStorage();
  if (!storage) {
    return {};
  }
  try {
    const parsed = JSON.parse(storage.getItem(SPACE_OVERRIDES_STORAGE_KEY) ?? "{}");
    if (!parsed || typeof parsed !== "object") {
      return {};
    }
    return parsed as SpaceLocalOverrides;
  } catch {
    return {};
  }
}

export function writeSpaceLocalOverrides(overrides: SpaceLocalOverrides): void {
  const storage = browserStorage();
  if (!storage) {
    return;
  }
  storage.setItem(SPACE_OVERRIDES_STORAGE_KEY, JSON.stringify(overrides));
  window.dispatchEvent(new CustomEvent(SPACE_OVERRIDES_CHANGED_EVENT));
}

export function setSpaceLocalOverride(
  spaceId: string,
  override: SpaceLocalOverride | null
): SpaceLocalOverrides {
  const current = readSpaceLocalOverrides();
  const next = { ...current };
  if (!override || (!override.name?.trim() && !override.icon?.trim())) {
    delete next[spaceId];
  } else {
    next[spaceId] = {
      name: override.name?.trim() || undefined,
      icon: override.icon?.trim() || undefined
    };
  }
  writeSpaceLocalOverrides(next);
  return next;
}

export function spaceDisplayName(
  spaceId: string | null,
  originalName: string,
  overrides: SpaceLocalOverrides
): string {
  if (!spaceId) {
    return originalName;
  }
  return overrides[spaceId]?.name?.trim() || originalName;
}

export function readDisplayDensity(): DisplayDensity {
  const storage = browserStorage();
  const value = storage?.getItem(DISPLAY_DENSITY_STORAGE_KEY);
  return value === "compact" || value === "default" || value === "comfortable"
    ? value
    : "comfortable";
}

export function writeDisplayDensity(density: DisplayDensity): void {
  browserStorage()?.setItem(DISPLAY_DENSITY_STORAGE_KEY, density);
}
