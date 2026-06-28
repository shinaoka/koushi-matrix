import { describe, expect, test } from "vitest";
import {
  createInitialUiNavigationState,
  uiNavigationReducer,
  type UiNavigationAction,
  type UiNavigationState
} from "./uiNavigationState";

function state(overrides?: Partial<UiNavigationState>): UiNavigationState {
  return { ...createInitialUiNavigationState(), ...overrides };
}

function reduce(
  initial: UiNavigationState,
  ...actions: UiNavigationAction[]
): UiNavigationState {
  return actions.reduce(uiNavigationReducer, initial);
}

describe("uiNavigationReducer", () => {
  describe("setPrimaryView", () => {
    test("changes only primaryView, preserves right panel and profile/people state", () => {
      const s = state({
        primaryView: "timeline",
        rightPanelMode: "roomInfo",
        selectedProfileUserId: "@bob:test",
        peoplePanelScope: { kind: "room", roomId: "!room:a" }
      });
      const next = reduce(s, { kind: "setPrimaryView", primaryView: "activity" });
      expect(next.primaryView).toBe("activity");
      expect(next.rightPanelMode).toBe("roomInfo");
      expect(next.selectedProfileUserId).toBe("@bob:test");
      expect(next.peoplePanelScope).toEqual({ kind: "room", roomId: "!room:a" });
    });
  });

  describe("setRightPanelMode", () => {
    test("clears profile and people scope when switching to a non-profile/people mode", () => {
      const s = state({
        selectedProfileUserId: "@alice:test",
        peoplePanelScope: { kind: "room", roomId: "!room:x" }
      });
      const next = reduce(s, { kind: "setRightPanelMode", mode: "roomInfo" });
      expect(next.rightPanelMode).toBe("roomInfo");
      expect(next.selectedProfileUserId).toBeNull();
      expect(next.peoplePanelScope).toBeNull();
    });

    test("when switching to profile, preserves selectedProfileUserId and peoplePanelScope", () => {
      const s = state({
        selectedProfileUserId: "@alice:test",
        peoplePanelScope: { kind: "room", roomId: "!room:x" }
      });
      const next = reduce(s, { kind: "setRightPanelMode", mode: "profile" });
      expect(next.rightPanelMode).toBe("profile");
      expect(next.selectedProfileUserId).toBe("@alice:test");
      expect(next.peoplePanelScope).toEqual({ kind: "room", roomId: "!room:x" });
    });

    test("when switching to people, clears selectedProfileUserId but preserves peoplePanelScope", () => {
      const s = state({
        selectedProfileUserId: "@alice:test",
        peoplePanelScope: { kind: "room", roomId: "!room:x" }
      });
      const next = reduce(s, { kind: "setRightPanelMode", mode: "people" });
      expect(next.rightPanelMode).toBe("people");
      expect(next.selectedProfileUserId).toBeNull();
      expect(next.peoplePanelScope).toEqual({ kind: "room", roomId: "!room:x" });
    });

    test("closing the panel clears both profile and people state", () => {
      const s = state({
        selectedProfileUserId: "@alice:test",
        peoplePanelScope: { kind: "room", roomId: "!room:x" }
      });
      const next = reduce(s, { kind: "setRightPanelMode", mode: "closed" });
      expect(next.rightPanelMode).toBe("closed");
      expect(next.selectedProfileUserId).toBeNull();
      expect(next.peoplePanelScope).toBeNull();
    });

    test("switch to search clears profile and people state", () => {
      const s = state({
        selectedProfileUserId: "@alice:test",
        peoplePanelScope: { kind: "room", roomId: "!room:x" }
      });
      const next = reduce(s, { kind: "setRightPanelMode", mode: "search" });
      expect(next.rightPanelMode).toBe("search");
      expect(next.selectedProfileUserId).toBeNull();
      expect(next.peoplePanelScope).toBeNull();
    });
  });

  describe("openPeoplePanel", () => {
    test("sets people panel mode with given scope and clears profile", () => {
      const s = state({
        selectedProfileUserId: "@alice:test",
        peoplePanelScope: null
      });
      const next = reduce(s, {
        kind: "openPeoplePanel",
        scope: { kind: "room", roomId: "!room:x" }
      });
      expect(next.rightPanelMode).toBe("people");
      expect(next.selectedProfileUserId).toBeNull();
      expect(next.peoplePanelScope).toEqual({ kind: "room", roomId: "!room:x" });
    });

    test("accepts null scope for the else branch", () => {
      const s = state({
        selectedProfileUserId: "@alice:test"
      });
      const next = reduce(s, { kind: "openPeoplePanel", scope: null });
      expect(next.rightPanelMode).toBe("people");
      expect(next.selectedProfileUserId).toBeNull();
      expect(next.peoplePanelScope).toBeNull();
    });
  });

  describe("openProfilePanel", () => {
    test("sets profile mode with the given userId and preserves peoplePanelScope", () => {
      const s = state({
        peoplePanelScope: { kind: "room", roomId: "!room:x" }
      });
      const next = reduce(s, { kind: "openProfilePanel", userId: "@alice:test" });
      expect(next.rightPanelMode).toBe("profile");
      expect(next.selectedProfileUserId).toBe("@alice:test");
      expect(next.peoplePanelScope).toEqual({ kind: "room", roomId: "!room:x" });
    });

    test("opening profile from a non-people context (no prior scope) leaves scope null", () => {
      const s = state({ peoplePanelScope: null });
      const next = reduce(s, { kind: "openProfilePanel", userId: "@bob:test" });
      expect(next.rightPanelMode).toBe("profile");
      expect(next.selectedProfileUserId).toBe("@bob:test");
      expect(next.peoplePanelScope).toBeNull();
    });
  });

  describe("backToPeople", () => {
    test("clears profile userId and switches to people mode, preserving scope", () => {
      const s = state({
        selectedProfileUserId: "@alice:test",
        peoplePanelScope: { kind: "room", roomId: "!room:x" }
      });
      const next = reduce(s, { kind: "backToPeople" });
      expect(next.rightPanelMode).toBe("people");
      expect(next.selectedProfileUserId).toBeNull();
      expect(next.peoplePanelScope).toEqual({ kind: "room", roomId: "!room:x" });
    });
  });

  describe("openActivityEvent (compound)", () => {
    test("switches to timeline primary view, closes right panel, clears profile and people", () => {
      const s = state({
        primaryView: "activity",
        rightPanelMode: "roomInfo",
        selectedProfileUserId: "@alice:test",
        peoplePanelScope: { kind: "room", roomId: "!room:x" }
      });
      const next = reduce(s, { kind: "openActivityEvent" });
      expect(next.primaryView).toBe("timeline");
      expect(next.rightPanelMode).toBe("closed");
      expect(next.selectedProfileUserId).toBeNull();
      expect(next.peoplePanelScope).toBeNull();
    });

    test("resets all four fields regardless of prior state", () => {
      const next = reduce(
        state({
          primaryView: "explore",
          rightPanelMode: "search",
          selectedProfileUserId: "@carol:test",
          peoplePanelScope: { kind: "space", spaceId: "!space:y" }
        }),
        { kind: "openActivityEvent" }
      );
      expect(next).toEqual(createInitialUiNavigationState());
    });

    test("do nothing when activity-event navigation already in clean state", () => {
      const s = createInitialUiNavigationState();
      const next = reduce(s, { kind: "openActivityEvent" });
      expect(next).toEqual(createInitialUiNavigationState());
    });
  });

  describe("selectSearchResult (compound)", () => {
    test("switches to timeline primary view and search right panel", () => {
      const s = state({ primaryView: "activity", rightPanelMode: "closed" });
      const next = reduce(s, { kind: "selectSearchResult" });
      expect(next.primaryView).toBe("timeline");
      expect(next.rightPanelMode).toBe("search");
    });

    test("preserves profile/people state", () => {
      const s = state({
        selectedProfileUserId: "@alice:test",
        peoplePanelScope: { kind: "room", roomId: "!room:x" }
      });
      const next = reduce(s, { kind: "selectSearchResult" });
      expect(next.selectedProfileUserId).toBe("@alice:test");
      expect(next.peoplePanelScope).toEqual({ kind: "room", roomId: "!room:x" });
    });
  });

  describe("openTimelineAtTimestamp (compound)", () => {
    test("switches to timeline primary view and focused context right panel", () => {
      const s = state({ primaryView: "activity", rightPanelMode: "closed" });
      const next = reduce(s, { kind: "openTimelineAtTimestamp" });
      expect(next.primaryView).toBe("timeline");
      expect(next.rightPanelMode).toBe("focusedContext");
    });
  });

  describe("exclusivity: profile vs people", () => {
    test("opening people from profile clears profile", () => {
      const s = state({
        rightPanelMode: "profile",
        selectedProfileUserId: "@alice:test",
        peoplePanelScope: null
      });
      const next = reduce(s, {
        kind: "openPeoplePanel",
        scope: { kind: "room", roomId: "!room:x" }
      });
      expect(next.rightPanelMode).toBe("people");
      expect(next.selectedProfileUserId).toBeNull();
      expect(next.peoplePanelScope).toEqual({ kind: "room", roomId: "!room:x" });
    });

    test("opening profile from people preserves scope for back navigation", () => {
      const s = state({
        rightPanelMode: "people",
        selectedProfileUserId: null,
        peoplePanelScope: { kind: "room", roomId: "!room:x" }
      });
      const next = reduce(s, { kind: "openProfilePanel", userId: "@bob:test" });
      expect(next.rightPanelMode).toBe("profile");
      expect(next.selectedProfileUserId).toBe("@bob:test");
      expect(next.peoplePanelScope).toEqual({ kind: "room", roomId: "!room:x" });
    });
  });

  describe("activity navigation closes panel/profile/people", () => {
    test("openActivityEvent from profile panel state", () => {
      const s = state({
        primaryView: "activity",
        rightPanelMode: "profile",
        selectedProfileUserId: "@alice:test",
        peoplePanelScope: { kind: "room", roomId: "!room:x" }
      });
      const next = reduce(s, { kind: "openActivityEvent" });
      expect(next.primaryView).toBe("timeline");
      expect(next.rightPanelMode).toBe("closed");
      expect(next.selectedProfileUserId).toBeNull();
      expect(next.peoplePanelScope).toBeNull();
    });

    test("openActivityEvent from people panel state", () => {
      const s = state({
        primaryView: "activity",
        rightPanelMode: "people",
        selectedProfileUserId: null,
        peoplePanelScope: { kind: "space", spaceId: "!space:x" }
      });
      const next = reduce(s, { kind: "openActivityEvent" });
      expect(next.primaryView).toBe("timeline");
      expect(next.rightPanelMode).toBe("closed");
      expect(next.selectedProfileUserId).toBeNull();
      expect(next.peoplePanelScope).toBeNull();
    });
  });

  describe("returning to timeline", () => {
    test("setPrimaryView to timeline preserves panel/modal state", () => {
      const s = state({
        primaryView: "activity",
        rightPanelMode: "roomInfo",
        selectedProfileUserId: null,
        peoplePanelScope: null
      });
      const next = reduce(s, { kind: "setPrimaryView", primaryView: "timeline" });
      expect(next.primaryView).toBe("timeline");
      expect(next.rightPanelMode).toBe("roomInfo");
      expect(next.selectedProfileUserId).toBeNull();
      expect(next.peoplePanelScope).toBeNull();
    });
  });

  describe("opening Activity", () => {
    test("setPrimaryView to activity preserves panel/modal state", () => {
      const s = state({
        primaryView: "timeline",
        rightPanelMode: "roomInfo",
        selectedProfileUserId: null,
        peoplePanelScope: null
      });
      const next = reduce(s, { kind: "setPrimaryView", primaryView: "activity" });
      expect(next.primaryView).toBe("activity");
      expect(next.rightPanelMode).toBe("roomInfo");
    });
  });

  describe("reducer purity", () => {
    test("does not mutate the input state", () => {
      const s = state({
        selectedProfileUserId: "@alice:test"
      });
      const frozen = { ...s };
      reduce(s, { kind: "setRightPanelMode", mode: "closed" });
      expect(s).toEqual(frozen);
    });

    test("produces new objects on each dispatch", () => {
      const s = createInitialUiNavigationState();
      const next = reduce(s, { kind: "setPrimaryView", primaryView: "explore" });
      expect(next).not.toBe(s);
    });
  });
});
