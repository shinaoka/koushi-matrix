import { expect, test, type Page } from "@playwright/test";

import { t } from "../src/i18n/messages";

import { HARNESS_ROOM_ID, gotoReadyShell, invocationCount, seedTimelineItems } from "./support/basicOperations";

/**
 * Explore and Invites are account-global, so they render only at Home (#330).
 * The harness boots with a space selected.
 */
async function selectAccountHome(page: Page): Promise<void> {
  await page
    .getByRole("navigation", { name: "Workspaces" })
    .getByRole("button", { name: /^Home/ })
    .click();
  await expect(page.getByRole("main", { name: "Activity" })).toBeVisible();
}

test("create-room dialog submits create_room and closes on success", async ({ page }) => {
  await gotoReadyShell(page);
  await page.evaluate(() => window.__harness.clearInvocations());

  await page.getByRole("button", { name: "Create room", exact: true }).click();
  const roomNameInput = page.getByRole("textbox", { name: "Room name" });
  await expect(roomNameInput).toBeVisible();

  await roomNameInput.fill("My New Room");
  await page.getByRole("button", { name: "Submit create room" }).click();

  // create_room was invoked.
  await expect.poll(() => invocationCount(page, "create_room")).toBeGreaterThanOrEqual(1);
  // Dialog closed on success (the name input is gone).
  await expect(roomNameInput).toBeHidden();
});

test("create-space dialog submits create_space and closes on success", async ({ page }) => {
  await gotoReadyShell(page);
  await page.evaluate(() => window.__harness.clearInvocations());

  await page.getByRole("button", { name: "Create space", exact: true }).click();
  const spaceNameInput = page.getByRole("textbox", { name: "Space name" });
  await expect(spaceNameInput).toBeVisible();

  await spaceNameInput.fill("My New Space");
  await page.getByRole("button", { name: "Submit create space" }).click();

  await expect.poll(() => invocationCount(page, "create_space")).toBeGreaterThanOrEqual(1);
  await expect(spaceNameInput).toBeHidden();
});

test("workspace rail space and Home clicks apply returned navigation snapshots", async ({
  page
}) => {
  await gotoReadyShell(page);
  await page.evaluate(() => {
    window.__harness.setCommandResponse(
      "select_space",
      ({ spaceId }: { spaceId: string | null }) => {
        const snapshot = window.__harness.currentSnapshot();
        const next = {
          ...snapshot,
          state: {
            ...snapshot.state,
            ui: {
              ...snapshot.state.ui,
              navigation: {
                ...snapshot.state.ui.navigation,
                active_space_id: spaceId
              }
            }
          },
          sidebar: {
            ...snapshot.sidebar,
            active_space_id: spaceId,
            account_home: {
              ...snapshot.sidebar.account_home,
              is_active: spaceId === null
            },
            space_rail: snapshot.sidebar.space_rail.map((space) => ({
              ...space,
              is_active: space.space_id === spaceId
            }))
          }
        };
        window.__harness.setSnapshot(next);
        return next;
      }
    );
    window.__harness.clearInvocations();
  });

  const rail = page.getByRole("navigation", { name: "Workspaces" });
  const home = rail.getByRole("button", { name: /^Home/ });
  const space = rail.getByRole("button", { name: "Harness Space", exact: true });

  await space.click({ force: true });
  await expect
    .poll(async () => page.evaluate(() => window.__harness.invocationsOf("select_space")[0]?.args))
    .toEqual({ spaceId: "!harness-space:example.invalid" });
  await expect(space).toHaveClass(/is-active/);
  await expect(home).not.toHaveClass(/is-active/);

  await page.evaluate(() => window.__harness.clearInvocations());
  await home.click({ force: true });
  await expect
    .poll(async () => page.evaluate(() => window.__harness.invocationsOf("select_space")[0]?.args))
    .toEqual({ spaceId: null });
  await expect(home).toHaveClass(/is-active/);
  await expect(space).not.toHaveClass(/is-active/);
});

test("workspace rail keeps a delta that arrives before its command receipt", async ({
  page
}) => {
  await gotoReadyShell(page);
  await page.evaluate(() => {
    const targetSpaceId = "!harness-space:example.invalid";
    const current = window.__harness.currentSnapshot();
    const staleSnapshot = {
      ...current,
      state_generation: 1,
      state: {
        ...current.state,
        ui: {
          ...current.state.ui,
          navigation: {
            ...current.state.ui.navigation,
            active_space_id: null
          }
        }
      },
      sidebar: {
        ...current.sidebar,
        active_space_id: null,
        account_home: {
          ...current.sidebar.account_home,
          is_active: true
        },
        space_rail: current.sidebar.space_rail.map((space) => ({
          ...space,
          is_active: false
        }))
      }
    };
    window.__harness.setSnapshot(staleSnapshot);
    window.__harness.setCommandResponse(
      "select_space",
      async ({ spaceId }: { spaceId: string | null }) => {
        const generation =
          (window.__harness.currentSnapshot().state_generation ?? 0) + 1;
        await window.__harness.pushStateUpdate({
          protocol_version: 1,
          kind: "delta",
          generation,
          changed: {
            state: {
              ui: {
                navigation: {
                  ...staleSnapshot.state.ui.navigation,
                  active_space_id: spaceId
                }
              }
            },
            sidebar: {
              ...staleSnapshot.sidebar,
              active_space_id: spaceId,
              account_home: {
                ...staleSnapshot.sidebar.account_home,
                is_active: spaceId === null
              },
              space_rail: staleSnapshot.sidebar.space_rail.map((space) => ({
                ...space,
                is_active: space.space_id === spaceId
              }))
            }
          }
        });
        return { protocolVersion: 1, admittedGeneration: generation };
      }
    );
    window.__harness.pushStateUpdate();
    window.__harness.clearInvocations();
  });

  const rail = page.getByRole("navigation", { name: "Workspaces" });
  const home = rail.getByRole("button", { name: /^Home/ });
  const space = rail.getByRole("button", { name: "Harness Space", exact: true });

  await expect(home).toHaveClass(/is-active/);
  await space.click({ force: true });

  await expect
    .poll(async () => page.evaluate(() => window.__harness.invocationsOf("select_space")[0]?.args))
    .toEqual({ spaceId: "!harness-space:example.invalid" });
  await expect(space).toHaveClass(/is-active/);
  await expect(home).not.toHaveClass(/is-active/);
});

test("space rail separates system buttons, reorders Spaces, and leaves a Space headlessly", async ({
  page
}) => {
  await gotoReadyShell(page);
  await page.evaluate(() => {
    const snapshot = window.__harness.currentSnapshot();
    const secondSpace = {
      space_id: "!second-harness-space:example.invalid",
      display_name: "Second Harness Space",
      avatar: null,
      child_room_ids: []
    };
    const secondRailItem = {
      space_id: secondSpace.space_id,
      display_name: secondSpace.display_name,
      avatar: null,
      unread_count: 0,
      highlight_count: 0,
      is_active: false
    };
    window.__harness.setSnapshot({
      ...snapshot,
      state: {
        ...snapshot.state,
        ui: {
          ...snapshot.state.ui,
          navigation: {
            ...snapshot.state.ui.navigation,
            space_order: [
              ...snapshot.state.domain.spaces.map((space) => space.space_id),
              secondSpace.space_id
            ]
          }
        },
        domain: {
          ...snapshot.state.domain,
          spaces: [...snapshot.state.domain.spaces, secondSpace]
        }
      },
      sidebar: {
        ...snapshot.sidebar,
        space_rail: [...snapshot.sidebar.space_rail, secondRailItem]
      }
    });
    window.__harness.pushStateUpdate();
    window.__harness.clearInvocations();
  });

  const rail = page.getByRole("navigation", { name: "Workspaces" });
  await expect(rail.locator('[role="separator"]')).toBeVisible();
  await expect(rail.getByRole("button", { name: /^Home/ })).toBeVisible();

  const firstSpace = rail.getByRole("button", { name: "Harness Space", exact: true });
  const secondSpace = rail.getByRole("button", { name: "Second Harness Space", exact: true });
  await expect(firstSpace).toHaveAttribute("draggable", "true");
  await expect(secondSpace).toHaveAttribute("draggable", "true");

  await rail.locator(".workspace-space-button").evaluateAll((buttons) => {
    const source = buttons[0];
    const target = buttons[1];
    if (!source || !target) {
      throw new Error("expected two Space rail buttons");
    }
    const dataTransfer = new DataTransfer();
    source.dispatchEvent(
      new DragEvent("dragstart", { bubbles: true, cancelable: true, dataTransfer })
    );
    target.dispatchEvent(
      new DragEvent("dragover", { bubbles: true, cancelable: true, dataTransfer })
    );
    target.dispatchEvent(new DragEvent("drop", { bubbles: true, cancelable: true, dataTransfer }));
    source.dispatchEvent(
      new DragEvent("dragend", { bubbles: true, cancelable: true, dataTransfer })
    );
  });

  await expect
    .poll(() =>
      page.evaluate(
        () => window.__harness.invocationsOf("reorder_spaces").at(-1)?.args.spaceIds ?? []
      )
    )
    .toEqual(["!second-harness-space:example.invalid", "!harness-space:example.invalid"]);
  await expect
    .poll(() =>
      rail
        .locator(".workspace-space-button")
        .evaluateAll((buttons) => buttons.map((button) => button.getAttribute("aria-label")))
    )
    .toEqual(["Second Harness Space", "Harness Space"]);

  await page.evaluate(() => window.__harness.clearInvocations());
  await rail.getByRole("button", { name: "Second Harness Space", exact: true }).click({
    button: "right"
  });
  await page.getByRole("menuitem", { name: "Leave Space", exact: true }).click();
  await expect
    .poll(() =>
      page.evaluate(
        () => window.__harness.invocationsOf("leave_room").at(-1)?.args.roomId ?? null
      )
    )
    .toBe("!second-harness-space:example.invalid");
  await expect(
    rail.getByRole("button", { name: "Second Harness Space", exact: true })
  ).toHaveCount(0);
});

test("Space member roles use authoritative success, failure retry, confirmation, and child-sync barriers", async ({
  page
}) => {
  await gotoReadyShell(page);
  await page.getByRole("button", { name: "Harness Space", exact: true }).click({ force: true });
  const membersButton = page.getByRole("button", { name: /^Members,/ });
  await expect(membersButton).toBeVisible();
  await membersButton.click();
  await expect(page.getByRole("heading", { name: "Space members", level: 2 })).toBeVisible();
  await expect(page.getByText("Some child rooms are still syncing")).toBeVisible();

  await page.evaluate(() => {
    let attempts = 0;
    window.__harness.setCommandResponse(
      "update_space_member_role",
      ({ spaceId, userId, generation, expectedPowerLevelsRevision, expectedPowerLevel, powerLevel }) => {
        attempts += 1;
        const current = window.__harness.currentSnapshot();
        const members = current.state.domain.space_members;
        const target = members.space_joined.find((entry) => entry.user_id === String(userId));
        if (
          !target ||
          members.selected_space_id !== String(spaceId) ||
          members.generation !== Number(generation) ||
          members.power_levels_revision !== (expectedPowerLevelsRevision ?? null) ||
          target.power_level !== Number(expectedPowerLevel)
        ) {
          return current;
        }
        if (attempts === 1) {
          return {
            ...current,
            state: {
              ...current.state,
              domain: {
                ...current.state.domain,
                space_members: {
                  ...members,
                  power_levels_revision: "revision-2",
                  operation: {
                    kind: "roleUpdateFailed",
                    request_id: 7_001,
                    space_id: String(spaceId),
                    user_id: String(userId),
                    generation: Number(generation),
                    expected_power_levels_revision: expectedPowerLevelsRevision ?? null,
                    expected_power_level: Number(expectedPowerLevel),
                    power_level: Number(powerLevel),
                    sent_revision: null,
                    failureKind: "stale"
                  }
                }
              }
            }
          };
        }
        const nextPower = Number(powerLevel);
        const nextRole = nextPower === 100 ? "administrator" : nextPower === 50 ? "moderator" : "user";
        return {
          ...current,
          state: {
            ...current.state,
            domain: {
              ...current.state.domain,
              space_members: {
                ...members,
                power_levels_revision: `revision-${attempts}`,
                space_joined: members.space_joined.map((entry) =>
                  entry.user_id === userId
                    ? {
                        ...entry,
                        power_level: nextPower,
                        role: nextRole,
                        role_options: [0, 50, 100]
                          .filter((candidate) => candidate !== nextPower)
                          .map((candidate) => ({
                            power_level: candidate,
                            role: candidate === 100 ? "administrator" : candidate === 50 ? "moderator" : "user",
                            requires_confirmation: nextPower >= 100 || candidate >= 100
                          }))
                      }
                    : entry
                ),
                operation: { kind: "idle" }
              }
            }
          }
        };
      }
    );
    window.__harness.clearInvocations();
  });

  const select = page.getByRole("combobox", { name: "Role for Harness Role Target" });
  await expect(select).toBeEnabled();
  await select.selectOption("50");
  await expect
    .poll(() => page.evaluate(() => window.__harness.invocationsOf("update_space_member_role").length))
    .toBe(1);
  await expect(page.getByRole("alert")).toHaveText("Could not update this member's role. Try again.");
  await expect(select).toHaveValue("0");

  await select.selectOption("50");
  await expect
    .poll(() => page.evaluate(() => window.__harness.invocationsOf("update_space_member_role").length))
    .toBe(2);
  await expect(select).toHaveValue("50");

  await select.selectOption("100");
  const dialog = page.getByRole("dialog", { name: "Confirm role change" });
  await expect(dialog).toBeVisible();
  const invocationCountBeforeCancel = await page.evaluate(
    () => window.__harness.invocationsOf("update_space_member_role").length
  );
  await dialog.getByRole("button", { name: "Cancel" }).click();
  await expect
    .poll(() => page.evaluate(() => window.__harness.invocationsOf("update_space_member_role").length))
    .toBe(invocationCountBeforeCancel);
  await expect(select).toHaveValue("50");

  await select.selectOption("100");
  await page.getByRole("dialog", { name: "Confirm role change" })
    .getByRole("button", { name: "Confirm role change" })
    .click();
  await expect
    .poll(() => page.evaluate(() => window.__harness.invocationsOf("update_space_member_role").at(-1)?.args))
    .toMatchObject({
      spaceId: "!harness-space:example.invalid",
      userId: "@harness-role-target:example.invalid",
      generation: 2,
      expectedPowerLevelsRevision: "revision-2",
      expectedPowerLevel: 50,
      powerLevel: 100,
      confirmed: true
    });
  await expect(select).toHaveValue("100");
  console.log("space_member_role=ok");
});

test("invite acceptance does not expose the previous timeline when room selection is uncommitted", async ({
  page
}) => {
  await gotoReadyShell(page);

  await page.evaluate(() => {
    const base = window.__harness.currentSnapshot();
    const roomId = "!invite-navigation-refused:example.invalid";
    const invite = {
      room_id: roomId,
      display_name: "Refused Navigation Invite",
      avatar: null,
      topic: null,
      inviter_display_name: "Synthetic Inviter",
      is_dm: false
    };
    window.__harness.setSnapshot({
      ...base,
      state: {
        ...base.state,
        domain: { ...base.state.domain, invites: [invite] }
      }
    });
    window.__harness.setCommandResponse("accept_invite", () => {
      const snapshot = window.__harness.currentSnapshot();
      const joinedRoom = {
        room_id: roomId,
        display_name: invite.display_name,
        avatar: null,
        is_dm: false,
        tags: { favourite: null, low_priority: null },
        unread_count: 0,
        notification_count: 0,
        highlight_count: 0,
        parent_space_ids: []
      };
      const joinedItem = {
        room_id: roomId,
        display_name: invite.display_name,
        avatar: null,
        tags: { favourite: null, low_priority: null },
        unread_count: 0,
        highlight_count: 0
      };
      const next = {
        ...snapshot,
        state: {
          ...snapshot.state,
          domain: {
            ...snapshot.state.domain,
            rooms: [...snapshot.state.domain.rooms, joinedRoom],
            invites: []
          }
        },
        sidebar: {
          ...snapshot.sidebar,
          space_rooms: [...snapshot.sidebar.space_rooms, joinedItem],
          sections: {
            ...snapshot.sidebar.sections,
            rooms: [...snapshot.sidebar.sections.rooms, joinedItem]
          }
        }
      };
      window.__harness.setSnapshot(next);
      return next;
    });
    window.__harness.setCommandResponse("select_room", () =>
      window.__harness.currentSnapshot()
    );
    window.__harness.pushStateUpdate();
    window.__harness.clearInvocations();
  });

  await selectAccountHome(page);
  await page.getByRole("button", { name: "Invites", exact: true }).click();
  await expect(page.getByRole("heading", { name: "Invites" })).toBeVisible();
  await page.getByRole("button", { name: "Accept invite" }).click();

  await expect.poll(() => invocationCount(page, "select_room")).toBe(1);
  await expect(page.getByRole("heading", { name: "Invites" })).toBeVisible();
  await expect(page.getByRole("main", { name: "Conversation timeline" })).toBeHidden();
});

test("invites view accepts a seeded invite and New DM renders the returned direct room", async ({
  page
}) => {
  await gotoReadyShell(page);

  await page.evaluate(() => {
    const base = window.__harness.currentSnapshot();
    const invite = {
      room_id: "!joined-from-invite:example.invalid",
      display_name: "Seeded Invite",
      avatar: null,
      topic: "Synthetic invite topic",
      inviter_display_name: "Synthetic Inviter",
      is_dm: false
    };
    window.__harness.setSnapshot({
      ...base,
      state: {
        ...base.state,
        domain: {
          ...base.state.domain,
          invites: [invite]
        }
      }
    });
    window.__harness.setCommandResponse("accept_invite", () => {
      const snapshot = window.__harness.currentSnapshot();
      const joinedRoom = {
        room_id: "!joined-from-invite:example.invalid",
        display_name: "Seeded Invite",
        avatar: null,
        is_dm: false,
        tags: { favourite: null, low_priority: null },
        unread_count: 0,
        notification_count: 0,
        highlight_count: 0,
        parent_space_ids: []
      };
      const joinedItem = {
        room_id: joinedRoom.room_id,
        display_name: joinedRoom.display_name,
        avatar: null,
        tags: { favourite: null, low_priority: null },
        unread_count: 0,
        highlight_count: 0
      };
      const next = {
        ...snapshot,
        state: {
          ...snapshot.state,
          domain: {
            ...snapshot.state.domain,
            rooms: [...snapshot.state.domain.rooms, joinedRoom],
            invites: []
          }
        },
        sidebar: {
          ...snapshot.sidebar,
          space_rooms: [...snapshot.sidebar.space_rooms, joinedItem],
          sections: {
            ...snapshot.sidebar.sections,
            rooms: [...snapshot.sidebar.sections.rooms, joinedItem]
          }
        }
      };
      window.__harness.setSnapshot(next);
      return next;
    });
    window.__harness.setCommandResponse("start_direct_message", ({ userId }) => {
      const snapshot = window.__harness.currentSnapshot();
      const dmRoom = {
        room_id: "!dm-started:example.invalid",
        display_name: String(userId),
        avatar: null,
        is_dm: true,
        tags: { favourite: null, low_priority: null },
        unread_count: 0,
        notification_count: 0,
        highlight_count: 0,
        parent_space_ids: []
      };
      const dmItem = {
        room_id: dmRoom.room_id,
        display_name: dmRoom.display_name,
        avatar: null,
        tags: { favourite: null, low_priority: null },
        unread_count: 0,
        highlight_count: 0
      };
      const next = {
        ...snapshot,
        state: {
          ...snapshot.state,
          domain: {
            ...snapshot.state.domain,
            rooms: [...snapshot.state.domain.rooms, dmRoom]
          },
          ui: {
            ...snapshot.state.ui,
            navigation: {
              ...snapshot.state.ui.navigation,
              active_room_id: dmRoom.room_id
            },
            timeline: {
              ...snapshot.state.ui.timeline,
              room_id: dmRoom.room_id,
              is_subscribed: true
            }
          }
        },
        sidebar: {
          ...snapshot.sidebar,
          global_dms: [...snapshot.sidebar.global_dms, dmItem],
          sections: {
            ...snapshot.sidebar.sections,
            people: [...snapshot.sidebar.sections.people, dmItem]
          }
        }
      };
      window.__harness.setSnapshot(next);
      return next;
    });
    window.__harness.setCommandResponse("select_room", ({ roomId }) => {
      const snapshot = window.__harness.currentSnapshot();
      const next = {
        ...snapshot,
        state: {
          ...snapshot.state,
          ui: {
            ...snapshot.state.ui,
            navigation: {
              ...snapshot.state.ui.navigation,
              active_room_id: String(roomId)
            },
            timeline: {
              ...snapshot.state.ui.timeline,
              room_id: String(roomId),
              is_subscribed: true
            },
            thread: { kind: "closed" }
          },
          domain: {
            ...snapshot.state.domain,
            thread_attention: { kind: "closed" }
          }
        },
        thread: null
      };
      window.__harness.setSnapshot(next);
      return next;
    });
    const inviteTargetFor = (status = "selectable") => ({
      user_id: "@invitee:example.invalid",
      display_label: "Invitee",
      original_display_label: "Invitee",
      avatar: null,
      source: "matrixId",
      status,
      status_message: null
    });
    const inviteWorkflowFor = ({ query = "", selected = false } = {}) => ({
      query: {
        room_id: "!joined-from-invite:example.invalid",
        query,
        candidates: [],
        explicit_user_id: query ? inviteTargetFor(selected ? "alreadySelected" : "selectable") : null
      },
      selected_targets: selected
        ? [
            {
              user_id: "@invitee:example.invalid",
              display_label: "Invitee",
              avatar: null
            }
          ]
        : [],
      scope_plan: null,
      selected_scope: { kind: "roomOnly" },
      history_policy: {
        current_visibility: "joined",
        encrypted: false,
        can_edit: true,
        readiness: "ready"
      },
      operation: { kind: "idle" }
    });
    window.__harness.setCommandResponse("open_invite_workflow", ({ roomId }) => {
      const snapshot = window.__harness.currentSnapshot();
      const previousWorkflow = snapshot.state.domain.invite_workflow;
      const next = {
        ...snapshot,
        state: {
          ...snapshot.state,
          domain: {
            ...snapshot.state.domain,
            invite_workflow: inviteWorkflowFor({
              query: previousWorkflow?.query.query ?? "",
              selected: (previousWorkflow?.selected_targets.length ?? 0) > 0
            })
          }
        }
      };
      window.__harness.setSnapshot(next);
      return next;
    });
    window.__harness.setCommandResponse("search_invite_targets", ({ query }) => {
      const snapshot = window.__harness.currentSnapshot();
      const next = {
        ...snapshot,
        state: {
          ...snapshot.state,
          domain: {
            ...snapshot.state.domain,
            invite_workflow: inviteWorkflowFor({ query: String(query), selected: false })
          }
        }
      };
      window.__harness.setSnapshot(next);
      return next;
    });
    window.__harness.setCommandResponse("select_invite_target", () => {
      const snapshot = window.__harness.currentSnapshot();
      const query = snapshot.state.domain.invite_workflow?.query.query ?? "";
      const next = {
        ...snapshot,
        state: {
          ...snapshot.state,
          domain: {
            ...snapshot.state.domain,
            invite_workflow: inviteWorkflowFor({ query, selected: true })
          }
        }
      };
      window.__harness.setSnapshot(next);
      return next;
    });
    window.__harness.setCommandResponse("invite_targets", () => window.__harness.currentSnapshot());
    window.__harness.pushStateUpdate();
    window.__harness.clearInvocations();
  });

  await selectAccountHome(page);
  await page.getByRole("button", { name: "Invites", exact: true }).click();
  await expect(page.getByRole("heading", { name: "Invites" })).toBeVisible();
  await expect(page.getByRole("button", { name: "Seeded Invite" })).toBeVisible();
  await expect(page.getByText("Synthetic Inviter", { exact: true })).toBeVisible();

  await page.getByRole("button", { name: "Accept invite" }).click();

  await expect.poll(() => invocationCount(page, "accept_invite")).toBeGreaterThanOrEqual(1);
  await expect
    .poll(async () =>
      page.evaluate(() => window.__harness.invocationsOf("accept_invite")[0]?.args)
    )
    .toEqual({ roomId: "!joined-from-invite:example.invalid" });
  await expect
    .poll(async () =>
      page.evaluate(() => window.__harness.invocationsOf("select_room").at(-1)?.args)
    )
    .toEqual({ roomId: "!joined-from-invite:example.invalid" });
  await expect
    .poll(async () =>
      page.evaluate(
        () => window.__harness.currentSnapshot().state.ui.navigation.active_room_id
      )
    )
    .toBe("!joined-from-invite:example.invalid");
  await expect(page.getByRole("button", { name: "Seeded Invite" })).toBeVisible();

  await page.getByRole("button", { name: "Seeded Invite" }).click();
  await page.getByRole("button", { name: "Room info" }).click();
  await page.getByRole("button", { name: "Invite people" }).click();
  const inviteUserInput = page.getByRole("textbox", { name: "Name, alias, or Matrix ID" });
  await inviteUserInput.fill("@invitee:example.invalid");
  const inviteHistoryPanel = page.getByRole("region", { name: t("dialog.inviteHistory") });
  const inviteHistoryHeading = inviteHistoryPanel.getByRole("heading", {
    name: t("dialog.inviteHistory")
  });
  await expect(inviteHistoryHeading).toBeVisible();
  await expect(inviteHistoryPanel.getByText("Current", { exact: true })).toBeVisible();
  await page.getByRole("button", { name: "Open Room info" }).click();
  await expect(page.getByRole("button", { name: "Return to invite" })).toBeVisible();
  await page.getByRole("button", { name: "Return to invite" }).click();
  const returnedInviteHistoryHeading = page
    .getByRole("region", { name: t("dialog.inviteHistory") })
    .getByRole("heading", { name: t("dialog.inviteHistory") });
  await expect(returnedInviteHistoryHeading).toBeVisible();
  await expect(inviteUserInput).toHaveValue("@invitee:example.invalid");
  await page.getByRole("button", { name: /Invitee.*@invitee:example\.invalid/ }).click();
  await page.getByRole("button", { name: "Send invite" }).click();

  await expect.poll(() => invocationCount(page, "invite_targets")).toBeGreaterThanOrEqual(1);
  await expect
    .poll(async () =>
      page.evaluate(() => window.__harness.invocationsOf("invite_targets")[0]?.args)
    )
    .toEqual({
      roomId: "!joined-from-invite:example.invalid",
      userIds: ["@invitee:example.invalid"],
      scope: { kind: "roomOnly" }
    });

  await selectAccountHome(page);
  await page.getByRole("button", { name: "Invites", exact: true }).click();
  await page.getByRole("main", { name: "Invites" }).getByRole("button", { name: "New DM" }).click();
  const userIdInput = page.getByRole("textbox", { name: "Matrix user ID" });
  await expect(userIdInput).toBeVisible();
  await userIdInput.fill("@target:example.invalid");
  await page.getByRole("button", { name: "Start DM" }).click();

  await expect.poll(() => invocationCount(page, "start_direct_message")).toBeGreaterThanOrEqual(1);
  await expect
    .poll(async () =>
      page.evaluate(() => window.__harness.invocationsOf("start_direct_message")[0]?.args)
    )
    .toEqual({ userId: "@target:example.invalid" });
  await page.getByRole("button", { name: /^DMs,/ }).click();
  await expect(
    page
      .locator('[data-room-section="people"]')
      .getByRole("button", { name: "@target:example.invalid" })
  ).toBeVisible();
});

test("Explore searches public rooms and joins only after Rust snapshot updates", async ({
  page
}) => {
  await gotoReadyShell(page);
  await page.evaluate(() => {
    window.__harness.setCommandResponse("query_directory", () =>
      window.__harness.currentSnapshot()
    );
    window.__harness.setCommandResponse("preview_join_target", () =>
      window.__harness.currentSnapshot()
    );
    window.__harness.setCommandResponse("join_directory_room", () =>
      window.__harness.currentSnapshot()
    );
    window.__harness.clearInvocations();
  });

  await selectAccountHome(page);
  await page.getByRole("button", { name: "Explore", exact: true }).click();
  await expect(page.getByRole("main", { name: "Explore" })).toBeVisible();

  const searchInput = page.getByRole("searchbox", { name: "Search term" });
  await searchInput.fill("public");
  await page.getByRole("button", { name: "Search", exact: true }).click();

  await expect.poll(() => invocationCount(page, "query_directory")).toBeGreaterThanOrEqual(1);
  await expect
    .poll(async () => page.evaluate(() => window.__harness.invocationsOf("query_directory")[0]?.args))
    .toEqual({
      term: "public",
      serverName: null,
      limit: 20,
      since: null
    });
  await expect(page.getByRole("heading", { name: "Public Search Result" })).toHaveCount(0);

  await page.evaluate(() => {
    const snapshot = window.__harness.currentSnapshot();
    const query = {
      term: "public",
      server_name: null,
      limit: 20,
      since: null
    };
    window.__harness.setSnapshot({
      ...snapshot,
      state: {
        ...snapshot.state,
        domain: {
          ...snapshot.state.domain,
          directory: {
            ...snapshot.state.domain.directory,
            query: {
              kind: "results",
              request_id: 44,
              query,
              rooms: [
                {
                  room_id: "!public-result:example.invalid",
                  canonical_alias: "#public-result:example.invalid",
                  room_type: null,
                  name: "Public Search Result",
                  topic: "Rust-owned public directory result",
                  avatar_url: null,
                  joined_members: 12,
                  world_readable: true,
                  guest_can_join: false
                }
              ],
              next_batch: null
            },
            preview: { kind: "closed" },
            join: { kind: "idle" }
          }
        }
      }
    });
    window.__harness.pushStateUpdate();
  });

  await expect(page.getByRole("heading", { name: "Public Search Result" })).toBeVisible();
  await page.getByRole("button", { name: "Join Public Search Result" }).click();

  // A result row must open the Rust-owned preview, not put the user straight
  // into a room they have not seen.
  await expect.poll(() => invocationCount(page, "preview_join_target")).toBeGreaterThanOrEqual(1);
  await expect
    .poll(async () =>
      page.evaluate(() => window.__harness.invocationsOf("preview_join_target")[0]?.args)
    )
    .toEqual({
      roomIdOrAlias: "#public-result:example.invalid",
      viaServers: ["example.invalid"]
    });
  expect(await invocationCount(page, "join_directory_room")).toBe(0);
  await expect(page.getByRole("dialog", { name: "Join this room?" })).toHaveCount(0);

  await page.evaluate(() => {
    const snapshot = window.__harness.currentSnapshot();
    window.__harness.setSnapshot({
      ...snapshot,
      state: {
        ...snapshot.state,
        domain: {
          ...snapshot.state.domain,
          directory: {
            ...snapshot.state.domain.directory,
            preview: {
              kind: "ready",
              request_id: 45,
              room_id_or_alias: "#public-result:example.invalid",
              via_servers: ["example.invalid"],
              room: {
                room_id: "!public-result:example.invalid",
                canonical_alias: "#public-result:example.invalid",
                room_type: "m.space",
                name: "Public Search Result",
                topic: "Rust-owned public directory result",
                joined_members: 12,
                joinability: "open",
                membership: "none"
              }
            }
          }
        }
      }
    });
    window.__harness.pushStateUpdate();
  });

  const previewDialog = page.getByRole("dialog", { name: "Join this room?" });
  await expect(previewDialog).toBeVisible();
  await expect(previewDialog.getByText("Rust-owned public directory result")).toBeVisible();
  await expect(previewDialog.getByText("12 members")).toBeVisible();
  await expect(previewDialog.getByText("Space", { exact: true })).toBeVisible();

  await previewDialog.getByRole("button", { name: "Join", exact: true }).click();

  await expect.poll(() => invocationCount(page, "join_directory_room")).toBeGreaterThanOrEqual(1);
  await expect
    .poll(async () =>
      page.evaluate(() => window.__harness.invocationsOf("join_directory_room")[0]?.args)
    )
    .toEqual({
      roomIdOrAlias: "#public-result:example.invalid",
      viaServers: ["example.invalid"]
    });

  const roomsSection = page.locator('[data-room-section="rooms"]');
  await expect(
    roomsSection.getByRole("button", { name: "Public Search Result" })
  ).toHaveCount(0);

  await page.evaluate(() => {
    const snapshot = window.__harness.currentSnapshot();
    const joinedRoom = {
      room_id: "!joined-public-result:example.invalid",
      display_name: "Public Search Result",
      avatar: null,
      is_dm: false,
      tags: { favourite: null, low_priority: null },
      unread_count: 0,
      notification_count: 0,
      highlight_count: 0,
      parent_space_ids: []
    };
    const roomListItem = {
      room_id: joinedRoom.room_id,
      display_name: joinedRoom.display_name,
      avatar: joinedRoom.avatar,
      tags: joinedRoom.tags,
      unread_count: joinedRoom.unread_count,
      notification_count: joinedRoom.notification_count,
      highlight_count: joinedRoom.highlight_count
    };
    window.__harness.setSnapshot({
      ...snapshot,
      state: {
        ...snapshot.state,
        domain: {
          ...snapshot.state.domain,
          rooms: [...snapshot.state.domain.rooms, joinedRoom],
          directory: {
            ...snapshot.state.domain.directory,
            preview: { kind: "closed" },
            join: { kind: "idle" }
          }
        }
      },
      sidebar: {
        ...snapshot.sidebar,
        space_rooms: [...snapshot.sidebar.space_rooms, roomListItem],
        sections: {
          ...snapshot.sidebar.sections,
          rooms: [...snapshot.sidebar.sections.rooms, roomListItem]
        }
      }
    });
    window.__harness.pushStateUpdate();
  });

  await expect(
    roomsSection.getByRole("button", { name: "Public Search Result" })
  ).toBeVisible();
});

test("room management panel updates settings, roles, and members from Rust state", async ({
  page
}) => {
  await gotoReadyShell(page);
  await page.evaluate((roomId) => {
    const snapshot = window.__harness.currentSnapshot();
    window.__harness.setSnapshot({
      ...snapshot,
      state: {
        ...snapshot.state,
        domain: {
          ...snapshot.state.domain,
          room_management: {
            selected_room_id: roomId,
            settings: {
              room_id: roomId,
              name: "Harness Room",
              topic: "Original managed topic",
              avatar_url: null,
              join_rule: "invite",
              history_visibility: "shared",
              permissions: {
                can_edit_settings: true,
                can_edit_roles: true,
                can_invite: true,
                can_kick: true,
                can_ban: false,
                can_unban: false
              },
              members: [
                {
                  user_id: "@target-member:example.invalid",
                  display_name: "Target Member",
                  display_label: "Target Member",
                  original_display_label: "Target Member",
                  avatar_url: null,
                  power_level: 0,
                  role: "user",
                  role_options: [
                    { power_level: 100, role: "administrator", requires_confirmation: true },
                    { power_level: 50, role: "moderator", requires_confirmation: false }
                  ]
                }
              ]
            },
            operation: { kind: "idle" }
          }
        }
      }
    });
    window.__harness.setCommandResponse("update_room_setting", () =>
      window.__harness.currentSnapshot()
    );
    window.__harness.setCommandResponse("moderate_room_member", () =>
      window.__harness.currentSnapshot()
    );
    window.__harness.setCommandResponse("update_room_member_role", () =>
      window.__harness.currentSnapshot()
    );
    window.__harness.setCommandResponse("load_room_settings", () =>
      window.__harness.currentSnapshot()
    );
    window.__harness.pushStateUpdate();
    window.__harness.clearInvocations();
  }, HARNESS_ROOM_ID);

  await page.getByRole("button", { name: "Room info" }).click();
  await expect(page.getByRole("heading", { name: "Harness Room" })).toBeVisible();
  const currentTopicRow = page.locator(".settings-detail-row").filter({
    hasText: "Current topic"
  });
  await expect(currentTopicRow.getByText("Original managed topic")).toBeVisible();

  const topicInput = page.getByRole("textbox", { name: "Room topic" });
  await topicInput.fill("Updated managed topic");
  await page.getByRole("button", { name: "Save topic" }).click();

  await expect.poll(() => invocationCount(page, "update_room_setting")).toBeGreaterThanOrEqual(1);
  await expect
    .poll(async () =>
      page.evaluate(() => window.__harness.invocationsOf("update_room_setting")[0]?.args)
    )
    .toEqual({
      roomId: HARNESS_ROOM_ID,
      change: { topic: "Updated managed topic" }
    });
  await expect(currentTopicRow.getByText("Original managed topic")).toBeVisible();

  await page.evaluate((roomId) => {
    const snapshot = window.__harness.currentSnapshot();
    const settings = snapshot.state.domain.room_management.settings;
    window.__harness.setSnapshot({
      ...snapshot,
      state: {
        ...snapshot.state,
        domain: {
          ...snapshot.state.domain,
          room_management: {
            selected_room_id: roomId,
            settings: settings
              ? { ...settings, topic: "Updated managed topic" }
              : settings,
            operation: { kind: "idle" }
          }
        }
      }
    });
    window.__harness.pushStateUpdate();
  }, HARNESS_ROOM_ID);

  await expect(currentTopicRow.getByText("Updated managed topic")).toBeVisible();
  await expect(currentTopicRow.getByText("Original managed topic")).toHaveCount(0);

  const currentAvatarRow = page.locator(".settings-detail-row").filter({
    hasText: "Current avatar"
  });
  await expect(currentAvatarRow.getByText("No avatar")).toBeVisible();
  const avatarInput = page.getByRole("textbox", { name: "Room avatar URL" });
  await avatarInput.fill("mxc://example.invalid/managed-avatar");
  await page.getByRole("button", { name: "Save avatar" }).click();

  await expect.poll(() => invocationCount(page, "update_room_setting")).toBeGreaterThanOrEqual(2);
  await expect
    .poll(async () =>
      page.evaluate(() => window.__harness.invocationsOf("update_room_setting")[1]?.args)
    )
    .toEqual({
      roomId: HARNESS_ROOM_ID,
      change: { avatarUrl: "mxc://example.invalid/managed-avatar" }
    });
  await expect(currentAvatarRow.getByText("No avatar")).toBeVisible();

  await page.evaluate((roomId) => {
    const snapshot = window.__harness.currentSnapshot();
    const settings = snapshot.state.domain.room_management.settings;
    window.__harness.setSnapshot({
      ...snapshot,
      state: {
        ...snapshot.state,
        domain: {
          ...snapshot.state.domain,
          room_management: {
            selected_room_id: roomId,
            settings: settings
              ? { ...settings, avatar_url: "mxc://example.invalid/managed-avatar" }
              : settings,
            operation: { kind: "idle" }
          }
        }
      }
    });
    window.__harness.pushStateUpdate();
  }, HARNESS_ROOM_ID);

  await expect(currentAvatarRow.getByText("mxc://example.invalid/managed-avatar")).toBeVisible();
  await expect(currentAvatarRow.getByText("No avatar")).toHaveCount(0);

  const historyVisibilitySelect = page.getByRole("combobox", { name: "History visibility" });
  await historyVisibilitySelect.selectOption("joined");
  await page.getByRole("button", { name: "Save history visibility" }).click();
  await expect
    .poll(async () =>
      page.evaluate(() => window.__harness.invocationsOf("update_room_setting").at(-1)?.args)
    )
    .toEqual({
      roomId: HARNESS_ROOM_ID,
      change: { historyVisibility: "joined" }
    });

  const joinRuleSelect = page.getByRole("combobox", { name: "Join rule" });
  await joinRuleSelect.selectOption("public");
  await page.getByRole("button", { name: "Save join rule" }).click();
  await expect
    .poll(async () =>
      page.evaluate(() => window.__harness.invocationsOf("update_room_setting").at(-1)?.args)
    )
    .toEqual({
      roomId: HARNESS_ROOM_ID,
      change: { joinRule: "public" }
    });

  await page
    .getByLabel("Context panel")
    .getByRole("button", { name: t("room.people"), exact: true })
    .click();
  await expect(page.getByRole("heading", { name: t("panel.people") })).toBeVisible();

  const targetMemberRow = page.locator(".people-list-row").filter({
    hasText: "Target Member"
  });
  await expect(targetMemberRow.getByText(t("room.roleUser"), { exact: true })).toBeVisible();
  await targetMemberRow
    .getByRole("button", { name: t("people.openProfile", { name: "Target Member" }) })
    .click();
  const targetMemberRoleSelect = page.getByRole("combobox", {
    name: "Member role for Target Member"
  });
  await expect(targetMemberRoleSelect).toHaveValue("0");
  await targetMemberRoleSelect.selectOption("50");

  await expect.poll(() => invocationCount(page, "update_room_member_role")).toBeGreaterThanOrEqual(1);
  await expect
    .poll(async () =>
      page.evaluate(() => window.__harness.invocationsOf("update_room_member_role")[0]?.args)
    )
    .toEqual({
      roomId: HARNESS_ROOM_ID,
      targetUserId: "@target-member:example.invalid",
      powerLevel: 50
    });
  await expect(targetMemberRoleSelect).toHaveValue("0");

  await page.evaluate(() => {
    const snapshot = window.__harness.currentSnapshot();
    window.__harness.setSnapshot({
      ...snapshot,
      state: {
        ...snapshot.state,
        domain: {
          ...snapshot.state.domain,
          room_management: {
            selected_room_id: snapshot.state.domain.room_management.selected_room_id,
            settings: snapshot.state.domain.room_management.settings
              ? {
                  ...snapshot.state.domain.room_management.settings,
                  members: snapshot.state.domain.room_management.settings.members.map((member) =>
                    member.user_id === "@target-member:example.invalid"
                      ? { ...member, power_level: 50, role: "moderator" }
                      : member
                  )
                }
              : null,
            operation: { kind: "idle" }
          }
        }
      }
    });
    window.__harness.pushStateUpdate();
  });

  await expect(targetMemberRoleSelect).toHaveValue("50");

  await page.getByRole("button", { name: "Kick Target Member" }).click();

  await expect.poll(() => invocationCount(page, "moderate_room_member")).toBeGreaterThanOrEqual(1);
  await expect
    .poll(async () =>
      page.evaluate(() => window.__harness.invocationsOf("moderate_room_member")[0]?.args)
    )
    .toEqual({
      roomId: HARNESS_ROOM_ID,
      targetUserId: "@target-member:example.invalid",
      action: "kick",
      reason: null
    });
  await expect(page.getByRole("button", { name: "Kick Target Member" })).toBeVisible();

  await page.evaluate(() => {
    const snapshot = window.__harness.currentSnapshot();
    window.__harness.setSnapshot({
      ...snapshot,
      state: {
        ...snapshot.state,
        domain: {
          ...snapshot.state.domain,
          room_management: {
            selected_room_id: snapshot.state.domain.room_management.selected_room_id,
            settings: snapshot.state.domain.room_management.settings
              ? {
                  ...snapshot.state.domain.room_management.settings,
                  members: []
                }
              : null,
            operation: { kind: "idle" }
          }
        }
      }
    });
    window.__harness.pushStateUpdate();
  });

  await page
    .getByRole("region", { name: t("panel.profile") })
    .getByRole("button", { name: t("action.back") })
    .click();
  await expect(page.locator(".people-list-row").filter({ hasText: "Target Member" })).toHaveCount(0);
});

test("local aliases dispatch typed account command and render Rust-projected labels", async ({
  page
}) => {
  await gotoReadyShell(page);
  await page.evaluate((roomId) => {
    const targetUserId = "@target-member:example.invalid";
    const snapshot = window.__harness.currentSnapshot();
    const dmRoom = {
      room_id: "!dm-target-member:example.invalid",
      display_name: "Target Member",
      display_label: "Target Member",
      original_display_label: "Target Member",
      avatar: null,
      is_dm: true,
      dm_user_ids: [targetUserId],
      tags: { favourite: null, low_priority: null },
      unread_count: 0,
      notification_count: 0,
      highlight_count: 0,
      parent_space_ids: []
    };
    window.__harness.setSnapshot({
      ...snapshot,
      state: {
        ...snapshot.state,
        domain: {
          ...snapshot.state.domain,
          profile: {
            ...snapshot.state.domain.profile,
            users: {
              ...snapshot.state.domain.profile.users,
              [targetUserId]: {
                user_id: targetUserId,
                display_name: "Target Member",
                display_label: "Target Member",
                original_display_label: "Target Member",
                mention_search_terms: ["Target Member", targetUserId],
                avatar: null
              }
            }
          },
          rooms: [...snapshot.state.domain.rooms, dmRoom],
          room_management: {
            selected_room_id: roomId,
            settings: {
              room_id: roomId,
              name: "Harness Room",
              topic: null,
              avatar_url: null,
              join_rule: "invite",
              history_visibility: "shared",
              permissions: {
                can_edit_settings: true,
                can_edit_roles: true,
                can_invite: true,
                can_kick: true,
                can_ban: false,
                can_unban: false
              },
              members: [
                {
                  user_id: targetUserId,
                  display_name: "Target Member",
                  display_label: "Target Member",
                  original_display_label: "Target Member",
                  avatar_url: null,
                  power_level: 0,
                  role: "user",
                  role_options: []
                }
              ]
            },
            operation: { kind: "idle" }
          }
        }
      },
      sidebar: {
        ...snapshot.sidebar,
        global_dms: [
          ...snapshot.sidebar.global_dms,
          {
            room_id: dmRoom.room_id,
            display_name: "Target Member",
            avatar: null,
            tags: { favourite: null, low_priority: null },
            unread_count: 0,
            highlight_count: 0
          }
        ],
        sections: {
          ...snapshot.sidebar.sections,
          people: [
            ...snapshot.sidebar.sections.people,
            {
              room_id: dmRoom.room_id,
              display_name: "Target Member",
              avatar: null,
              tags: { favourite: null, low_priority: null },
              unread_count: 0,
              highlight_count: 0
            }
          ]
        }
      }
    });
    window.__harness.setCommandResponse("load_room_settings", () =>
      window.__harness.currentSnapshot()
    );
    window.__harness.pushStateUpdate();
    window.__harness.clearInvocations();
  }, HARNESS_ROOM_ID);

  await page.locator(".channel-actions").getByRole("button", { name: t("panel.people") }).click();
  await expect(page.getByRole("heading", { name: t("panel.people") })).toBeVisible();
  const targetMemberRow = page.locator(".people-list-row").filter({
    hasText: "Target Member"
  });
  await expect(targetMemberRow).toBeVisible();
  await targetMemberRow
    .getByRole("button", { name: t("people.openProfile", { name: "Target Member" }) })
    .click();
  const profilePanel = page.getByLabel("Context panel");
  await profilePanel.getByRole("button", { name: t("people.setAlias") }).click();
  const aliasInput = profilePanel.getByRole("textbox", { name: "Alias" });
  await aliasInput.fill("Desk Alias");
  await profilePanel.getByRole("button", { name: "Done" }).click();

  await expect.poll(() => invocationCount(page, "set_local_user_alias")).toBe(1);
  await expect
    .poll(async () =>
      page.evaluate(() => window.__harness.invocationsOf("set_local_user_alias")[0]?.args)
    )
    .toEqual({
      userId: "@target-member:example.invalid",
      alias: "Desk Alias"
    });
  await expect(profilePanel.getByRole("heading", { name: "Desk Alias" })).toBeVisible();
  await expect(profilePanel.getByText("Original: Target Member")).toBeVisible();
  await page.getByRole("button", { name: /^DMs,/ }).click();
  await expect(page.locator('[data-room-section="people"]').getByText("Desk Alias")).toBeVisible();

  await seedTimelineItems(
    page,
    [
      {
        id: { Event: { event_id: "$alias-menu-target:example.invalid" } },
        sender: "@target-member:example.invalid",
        sender_label: "Desk Alias",
        body: "Alias menu target",
        timestamp_ms: 1_800_000_003_000,
        in_reply_to_event_id: null,
        thread_root: null,
        thread_summary: null,
        reactions: [],
        can_react: true,
        is_redacted: false,
        is_hidden: false,
        can_redact: false,
        is_edited: false,
        can_edit: false
      }
    ],
    63
  );
  const timelineAliasRow = page.locator(".message").filter({ hasText: "Alias menu target" });
  await timelineAliasRow.hover();
  await timelineAliasRow.getByRole("button", { name: "Message actions" }).click();
  await timelineAliasRow.getByRole("menuitem", { name: "Edit alias for Desk Alias" }).click();
  const timelineAliasInput = page.getByRole("textbox", { name: "Alias" });
  await timelineAliasInput.fill("Timeline Alias");
  await page.getByRole("button", { name: "Done" }).click();
  await expect.poll(() => invocationCount(page, "set_local_user_alias")).toBe(2);
  await expect
    .poll(async () =>
      page.evaluate(() => window.__harness.invocationsOf("set_local_user_alias")[1]?.args)
    )
    .toEqual({
      userId: "@target-member:example.invalid",
      alias: "Timeline Alias"
    });
  await page.evaluate(async () => {
    await window.__harness.pushCoreEvent({
      kind: "Timeline",
      event: {
        DisplayLabelsUpdated: {
          labels: [
            {
              user_id: "@target-member:example.invalid",
              display_label: "Timeline Alias"
            }
          ]
        }
      }
      // eslint-disable-next-line @typescript-eslint/no-explicit-any
    } as any);
  });
  await expect(timelineAliasRow.locator(".sender")).toHaveText("Timeline Alias");
  await expect(profilePanel.getByRole("heading", { name: "Timeline Alias" })).toBeVisible();
  await expect(
    page.locator('[data-room-section="people"]').getByText("Timeline Alias")
  ).toBeVisible();

  await profilePanel.getByRole("button", { name: t("people.setAlias") }).click();
  const clearAliasInput = profilePanel.getByRole("textbox", { name: "Alias" });
  await clearAliasInput.fill("");
  await profilePanel.getByRole("button", { name: "Done" }).click();
  await expect.poll(() => invocationCount(page, "set_local_user_alias")).toBe(3);
  await expect
    .poll(async () =>
      page.evaluate(() => window.__harness.invocationsOf("set_local_user_alias")[2]?.args)
    )
    .toEqual({
      userId: "@target-member:example.invalid",
      alias: null
    });
  await expect(profilePanel.getByRole("heading", { name: "Target Member" })).toBeVisible();
  await expect(
    page.locator('[data-room-section="people"]').getByText("Target Member")
  ).toBeVisible();
  await expect(page.locator('[data-room-section="people"]').getByText("Desk Alias")).toHaveCount(0);
});

test("room tag context menu dispatches typed commands and waits for Rust section state", async ({
  page
}) => {
  await gotoReadyShell(page);
  await page.getByRole("button", { name: "Harness Space" }).click();
  const roomsSection = page.locator('[data-room-section="rooms"]');
  const favouritesSection = page.locator('[data-room-section="favourites"]');

  await expect(roomsSection.getByRole("button", { name: "Harness Room" })).toBeVisible();
  await expect(favouritesSection).toHaveCount(0);
  await page.evaluate(() => {
    window.__harness.setCommandResponse("set_room_tag", () =>
      window.__harness.currentSnapshot()
    );
    window.__harness.setCommandResponse("remove_room_tag", () =>
      window.__harness.currentSnapshot()
    );
    window.__harness.clearInvocations();
  });

  await page.getByRole("button", { name: "Harness Room" }).click({ button: "right" });
  await page.getByRole("menuitem", { name: "Add to Favourites" }).click();

  await expect.poll(() => invocationCount(page, "set_room_tag")).toBeGreaterThanOrEqual(1);
  await expect
    .poll(async () => page.evaluate(() => window.__harness.invocationsOf("set_room_tag")[0]?.args))
    .toEqual({
      roomId: HARNESS_ROOM_ID,
      tag: "favourite",
      order: null
    });
  await expect(favouritesSection).toHaveCount(0);
  await expect(roomsSection.getByRole("button", { name: "Harness Room" })).toBeVisible();

  await page.evaluate((roomId) => {
    const snapshot = window.__harness.currentSnapshot();
    const tags = { favourite: { order: null }, low_priority: null };
    const sourceItem = snapshot.sidebar.space_rooms.find((room) => room.room_id === roomId);
    if (!sourceItem) throw new Error("authoritative room item missing");
    const favouriteItem = { ...sourceItem, tags };
    window.__harness.setSnapshot({
      ...snapshot,
      state: {
        ...snapshot.state,
        domain: {
          ...snapshot.state.domain,
          rooms: snapshot.state.domain.rooms.map((room) =>
            room.room_id === roomId ? { ...room, tags } : room
          )
        }
      },
      sidebar: {
        ...snapshot.sidebar,
        space_rooms: snapshot.sidebar.space_rooms.map((room) =>
          room.room_id === roomId ? { ...room, tags } : room
        ),
        sections: {
          ...snapshot.sidebar.sections,
          rooms: snapshot.sidebar.sections.rooms.filter((room) => room.room_id !== roomId),
          favourites: [
            ...snapshot.sidebar.sections.favourites,
            favouriteItem
          ]
        }
      }
    });
    window.__harness.pushStateUpdate();
  }, HARNESS_ROOM_ID);

  await expect
    .poll(() =>
      page.evaluate(() =>
        window.__harness.currentSnapshot().sidebar.sections.favourites.map(
          (room) => room.display_name
        )
      )
    )
    .toEqual(["Harness Room"]);
  await expect(roomsSection.getByRole("button", { name: "Harness Room" })).toHaveCount(0);
  await page.evaluate(() => {
    const snapshot = window.__harness.currentSnapshot();
    window.__harness.setSnapshot({
      ...snapshot,
      state: {
        ...snapshot.state,
        ui: {
          ...snapshot.state.ui,
          room_list: {
            ...snapshot.state.ui.room_list,
            active_filter: { kind: "favourites" }
          }
        }
      }
    });
    window.__harness.pushStateUpdate();
  });
  await expect(favouritesSection.getByRole("button", { name: "Harness Room" })).toBeVisible();

  await favouritesSection.getByRole("button", { name: "Harness Room" }).click({
    button: "right"
  });
  await page.getByRole("menuitem", { name: "Remove from Favourites" }).click();

  await expect.poll(() => invocationCount(page, "remove_room_tag")).toBeGreaterThanOrEqual(1);
  await expect
    .poll(async () =>
      page.evaluate(() => window.__harness.invocationsOf("remove_room_tag")[0]?.args)
    )
    .toEqual({
      roomId: HARNESS_ROOM_ID,
      tag: "favourite"
    });
  await expect(favouritesSection.getByRole("button", { name: "Harness Room" })).toBeVisible();

  await page.evaluate((roomId) => {
    const snapshot = window.__harness.currentSnapshot();
    const tags = { favourite: null, low_priority: null };
    const sourceItem = snapshot.sidebar.sections.favourites.find(
      (room) => room.room_id === roomId
    );
    if (!sourceItem) throw new Error("authoritative favourite item missing");
    const roomItem = { ...sourceItem, tags };
    window.__harness.setSnapshot({
      ...snapshot,
      state: {
        ...snapshot.state,
        domain: {
          ...snapshot.state.domain,
          rooms: snapshot.state.domain.rooms.map((room) =>
            room.room_id === roomId ? { ...room, tags } : room
          )
        }
      },
      sidebar: {
        ...snapshot.sidebar,
        space_rooms: snapshot.sidebar.space_rooms.map((room) =>
          room.room_id === roomId ? { ...room, tags } : room
        ),
        sections: {
          ...snapshot.sidebar.sections,
          favourites: snapshot.sidebar.sections.favourites.filter(
            (room) => room.room_id !== roomId
          ),
          rooms: [
            ...snapshot.sidebar.sections.rooms,
            roomItem
          ]
        }
      }
    });
    window.__harness.pushStateUpdate();
  }, HARNESS_ROOM_ID);

  await page.evaluate(() => {
    const snapshot = window.__harness.currentSnapshot();
    window.__harness.setSnapshot({
      ...snapshot,
      state: {
        ...snapshot.state,
        ui: {
          ...snapshot.state.ui,
          room_list: {
            ...snapshot.state.ui.room_list,
            active_filter: { kind: "rooms" }
          }
        }
      }
    });
    window.__harness.pushStateUpdate();
  });
  await expect(roomsSection.getByRole("button", { name: "Harness Room" })).toBeVisible();
  await expect(favouritesSection).toHaveCount(0);
});

test("room sections follow Element-aligned order and render Rust-owned counts", async ({
  page
}) => {
  await gotoReadyShell(page);

  await page.evaluate(() => {
    const snapshot = window.__harness.currentSnapshot();
    const favouriteTags = { favourite: { order: null }, low_priority: null };
    const plainTags = { favourite: null, low_priority: null };
    const lowPriorityTags = { favourite: null, low_priority: { order: null } };
    const rooms = [
      {
        room_id: "!favourite-room:example.invalid",
        display_name: "Favourite Room",
        display_label: "Favourite Room",
        original_display_label: "Favourite Room",
        avatar: null,
        is_dm: false,
        dm_user_ids: [],
        tags: favouriteTags,
        unread_count: 1,
        notification_count: 1,
        highlight_count: 1,
        parent_space_ids: [],
        is_encrypted: false,
        joined_members: 3
      },
      {
        room_id: "!plain-room:example.invalid",
        display_name: "Plain Room",
        display_label: "Plain Room",
        original_display_label: "Plain Room",
        avatar: null,
        is_dm: false,
        dm_user_ids: [],
        tags: plainTags,
        unread_count: 1,
        notification_count: 1,
        highlight_count: 0,
        parent_space_ids: [],
        is_encrypted: false,
        joined_members: 3
      },
      {
        room_id: "!low-room:example.invalid",
        display_name: "Low Priority Room",
        display_label: "Low Priority Room",
        original_display_label: "Low Priority Room",
        avatar: null,
        is_dm: false,
        dm_user_ids: [],
        tags: lowPriorityTags,
        unread_count: 1,
        notification_count: 1,
        highlight_count: 0,
        parent_space_ids: [],
        is_encrypted: false,
        joined_members: 3
      },
      {
        room_id: "!dm-room:example.invalid",
        display_name: "Direct Person",
        display_label: "Direct Person",
        original_display_label: "Direct Person",
        avatar: null,
        is_dm: true,
        dm_user_ids: ["@direct-person:example.invalid"],
        tags: plainTags,
        unread_count: 2,
        notification_count: 2,
        highlight_count: 0,
        parent_space_ids: [],
        is_encrypted: false,
        joined_members: 2
      }
    ];
    const toRoomListItem = (room: (typeof rooms)[number]) => ({
      room_id: room.room_id,
      display_name: room.display_name,
      avatar: room.avatar,
      tags: room.tags,
      unread_count: room.unread_count,
      notification_count: room.notification_count,
      highlight_count: room.highlight_count
    });

    window.__harness.setSnapshot({
      ...snapshot,
      state: {
        ...snapshot.state,
        domain: {
          ...snapshot.state.domain,
          rooms
        },
        ui: {
          ...snapshot.state.ui,
          navigation: {
            ...snapshot.state.ui.navigation,
            active_space_id: null,
            active_room_id: "!plain-room:example.invalid"
          },
          room_list: {
            ...snapshot.state.ui.room_list,
            active_filter: { kind: "unread" },
            items: rooms.map((room) => ({ kind: "room" as const, room_id: room.room_id }))
          },
          timeline: {
            ...snapshot.state.ui.timeline,
            room_id: "!plain-room:example.invalid"
          }
        }
      },
      sidebar: {
        ...snapshot.sidebar,
        account_home: {
          ...snapshot.sidebar.account_home,
          is_active: false,
          unread_count: 1,
          // The rail badge renders the Rust-owned total; no invites here (#330).
          attention_count: 1,
          highlight_count: 1
        },
        active_space_id: null,
        space_rail: snapshot.sidebar.space_rail.map((space) => ({
          ...space,
          is_active: false,
          unread_count: 1,
          highlight_count: 1
        })),
        space_rooms: rooms.filter((room) => !room.is_dm).map(toRoomListItem),
        global_dms: rooms.filter((room) => room.is_dm).map(toRoomListItem),
        space_unread_count: 1,
        dm_unread_count: 2,
        space_highlight_count: 1,
        dm_highlight_count: 0,
        sections: {
          favourites: rooms.filter((room) => room.tags.favourite).map(toRoomListItem),
          rooms: rooms
            .filter((room) => !room.is_dm && !room.tags.favourite && !room.tags.low_priority)
            .map(toRoomListItem),
          people: rooms.filter((room) => room.is_dm).map(toRoomListItem),
          low_priority: rooms.filter((room) => room.tags.low_priority).map(toRoomListItem),
          not_joined: []
        }
      }
    });
    window.__harness.pushStateUpdate();
  });

  await expect(page.getByRole("button", { name: "DMs, 2 unread, 1 total" })).toBeVisible();
  await expect(
    page.getByRole("button", { name: "Rooms, 1 unread, 3 total, 1 mentions" })
  ).toBeVisible();

  await page.getByRole("button", { name: "DMs, 2 unread, 1 total" }).click();
  await expect(page.locator('[data-room-section="people"]')).toBeVisible();
  await expect(
    page.locator('[data-room-section="people"]').getByRole("button", { name: "Direct Person" })
  ).toBeVisible();

  await page.getByRole("button", { name: "Rooms, 1 unread, 3 total, 1 mentions" }).click();
  await expect(page.locator('[data-room-section="people"]')).toHaveCount(0);
  await expect(page.locator('[data-room-section="rooms"]')).toBeVisible();
  await expect(page.locator('[data-room-section="favourites"]')).toBeVisible();
  await expect(page.locator('[data-room-section="low-priority"]')).toBeVisible();

  await expect
    .poll(() =>
      page.locator(".sidebar .room-section").evaluateAll((sections) =>
        sections.map((section) => section.getAttribute("data-room-section"))
      )
    )
    .toEqual(["rooms", "favourites", "low-priority"]);

  await expect(page.locator('[data-room-section="favourites"] .section-count')).toHaveText("1");
  await expect(page.locator('[data-room-section="low-priority"] .section-count')).toHaveText("1");
  await expect(
    page.locator('[data-room-section="rooms"]').getByRole("button", { name: "Plain Room" })
  ).toBeVisible();

  const favouriteRoom = page
    .locator('[data-room-section="favourites"]')
    .getByRole("button", { name: "Favourite Room" });
  await expect(favouriteRoom).toHaveAttribute("data-mention-count", "1");
  await expect(favouriteRoom.locator(".room-mention-dot")).toBeVisible();
  await expect(favouriteRoom.locator(".room-count")).toHaveText("1");
  await expect(page.locator(".workspace-rail .workspace-button").first()).toHaveAttribute(
    "data-count",
    "1"
  );
  await expect(page.locator(".workspace-rail .workspace-button").first()).not.toHaveAttribute(
    "data-mention-count",
    "1"
  );

  await page.evaluate(() => {
    const snapshot = window.__harness.currentSnapshot();
    window.__harness.setSnapshot({
      ...snapshot,
      sidebar: {
        ...snapshot.sidebar,
        account_home: { ...snapshot.sidebar.account_home, is_active: true }
      }
    });
    window.__harness.pushStateUpdate();
  });
  await expect(page.locator('[data-room-section="favourites"]')).toHaveCount(0);
  await expect(page.locator('[data-room-section="low-priority"]')).toBeVisible();
});

test("category unread badges keep DMs and Rooms attention visible from Rust sidebar counts", async ({
  page
}) => {
  await gotoReadyShell(page);

  await page.evaluate(() => {
    const snapshot = window.__harness.currentSnapshot();
    const roomListItems = (prefix: string, count: number) =>
      Array.from({ length: count }, (_, index) => ({
        room_id: `!${prefix}-${index}:example.invalid`,
        display_name: `${prefix} ${index}`,
        avatar: null,
        tags: { favourite: null, low_priority: null },
        unread_count: 0,
        highlight_count: 0
      }));
    window.__harness.setSnapshot({
      ...snapshot,
      sidebar: {
        ...snapshot.sidebar,
        global_dms: roomListItems("dm", 58),
        space_rooms: roomListItems("room", 46),
        dm_unread_count: 3,
        space_unread_count: 5,
        dm_highlight_count: 0,
        space_highlight_count: 2,
        sections: {
          favourites: [],
          rooms: roomListItems("room", 46),
          people: roomListItems("dm", 58),
          low_priority: [],
          not_joined: []
        }
      }
    });
    window.__harness.pushStateUpdate();
  });

  const dms = page.getByRole("button", { name: "DMs, 3 unread, 58 total" });
  const rooms = page.getByRole("button", {
    name: "Rooms, 5 unread, 46 total, 2 mentions"
  });
  await expect(dms).toBeVisible();
  await expect(rooms).toBeVisible();
  await expect(dms.locator(".room-list-chip-total")).toHaveText("58");
  await expect(dms.locator(".room-list-chip-unread")).toHaveText("3");
  await expect(rooms.locator(".room-list-chip-total")).toHaveText("46");
  await expect(rooms.locator(".room-list-chip-unread")).toHaveText("5");
  await expect(rooms.locator(".room-list-chip-unread")).toHaveClass(/is-highlight/);

  await dms.click();
  await expect(dms).toHaveAttribute("aria-pressed", "true");
  await expect(rooms).toBeVisible();
  await expect
    .poll(() =>
      page.evaluate(
        () => window.__harness.currentSnapshot().state.domain.settings.values.sidebar.category
      )
    )
    .toBe("people");

  await page.evaluate(() => {
    const snapshot = window.__harness.currentSnapshot();
    window.__harness.setSnapshot({
      ...snapshot,
      sidebar: {
        ...snapshot.sidebar,
        dm_unread_count: 0,
        space_unread_count: 120,
        dm_highlight_count: 0,
        space_highlight_count: 0
      }
    });
    window.__harness.pushStateUpdate();
  });

  const clearedDms = page.getByRole("button", { name: "DMs, 0 unread, 58 total" });
  const largeRooms = page.getByRole("button", { name: "Rooms, 120 unread, 46 total" });
  await expect(clearedDms.locator(".room-list-chip-unread")).toHaveCount(0);
  await expect(largeRooms.locator(".room-list-chip-unread")).toHaveText("99+");
  await expect(largeRooms).toBeVisible();
});

test("notification attention snapshot drives room, space, thread, and click routing headlessly", async ({
  page
}) => {
  await gotoReadyShell(page);

  await page.evaluate(() => {
    const snapshot = window.__harness.currentSnapshot();
    const plainTags = { favourite: null, low_priority: null };
    const lowPriorityTags = { favourite: null, low_priority: { order: null } };
    const rooms = [
      {
        room_id: "!attention-room:example.invalid",
        display_name: "Attention Room",
        display_label: "Attention Room",
        original_display_label: "Attention Room",
        avatar: null,
        is_dm: false,
        dm_user_ids: [],
        tags: plainTags,
        unread_count: 4,
        notification_count: 4,
        highlight_count: 1,
        parent_space_ids: ["!attention-space:example.invalid"],
        is_encrypted: false,
        joined_members: 3
      },
      {
        room_id: "!quiet-low:example.invalid",
        display_name: "Quiet Low Priority",
        display_label: "Quiet Low Priority",
        original_display_label: "Quiet Low Priority",
        avatar: null,
        is_dm: false,
        dm_user_ids: [],
        tags: lowPriorityTags,
        unread_count: 8,
        notification_count: 8,
        highlight_count: 0,
        parent_space_ids: ["!attention-space:example.invalid"],
        is_encrypted: false,
        joined_members: 3
      }
    ];
    const toRoomListItem = (room: (typeof rooms)[number]) => ({
      room_id: room.room_id,
      display_name: room.display_name,
      avatar: room.avatar,
      tags: room.tags,
      unread_count: room.unread_count,
      notification_count: room.notification_count,
      highlight_count: room.highlight_count
    });
    const next = {
      ...snapshot,
      state: {
        ...snapshot.state,
        ui: {
          ...snapshot.state.ui,
          navigation: {
            ...snapshot.state.ui.navigation,
            active_room_id: "!quiet-low:example.invalid",
            active_space_id: "!attention-space:example.invalid"
          },
          room_list: {
            ...snapshot.state.ui.room_list,
            active_filter: { kind: "unread" }
          },
          timeline: {
            ...snapshot.state.ui.timeline,
            room_id: "!quiet-low:example.invalid",
            is_subscribed: true
          }
        },
        domain: {
          ...snapshot.state.domain,
          rooms,
          spaces: [
            {
              space_id: "!attention-space:example.invalid",
              display_name: "Attention Space",
              avatar: null,
              child_room_ids: rooms.map((room) => room.room_id)
            }
          ],
          thread_attention: {
            kind: "tracking",
            room_id: "!attention-room:example.invalid",
            root_event_id: "$attention-thread:example.invalid",
            notification_count: 2,
            highlight_count: 1,
            live_event_marker_count: 3
          },
          native_attention: {
            summary: {
              unread_count: 4,
              highlight_count: 1,
              badge_count: 4,
              candidate: {
                room_display_name: "Attention Room",
                kind: "mention",
                unread_count: 4,
                highlight_count: 1
              },
              capabilities: {
                notifications: "available",
                badge: "available",
                overlay_icon: "unavailable",
                sound: "available",
                tray: "available",
                activation: "available"
              }
            },
            dispatch: { kind: "idle" }
          }
        }
      },
      sidebar: {
        ...snapshot.sidebar,
        active_space_id: "!attention-space:example.invalid",
        account_home: {
          ...snapshot.sidebar.account_home,
          is_active: false
        },
        space_rail: [
          {
            space_id: "!attention-space:example.invalid",
            display_name: "Attention Space",
            avatar: null,
            unread_count: 4,
            highlight_count: 1,
            is_active: true
          }
        ],
        space_rooms: rooms.map(toRoomListItem),
        global_dms: [],
        space_unread_count: 4,
        dm_unread_count: 0,
        space_highlight_count: 1,
        dm_highlight_count: 0,
        sections: {
          favourites: [],
          rooms: rooms.filter((room) => !room.tags.low_priority).map(toRoomListItem),
          people: [],
          low_priority: rooms.filter((room) => room.tags.low_priority).map(toRoomListItem),
          not_joined: []
        }
      }
    };
    window.__harness.setSnapshot(next);
    window.__harness.setCommandResponse("select_room", ({ roomId }) => {
      const current = window.__harness.currentSnapshot();
      const updated = {
        ...current,
        state: {
          ...current.state,
          ui: {
            ...current.state.ui,
            navigation: {
              ...current.state.ui.navigation,
              active_room_id: String(roomId)
            },
            timeline: {
              ...current.state.ui.timeline,
              room_id: String(roomId),
              is_subscribed: true
            }
          }
        }
      };
      window.__harness.setSnapshot(updated);
      return updated;
    });
    window.__harness.pushStateUpdate();
    window.__harness.clearInvocations();
  });

  await expect(page.locator('[data-room-section="rooms"]')).toBeVisible();
  await expect(page.locator('[data-room-section="low-priority"]')).toBeVisible();
  const attentionRoom = page.getByRole("button", { name: "Attention Room" });
  const lowPriorityRoom = page.getByRole("button", { name: "Quiet Low Priority" });
  await expect(attentionRoom.locator(".room-count")).toHaveText("4");
  await expect(attentionRoom.locator(".room-mention-dot")).toBeVisible();
  await expect(lowPriorityRoom.locator(".room-count")).toHaveText("8");
  const attentionSpace = page.getByRole("button", { name: "Attention Space" });
  await expect(attentionSpace).toHaveAttribute("data-count", "4");
  await expect(attentionSpace).not.toHaveAttribute("data-mention-count", "1");
  // #330: thread attention renders on the room-header entry point. This fixture
  // tracks a thread in `!attention-room` while `!quiet-low` is open, so the
  // header carries no badge — the removed sidebar entry showed that count
  // regardless of which room was open, which is the cross-scope display this
  // issue set out to remove.
  const headerThreadsButton = page
    .locator(".channel-actions")
    .getByRole("button", { name: "Threads" });
  await expect(headerThreadsButton).toBeVisible();
  await expect(headerThreadsButton).not.toHaveAttribute("data-count", /.+/);
  await expect(headerThreadsButton).not.toHaveAttribute("data-mention-count", /.+/);
  await expect(headerThreadsButton).not.toHaveAttribute("data-live-count", /.+/);
  await expect(page).toHaveTitle("Koushi · 4 unread");

  await attentionRoom.click();
  await expect.poll(() => invocationCount(page, "select_room")).toBeGreaterThanOrEqual(1);
  await expect
    .poll(async () =>
      page.evaluate(() => window.__harness.invocationsOf("select_room")[0]?.args)
    )
    .toEqual({ roomId: "!attention-room:example.invalid" });
  await expect(attentionRoom).toHaveClass(/is-active/);
});

test("Home rail tooltip is absent while Space rail tooltip remains on hover and focus", async ({
  page
}) => {
  await gotoReadyShell(page);

  const rail = page.getByRole("navigation", { name: "Workspaces" });
  const spaceButton = rail.getByRole("button", { name: "Harness Space" });
  const accountButton = rail.getByRole("button", { name: "Home" });

  await spaceButton.hover();
  const spaceTooltip = page.getByRole("tooltip", { name: "Harness Space" });
  await expect(spaceTooltip).toBeVisible();
  const tooltipId = await spaceTooltip.getAttribute("id");
  if (!tooltipId) {
    throw new Error("workspace tooltip id missing");
  }
  await expect(spaceButton).toHaveAttribute("aria-describedby", tooltipId);

  await page.keyboard.press("Escape");
  await expect(spaceTooltip).toBeHidden();
  await expect(spaceButton).not.toHaveAttribute("aria-describedby", /.+/);

  await accountButton.hover();
  await expect(page.getByRole("tooltip", { name: "Home" })).toHaveCount(0);
  await accountButton.focus();
  await expect(page.getByRole("tooltip", { name: "Home" })).toHaveCount(0);
  await expect(accountButton).not.toHaveAttribute("title", /.+/);
  await expect(accountButton).not.toHaveAttribute("aria-describedby", /.+/);

  await spaceButton.focus();
  await expect(spaceTooltip).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(spaceTooltip).toBeHidden();
  await expect(spaceButton).not.toHaveAttribute("aria-describedby", /.+/);
});

test("room selection keeps a delta that arrives before its command receipt", async ({
  page
}) => {
  await gotoReadyShell(page);
  await page.evaluate(() => {
    const primaryRoomId = "!harness-room:example.invalid";
    const secondaryRoomId = "!delta-selected-room:example.invalid";
    const current = window.__harness.currentSnapshot();
    const secondaryRoom = {
      room_id: secondaryRoomId,
      display_name: "Delta Selected Room",
      display_label: "Delta Selected Room",
      original_display_label: "Delta Selected Room",
      avatar: null,
      is_dm: false,
      dm_user_ids: [],
      tags: { favourite: null, low_priority: null },
      unread_count: 0,
      notification_count: 0,
      highlight_count: 0,
      parent_space_ids: []
    };
    const rooms = [
      ...current.state.domain.rooms.filter((room) => room.room_id !== secondaryRoomId),
      secondaryRoom
    ];
    const roomListItems = rooms.map((room) => ({
      room_id: room.room_id,
      display_name: room.display_label,
      avatar: room.avatar,
      tags: room.tags,
      unread_count: room.unread_count,
      highlight_count: room.highlight_count
    }));
    const roomProjectionItems = rooms.map((room) => ({
      kind: "room" as const,
      room_id: room.room_id
    }));
    const staleSnapshot = {
      ...current,
      state_generation: 1,
      state: {
        ...current.state,
        domain: {
          ...current.state.domain,
          rooms
        },
        ui: {
          ...current.state.ui,
          navigation: {
            ...current.state.ui.navigation,
            active_space_id: null,
            active_room_id: primaryRoomId
          },
          room_list: {
            ...current.state.ui.room_list,
            active_filter: { kind: "rooms" },
            items: roomProjectionItems
          },
          timeline: {
            ...current.state.ui.timeline,
            room_id: primaryRoomId,
            is_subscribed: true
          },
          thread: { kind: "closed" },
          focused_context: { kind: "closed" }
        }
      },
      sidebar: {
        ...current.sidebar,
        active_space_id: null,
        account_home: {
          ...current.sidebar.account_home,
          is_active: true
        },
        space_rail: current.sidebar.space_rail.map((space) => ({
          ...space,
          is_active: false
        })),
        space_rooms: roomListItems,
        sections: { ...current.sidebar.sections, rooms: roomListItems }
      },
      thread: null
    };
    window.__harness.setSnapshot(staleSnapshot);
    window.__harness.setCommandResponse("select_room", async ({ roomId }: { roomId: string }) => {
      const targetRoomId = String(roomId);
      const generation =
        (window.__harness.currentSnapshot().state_generation ?? 0) + 1;
      await window.__harness.pushStateUpdate({
        protocol_version: 1,
        kind: "delta",
        generation,
        changed: {
          state: {
            ui: {
              navigation: {
                ...staleSnapshot.state.ui.navigation,
                active_room_id: targetRoomId
              },
              room_list: staleSnapshot.state.ui.room_list,
              timeline: {
                ...staleSnapshot.state.ui.timeline,
                room_id: targetRoomId,
                is_subscribed: true
              },
              thread: { kind: "closed" },
              focused_context: { kind: "closed" }
            }
          },
          sidebar: {
            ...staleSnapshot.sidebar,
            space_rooms: roomListItems,
            sections: { ...staleSnapshot.sidebar.sections, rooms: roomListItems }
          },
          thread: null
        }
      });
      return { protocolVersion: 1, publishedGeneration: generation };
    });
    window.__harness.pushStateUpdate();
    window.__harness.clearInvocations();
  });

  await page.getByRole("button", { name: /^Rooms,/ }).click();
  const targetRoom = page.getByRole("button", { name: "Delta Selected Room" });
  await expect(targetRoom).toBeVisible();
  await targetRoom.click();

  await expect
    .poll(async () => page.evaluate(() => window.__harness.invocationsOf("select_room")[0]?.args))
    .toEqual({ roomId: "!delta-selected-room:example.invalid" });
  await expect(targetRoom).toHaveClass(/is-active/);
  await expect(page.locator(".channel-title").first()).toContainText("Delta Selected Room");
});

test("room selection ignores unrelated avatar thumbnail bursts headlessly", async ({
  page
}) => {
  await gotoReadyShell(page);
  await page.evaluate(() => {
    const current = window.__harness.currentSnapshot();
    const targetRoomId = "!avatar-burst-target:example.invalid";
    const targetRoom = {
      room_id: targetRoomId,
      display_name: "Avatar Burst Target",
      display_label: "Avatar Burst Target",
      original_display_label: "Avatar Burst Target",
      avatar: null,
      is_dm: false,
      dm_user_ids: [],
      tags: { favourite: null, low_priority: null },
      unread_count: 0,
      notification_count: 0,
      highlight_count: 0,
      parent_space_ids: []
    };
    const rooms = [
      ...current.state.domain.rooms.filter((room) => room.room_id !== targetRoomId),
      targetRoom
    ];
    const spaceRooms = [
      ...current.sidebar.space_rooms.filter((room) => room.room_id !== targetRoomId),
      {
        room_id: targetRoom.room_id,
        display_name: targetRoom.display_label,
        avatar: targetRoom.avatar,
        tags: targetRoom.tags,
        unread_count: targetRoom.unread_count,
        highlight_count: targetRoom.highlight_count
      }
    ];
    const seeded = {
      ...current,
      state: {
        ...current.state,
        domain: {
          ...current.state.domain,
          rooms
        }
      },
      sidebar: {
        ...current.sidebar,
        space_rooms: spaceRooms,
        sections: { ...current.sidebar.sections, rooms: spaceRooms }
      }
    };
    window.__harness.setSnapshot(seeded);
    window.__harness.setCommandResponse("select_room", ({ roomId }: { roomId: string }) => {
      const snapshot = window.__harness.currentSnapshot();
      const selectedRoomId = String(roomId);
      const next = {
        ...snapshot,
        state_generation: snapshot.state_generation + 1,
        state: {
          ...snapshot.state,
          ui: {
            ...snapshot.state.ui,
            navigation: {
              ...snapshot.state.ui.navigation,
              active_room_id: selectedRoomId
            },
            timeline: {
              ...snapshot.state.ui.timeline,
              room_id: selectedRoomId,
              is_subscribed: true
            },
            thread: { kind: "closed" },
            focused_context: { kind: "closed" }
          }
        },
        sidebar: {
          ...snapshot.sidebar,
          space_rooms: spaceRooms,
          sections: { ...snapshot.sidebar.sections, rooms: spaceRooms }
        },
        thread: null
      };
      window.__harness.setSnapshot(next);
      return next;
    });
    window.__harness.pushStateUpdate();
    window.__harness.clearInvocations();
  });

  await seedTimelineItems(page, [
    {
      id: { Event: { event_id: "$avatar-burst-mounted:example.invalid" } },
      sender: "@avatar-burst-sender:example.invalid",
      sender_label: "Avatar Burst Sender",
      body: "Mounted avatar row",
      timestamp_ms: 1_800_000_011_000,
      in_reply_to_event_id: null,
      thread_root: null,
      thread_summary: null,
      sender_avatar: {
        mxc_uri: "mxc://example.invalid/avatar-burst-mounted",
        thumbnail: { kind: "notRequested" }
      },
      media: null,
      is_redacted: false,
      is_hidden: false,
      can_redact: false,
      is_edited: false,
      can_edit: false,
      reactions: []
    }
  ]);

  await page.evaluate(async () => {
    for (let index = 0; index < 40; index += 1) {
      await window.__harness.pushCoreEvent({
        kind: "Account",
        event: {
          AvatarThumbnailDownloaded: {
            request_id: { connection_id: 3, sequence: 10_000 + index },
            mxc_uri: `mxc://example.invalid/unrelated-avatar-${index}`,
            thumbnail: {
              kind: "ready",
              source_url: "data:image/gif;base64,R0lGODlhAQABAAAAACw=",
              width: 1,
              height: 1,
              mime_type: "image/gif"
            }
          }
        }
        // eslint-disable-next-line @typescript-eslint/no-explicit-any
      } as any);
    }
  });

  const targetRoom = page.getByRole("button", { name: "Avatar Burst Target" });
  await expect(targetRoom).toBeVisible();
  await targetRoom.click();

  await expect
    .poll(async () => page.evaluate(() => window.__harness.invocationsOf("select_room")[0]?.args))
    .toEqual({ roomId: "!avatar-burst-target:example.invalid" });
  await expect(targetRoom).toHaveClass(/is-active/);
  await expect(page.locator(".channel-title").first()).toContainText("Avatar Burst Target");

  await page.getByRole("button", { name: "Open diagnostics" }).click();
  const report = page.locator(".diagnostics-output");
  await expect(report).toContainText("timeline_matches_active=true");
  await expect(report).not.toContainText("avatar thumbnail ready");
});

test("room context menu mark unread dispatches Rust-owned commands", async ({ page }) => {
  await gotoReadyShell(page);
  await page.evaluate(() => {
    const snapshot = window.__harness.currentSnapshot();
    window.__harness.setSnapshot({
      ...snapshot,
      state: {
        ...snapshot.state,
        domain: {
          ...snapshot.state.domain,
          rooms: [
            ...snapshot.state.domain.rooms,
            {
              room_id: "!room-alpha:example.invalid",
              display_name: "Room Alpha",
              display_label: "Room Alpha",
              original_display_label: "Room Alpha",
              avatar: null,
              is_dm: false,
              dm_user_ids: [],
              tags: { favourite: null, low_priority: null },
              unread_count: 3,
              notification_count: 0,
              highlight_count: 0,
              parent_space_ids: []
            },
            {
              room_id: "!room-beta:example.invalid",
              display_name: "Room Beta",
              display_label: "Room Beta",
              original_display_label: "Room Beta",
              avatar: null,
              is_dm: false,
              dm_user_ids: [],
              tags: { favourite: null, low_priority: null },
              unread_count: 0,
              notification_count: 0,
              highlight_count: 0,
              parent_space_ids: []
            },
            {
              room_id: "!dm-alpha:example.invalid",
              display_name: "DM Alpha",
              display_label: "DM Alpha",
              original_display_label: "DM Alpha",
              avatar: null,
              is_dm: true,
              dm_user_ids: ["@dm-alpha:example.invalid"],
              tags: { favourite: null, low_priority: null },
              unread_count: 0,
              notification_count: 0,
              highlight_count: 0,
              parent_space_ids: []
            }
          ]
        },
        ui: {
          ...snapshot.state.ui,
          room_list: {
            readiness: { kind: "ready", source: "cache", generation: 0 },
            active_filter: { kind: "rooms" },
            sort: { kind: "activity" },
            items: [
              { room_id: "!room-alpha:example.invalid", kind: "room" },
              { room_id: "!room-beta:example.invalid", kind: "room" },
              { room_id: "!dm-alpha:example.invalid", kind: "room" }
            ]
          }
        }
      },
      sidebar: {
        ...snapshot.sidebar,
        space_rooms: [
          { ...snapshot.sidebar.space_rooms[0], room_id: "!room-alpha:example.invalid", display_name: "Room Alpha", unread_count: 3 },
          { ...snapshot.sidebar.space_rooms[0], room_id: "!room-beta:example.invalid", display_name: "Room Beta", unread_count: 0 }
        ],
        global_dms: [
          { ...snapshot.sidebar.space_rooms[0], room_id: "!dm-alpha:example.invalid", display_name: "DM Alpha", unread_count: 0 }
        ],
        sections: {
          favourites: [],
          rooms: [
            { ...snapshot.sidebar.space_rooms[0], room_id: "!room-alpha:example.invalid", display_name: "Room Alpha", unread_count: 3 },
            { ...snapshot.sidebar.space_rooms[0], room_id: "!room-beta:example.invalid", display_name: "Room Beta", unread_count: 0 }
          ],
          people: [
            { ...snapshot.sidebar.space_rooms[0], room_id: "!dm-alpha:example.invalid", display_name: "DM Alpha", unread_count: 0 }
          ],
          low_priority: [],
          not_joined: []
        }
      }
    });
    window.__harness.pushStateUpdate();
    window.__harness.clearInvocations();
  });

  const alphaRow = page.getByTestId("room-item").filter({ hasText: "Room Alpha" }).first();
  await expect(alphaRow).toBeVisible();

  await alphaRow.click({ button: "right" });
  await page.getByRole("menuitem", { name: t("room.markAsUnread") }).click();
  await expect.poll(() => invocationCount(page, "mark_room_as_unread")).toBe(1);
  await expect
    .poll(async () =>
      page.evaluate(() => window.__harness.invocationsOf("mark_room_as_unread")[0]?.args)
    )
    .toEqual({ roomId: "!room-alpha:example.invalid", unread: true });
});

test("room context menu reports room with a reason", async ({ page }) => {
  await gotoReadyShell(page);
  await page.evaluate(() => {
    window.__harness.setCommandResponse("report_room", () => window.__harness.currentSnapshot());
    window.__harness.clearInvocations();
  });

  await page.getByRole("button", { name: "Harness Room" }).click({ button: "right" });
  await page.getByRole("menuitem", { name: "Report room" }).click();

  const reasonInput = page.getByRole("textbox", { name: "Reason" });
  await expect(reasonInput).toBeVisible();
  await reasonInput.fill("Toxic room");
  await page.getByRole("button", { name: "Report", exact: true }).click();

  await expect.poll(() => invocationCount(page, "report_room")).toBeGreaterThanOrEqual(1);
  await expect
    .poll(async () => page.evaluate(() => window.__harness.invocationsOf("report_room")[0]?.args))
    .toEqual({
      roomId: HARNESS_ROOM_ID,
      reason: "Toxic room"
    });
});

test("timeline sender profile navigation uses stable user ids and latest-wins settings", async ({ page }) => {
  await gotoReadyShell(page);
  const firstUserId = "@duplicate-first:example.invalid";
  const secondUserId = "@duplicate-second:example.invalid";
  await seedTimelineItems(page, [
    {
      id: { Event: { event_id: "$duplicate-first:example.invalid" } },
      sender: firstUserId,
      sender_label: "Duplicate Name",
      body: "First duplicate sender",
      timestamp_ms: 1_800_000_020_000,
      in_reply_to_event_id: null,
      thread_root: null,
      thread_summary: null,
      media: null,
      is_redacted: false,
      is_hidden: false,
      can_redact: false,
      is_edited: false,
      can_edit: false,
      reactions: []
    },
    {
      id: { Event: { event_id: "$duplicate-second:example.invalid" } },
      sender: secondUserId,
      sender_label: "Duplicate Name",
      body: "Second duplicate sender",
      timestamp_ms: 1_800_000_021_000,
      in_reply_to_event_id: null,
      thread_root: null,
      thread_summary: null,
      media: null,
      is_redacted: false,
      is_hidden: false,
      can_redact: false,
      is_edited: false,
      can_edit: false,
      reactions: []
    },
    {
      id: { Event: { event_id: "$duplicate-continuation:example.invalid" } },
      sender: secondUserId,
      sender_label: "Duplicate Name",
      body: "Continuation sender",
      timestamp_ms: 1_800_000_021_500,
      in_reply_to_event_id: null,
      thread_root: null,
      thread_summary: null,
      media: null,
      is_redacted: false,
      is_hidden: false,
      can_redact: false,
      is_edited: false,
      can_edit: false,
      reactions: []
    },
    {
      id: { Event: { event_id: "$missing-sender:example.invalid" } },
      sender: null,
      sender_label: "Unbound Sender",
      body: "Missing stable sender id",
      timestamp_ms: 1_800_000_022_000,
      in_reply_to_event_id: null,
      thread_root: null,
      thread_summary: null,
      media: null,
      is_redacted: false,
      is_hidden: false,
      can_redact: false,
      is_edited: false,
      can_edit: false,
      reactions: []
    }
  ]);
  await page.evaluate(
    ({ firstUserId: first, secondUserId: second }) => {
      const pending: Array<{ roomId: string; resolve: (snapshot: ReturnType<typeof window.__harness.currentSnapshot>) => void }> = [];
      window.__harness.setCommandResponse("load_room_settings", ({ roomId }) =>
        new Promise((resolve) => pending.push({ roomId: String(roomId), resolve }))
      );
      const release = (index: number) => {
        const request = pending[index];
        if (!request) throw new Error(`missing profile load ${index}`);
        const current = window.__harness.currentSnapshot();
        const member = (user_id: string) => ({
          user_id,
          display_name: "Duplicate Name",
          display_label: "Duplicate Name",
          original_display_label: "Duplicate Name",
          avatar_url: null,
          power_level: 0,
          role: "user" as const,
          role_options: []
        });
        const next = {
          ...current,
          state: {
            ...current.state,
            domain: {
              ...current.state.domain,
              room_management: {
                selected_room_id: request.roomId,
                settings: {
                  room_id: request.roomId,
                  name: "Harness Room",
                  topic: null,
                  avatar_url: null,
                  join_rule: "invite" as const,
                  history_visibility: "shared" as const,
                  permissions: {
                    can_edit_settings: true,
                    can_edit_roles: true,
                    can_invite: true,
                    can_kick: true,
                    can_ban: false,
                    can_unban: false
                  },
                  members: [member(first), member(second)]
                },
                operation: { kind: "idle" as const }
              }
            }
          }
        };
        window.__harness.setSnapshot(next);
        request.resolve(next);
      };
      (window as unknown as { __releaseProfileLoad: (index: number) => void }).__releaseProfileLoad = release;
      window.__harness.clearInvocations();
    },
    { firstUserId, secondUserId }
  );

  const firstRow = page.locator("article.message").filter({ hasText: "First duplicate sender" });
  const secondRow = page.locator("article.message").filter({ hasText: "Second duplicate sender" });
  const missingSenderRow = page.locator("article.message").filter({ hasText: "Missing stable sender id" });
  const continuationRow = page.locator("article.message").filter({ hasText: "Continuation sender" });
  await expect(missingSenderRow).toBeVisible();
  await expect(continuationRow).toBeVisible();
  await expect(missingSenderRow.getByRole("button", { name: /Open profile for/ })).toHaveCount(0);
  await expect(continuationRow.getByRole("button", { name: /Open profile for/ })).toHaveCount(0);
  const firstSender = firstRow.getByRole("button", { name: "Open profile for Duplicate Name" });
  await page.keyboard.press("Tab");
  await firstSender.focus();
  await expect(firstSender).toBeFocused();
  expect(await firstSender.evaluate((element) => element.matches(":focus-visible"))).toBe(true);
  expect(await firstSender.evaluate((element) => getComputedStyle(element).outlineStyle)).not.toBe("none");
  await firstSender.press("Enter");
  await expect(page.getByRole("heading", { name: t("panel.profile") })).toHaveCount(0);
  await expect(page.getByRole("heading", { name: t("panel.people") })).toHaveCount(0);
  await secondRow.getByRole("button", { name: "Open profile for Duplicate Name" }).click();
  await expect.poll(() => invocationCount(page, "load_room_settings")).toBe(2);

  await page.evaluate(() =>
    (window as unknown as { __releaseProfileLoad: (index: number) => void }).__releaseProfileLoad(1)
  );
  await expect(page.getByRole("heading", { name: t("panel.profile") })).toBeVisible();
  await expect(page.locator(".profile-identity")).toContainText(secondUserId);
  await page.evaluate(() =>
    (window as unknown as { __releaseProfileLoad: (index: number) => void }).__releaseProfileLoad(0)
  );
  await expect(page.locator(".profile-identity")).toContainText(secondUserId);

  await page.getByRole("button", { name: t("action.back"), exact: true }).click();
  await expect(page.getByRole("heading", { name: t("panel.people") })).toBeVisible();
  await expect(
    page
      .locator('aside[aria-label="Context panel"]')
      .getByRole("button", { name: "Open profile for Duplicate Name" })
  ).toHaveCount(2);
  await page.getByRole("button", {
    name: t("action.close", { title: t("panel.people") }),
    exact: true
  }).click();

  await firstSender.focus();
  await firstSender.press("Space");
  await expect.poll(() => invocationCount(page, "load_room_settings")).toBe(3);
  await page
    .getByRole("navigation", { name: t("workspace.workspaces") })
    .getByRole("button", { name: /^Home/ })
    .click();
  await page.evaluate(() =>
    (window as unknown as { __releaseProfileLoad: (index: number) => void }).__releaseProfileLoad(2)
  );
  await expect(page.getByRole("heading", { name: t("panel.profile") })).toHaveCount(0);
});

test("room member panel ignores, unignores, and reports a user", async ({ page }) => {
  await gotoReadyShell(page);
  const targetUserId = "@target-member:example.invalid";

  await page.evaluate(
    ({ roomId, targetUserId: userId }) => {
      window.__harness.setCommandResponse("load_room_settings", ({ roomId: incomingRoomId }) => {
        const current = window.__harness.currentSnapshot();
        const next = {
          ...current,
          state: {
            ...current.state,
            domain: {
              ...current.state.domain,
              room_management: {
                selected_room_id: String(incomingRoomId),
                settings: {
                  room_id: String(incomingRoomId),
                  name: "Harness Room",
                  topic: null,
                  avatar_url: null,
                  join_rule: "invite",
                  history_visibility: "shared",
                  permissions: {
                    can_edit_settings: true,
                    can_edit_roles: true,
                    can_invite: true,
                    can_kick: true,
                    can_ban: false,
                    can_unban: false
                  },
                  members: [
                    {
                      user_id: userId,
                      display_name: "Target Member",
                      display_label: "Target Member",
                      original_display_label: "Target Member",
                      avatar_url: null,
                      power_level: 0,
                      role: "user",
                      role_options: []
                    }
                  ]
                },
                operation: { kind: "idle" }
              }
            }
          }
        };
        window.__harness.setSnapshot(next);
        return next;
      });
      window.__harness.setCommandResponse("ignore_user", ({ userId: incomingUserId }) => {
        const current = window.__harness.currentSnapshot();
        const next = {
          ...current,
          state: {
            ...current.state,
            domain: {
              ...current.state.domain,
              profile: {
                ...current.state.domain.profile,
                ignored_user_ids: [...current.state.domain.profile.ignored_user_ids, String(incomingUserId)]
              }
            }
          }
        };
        window.__harness.setSnapshot(next);
        return next;
      });
      window.__harness.setCommandResponse("unignore_user", ({ userId: incomingUserId }) => {
        const current = window.__harness.currentSnapshot();
        const next = {
          ...current,
          state: {
            ...current.state,
            domain: {
              ...current.state.domain,
              profile: {
                ...current.state.domain.profile,
                ignored_user_ids: current.state.domain.profile.ignored_user_ids.filter(
                  (id) => id !== String(incomingUserId)
                )
              }
            }
          }
        };
        window.__harness.setSnapshot(next);
        return next;
      });
      window.__harness.setCommandResponse("report_user", () => window.__harness.currentSnapshot());
      window.__harness.clearInvocations();
    },
    { roomId: HARNESS_ROOM_ID, targetUserId }
  );

  await page.locator(".channel-actions").getByRole("button", { name: t("panel.people") }).click();
  await expect(page.getByRole("heading", { name: t("panel.people") })).toBeVisible();
  const targetMemberRow = page.locator(".people-list-row").filter({ hasText: "Target Member" });
  await expect(targetMemberRow).toBeVisible();
  await targetMemberRow
    .getByRole("button", { name: t("people.openProfile", { name: "Target Member" }) })
    .click();
  const profilePanel = page.getByLabel("Context panel");

  await profilePanel.getByRole("button", { name: "Ignore" }).click();
  await expect.poll(() => invocationCount(page, "ignore_user")).toBeGreaterThanOrEqual(1);
  await expect
    .poll(async () => page.evaluate(() => window.__harness.invocationsOf("ignore_user")[0]?.args))
    .toEqual({ userId: targetUserId });
  await expect(profilePanel.getByRole("button", { name: "Unignore" })).toBeVisible();

  await profilePanel.getByRole("button", { name: "Unignore" }).click();
  await expect.poll(() => invocationCount(page, "unignore_user")).toBeGreaterThanOrEqual(1);
  await expect
    .poll(async () => page.evaluate(() => window.__harness.invocationsOf("unignore_user")[0]?.args))
    .toEqual({ userId: targetUserId });
  await expect(profilePanel.getByRole("button", { name: "Ignore" })).toBeVisible();

  await profilePanel.getByRole("button", { name: "Report user" }).click();
  const reasonInput = page.getByRole("textbox", { name: "Reason" });
  await expect(reasonInput).toBeVisible();
  await reasonInput.fill("Harassment");
  await page.getByRole("button", { name: "Report", exact: true }).click();

  await expect.poll(() => invocationCount(page, "report_user")).toBeGreaterThanOrEqual(1);
  await expect
    .poll(async () => page.evaluate(() => window.__harness.invocationsOf("report_user")[0]?.args))
    .toEqual({
      userId: targetUserId,
      reason: "Harassment"
    });
});

test("room header People button opens People panel and shows the Rust-owned member count", async ({
  page
}) => {
  await gotoReadyShell(page);

  const peopleButton = page
    .locator(".channel-actions")
    .getByRole("button", { name: t("panel.people") });
  await expect(peopleButton).toBeVisible();
  await peopleButton.click();

  await expect(page.getByRole("heading", { name: t("panel.people") })).toBeVisible();
  await expect(page.getByText(t("people.memberCount", { count: "3" }))).toBeVisible();
});

test("People reopens immediately and a late settings load cannot override Threads", async ({ page }) => {
  await gotoReadyShell(page);
  await page.evaluate(() => {
    const releases: Array<() => void> = [];
    window.__harness.setCommandResponse("load_room_settings", () =>
      new Promise((resolve) => {
        releases.push(() => resolve(window.__harness.currentSnapshot()));
      })
    );
    window.__harness.setCommandResponse("open_threads_list", () => {
      const snapshot = window.__harness.currentSnapshot();
      const next = {
        ...snapshot,
        state: {
          ...snapshot.state,
          ui: {
            ...snapshot.state.ui,
            threads_list: {
              kind: "open" as const,
              room_id: "!harness-room:example.invalid",
              request_id: 1,
              items: [],
              is_paginating: false,
              end_reached: true
            }
          }
        }
      };
      window.__harness.setSnapshot(next);
      return next;
    });
    (window as unknown as { __releasePeopleLoads: () => void }).__releasePeopleLoads = () => {
      releases.splice(0).forEach((release) => release());
    };
  });

  const actions = page.locator(".channel-actions");
  const peopleButton = actions.getByRole("button", { name: t("panel.people") });
  await peopleButton.click();
  await expect(page.getByRole("heading", { name: t("panel.people") })).toBeVisible();
  await page.getByLabel("Context panel").getByRole("button", { name: "Close" }).click();
  await expect(page.getByRole("heading", { name: t("panel.people") })).toBeHidden();

  await peopleButton.click();
  await expect(page.getByRole("heading", { name: t("panel.people") })).toBeVisible();
  await actions.getByRole("button", { name: "Threads" }).click();
  const threadsTitle = page
    .locator('aside[aria-label="Context panel"]')
    .getByText(t("threads.title"), { exact: true });
  await expect(threadsTitle).toBeVisible();

  await page.evaluate(() =>
    (window as unknown as { __releasePeopleLoads: () => void }).__releasePeopleLoads()
  );
  await expect(threadsTitle).toBeVisible();
  await expect(page.getByRole("heading", { name: t("panel.people") })).toBeHidden();
});

test("room info People entry opens the standalone People panel", async ({ page }) => {
  await gotoReadyShell(page);
  await page.getByRole("button", { name: t("room.roomInfo") }).click();

  const peopleButton = page
    .getByLabel("Context panel")
    .getByRole("button", { name: t("room.people"), exact: true });
  await expect(peopleButton).toBeEnabled();
  await peopleButton.click();

  await expect(page.getByRole("heading", { name: t("panel.people") })).toBeVisible();
});
