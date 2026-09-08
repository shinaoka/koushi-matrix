// @vitest-environment jsdom

import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type {
  InviteTargetCandidate,
  SpaceMemberEntry,
  SpaceMembersState,
  UserProfile
} from "../domain/types";
import { requestAvatarThumbnailWithDedupe } from "../domain/avatarThumbnails";
import { SpaceMembersPanel } from "./SpaceMembersPanel";

class MockIntersectionObserver {
  static instances: MockIntersectionObserver[] = [];

  private readonly callback: IntersectionObserverCallback;
  private observedElement: Element | null = null;

  constructor(callback: IntersectionObserverCallback) {
    this.callback = callback;
    MockIntersectionObserver.instances.push(this);
  }

  observe(element: Element): void {
    this.observedElement = element;
  }

  unobserve(_element: Element): void {}

  disconnect(): void {}

  takeRecords(): IntersectionObserverEntry[] {
    return [];
  }

  trigger(element = this.observedElement): void {
    if (!element || this.observedElement !== element) {
      return;
    }
    this.callback(
      [
        {
          isIntersecting: true,
          intersectionRatio: 1,
          target: element
        } as IntersectionObserverEntry
      ],
      this as unknown as IntersectionObserver
    );
  }
}

beforeEach(() => {
  MockIntersectionObserver.instances = [];
  vi.stubGlobal("IntersectionObserver", MockIntersectionObserver);
});

afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
  vi.useRealTimers();
});

function member(
  userId: string,
  displayLabel: string,
  membership: SpaceMemberEntry["membership"],
  overrides: Partial<SpaceMemberEntry> = {}
): SpaceMemberEntry {
  return {
    user_id: userId,
    display_name: displayLabel,
    display_label: displayLabel,
    original_display_label: displayLabel,
    avatar_url: null,
    power_level: null,
    role: "user",
    membership,
    child_room_ids: [],
    invite_pending: false,
    role_options: [],
    ...overrides
  };
}

function state(overrides: Partial<SpaceMembersState> = {}): SpaceMembersState {
  return {
    selected_space_id: "!space:example.invalid",
    generation: 4,
    space_joined: [
      member("@alice:example.invalid", "Alice", "space_joined", {
        original_display_label: "Alicia"
      })
    ],
    space_invited: [member("@bob:example.invalid", "Bob", "space_invited")],
    child_room_only: [
      member("@carol:example.invalid", "Carol", "child_room_only", {
        child_room_ids: ["!room-alpha:example.invalid", "!room-beta:example.invalid"]
      })
    ],
    child_room_count: 2,
    complete_child_room_count: 2,
    incomplete_child_room_count: 0,
    power_levels_revision: null,
    can_edit_roles: false,
    operation: { kind: "idle" },
    ...overrides
  };
}

function profile(userId: string, avatar: UserProfile["avatar"]): UserProfile {
  return {
    user_id: userId,
    display_name: "Alice",
    display_label: "Alice",
    original_display_label: "Alice",
    mention_search_terms: ["alice"],
    avatar
  };
}

describe("SpaceMembersPanel space invite search (#508)", () => {
  const candidate = (
    status: InviteTargetCandidate["status"] = "selectable",
    userId = "@new:example.invalid"
  ): InviteTargetCandidate => ({
    user_id: userId,
    display_label: "New Person",
    original_display_label: "New Person",
    avatar: null,
    source: "profile",
    status,
    status_message: null
  });

  it("opens the invite search, resolves candidates, and invites a brand-new user", async () => {
    const onInviteUser = vi.fn();
    const onInviteSearchCandidate = vi.fn();
    const onSearchInviteTargets = vi.fn(async () => [candidate()]);
    render(
      <SpaceMembersPanel
        state={state()}
        canInvite={true}
        onInviteUser={onInviteUser}
        onInviteSearchCandidate={onInviteSearchCandidate}
        onSearchInviteTargets={onSearchInviteTargets}
        onOpenProfile={vi.fn()}
      />
    );

    fireEvent.click(screen.getByRole("button", { name: "Invite people" }));
    const input = screen.getByRole("searchbox", { name: "Name, alias, or Matrix ID" });
    fireEvent.change(input, { target: { value: "new" } });

    await waitFor(() =>
      expect(screen.getByRole("button", { name: /New Person/ })).toBeTruthy()
    );
    expect(onSearchInviteTargets).toHaveBeenCalledWith("new");

    fireEvent.click(screen.getByRole("button", { name: /New Person/ }));
    expect(onInviteSearchCandidate).toHaveBeenCalledWith("@new:example.invalid");
    expect(onInviteUser).not.toHaveBeenCalled();
  });

  it("hides the invite trigger without permission and disables non-selectable candidates", async () => {
    const onInviteUser = vi.fn();
    const onInviteSearchCandidate = vi.fn();
    const onSearchInviteTargets = vi.fn(async () => [
      candidate("alreadyInDestination"),
      candidate("selectable", "@other-new:example.invalid")
    ]);
    const { rerender } = render(
      <SpaceMembersPanel
        state={state()}
        canInvite={false}
        onInviteUser={onInviteUser}
        onInviteSearchCandidate={onInviteSearchCandidate}
        onSearchInviteTargets={onSearchInviteTargets}
        onOpenProfile={vi.fn()}
      />
    );
    expect(screen.queryByRole("button", { name: "Invite people" })).toBeNull();

    rerender(
      <SpaceMembersPanel
        state={state()}
        canInvite={true}
        onInviteUser={onInviteUser}
        onSearchInviteTargets={onSearchInviteTargets}
        onOpenProfile={vi.fn()}
      />
    );
    fireEvent.click(screen.getByRole("button", { name: "Invite people" }));
    fireEvent.change(screen.getByRole("searchbox", { name: "Name, alias, or Matrix ID" }), {
      target: { value: "new" }
    });
    await waitFor(() => expect(screen.getAllByRole("button", { name: /New Person/ }).length).toBeGreaterThan(0));
    const buttons = screen.getAllByRole("button", { name: /New Person/ });
    expect(buttons[0]).toHaveProperty("disabled", true);
    expect(buttons[1]).toHaveProperty("disabled", false);
    fireEvent.click(buttons[0]!);
    expect(onInviteSearchCandidate).not.toHaveBeenCalled();
  });

  it("debounces the invite search and sends only the latest query", async () => {
    vi.useFakeTimers();
    const onSearchInviteTargets = vi.fn(async () => []);
    render(
      <SpaceMembersPanel
        state={state()}
        canInvite={true}
        onInviteUser={vi.fn()}
        onSearchInviteTargets={onSearchInviteTargets}
        onOpenProfile={vi.fn()}
      />
    );
    fireEvent.click(screen.getByRole("button", { name: "Invite people" }));
    const input = screen.getByRole("searchbox", { name: "Name, alias, or Matrix ID" });

    fireEvent.change(input, { target: { value: "a" } });
    fireEvent.change(input, { target: { value: "ab" } });
    // Typing before the debounce elapses must not fire the search.
    await act(async () => {
      vi.advanceTimersByTime(249);
    });
    expect(onSearchInviteTargets).not.toHaveBeenCalled();

    await act(async () => {
      vi.advanceTimersByTime(1);
    });
    expect(onSearchInviteTargets).toHaveBeenCalledTimes(1);
    expect(onSearchInviteTargets).toHaveBeenCalledWith("ab");
  });

  it("discards a stale search response when a newer query wins", async () => {
    vi.useFakeTimers();
    const resolvers: Array<(value: InviteTargetCandidate[]) => void> = [];
    const onSearchInviteTargets = vi.fn(
      () => new Promise<InviteTargetCandidate[]>((resolve) => resolvers.push(resolve))
    );
    render(
      <SpaceMembersPanel
        state={state()}
        canInvite={true}
        onInviteUser={vi.fn()}
        onSearchInviteTargets={onSearchInviteTargets}
        onOpenProfile={vi.fn()}
      />
    );
    fireEvent.click(screen.getByRole("button", { name: "Invite people" }));
    const input = screen.getByRole("searchbox", { name: "Name, alias, or Matrix ID" });

    fireEvent.change(input, { target: { value: "first" } });
    await act(async () => {
      vi.advanceTimersByTime(250);
    });
    fireEvent.change(input, { target: { value: "second" } });
    await act(async () => {
      vi.advanceTimersByTime(250);
    });
    expect(resolvers.length).toBe(2);

    // The older response resolves after the newer one's query was sent: its
    // result must be discarded.
    await act(async () => {
      resolvers[0]!([candidate("selectable")]);
    });
    expect(screen.queryByRole("button", { name: /New Person/ })).toBeNull();

    await act(async () => {
      resolvers[1]!([
        {
          ...candidate("selectable"),
          user_id: "@winner:example.invalid",
          display_label: "Winner"
        }
      ]);
    });
    expect(screen.getByRole("button", { name: /Winner/ })).toBeTruthy();
  });

  it("cancels back out of invite search without inviting", async () => {
    const onInviteUser = vi.fn();
    render(
      <SpaceMembersPanel
        state={state()}
        canInvite={true}
        onInviteUser={onInviteUser}
        onSearchInviteTargets={async () => []}
        onOpenProfile={vi.fn()}
      />
    );
    fireEvent.click(screen.getByRole("button", { name: "Invite people" }));
    expect(screen.getByRole("searchbox", { name: "Name, alias, or Matrix ID" })).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
    expect(screen.queryByRole("searchbox", { name: "Name, alias, or Matrix ID" })).toBeNull();
    expect(onInviteUser).not.toHaveBeenCalled();
  });
});

describe("SpaceMembersPanel", () => {
  it("renders cancellation only for invited rows and forwards the invited user", () => {
    const onCancelInvite = vi.fn();
    const diagnostics: string[] = [];
    render(
      <SpaceMembersPanel
        state={state()}
        canInvite={true}
        canCancelInvite={true}
        cancelAvailabilityReason="available"
        onCancelInvite={onCancelInvite}
        onInviteUser={vi.fn()}
        onOpenProfile={vi.fn()}
        onDiagnostic={(message) => diagnostics.push(message)}
      />
    );

    expect(screen.getByRole("button", { name: "Cancel invitation" })).toBeTruthy();
    expect(screen.getAllByRole("button", { name: "Cancel invitation" })).toHaveLength(1);

    fireEvent.click(screen.getByRole("button", { name: "Cancel invitation" }));

    expect(onCancelInvite).toHaveBeenCalledWith("@bob:example.invalid");
    expect(diagnostics).toContain("cancel trigger=inline availability_reason=available");
    expect(diagnostics.join("\n")).not.toMatch(/@bob|Bob|mxc:|https?:/);
  });

  it("disables cancellation without kick permission or while a member operation is pending", () => {
    const onCancelInvite = vi.fn();
    const { rerender } = render(
      <SpaceMembersPanel
        state={state()}
        canInvite={true}
        canCancelInvite={false}
        cancelAvailabilityReason="permission_denied"
        onCancelInvite={onCancelInvite}
        onInviteUser={vi.fn()}
        onOpenProfile={vi.fn()}
      />
    );

    expect(screen.getByRole("button", { name: "Cancel invitation" })).toHaveProperty(
      "disabled",
      true
    );

    rerender(
      <SpaceMembersPanel
        state={state({
          operation: {
            kind: "loading",
            request_id: 12,
            space_id: "!space:example.invalid",
            generation: 4
          }
        })}
        canInvite={true}
        canCancelInvite={true}
        cancelAvailabilityReason="operation_pending"
        onCancelInvite={onCancelInvite}
        onInviteUser={vi.fn()}
        onOpenProfile={vi.fn()}
      />
    );

    expect(screen.getByRole("button", { name: "Cancel invitation" })).toHaveProperty(
      "disabled",
      true
    );
    expect(onCancelInvite).not.toHaveBeenCalled();
  });

  it("keeps the invited row visible and labels its own cancellation as pending", () => {
    render(
      <SpaceMembersPanel
        state={state({
          operation: {
            kind: "cancellingInvite",
            request_id: 13,
            space_id: "!space:example.invalid",
            user_id: "@bob:example.invalid",
            generation: 4
          }
        })}
        canInvite={true}
        canCancelInvite={true}
        cancelAvailabilityReason="operation_pending"
        onCancelInvite={vi.fn()}
        onInviteUser={vi.fn()}
        onOpenProfile={vi.fn()}
      />
    );

    expect(screen.getByText("Bob")).toBeTruthy();
    expect(screen.getByRole("button", { name: "Cancelling…" })).toHaveProperty(
      "disabled",
      true
    );
  });

  it("reports a localized cancellation failure while retaining the invited row", () => {
    render(
      <SpaceMembersPanel
        state={state({
          operation: {
            kind: "failed",
            request_id: 14,
            space_id: "!space:example.invalid",
            user_id: "@bob:example.invalid",
            generation: 4,
            failureKind: "network"
          }
        })}
        canInvite={true}
        canCancelInvite={true}
        onCancelInvite={vi.fn()}
        onInviteUser={vi.fn()}
        onOpenProfile={vi.fn()}
      />
    );

    expect(screen.getByRole("alert").textContent).toBe(
      "Could not cancel the invitation. Try again."
    );
    expect(screen.getByText("Bob")).toBeTruthy();
  });

  it("closes the Space members panel and renders elevated role badges", () => {
    const onClose = vi.fn();
    render(
      <SpaceMembersPanel
        state={state({
          space_joined: [
            member("@administrator:example.invalid", "Ada", "space_joined", {
              role: "administrator"
            }),
            member("@creator:example.invalid", "Cora", "space_joined", {
              role: "creator"
            })
          ],
          space_invited: [],
          child_room_only: []
        })}
        canInvite={true}
        onClose={onClose}
        onInviteUser={vi.fn()}
        onOpenProfile={vi.fn()}
      />
    );

    fireEvent.click(screen.getByRole("button", { name: "Close Space members" }));

    expect(onClose).toHaveBeenCalledTimes(1);
    expect(screen.getByText("Administrator")).toBeTruthy();
    expect(screen.getByText("Creator")).toBeTruthy();
  });

  it("renders a ready cached avatar and keeps deterministic initials after image failure", () => {
    render(
      <SpaceMembersPanel
        state={state()}
        canInvite={true}
        onInviteUser={vi.fn()}
        onOpenProfile={vi.fn()}
        profileUsers={{
          "@alice:example.invalid": profile("@alice:example.invalid", {
            mxc_uri: "mxc://example.invalid/alice-avatar",
            thumbnail: {
              kind: "ready",
              source_ref: "asset://alice-avatar",
              width: null,
              height: null,
              mime_type: null
            }
          }),
          "@bob:example.invalid": profile("@bob:example.invalid", {
            mxc_uri: "mxc://example.invalid/bob-avatar",
            thumbnail: {
              kind: "failed",
              request_id: 7,
              failureKind: "network"
            }
          })
        }}
      />
    );

    const avatar = screen.getByRole("img", { name: "" });
    const image = avatar.querySelector("img");
    expect(image?.getAttribute("src")).toBe("asset://alice-avatar");
    expect(screen.getByText("BO")).toBeTruthy();

    fireEvent.error(image!);
    expect(screen.getByText("AL")).toBeTruthy();
  });

  it("requests an unresolved avatar only once after its row becomes visible", () => {
    const onRequestAvatarThumbnail = vi.fn();
    render(
      <SpaceMembersPanel
        state={state()}
        canInvite={true}
        onInviteUser={vi.fn()}
        onOpenProfile={vi.fn()}
        onRequestAvatarThumbnail={onRequestAvatarThumbnail}
        profileUsers={{
          "@alice:example.invalid": profile("@alice:example.invalid", {
            mxc_uri: "mxc://example.invalid/alice-avatar",
            thumbnail: { kind: "notRequested" }
          })
        }}
      />
    );

    expect(onRequestAvatarThumbnail).not.toHaveBeenCalled();
    const row = screen.getByText("Alice").closest("li");
    expect(row).not.toBeNull();
    expect(MockIntersectionObserver.instances).toHaveLength(1);

    MockIntersectionObserver.instances[0]?.trigger(row);
    MockIntersectionObserver.instances[0]?.trigger(row);

    expect(onRequestAvatarThumbnail).toHaveBeenCalledTimes(1);
    expect(onRequestAvatarThumbnail).toHaveBeenCalledWith(
      "mxc://example.invalid/alice-avatar"
    );
  });

  it("deduplicates member avatar requests across filter remounts and retries rejection", async () => {
    const requestedMxcUris = new Set<string>();
    const memberRequestedMxcUris = new Set<string>();
    let rejectFirst: ((reason?: unknown) => void) | undefined;
    const request = vi
      .fn<(mxcUri: string) => Promise<void>>()
      .mockImplementationOnce(
        () =>
          new Promise<void>((_resolve, reject) => {
            rejectFirst = reject;
          })
      )
      .mockResolvedValue(undefined);
    const onRequestAvatarThumbnail = (mxcUri: string) =>
      requestAvatarThumbnailWithDedupe(
        mxcUri,
        requestedMxcUris,
        memberRequestedMxcUris,
        request
      );

    render(
      <SpaceMembersPanel
        state={state()}
        canInvite={true}
        onInviteUser={vi.fn()}
        onOpenProfile={vi.fn()}
        onRequestAvatarThumbnail={onRequestAvatarThumbnail}
        profileUsers={{
          "@alice:example.invalid": profile("@alice:example.invalid", {
            mxc_uri: "mxc://example.invalid/alice-avatar",
            thumbnail: { kind: "notRequested" }
          })
        }}
      />
    );

    const search = screen.getByRole("searchbox", { name: "Search space members" });
    const firstRow = screen.getByText("Alice").closest("li");
    MockIntersectionObserver.instances[0]?.trigger(firstRow);
    expect(request).toHaveBeenCalledTimes(1);

    fireEvent.change(search, { target: { value: "nobody" } });
    fireEvent.change(search, { target: { value: "Alice" } });
    const remountedRow = screen.getByText("Alice").closest("li");
    MockIntersectionObserver.instances.at(-1)?.trigger(remountedRow);
    expect(request).toHaveBeenCalledTimes(1);

    await act(async () => {
      rejectFirst?.(new Error("temporary thumbnail failure"));
      await Promise.resolve();
    });
    fireEvent.change(search, { target: { value: "nobody" } });
    fireEvent.change(search, { target: { value: "Alice" } });
    MockIntersectionObserver.instances.at(-1)?.trigger(screen.getByText("Alice").closest("li"));

    expect(request).toHaveBeenCalledTimes(2);
  });

  it("does not retry a failed avatar after Core reaches a terminal state", () => {
    const requestedMxcUris = new Set<string>();
    const memberRequestedMxcUris = new Set<string>();
    const request = vi.fn<(mxcUri: string) => Promise<void>>().mockResolvedValue(undefined);
    const requestMemberAvatar = (mxcUri: string) =>
      requestAvatarThumbnailWithDedupe(
        mxcUri,
        requestedMxcUris,
        memberRequestedMxcUris,
        request
      );
    const aliceMxcUri = "mxc://example.invalid/alice-avatar";
    const bobMxcUri = "mxc://example.invalid/bob-avatar";
    const profileUsers = (aliceThumbnail: NonNullable<UserProfile["avatar"]>["thumbnail"]) => ({
      "@alice:example.invalid": profile("@alice:example.invalid", {
        mxc_uri: aliceMxcUri,
        thumbnail: aliceThumbnail
      }),
      "@bob:example.invalid": profile("@bob:example.invalid", {
        mxc_uri: bobMxcUri,
        thumbnail: { kind: "notRequested" }
      })
    });
    const { rerender } = render(
      <SpaceMembersPanel
        state={state()}
        canInvite={true}
        onInviteUser={vi.fn()}
        onOpenProfile={vi.fn()}
        onRequestAvatarThumbnail={requestMemberAvatar}
        profileUsers={profileUsers({ kind: "notRequested" })}
      />
    );

    MockIntersectionObserver.instances[0]?.trigger(screen.getByText("Alice").closest("li"));
    expect(request).toHaveBeenCalledTimes(1);
    expect(request).toHaveBeenLastCalledWith(aliceMxcUri);

    rerender(
      <SpaceMembersPanel
        state={state()}
        canInvite={true}
        onInviteUser={vi.fn()}
        onOpenProfile={vi.fn()}
        onRequestAvatarThumbnail={requestMemberAvatar}
        profileUsers={profileUsers({
          kind: "loading",
          request_id: 1
        })}
      />
    );
    rerender(
      <SpaceMembersPanel
        state={state()}
        canInvite={true}
        onInviteUser={vi.fn()}
        onOpenProfile={vi.fn()}
        onRequestAvatarThumbnail={requestMemberAvatar}
        profileUsers={profileUsers({
          kind: "failed",
          request_id: 1,
          failureKind: "network"
        })}
      />
    );
    const search = screen.getByRole("searchbox", { name: "Search space members" });
    fireEvent.change(search, { target: { value: "nobody" } });
    fireEvent.change(search, { target: { value: "Alice" } });
    const visibleAliceRow = screen.getByText("Alice").closest("li");
    MockIntersectionObserver.instances.forEach((observer) => observer.trigger(visibleAliceRow));

    expect(request).toHaveBeenCalledTimes(1);
    expect(request).toHaveBeenLastCalledWith(aliceMxcUri);
    expect(request.mock.calls.flat()).not.toContain(bobMxcUri);
  });

  it("does not observe non-retryable failed member avatars", () => {
    const request = vi.fn<(mxcUri: string) => Promise<void>>().mockResolvedValue(undefined);
    render(
      <SpaceMembersPanel
        state={state()}
        canInvite={true}
        onInviteUser={vi.fn()}
        onOpenProfile={vi.fn()}
        onRequestAvatarThumbnail={request}
        profileUsers={{
          "@alice:example.invalid": profile("@alice:example.invalid", {
            mxc_uri: "mxc://example.invalid/alice-avatar",
            thumbnail: {
              kind: "failed",
              request_id: 1,
              failureKind: "forbidden"
            }
          })
        }}
      />
    );

    expect(MockIntersectionObserver.instances).toHaveLength(0);
    expect(request).not.toHaveBeenCalled();
  });

  it("renders the classified sections in Space, pending, then child-room order", () => {
    render(
      <SpaceMembersPanel
        state={state()}
        canInvite={true}
        onInviteUser={vi.fn()}
        onOpenProfile={vi.fn()}
      />
    );

    expect(screen.getAllByRole("heading", { level: 3 }).map((heading) => heading.textContent)).toEqual([
      "Space members",
      "Invitation pending",
      "Not in Space"
    ]);
    expect(screen.getByText("Alice")).toBeTruthy();
    expect(screen.getByText("Bob")).toBeTruthy();
    expect(screen.getByText("Carol")).toBeTruthy();
  });

  it("searches all sections by label, original label, and user id", () => {
    render(
      <SpaceMembersPanel
        state={state()}
        canInvite={true}
        onInviteUser={vi.fn()}
        onOpenProfile={vi.fn()}
      />
    );
    const search = screen.getByRole("searchbox", { name: "Search space members" });

    fireEvent.change(search, { target: { value: "Alicia" } });
    expect(screen.getByText("Alice")).toBeTruthy();
    expect(screen.queryByText("Bob")).toBeNull();
    expect(screen.queryByText("Carol")).toBeNull();

    fireEvent.change(search, { target: { value: "@carol:example.invalid" } });
    expect(screen.getByText("Carol")).toBeTruthy();
    expect(screen.queryByText("Alice")).toBeNull();
  });

  it("uses compact child-room labels without exposing raw room ids", () => {
    const onInviteUser = vi.fn();
    render(
      <SpaceMembersPanel
        state={state()}
        canInvite={true}
        onInviteUser={onInviteUser}
        onOpenProfile={vi.fn()}
        childRoomLabels={new Map([
          ["!room-alpha:example.invalid", "Alpha"],
          ["!room-beta:example.invalid", "Beta"]
        ])}
      />
    );

    expect(screen.getByText("In child rooms: Alpha, Beta")).toBeTruthy();
    expect(screen.queryByText(/!room-alpha:example\.invalid/)).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "Invite to Space" }));

    expect(onInviteUser).toHaveBeenCalledWith("@carol:example.invalid");
  });

  it("uses a localized count for more than two child rooms without exposing ids", () => {
    render(
      <SpaceMembersPanel
        state={state({
          child_room_only: [
            member("@carol:example.invalid", "Carol", "child_room_only", {
              child_room_ids: [
                "!room-alpha:example.invalid",
                "!room-beta:example.invalid",
                "!room-gamma:example.invalid"
              ]
            })
          ],
          child_room_count: 3
        })}
        canInvite={true}
        onInviteUser={vi.fn()}
        onOpenProfile={vi.fn()}
        childRoomLabels={new Map([
          ["!room-alpha:example.invalid", "Alpha"],
          ["!room-beta:example.invalid", "Beta"],
          ["!room-gamma:example.invalid", "Gamma"]
        ])}
      />
    );

    expect(screen.getByText("In 3 child rooms")).toBeTruthy();
    expect(screen.queryByText(/!room-(alpha|beta|gamma):example\.invalid/)).toBeNull();
  });

  it("uses a singular English fallback for one child room", () => {
    render(
      <SpaceMembersPanel
        state={state({
          child_room_only: [
            member("@carol:example.invalid", "Carol", "child_room_only", {
              child_room_ids: ["!room-alpha:example.invalid"]
            })
          ],
          child_room_count: 1
        })}
        canInvite={true}
        onInviteUser={vi.fn()}
        onOpenProfile={vi.fn()}
      />
    );

    expect(screen.getByText("In 1 child room")).toBeTruthy();
  });

  it("announces invite failure and keeps the child-only row retryable", () => {
    const onInviteUser = vi.fn();
    render(
      <SpaceMembersPanel
        state={state({
          operation: {
            kind: "failed",
            request_id: 12,
            space_id: "!space:example.invalid",
            user_id: "@carol:example.invalid",
            generation: 4,
            failureKind: "network"
          }
        })}
        canInvite={true}
        onInviteUser={onInviteUser}
        onOpenProfile={vi.fn()}
      />
    );

    expect(screen.getByRole("alert").textContent).toMatch(/invite failed/i);
    const inviteButton = screen.getByRole("button", { name: "Invite to Space" });
    expect((inviteButton as HTMLButtonElement).disabled).toBe(false);
    fireEvent.click(inviteButton);
    expect(onInviteUser).toHaveBeenCalledWith("@carol:example.invalid");
  });

  it("uses member-load failure copy when no invite user is associated", () => {
    render(
      <SpaceMembersPanel
        state={state({
          operation: {
            kind: "failed",
            request_id: 13,
            space_id: "!space:example.invalid",
            user_id: null,
            generation: 4,
            failureKind: "network"
          }
        })}
        canInvite={true}
        onInviteUser={vi.fn()}
        onOpenProfile={vi.fn()}
      />
    );

    expect(screen.getByRole("alert").textContent).toMatch(/member load failed/i);
    expect(screen.getByRole("alert").textContent).not.toMatch(/invite failed/i);
  });

  it("forwards child-only row context menus with the exact Space target fence", () => {
    const onOpenContextMenu = vi.fn();
    render(
      <SpaceMembersPanel
        state={state()}
        canInvite={true}
        onInviteUser={vi.fn()}
        onOpenProfile={vi.fn()}
        onOpenContextMenu={onOpenContextMenu}
      />
    );

    fireEvent.contextMenu(screen.getByText("Carol").closest("li")!);

    expect(onOpenContextMenu).toHaveBeenCalledTimes(1);
    expect(onOpenContextMenu.mock.calls[0]?.[1]).toEqual({
      kind: "spaceMember",
      spaceId: "!space:example.invalid",
      userId: "@carol:example.invalid",
      generation: 4
    });
    expect(
      (onOpenContextMenu.mock.calls[0]?.[2] as Array<{ id: string }>).map((item) => item.id)
    ).toEqual(["inviteUserToSpace"]);
  });

  it("disables invite actions while pending or when the caller lacks permission", () => {
    const pendingState = state({
      child_room_only: [
        member("@carol:example.invalid", "Carol", "child_room_only", {
          invite_pending: true
        })
      ]
    });
    const { rerender } = render(
      <SpaceMembersPanel
        state={pendingState}
        canInvite={true}
        onInviteUser={vi.fn()}
        onOpenProfile={vi.fn()}
      />
    );
    expect(screen.getByRole("button", { name: "Invite to Space" }).hasAttribute("disabled")).toBe(
      true
    );

    rerender(
      <SpaceMembersPanel
        state={state()}
        canInvite={false}
        onInviteUser={vi.fn()}
        onOpenProfile={vi.fn()}
      />
    );
    expect(screen.getByRole("button", { name: "Invite to Space" }).hasAttribute("disabled")).toBe(
      true
    );
  });

  it("announces incomplete child-room synchronization from Rust state", () => {
    render(
      <SpaceMembersPanel
        state={state({ incomplete_child_room_count: 1 })}
        canInvite={true}
        onInviteUser={vi.fn()}
        onOpenProfile={vi.fn()}
      />
    );

    expect(screen.getByRole("status").textContent).toContain("Some child rooms are still syncing");
  });

  it("renders a useful empty state when search matches no section", () => {
    render(
      <SpaceMembersPanel
        state={state()}
        canInvite={true}
        onInviteUser={vi.fn()}
        onOpenProfile={vi.fn()}
      />
    );
    fireEvent.change(screen.getByRole("searchbox"), { target: { value: "nobody" } });

    expect(screen.getByRole("status").textContent).toContain("No space members found");
  });

  it("opens a member profile from an accessible row action", () => {
    const onOpenProfile = vi.fn();
    render(
      <SpaceMembersPanel
        state={state()}
        canInvite={true}
        onInviteUser={vi.fn()}
        onOpenProfile={onOpenProfile}
      />
    );

    fireEvent.click(screen.getByRole("button", { name: "Open profile for Alice" }));

    expect(onOpenProfile).toHaveBeenCalledWith("@alice:example.invalid");
  });

  it("disables invite actions while any Rust-owned operation is pending", () => {
    const pendingOperation = state({
      operation: {
        kind: "loading",
        request_id: 8,
        space_id: "!space:example.invalid",
        generation: 4
      }
    });

    render(
      <SpaceMembersPanel
        state={pendingOperation}
        canInvite={true}
        onInviteUser={vi.fn()}
        onOpenProfile={vi.fn()}
      />
    );

    expect(
      screen.getByRole("button", { name: "Invite to Space" }).hasAttribute("disabled")
    ).toBe(true);
  });

  it("treats a Rust-owned invite cancellation as a pending operation", () => {
    const pendingOperation: SpaceMembersState["operation"] = {
      kind: "cancellingInvite",
      request_id: 10,
      space_id: "!space:example.invalid",
      user_id: "@bob:example.invalid",
      generation: 4
    };
    const diagnostics: string[] = [];

    render(
      <SpaceMembersPanel
        state={state({ operation: pendingOperation })}
        canInvite={true}
        onInviteUser={vi.fn()}
        onOpenProfile={vi.fn()}
        onDiagnostic={(message) => diagnostics.push(message)}
      />
    );

    expect(
      screen.getByRole("button", { name: "Invite to Space" }).hasAttribute("disabled")
    ).toBe(true);
    expect(diagnostics.some((message) => message.includes("availability_reason=operation_pending"))).toBe(
      true
    );
  });

  it("disables the in-flight invite target from state even without an entry flag", () => {
    const invitingState = state({
      operation: {
        kind: "inviting",
        request_id: 9,
        space_id: "!space:example.invalid",
        user_id: "@carol:example.invalid",
        generation: 4
      },
      child_room_only: [
        member("@carol:example.invalid", "Carol", "child_room_only", {
          invite_pending: false
        })
      ]
    });

    render(
      <SpaceMembersPanel
        state={invitingState}
        canInvite={true}
        onInviteUser={vi.fn()}
        onOpenProfile={vi.fn()}
      />
    );

    expect(
      screen.getByRole("button", { name: "Invite to Space" }).hasAttribute("disabled")
    ).toBe(true);
  });

  it("records only private-data-free UI diagnostic facts", () => {
    const diagnostics: string[] = [];
    render(
      <SpaceMembersPanel
        state={state({
          space_joined: [
            member("@private-user:example.invalid", "Private Person", "space_joined", {
              avatar_url: "mxc://example.invalid/private-avatar"
            })
          ],
          child_room_only: [
            member("@private-child:example.invalid", "Private Child", "child_room_only", {
              child_room_ids: ["!private-room:example.invalid"]
            })
          ],
          incomplete_child_room_count: 1
        })}
        canInvite={true}
        onInviteUser={vi.fn()}
        onOpenProfile={vi.fn()}
        onDiagnostic={(message) => diagnostics.push(message)}
      />
    );

    fireEvent.change(screen.getByRole("searchbox"), { target: { value: "Private" } });
    fireEvent.click(screen.getByRole("button", { name: "Invite to Space" }));

    const joined = diagnostics.join("\n");
    expect(joined).toContain("rendered");
    expect(joined).toContain("search");
    expect(joined).toContain("availability");
    expect(joined).toContain("incomplete_notice=true");
    for (const privateValue of [
      "@private-user:example.invalid",
      "@private-child:example.invalid",
      "Private Person",
      "Private Child",
      "!private-room:example.invalid",
      "mxc://example.invalid/private-avatar"
    ]) {
      expect(joined).not.toContain(privateValue);
    }
  });

  it("renders a native role select for authorized options and dispatches a selected role", () => {
    const onUpdateRole = vi.fn();
    render(
      <SpaceMembersPanel
        state={state({
          power_levels_revision: "revision-1",
          can_edit_roles: true,
          space_joined: [
            member("@alice:example.invalid", "Alice", "space_joined", {
              power_level: 0,
              role: "user",
              role_options: [
                { power_level: 50, role: "moderator", requires_confirmation: false },
                { power_level: 100, role: "administrator", requires_confirmation: true }
              ]
            })
          ]
        })}
        canInvite={true}
        onInviteUser={vi.fn()}
        onOpenProfile={vi.fn()}
        onUpdateRole={onUpdateRole}
      />
    );

    const select = screen.getByRole("combobox", { name: "Role for Alice" });
    expect(select).toBeTruthy();
    fireEvent.change(select, { target: { value: "50" } });
    expect((select as HTMLSelectElement).value).toBe("0");
    expect(onUpdateRole).toHaveBeenCalledWith("@alice:example.invalid", {
      power_level: 50,
      role: "moderator",
      requires_confirmation: false
    });
  });

  it("requires confirmation for an administrator change and keeps Cancel inert", () => {
    const onUpdateRole = vi.fn();
    render(
      <SpaceMembersPanel
        state={state({
          power_levels_revision: "revision-1",
          can_edit_roles: true,
          space_joined: [
            member("@alice:example.invalid", "Alice", "space_joined", {
              power_level: 0,
              role: "user",
              role_options: [
                { power_level: 100, role: "administrator", requires_confirmation: true }
              ]
            })
          ]
        })}
        canInvite={true}
        onInviteUser={vi.fn()}
        onOpenProfile={vi.fn()}
        onUpdateRole={onUpdateRole}
      />
    );

    const select = screen.getByRole("combobox", { name: "Role for Alice" });
    fireEvent.change(select, { target: { value: "100" } });
    expect(screen.getByRole("dialog", { name: "Confirm role change" })).toBeTruthy();
    expect(onUpdateRole).not.toHaveBeenCalled();

    fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
    expect(onUpdateRole).not.toHaveBeenCalled();
    expect((select as HTMLSelectElement).value).toBe("0");

    fireEvent.change(select, { target: { value: "100" } });
    fireEvent.click(screen.getByRole("button", { name: "Confirm role change" }));
    expect(onUpdateRole).toHaveBeenCalledWith("@alice:example.invalid", {
      power_level: 100,
      role: "administrator",
      requires_confirmation: true
    });
  });

  it("does not render role controls for an unauthorized Space projection", () => {
    render(
      <SpaceMembersPanel
        state={state({
          can_edit_roles: false,
          space_joined: [
            member("@alice:example.invalid", "Alice", "space_joined", {
              power_level: 0,
              role_options: [{ power_level: 50, role: "moderator", requires_confirmation: false }]
            })
          ]
        })}
        canInvite={true}
        onInviteUser={vi.fn()}
        onOpenProfile={vi.fn()}
      />
    );

    expect(screen.queryByRole("combobox")).toBeNull();
  });

  it("keeps authorized role controls enabled while child rooms are syncing", () => {
    render(
      <SpaceMembersPanel
        state={state({
          incomplete_child_room_count: 1,
          can_edit_roles: true,
          power_levels_revision: "revision-1",
          space_joined: [
            member("@alice:example.invalid", "Alice", "space_joined", {
              power_level: 0,
              role_options: [{ power_level: 50, role: "moderator", requires_confirmation: false }]
            })
          ]
        })}
        canInvite={true}
        onInviteUser={vi.fn()}
        onOpenProfile={vi.fn()}
      />
    );

    expect(screen.getByRole("status").textContent).toContain("Some child rooms are still syncing");
    expect(screen.getByRole("combobox", { name: "Role for Alice" })).toHaveProperty(
      "disabled",
      false
    );
  });

  it.each(["forbidden", "stale", "network"] as const)(
    "shows a %s role failure while preserving the exact retry target",
    (failureKind) => {
      const onUpdateRole = vi.fn();
      render(
        <SpaceMembersPanel
          state={state({
            can_edit_roles: true,
            power_levels_revision: "revision-1",
            space_joined: [
              member("@alice:example.invalid", "Alice", "space_joined", {
                power_level: 0,
                role_options: [{ power_level: 50, role: "moderator", requires_confirmation: false }]
              })
            ],
            operation: {
              kind: "roleUpdateFailed",
              request_id: 9,
              space_id: "!space:example.invalid",
              user_id: "@alice:example.invalid",
              generation: 4,
              expected_power_levels_revision: "revision-1",
              expected_power_level: 0,
              power_level: 50,
              sent_revision: null,
              failureKind
            }
          })}
          canInvite={true}
          onInviteUser={vi.fn()}
          onOpenProfile={vi.fn()}
          onUpdateRole={onUpdateRole}
        />
      );

      expect(screen.getByRole("alert").textContent).toBe(
        "Could not update this member's role. Try again."
      );
      const select = screen.getByRole("combobox", { name: "Role for Alice" });
      fireEvent.change(select, { target: { value: "50" } });
      expect(onUpdateRole).toHaveBeenCalledWith("@alice:example.invalid", {
        power_level: 50,
        role: "moderator",
        requires_confirmation: false
      });
    }
  );
});
