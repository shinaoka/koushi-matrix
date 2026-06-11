import { invoke } from "@tauri-apps/api/core";

import { createBrowserFakeApi, type DesktopApi } from "./browserFakeApi";
import type { DesktopSnapshot, SearchScopeKind } from "../domain/types";

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

  async selectSpace(spaceId: string | null): Promise<DesktopSnapshot> {
    return invoke<DesktopSnapshot>("select_space", { spaceId });
  }

  async selectRoom(roomId: string): Promise<DesktopSnapshot> {
    return invoke<DesktopSnapshot>("select_room", { roomId });
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
