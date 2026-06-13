import { invoke } from "@tauri-apps/api/core";

import { createBrowserFakeApi, type DesktopApi } from "./browserFakeApi";
import type { DesktopSnapshot, SavedSessionInfo, SearchScopeKind } from "../domain/types";

export function createDesktopApi(): DesktopApi {
  if (isTauriRuntime()) {
    return new TauriDesktopApi();
  }

  return createBrowserFakeApi();
}

class TauriDesktopApi implements DesktopApi {
  async getSnapshot(): Promise<DesktopSnapshot> {
    return invoke<DesktopSnapshot>("get_snapshot");
  }

  async discoverLoginMethods(homeserver: string): Promise<DesktopSnapshot> {
    return invoke<DesktopSnapshot>("discover_login_methods", { homeserver });
  }

  async submitLogin(
    homeserver: string,
    username: string,
    password: string,
    deviceDisplayName: string
  ): Promise<DesktopSnapshot> {
    return invoke<DesktopSnapshot>("submit_login", {
      homeserver,
      username,
      password,
      deviceDisplayName
    });
  }

  async listSavedSessions(): Promise<SavedSessionInfo[]> {
    return invoke<SavedSessionInfo[]>("list_saved_sessions");
  }

  async switchAccount(session: SavedSessionInfo): Promise<DesktopSnapshot> {
    return invoke<DesktopSnapshot>("switch_account", {
      homeserver: session.homeserver,
      userId: session.user_id,
      deviceId: session.device_id
    });
  }

  async submitRecovery(secret: string): Promise<DesktopSnapshot> {
    return invoke<DesktopSnapshot>("submit_recovery", { secret });
  }

  async restartSync(): Promise<DesktopSnapshot> {
    return invoke<DesktopSnapshot>("restart_sync");
  }

  async selectSpace(spaceId: string | null): Promise<DesktopSnapshot> {
    return invoke<DesktopSnapshot>("select_space", { spaceId });
  }

  async selectRoom(roomId: string): Promise<DesktopSnapshot> {
    return invoke<DesktopSnapshot>("select_room", { roomId });
  }

  async paginateTimelineBackwards(roomId: string): Promise<DesktopSnapshot> {
    return invoke<DesktopSnapshot>("paginate_timeline_backwards", { roomId });
  }

  async sendText(roomId: string, body: string): Promise<DesktopSnapshot> {
    return invoke<DesktopSnapshot>("send_text", { roomId, body });
  }

  async editMessage(roomId: string, eventId: string, body: string): Promise<DesktopSnapshot> {
    return invoke<DesktopSnapshot>("edit_message", { roomId, eventId, body });
  }

  async redactMessage(roomId: string, eventId: string): Promise<DesktopSnapshot> {
    return invoke<DesktopSnapshot>("redact_message", { roomId, eventId });
  }

  async leaveRoom(roomId: string): Promise<DesktopSnapshot> {
    return invoke<DesktopSnapshot>("leave_room", { roomId });
  }

  async forgetRoom(roomId: string): Promise<DesktopSnapshot> {
    return invoke<DesktopSnapshot>("forget_room", { roomId });
  }

  async openThread(roomId: string, rootEventId: string): Promise<DesktopSnapshot> {
    return invoke<DesktopSnapshot>("open_thread", { roomId, rootEventId });
  }

  async closeThread(): Promise<DesktopSnapshot> {
    return invoke<DesktopSnapshot>("close_thread");
  }

  async submitSearch(query: string, scope: SearchScopeKind): Promise<DesktopSnapshot> {
    return invoke<DesktopSnapshot>("submit_search", { query, scope });
  }
}

function isTauriRuntime(): boolean {
  return "__TAURI_INTERNALS__" in window;
}
