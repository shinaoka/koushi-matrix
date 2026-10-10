import {
  createRoom,
  joinRoom,
  registerUser,
  sendReadMarkers,
  sendRoomMessage
} from "./local-homeserver-qa.mjs";

export const AVATAR_FIXTURE_READERS = 1500;

/**
 * Count the distinct readers the server reports for `eventId` inside one
 * `m.receipt` event body.
 */
function receiptReadersForEvent(receiptEvent, eventId) {
  const readers = new Set();
  for (const receiptType of Object.values(receiptEvent?.content?.[eventId] ?? {})) {
    for (const sender of Object.keys(receiptType ?? {})) readers.add(sender);
  }
  return readers;
}

/**
 * Read the seeded reader population back the two ways the product can.
 *
 * `stored` counts it in a plain v3 `/sync` (server storage truth). `packed`
 * counts it through the Simplified Sliding Sync receipts extension, which
 * packs one `m.receipt` event per room and is the read Koushi Core projects
 * reader windows from. Returns counts only: no identities, room or event IDs
 * leave this function.
 */
async function readReceiptPopulation({ homeserver, accessToken, roomId, eventId }) {
  const authorization = `Bearer ${accessToken}`;
  const ephemeralReaders = (payload) => {
    const readers = new Set();
    for (const event of payload?.rooms?.join?.[roomId]?.ephemeral?.events ?? []) {
      if (event?.type !== "m.receipt") continue;
      for (const sender of receiptReadersForEvent(event, eventId)) readers.add(sender);
    }
    return readers.size;
  };

  const sync = await fetch(`${homeserver}/_matrix/client/v3/sync?timeout=0`, {
    headers: { authorization }
  });
  const stored = sync.ok ? ephemeralReaders(await sync.json()) : null;

  const sliding = await fetch(
    `${homeserver}/_matrix/client/unstable/org.matrix.simplified_msc3575/sync?timeout=0`,
    {
      method: "POST",
      headers: { authorization, "content-type": "application/json" },
      body: JSON.stringify({
        conn_id: "avatar-fixture-readback",
        lists: {
          readback: {
            ranges: [[0, 9]],
            required_state: [["m.room.name", ""]],
            timeline_limit: 1
          }
        },
        extensions: { receipts: { enabled: true } }
      })
    }
  );
  let packed = null;
  if (sliding.ok) {
    const payload = await sliding.json();
    const extension = payload?.extensions?.receipts ?? payload?.extensions?.["m.receipt"];
    const packedEvent = extension?.rooms?.[roomId];
    packed = packedEvent ? receiptReadersForEvent(packedEvent, eventId).size : 0;
  }
  return { stored, packed };
}

/**
 * Decide whether the fixture's 1500-reader premise holds on this server.
 *
 * A lane may only run `avatar_demand` when the server reports the seeded
 * population back. tuwunel's pinned Simplified Sliding Sync receipt packing
 * collapses every reader of one event into a single user, so the lane would
 * wait out its deadline for a population the server cannot report; that is
 * declared per server in `homeserverFixtureCapabilities`. A declared
 * limitation is re-checked here, so a server that starts reporting the full
 * population fails loudly instead of staying silently skipped (#1168).
 */
export function classifyAvatarReceiptReadback({ serverKind, capability, readback, expected }) {
  const { stored, packed } = readback;
  if (capability.supported) {
    const observed = packed ?? stored;
    if (observed !== expected) {
      throw new Error(
        `avatar fixture receipt readback failed server=${serverKind} ` +
          `packed=${packed ?? "unreadable"} stored=${stored ?? "unreadable"} expected=${expected}`
      );
    }
    return { limited: false };
  }
  if (packed === null) {
    throw new Error(
      `avatar fixture receipt readback unreadable server=${serverKind} ` +
        `reason=${capability.limitation} stored=${stored ?? "unreadable"}`
    );
  }
  if (packed >= expected) {
    throw new Error(
      `avatar receipt limitation no longer applies server=${serverKind} ` +
        `packed=${packed} expected=${expected}; re-enable avatar_demand for this server`
    );
  }
  return {
    limited: true,
    token:
      `avatar_demand=server_limited reason=${capability.limitation} server=${serverKind} ` +
      `receipt_readback=${packed} expected=${expected}`
  };
}
const PNG = Buffer.from(
  "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+ip1sAAAAASUVORK5CYII=",
  "base64"
);

// Bound fixture traffic independently of the product's avatar scheduler.
async function inBatches(values, operation) {
  for (let start = 0; start < values.length; start += 8) {
    const results = await Promise.allSettled(values.slice(start, start + 8).map(operation));
    if (results.some((result) => result.status === "rejected")) {
      // Do not propagate HTTP URLs, credentials or registration identities.
      throw new Error("avatar fixture batch failed");
    }
  }
}

export async function setFixtureAvatar(homeserver, registration) {
  const authorization = `Bearer ${registration.access_token}`;
  const upload = await fetch(`${homeserver}/_matrix/media/v3/upload`, {
    method: "POST",
    headers: { authorization, "content-type": "image/png" },
    body: PNG
  });
  if (!upload.ok) throw new Error("avatar fixture upload failed");
  const { content_uri: uri } = await upload.json();
  if (typeof uri !== "string" || !uri.startsWith("mxc://")) {
    throw new Error("avatar fixture upload identity missing");
  }
  const profile = await fetch(
    `${homeserver}/_matrix/client/v3/profile/${encodeURIComponent(registration.user_id)}/avatar_url`,
    {
      method: "PUT",
      headers: { authorization, "content-type": "application/json" },
      body: JSON.stringify({ avatar_url: uri })
    }
  );
  if (!profile.ok) throw new Error("avatar fixture profile update failed");
  return uri;
}

/** Seed server data only; returned metadata contains no credentials or MXCs. */
export async function seedAvatarDemandFixture({ homeserver, ownerAccessToken, runId }) {
  const { room_id: roomId } = await createRoom(homeserver, ownerAccessToken, {
    preset: "public_chat",
    name: "Avatar demand QA"
  });
  const credentials = [];
  const mediaUris = new Set();
  await inBatches(Array.from({ length: AVATAR_FIXTURE_READERS }, (_, i) => i), async (index) => {
    const registration = await registerUser(
      homeserver,
      `qa_avatar_${runId}_${String(index).padStart(4, "0")}`,
      `qa-avatar-password-${runId}-${index}`
    );
    const uri = await setFixtureAvatar(homeserver, registration);
    if (mediaUris.has(uri)) {
      throw new Error("avatar fixture requires distinct media resources");
    }
    mediaUris.add(uri);
    await joinRoom(homeserver, registration.access_token, roomId);
    credentials.push(registration.access_token);
  });
  // Keep the observed message newer than all membership/profile timeline events.
  const { event_id: eventId } = await sendRoomMessage(
    homeserver, ownerAccessToken, roomId, "Avatar demand QA target", `avatar-target-${runId}`
  );
  await inBatches(credentials, (token) => sendReadMarkers(homeserver, token, roomId, eventId));
  // Seed HTTP success is not receipt-readback proof: prove the population the
  // stage is about to wait for through the reads the product itself uses.
  const receiptReadback = await readReceiptPopulation({
    homeserver,
    accessToken: ownerAccessToken,
    roomId,
    eventId
  });
  return { roomId, eventId, readerCount: AVATAR_FIXTURE_READERS, receiptReadback };
}
