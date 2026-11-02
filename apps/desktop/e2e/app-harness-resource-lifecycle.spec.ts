import { expect, test, type Page } from "@playwright/test";

type MainTarget = { kind: "main"; room_id: string };

type Lease = {
  rendererGeneration: string;
  leaseId: string;
};

const MAIN_TARGET: MainTarget = { kind: "main", room_id: "!harness-room:example.invalid" };
const STAGED_ID = "staged-resource-lifecycle";
const VARIANT_ID = "original-keep";
const ACCOUNT = {
  accountHomeserver: "https://harness.example.invalid",
  accountUserId: "@harness-user:example.invalid",
  accountDeviceId: "HARNESSDEVICE"
};
const SEED_EVENT_ID = "$seed-event:example.invalid";

async function gotoReadyShell(page: Page): Promise<void> {
  await page.goto("/appHarness.html");
  await expect(page.getByRole("main", { name: "Conversation timeline" })).toBeVisible();
  await expect(page.getByRole("button", { name: "Reply to message" }).first()).toBeVisible();
}

async function invoke<T = unknown>(
  page: Page,
  command: string,
  args: Record<string, unknown> = {}
): Promise<T> {
  return page.evaluate(
    async ({ commandName, commandArgs }) =>
      (await window.__harness.invoke(commandName, commandArgs)) as T,
    { commandName: command, commandArgs: args }
  );
}

async function expectRejected(
  page: Page,
  command: string,
  args: Record<string, unknown>
): Promise<void> {
  await expect(
    page.evaluate(
      ({ commandName, commandArgs }) => window.__harness.invoke(commandName, commandArgs),
      { commandName: command, commandArgs: args }
    )
  ).rejects.toThrow();
}

async function stageUpload(page: Page, target: MainTarget = MAIN_TARGET): Promise<void> {
  await invoke(page, "stage_upload_bytes", {
    target,
    items: [
      {
        stagedId: STAGED_ID,
        position: 1,
        filename: "resource-lifecycle.txt",
        mimeType: "text/plain",
        bytes: [11, 22, 33]
      }
    ]
  });
}

async function preview(page: Page, target: MainTarget = MAIN_TARGET): Promise<number[]> {
  return invoke<number[]>(page, "prepared_upload_preview", {
    target,
    stagedId: STAGED_ID,
    variantId: VARIANT_ID
  });
}

async function beginLease(page: Page): Promise<string> {
  return invoke<string>(page, "begin_composer_draft_renderer_generation");
}

async function acquireLease(
  page: Page,
  rendererGeneration: string,
  target: MainTarget = MAIN_TARGET,
  account = ACCOUNT
): Promise<Lease> {
  return invoke<Lease>(page, "acquire_composer_draft_lease", {
    ...account,
    target,
    rendererGeneration
  });
}

/**
 * Lease the composer with a renderer generation this test began.
 *
 * The App runs its own composer renderer-generation cycle whenever a snapshot
 * changes its account or open timelines, and a generation it begins between our
 * begin and our acquire invalidates the one we picked. Retrying the pair keeps
 * the assertion - a renderer generation begun here can lease this composer -
 * without pinning the test to App boot and render order. The rejected
 * generation and account cases are asserted separately and are unaffected.
 */
async function leaseCurrentComposer(
  page: Page,
  target: MainTarget = MAIN_TARGET,
  account = ACCOUNT
): Promise<Lease> {
  let leased: Lease | null = null;
  await expect
    .poll(async () => {
      const rendererGeneration = await beginLease(page);
      try {
        leased = await acquireLease(page, rendererGeneration, target, account);
        return true;
      } catch {
        return false;
      }
    })
    .toBe(true);
  return leased!;
}

async function releaseLease(page: Page, lease: Lease): Promise<void> {
  await invoke(page, "release_composer_draft_lease", {
    leaseId: lease.leaseId,
    rendererGeneration: lease.rendererGeneration
  });
}

async function replaceActiveMainRoom(page: Page, roomId: string): Promise<void> {
  await page.evaluate((nextRoomId) => {
    const next = structuredClone(window.__harness.currentSnapshot());
    const currentRoom = next.state.domain.rooms[0];
    if (currentRoom) {
      next.state.domain.rooms = [
        ...next.state.domain.rooms,
        {
          ...currentRoom,
          room_id: nextRoomId,
          display_name: "Replacement Room",
          display_label: "Replacement Room",
          original_display_label: "Replacement Room",
          parent_space_ids: []
        }
      ];
    }
    next.state.ui.navigation.active_room_id = nextRoomId;
    next.state.ui.timeline.room_id = nextRoomId;
    next.state.ui.timeline.staged_uploads = [];
    window.__harness.setSnapshot(next);
  }, roomId);
}

async function replaceReadyAccountRetainingStagedProjection(page: Page): Promise<void> {
  await page.evaluate(() => {
    const next = structuredClone(window.__harness.currentSnapshot());
    next.state.domain.session = {
      kind: "ready",
      homeserver: "https://other-account.example.invalid",
      user_id: "@other-account:example.invalid",
      device_id: "OTHERDEVICE"
    };
    window.__harness.setSnapshot(next);
  });
}

test("composer transport uses canonical numeric tokens", async ({ page }) => {
  await gotoReadyShell(page);
  const lease = await leaseCurrentComposer(page);

  expect(lease.rendererGeneration).toMatch(/^[1-9][0-9]*$/);
  expect(lease.leaseId).toMatch(/^[1-9][0-9]*$/);
});

test("clear staging releases prepared bytes", async ({ page }) => {
  await gotoReadyShell(page);
  await stageUpload(page);
  await expect.poll(() => preview(page)).toEqual([11, 22, 33]);

  await invoke(page, "clear_upload_staging", { target: MAIN_TARGET });

  await expect.poll(() => preview(page)).toEqual([]);
});

test("replacing the active target releases prepared bytes", async ({ page }) => {
  await gotoReadyShell(page);
  await stageUpload(page);
  await expect.poll(() => preview(page)).toEqual([11, 22, 33]);

  await replaceActiveMainRoom(page, "!replacement-room:example.invalid");

  await expect.poll(() => preview(page)).toEqual([]);
});

test("logout releases bytes even when its fixture retains staged projections", async ({ page }) => {
  await gotoReadyShell(page);
  await stageUpload(page);
  await expect.poll(() => preview(page)).toEqual([11, 22, 33]);

  await invoke(page, "logout");

  await expect.poll(() => preview(page)).toEqual([]);
});

test("replacing the Ready account releases stale staged projections", async ({ page }) => {
  await gotoReadyShell(page);
  await stageUpload(page);
  await expect.poll(() => preview(page)).toEqual([11, 22, 33]);

  await replaceReadyAccountRetainingStagedProjection(page);

  await expect.poll(() => preview(page)).toEqual([]);
});

test("target retirement rejects release and acquire for the old lease", async ({ page }) => {
  await gotoReadyShell(page);
  const lease = await leaseCurrentComposer(page);

  await replaceActiveMainRoom(page, "!retired-target-room:example.invalid");

  await expectRejected(page, "release_composer_draft_lease", {
    leaseId: lease.leaseId,
    rendererGeneration: lease.rendererGeneration
  });
  await expectRejected(page, "acquire_composer_draft_lease", {
    ...ACCOUNT,
    target: MAIN_TARGET,
    rendererGeneration: lease.rendererGeneration
  });
});

test("logout retires the session lease and rejects release and acquire", async ({ page }) => {
  await gotoReadyShell(page);
  const lease = await leaseCurrentComposer(page);

  await invoke(page, "logout");

  await expectRejected(page, "release_composer_draft_lease", {
    leaseId: lease.leaseId,
    rendererGeneration: lease.rendererGeneration
  });
  await expectRejected(page, "acquire_composer_draft_lease", {
    ...ACCOUNT,
    target: MAIN_TARGET,
    rendererGeneration: lease.rendererGeneration
  });
});

test("account retirement rejects the old lease while the new account can acquire", async ({ page }) => {
  await gotoReadyShell(page);
  const lease = await leaseCurrentComposer(page);

  await replaceReadyAccountRetainingStagedProjection(page);

  await expectRejected(page, "release_composer_draft_lease", {
    leaseId: lease.leaseId,
    rendererGeneration: lease.rendererGeneration
  });
  await expectRejected(page, "acquire_composer_draft_lease", {
    ...ACCOUNT,
    target: MAIN_TARGET,
    rendererGeneration: lease.rendererGeneration
  });

  // Account retirement invalidates all prior renderer leases. Begin a fresh
  // renderer generation for the replacement account instead of racing whatever
  // generation was current before the snapshot transition.
  const newLease = await leaseCurrentComposer(page, MAIN_TARGET, {
    accountHomeserver: "https://other-account.example.invalid",
    accountUserId: "@other-account:example.invalid",
    accountDeviceId: "OTHERDEVICE"
  });
  await releaseLease(page, newLease);
});

test("an unchanged lease releases once and rejects a second release", async ({ page }) => {
  await gotoReadyShell(page);
  const lease = await leaseCurrentComposer(page);

  await releaseLease(page, lease);
  await expectRejected(page, "release_composer_draft_lease", {
    leaseId: lease.leaseId,
    rendererGeneration: lease.rendererGeneration
  });
});

test("boot clears startup history but preserves command history across snapshots", async ({ page }) => {
  await gotoReadyShell(page);

  expect(await page.evaluate(() => window.__harness.invocationsOf("get_snapshot"))).toEqual([]);
  await invoke(page, "get_snapshot");
  expect(await page.evaluate(() => window.__harness.invocationsOf("get_snapshot").length)).toBe(1);

  await page.evaluate(() => {
    window.__harness.setSnapshot(window.__harness.currentSnapshot());
  });
  await invoke(page, "get_snapshot");
  expect(await page.evaluate(() => window.__harness.invocationsOf("get_snapshot").length)).toBe(2);

  const snapshotInvocationIndices = await page.evaluate(() =>
    window.__harness
      .invocations()
      .map((item, index) => (item.command === "get_snapshot" ? index : -1))
      .filter((index) => index >= 0)
  );
  expect(snapshotInvocationIndices).toHaveLength(2);
  expect(snapshotInvocationIndices[0]).toBeLessThan(snapshotInvocationIndices[1]!);
});

test("immediate Reply action records its typed target after boot settlement", async ({ page }) => {
  await gotoReadyShell(page);

  await page.getByRole("button", { name: "Reply to message" }).first().click();

  await expect
    .poll(() =>
      page.evaluate(() => window.__harness.invocationsOf("set_composer_reply_target").length)
    )
    .toBe(1);
  await expect
    .poll(async () =>
      page.evaluate(() => window.__harness.invocationsOf("set_composer_reply_target")[0]?.args)
    )
    .toEqual({
      roomId: "!harness-room:example.invalid",
      eventId: SEED_EVENT_ID
    });
});
