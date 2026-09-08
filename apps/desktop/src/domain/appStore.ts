import { create } from "zustand";
import { subscribeWithSelector } from "zustand/middleware";

import type { StateDeltaChangedSlices, StateDeltaPayload } from "./coreEvents";
import type { MentionCandidate, TimelineForwardDestination } from "./projectionTypes";
import type {
  ActivityRow,
  AppState,
  DesktopSnapshot,
  MentionCandidatesTarget,
  MentionSurface
} from "./types";

interface AppStoreState {
  snapshot: DesktopSnapshot | null;
  stateGeneration: number | null;
}

const EMPTY_FORWARD_DESTINATIONS: TimelineForwardDestination[] = [];
const EMPTY_MENTION_CANDIDATES: MentionCandidate[] = [];
const ROOM_MENTION_CANDIDATE: MentionCandidate = {
  key: "roomMention",
  label: "@room",
  target: { kind: "roomMention", display_label: "room" }
};

let cachedForwardRooms: DesktopSnapshot["state"]["domain"]["rooms"] | null = null;
let cachedForwardDestinations: TimelineForwardDestination[] = EMPTY_FORWARD_DESTINATIONS;
let cachedMentionTarget: MentionCandidatesTarget | null = null;
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
  resetAppStoreDeltaStats();
  setAppStoreSnapshot(null);
}

/**
 * Private-data-free counters for the #111 state-delta transport, surfaced in
 * the diagnostic report. A large `gapRefreshRequested` relative to `applied`
 * is the signature of a delta/refresh storm under high background-sync volume.
 */
export interface AppStoreDeltaStats {
  applied: number;
  staleIgnored: number;
  gapRefreshRequested: number;
}

let deltaStats: AppStoreDeltaStats = { applied: 0, staleIgnored: 0, gapRefreshRequested: 0 };

export function getAppStoreDeltaStats(): AppStoreDeltaStats {
  return { ...deltaStats };
}

function resetAppStoreDeltaStats(): void {
  deltaStats = { applied: 0, staleIgnored: 0, gapRefreshRequested: 0 };
}

export type DesktopSnapshotDelta = StateDeltaPayload;
export type DesktopSnapshotChangedSlices = StateDeltaChangedSlices;
export type AppStateChangedSlices = NonNullable<StateDeltaChangedSlices["state"]>;

export function applyAppStoreDelta(delta: DesktopSnapshotDelta): boolean {
  return applyAppStoreDeltas([delta]);
}

export function applyAppStoreDeltas(deltas: readonly DesktopSnapshotDelta[]): boolean {
  const current = useAppStore.getState();
  let snapshot = current.snapshot;
  let stateGeneration = current.stateGeneration;
  let changed = false;
  if (snapshot === null) {
    return false;
  }
  for (const delta of deltas) {
    if (stateGeneration !== null) {
    // Already-applied / duplicate delta: the current state (advanced by a
    // newer delta or a full command-response/refresh snapshot) already
    // subsumes it, so ignore it as handled. Returning false here would make
    // the caller refresh, which lands a still-newer generation and turns the
    // next trailing background delta stale too — a self-amplifying refresh
    // storm under large-account sync volume.
      if (delta.generation <= stateGeneration) {
        deltaStats.staleIgnored += 1;
        continue;
      }
    // Genuine forward gap: a delta was missed, so the caller must resync from
    // a full snapshot before later deltas can apply contiguously.
      if (delta.generation !== stateGeneration + 1) {
        deltaStats.gapRefreshRequested += 1;
        if (changed) {
          useAppStore.setState({ snapshot, stateGeneration });
        }
        return false;
      }
    }
    deltaStats.applied += 1;
    const nextSnapshot = applyDeltaToState(snapshot, delta);
    if (nextSnapshot === null) {
      return false;
    }
    changed ||= !Object.is(snapshot, nextSnapshot) || stateGeneration !== delta.generation;
    snapshot = nextSnapshot;
    stateGeneration = delta.generation;
  }
  if (changed) {
    useAppStore.setState({ snapshot, stateGeneration });
  }
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

function applyActivityRowDeltas(
  activity: AppState["domain"]["activity"],
  recentChanges: Record<string, ActivityRow | null> | undefined,
  unreadChanges: Record<string, ActivityRow | null> | undefined
): AppState["domain"]["activity"] {
  if (activity.kind !== "open") {
    return activity;
  }

  const apply = (
    rows: ActivityRow[],
    changes: Record<string, ActivityRow | null> | undefined
  ): ActivityRow[] => {
    if (!changes) {
      return rows;
    }
    return rows
      .map((row) => {
        const replacement = changes[activityRowKey(row)];
        return replacement === null ? null : replacement ?? row;
      })
      .filter((row): row is ActivityRow => row !== null);
  };

  return {
    ...activity,
    recent: { ...activity.recent, rows: apply(activity.recent.rows, recentChanges) },
    unread: { ...activity.unread, rows: apply(activity.unread.rows, unreadChanges) }
  };
}

function activityRowKey(row: ActivityRow): string {
  return row.event_id === null ? `room-unread:${row.room_id}` : `event:${row.event_id}`;
}

function applyStateDelta(
  previous: AppState,
  changed: AppStateChangedSlices | undefined
): AppState {
  if (!changed) {
    return previous;
  }
  let domain = previous.domain;
  if (changed.domain) {
    const {
      live_signals_rooms,
      live_signals_receipts_by_room_event,
      live_signals_room_metadata_by_id,
      live_signals_presence_by_user,
      rooms_by_id,
      spaces_by_id,
      invites_by_id,
      profile_own,
      profile_users_by_id,
      profile_room_users_by_room,
      profile_local_aliases_by_id,
      profile_ignored_user_ids_by_id,
      profile_local_alias_update,
      profile_ignored_user_update,
      profile_update,
      room_notification_settings_by_id,
      room_interactions_by_id,
      search_crawler_rooms_by_id,
      search_crawler_last_active,
      activity_recent_rows_by_id,
      activity_unread_rows_by_id,
      ...domainSlices
    } = changed.domain;
    domain = { ...previous.domain, ...domainSlices };
    if (activity_recent_rows_by_id || activity_unread_rows_by_id) {
      domain = {
        ...domain,
        activity: applyActivityRowDeltas(
          domain.activity,
          activity_recent_rows_by_id,
          activity_unread_rows_by_id
        )
      };
    }
    if (profile_users_by_id) {
      const users = { ...domain.profile.users };
      for (const [userId, user] of Object.entries(profile_users_by_id)) {
        if (user === null) {
          delete users[userId];
        } else {
          users[userId] = user;
        }
      }
      domain = { ...domain, profile: { ...domain.profile, users } };
    }
    if (profile_room_users_by_room) {
      const roomUsers = { ...domain.profile.room_users };
      for (const [roomId, updates] of Object.entries(profile_room_users_by_room)) {
        if (updates === null) {
          delete roomUsers[roomId];
          continue;
        }
        const users = { ...(roomUsers[roomId] ?? {}) };
        for (const [userId, user] of Object.entries(updates)) {
          if (user === null) {
            delete users[userId];
          } else {
            users[userId] = user;
          }
        }
        roomUsers[roomId] = users;
      }
      domain = { ...domain, profile: { ...domain.profile, room_users: roomUsers } };
    }
    if (
      profile_own !== undefined ||
      profile_local_aliases_by_id ||
      profile_ignored_user_ids_by_id ||
      profile_local_alias_update !== undefined ||
      profile_ignored_user_update !== undefined ||
      profile_update !== undefined
    ) {
      let profile = domain.profile;
      if (profile_own !== undefined) {
        profile = { ...profile, own: profile_own };
      }
      if (profile_local_aliases_by_id) {
        const aliases = { ...profile.local_aliases };
        for (const [userId, alias] of Object.entries(profile_local_aliases_by_id)) {
          if (alias === null) {
            delete aliases[userId];
          } else {
            aliases[userId] = alias;
          }
        }
        profile = { ...profile, local_aliases: aliases };
      }
      if (profile_ignored_user_ids_by_id) {
        const ignored = new Set(profile.ignored_user_ids);
        for (const [userId, isIgnored] of Object.entries(profile_ignored_user_ids_by_id)) {
          if (isIgnored) {
            ignored.add(userId);
          } else {
            ignored.delete(userId);
          }
        }
        profile = { ...profile, ignored_user_ids: [...ignored].sort() };
      }
      if (profile_local_alias_update !== undefined) {
        profile = { ...profile, local_alias_update: profile_local_alias_update };
      }
      if (profile_ignored_user_update !== undefined) {
        profile = { ...profile, ignored_user_update: profile_ignored_user_update };
      }
      if (profile_update !== undefined) {
        profile = { ...profile, update: profile_update };
      }
      domain = { ...domain, profile };
    }
    if (search_crawler_rooms_by_id) {
      const rooms = { ...domain.search_crawler.rooms };
      for (const [roomId, room] of Object.entries(search_crawler_rooms_by_id)) {
        if (room === null) {
          delete rooms[roomId];
        } else {
          rooms[roomId] = room;
        }
      }
      domain = {
        ...domain,
        search_crawler: { ...domain.search_crawler, rooms }
      };
    }
    if (search_crawler_last_active !== undefined) {
      domain = {
        ...domain,
        search_crawler: { ...domain.search_crawler, last_active: search_crawler_last_active }
      };
    }
    if (live_signals_presence_by_user) {
      const presence = { ...domain.live_signals.presence };
      for (const [userId, status] of Object.entries(live_signals_presence_by_user)) {
        if (status === null) {
          delete presence[userId];
        } else {
          presence[userId] = status;
        }
      }
      domain = {
        ...domain,
        live_signals: { ...domain.live_signals, presence }
      };
    }
    if (room_notification_settings_by_id) {
      const settings = { ...domain.room_notification_settings };
      for (const [roomId, setting] of Object.entries(room_notification_settings_by_id)) {
        if (setting === null) {
          delete settings[roomId];
        } else {
          settings[roomId] = setting;
        }
      }
      domain = { ...domain, room_notification_settings: settings };
    }
    if (room_interactions_by_id) {
      const interactions = { ...domain.room_interactions };
      for (const [roomId, interaction] of Object.entries(room_interactions_by_id)) {
        if (interaction === null) {
          delete interactions[roomId];
        } else {
          interactions[roomId] = interaction;
        }
      }
      domain = { ...domain, room_interactions: interactions };
    }
    if (invites_by_id) {
      domain = {
        ...domain,
        invites: domain.invites
          .map((invite) => {
            const update = invites_by_id[invite.room_id];
            return update === null ? null : update ?? invite;
          })
          .filter((invite): invite is NonNullable<typeof invite> => invite !== null)
      };
    }
    if (spaces_by_id) {
      domain = {
        ...domain,
        spaces: domain.spaces
          .map((space) => {
            const update = spaces_by_id[space.space_id];
            return update === null ? null : update ?? space;
          })
          .filter((space): space is NonNullable<typeof space> => space !== null)
      };
    }
    if (rooms_by_id) {
      domain = {
        ...domain,
        rooms: domain.rooms
          .map((room) => {
            const update = rooms_by_id[room.room_id];
            return update === null ? null : update ?? room;
          })
          .filter((room): room is NonNullable<typeof room> => room !== null)
      };
    }
    if (live_signals_rooms) {
      const rooms = { ...domain.live_signals.rooms };
      for (const [roomId, room] of Object.entries(live_signals_rooms)) {
        if (room === null) {
          delete rooms[roomId];
        } else {
          rooms[roomId] = room;
        }
      }
      domain = {
        ...domain,
        live_signals: { ...domain.live_signals, rooms }
      };
    }
    if (live_signals_room_metadata_by_id) {
      const rooms = { ...domain.live_signals.rooms };
      for (const [roomId, metadata] of Object.entries(live_signals_room_metadata_by_id)) {
        const room = rooms[roomId];
        if (!room || metadata === null) {
          continue;
        }
        rooms[roomId] = { ...room, ...metadata };
      }
      domain = {
        ...domain,
        live_signals: { ...domain.live_signals, rooms }
      };
    }
    if (live_signals_receipts_by_room_event) {
      const rooms = { ...domain.live_signals.rooms };
      for (const [roomId, updates] of Object.entries(live_signals_receipts_by_room_event)) {
        const room = rooms[roomId];
        if (!room) {
          continue;
        }
        const receipts = { ...room.receipts_by_event };
        for (const [eventId, summary] of Object.entries(updates)) {
          if (summary === null) {
            delete receipts[eventId];
          } else {
            receipts[eventId] = summary;
          }
        }
        rooms[roomId] = { ...room, receipts_by_event: receipts };
      }
      domain = {
        ...domain,
        live_signals: { ...domain.live_signals, rooms }
      };
    }
  }
  return {
    schema_version: changed.schema_version ?? previous.schema_version,
    domain,
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
  state: Pick<AppStoreState, "snapshot">,
  roomId: string | null,
  surface: MentionSurface
): MentionCandidate[] {
  const target =
    state.snapshot?.state.domain.mention_candidates.targets.find(
      (candidateTarget) =>
        candidateTarget.room_id === roomId && candidateTarget.surface === surface
    ) ?? null;
  if (target === null) {
    cachedMentionTarget = null;
    cachedMentionCandidates = EMPTY_MENTION_CANDIDATES;
    return cachedMentionCandidates;
  }
  if (target === cachedMentionTarget) {
    return cachedMentionCandidates;
  }
  cachedMentionTarget = target;
  cachedMentionCandidates = [
    ...target.candidates.map((candidate) => ({
      key: candidate.user_id,
      label: candidate.display_label?.trim() ?? "",
      avatar: candidate.avatar,
      target: {
        kind: "user" as const,
        user_id: candidate.user_id,
        display_label: candidate.display_label?.trim() ?? ""
      }
    })),
    ...(target.room_mention_allowed === "allowed" ? [ROOM_MENTION_CANDIDATE] : [])
  ];
  return cachedMentionCandidates;
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
