import type { PrimaryView } from "../app/uiShared";
import type { PeoplePanelScope, RightPanelMode } from "./rightPanel";

export interface UiNavigationState {
  primaryView: PrimaryView;
  rightPanelMode: RightPanelMode;
  selectedProfileUserId: string | null;
  peoplePanelScope: PeoplePanelScope | null;
}

export type UiNavigationAction =
  | { kind: "setPrimaryView"; primaryView: PrimaryView }
  | { kind: "setRightPanelMode"; mode: RightPanelMode }
  | { kind: "openPeoplePanel"; scope: PeoplePanelScope | null }
  | { kind: "openProfilePanel"; userId: string }
  | { kind: "backToPeople" }
  | { kind: "openActivityEvent" }
  | { kind: "selectSearchResult" }
  | { kind: "openTimelineAtTimestamp" };

const INITIAL: UiNavigationState = {
  primaryView: "timeline",
  rightPanelMode: "closed",
  selectedProfileUserId: null,
  peoplePanelScope: null
};

export function createInitialUiNavigationState(): UiNavigationState {
  return { ...INITIAL };
}

export function uiNavigationReducer(
  state: UiNavigationState,
  action: UiNavigationAction
): UiNavigationState {
  switch (action.kind) {
    case "setPrimaryView":
      return { ...state, primaryView: action.primaryView };

    case "setRightPanelMode": {
      let next = { ...state, rightPanelMode: action.mode };
      if (action.mode !== "profile") {
        next.selectedProfileUserId = null;
      }
      if (action.mode !== "people" && action.mode !== "profile") {
        next.peoplePanelScope = null;
      }
      return next;
    }

    case "openPeoplePanel":
      return {
        ...state,
        rightPanelMode: "people",
        selectedProfileUserId: null,
        peoplePanelScope: action.scope
      };

    case "openProfilePanel":
      return {
        ...state,
        rightPanelMode: "profile",
        selectedProfileUserId: action.userId
      };

    case "backToPeople":
      return {
        ...state,
        rightPanelMode: "people",
        selectedProfileUserId: null
      };

    case "openActivityEvent":
      return {
        primaryView: "timeline",
        rightPanelMode: "closed",
        selectedProfileUserId: null,
        peoplePanelScope: null
      };

    case "selectSearchResult":
      return {
        ...state,
        primaryView: "timeline",
        rightPanelMode: "search"
      };

    case "openTimelineAtTimestamp":
      return {
        ...state,
        primaryView: "timeline",
        rightPanelMode: "focusedContext"
      };
  }
}
