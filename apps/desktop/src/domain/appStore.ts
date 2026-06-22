import { create } from "zustand";
import { subscribeWithSelector } from "zustand/middleware";

import type { StateDeltaChangedSlices, StateDeltaPayload } from "./coreEvents";
import type { MentionCandidate, TimelineForwardDestination } from "./projectionTypes";
import type {
  AppState,
  DesktopSnapshot,
  UserProfile
} from "./types";

interface AppStoreState {
  snapshot: DesktopSnapshot | null;
  stateGeneration: number | null;
}

const EMPTY_FORWARD_DESTINATIONS: TimelineForwardDestination[] = [];
const EMPTY_MENTION_CANDIDATES: MentionCandidate[] = [];

let cachedForwardRooms: DesktopSnapshot["state"]["domain"]["rooms"] | null = null;
let cachedForwardDestinations: TimelineForwardDestination[] = EMPTY_FORWARD_DESTINATIONS;
let cachedMentionUsers: DesktopSnapshot["state"]["domain"]["profile"]["users"] | null = null;
let cachedMentionCandidates: MentionCandidate[] = EMPTY_MENTION_CANDIDATES;

export const useAppStore = create<AppStoreState>()(
  subscribeWithSelector((): AppStoreState => ({
    snapshot: null,
    stateGeneration: null
  }))
);

export function getAppStoreSnapshot(): DesktopSnapshot | null {
  return useAppStore.getState().snapshot;
}

export function setAppStoreSnapshot(next: DesktopSnapshot | null): void {
  const current = useAppStore.getState();
  const previous = current.snapshot;
  const incomingGeneration = next?.state_generation ?? null;
  if (
    current.stateGeneration !== null &&
    incomingGeneration !== null &&
    incomingGeneration < current.stateGeneration
  ) {
    return;
  }
  const snapshot = applySnapshotToState(previous, next);
  const previousGeneration = current.stateGeneration;
  const nextGeneration = snapshot?.state_generation ?? null;
  if (Object.is(previous, snapshot) && previousGeneration === nextGeneration) {
    return;
  }
  useAppStore.setState({ snapshot, stateGeneration: nextGeneration });
}

export function clearAppStoreSnapshot(): void {
  setAppStoreSnapshot(null);
}

export type DesktopSnapshotDelta = StateDeltaPayload;
export type DesktopSnapshotChangedSlices = StateDeltaChangedSlices;
export type AppStateChangedSlices = NonNullable<StateDeltaChangedSlices["state"]>;

export function applyAppStoreDelta(delta: DesktopSnapshotDelta): boolean {
  const current = useAppStore.getState();
  if (current.snapshot === null) {
    return false;
  }
  if (
    current.stateGeneration !== null &&
    delta.generation !== current.stateGeneration + 1
  ) {
    return false;
  }
  const snapshot = applyDeltaToState(current.snapshot, delta);
  if (snapshot === null) {
    return false;
  }
  if (
    Object.is(current.snapshot, snapshot) &&
    current.stateGeneration === delta.generation
  ) {
    return true;
  }
  useAppStore.setState({ snapshot, stateGeneration: delta.generation });
  return true;
}

export function applyDeltaToState(
  previous: DesktopSnapshot | null,
  delta: DesktopSnapshotDelta
): DesktopSnapshot | null {
  if (previous === null) {
    return null;
  }
  const next: DesktopSnapshot = {
    state_generation: delta.generation,
    state: applyStateDelta(previous.state, delta.changed.state),
    sidebar: delta.changed.sidebar ?? previous.sidebar,
    timeline: delta.changed.timeline ?? previous.timeline,
    thread: Object.prototype.hasOwnProperty.call(delta.changed, "thread")
      ? (delta.changed.thread ?? null)
      : previous.thread
  };
  return reconcileJsonValue(previous, next);
}

function applyStateDelta(
  previous: AppState,
  changed: AppStateChangedSlices | undefined
): AppState {
  if (!changed) {
    return previous;
  }
  return {
    schema_version: changed.schema_version ?? previous.schema_version,
    domain: changed.domain ? { ...previous.domain, ...changed.domain } : previous.domain,
    ui: changed.ui ? { ...previous.ui, ...changed.ui } : previous.ui
  };
}

export function applySnapshotToState(
  previous: DesktopSnapshot | null,
  next: DesktopSnapshot | null
): DesktopSnapshot | null {
  if (Object.is(previous, next)) {
    return previous;
  }
  if (previous === null || next === null) {
    return next;
  }
  return reconcileJsonValue(previous, next);
}

export function selectSnapshot(state: Pick<AppStoreState, "snapshot">): DesktopSnapshot | null {
  return state.snapshot;
}

export function selectForwardDestinations(
  state: Pick<AppStoreState, "snapshot">
): TimelineForwardDestination[] {
  const rooms = state.snapshot?.state.domain.rooms ?? null;
  if (rooms === null) {
    cachedForwardRooms = null;
    cachedForwardDestinations = EMPTY_FORWARD_DESTINATIONS;
    return cachedForwardDestinations;
  }
  if (rooms === cachedForwardRooms) {
    return cachedForwardDestinations;
  }
  cachedForwardRooms = rooms;
  cachedForwardDestinations = rooms.map((room) => ({
    room_id: room.room_id,
    display_name: room.display_label
  }));
  return cachedForwardDestinations;
}

export function selectMentionCandidates(
  state: Pick<AppStoreState, "snapshot">
): MentionCandidate[] {
  const users = state.snapshot?.state.domain.profile.users ?? null;
  if (users === null) {
    cachedMentionUsers = null;
    cachedMentionCandidates = EMPTY_MENTION_CANDIDATES;
    return cachedMentionCandidates;
  }
  if (users === cachedMentionUsers) {
    return cachedMentionCandidates;
  }
  cachedMentionUsers = users;
  cachedMentionCandidates = Object.values(users)
    .map((profile) => mentionCandidateFromProfile(profile))
    .sort(
      (a, b) =>
        a.label.localeCompare(b.label, undefined, { sensitivity: "base" }) ||
        a.key.localeCompare(b.key)
    );
  return cachedMentionCandidates;
}

function mentionCandidateFromProfile(profile: UserProfile): MentionCandidate {
  const label = profile.display_label.trim() || profile.user_id;
  return {
    key: profile.user_id,
    label,
    searchText: (
      profile.mention_search_terms.length
        ? profile.mention_search_terms.join(" ")
        : `${label} ${profile.user_id}`
    ).toLowerCase(),
    target: {
      kind: "user",
      user_id: profile.user_id,
      display_label: label
    }
  };
}

function reconcileJsonValue<T>(previous: T, next: T): T {
  if (Object.is(previous, next)) {
    return previous;
  }

  if (!isMergeable(previous) || !isMergeable(next)) {
    return next;
  }

  if (Array.isArray(previous) && Array.isArray(next)) {
    return reconcileArray(previous, next) as T;
  }

  if (isPlainObject(previous) && isPlainObject(next)) {
    return reconcileObject(previous, next) as T;
  }

  return next;
}

function reconcileArray<T>(previous: readonly T[], next: readonly T[]): readonly T[] {
  let changed = previous.length !== next.length;
  const reconciled = next.map((value, index) => {
    const merged = index < previous.length ? reconcileJsonValue(previous[index], value) : value;
    if (!Object.is(merged, previous[index])) {
      changed = true;
    }
    return merged;
  });
  return changed ? reconciled : previous;
}

function reconcileObject<T extends Record<string, unknown>>(
  previous: T,
  next: T
): T {
  const previousKeys = Object.keys(previous);
  const nextKeys = Object.keys(next);
  let changed = previousKeys.length !== nextKeys.length;

  if (!changed) {
    for (const key of nextKeys) {
      if (!Object.prototype.hasOwnProperty.call(previous, key)) {
        changed = true;
        break;
      }
    }
  }

  const reconciled: Record<string, unknown> = {};
  for (const key of nextKeys) {
    const merged = reconcileJsonValue(previous[key], next[key]);
    reconciled[key] = merged;
    if (!Object.is(merged, previous[key])) {
      changed = true;
    }
  }

  return changed ? (reconciled as T) : previous;
}

function isMergeable(value: unknown): value is Record<string, unknown> | readonly unknown[] {
  return typeof value === "object" && value !== null;
}

function isPlainObject(value: unknown): value is Record<string, unknown> {
  return Object.prototype.toString.call(value) === "[object Object]";
}
