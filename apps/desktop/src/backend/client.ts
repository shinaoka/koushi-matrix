import { invoke } from "@tauri-apps/api/core";

import { createBrowserFakeApi, type DesktopApi } from "./browserFakeApi";
import type {
  DesktopSnapshot,
  SavedSessionInfo,
  SearchScopeKind,
  SettingsPatch
} from "../domain/types";

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

  async updateSettings(patch: SettingsPatch): Promise<DesktopSnapshot> {
    return invoke<DesktopSnapshot>("update_settings", { patch });
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

  async setThreadComposerDraft(
    roomId: string,
    rootEventId: string,
    draft: string
  ): Promise<DesktopSnapshot> {
    return invoke<DesktopSnapshot>("set_thread_composer_draft", { roomId, rootEventId, draft });
  }

  async sendThreadReply(
    roomId: string,
    rootEventId: string,
    body: string
  ): Promise<DesktopSnapshot> {
    return invoke<DesktopSnapshot>("send_thread_reply", { roomId, rootEventId, body });
  }

  async selectSearchResult(roomId: string, eventId: string): Promise<DesktopSnapshot> {
    return invoke<DesktopSnapshot>("select_search_result", { roomId, eventId });
  }

  async closeFocusedContext(): Promise<DesktopSnapshot> {
    return invoke<DesktopSnapshot>("close_focused_context");
  }

  async submitSearch(query: string, scope: SearchScopeKind): Promise<DesktopSnapshot> {
    return invoke<DesktopSnapshot>("submit_search", { query, scope });
  }

  async createRoom(name: string): Promise<DesktopSnapshot> {
    return invoke<DesktopSnapshot>("create_room", { name });
  }

  async createSpace(name: string): Promise<DesktopSnapshot> {
    return invoke<DesktopSnapshot>("create_space", { name });
  }

  async setSpaceChild(spaceId: string, childRoomId: string, viaServer: string): Promise<DesktopSnapshot> {
    return invoke<DesktopSnapshot>("set_space_child", { spaceId, childRoomId, viaServer });
  }

  async setComposerReplyTarget(roomId: string, eventId: string): Promise<DesktopSnapshot> {
    return invoke<DesktopSnapshot>("set_composer_reply_target", { roomId, eventId });
  }

  async cancelComposerReply(): Promise<DesktopSnapshot> {
    return invoke<DesktopSnapshot>("cancel_composer_reply");
  }

  async sendReply(roomId: string, inReplyToEventId: string, body: string): Promise<DesktopSnapshot> {
    return invoke<DesktopSnapshot>("send_reply", { roomId, inReplyToEventId, body });
  }
}

function isTauriRuntime(): boolean {
  return "__TAURI_INTERNALS__" in window;
}
