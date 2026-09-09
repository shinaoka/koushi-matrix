import {
  createRoom,
  joinRoom,
  registerUser,
  sendReadMarkers,
  sendRoomMessage
} from "./local-homeserver-qa.mjs";

export const AVATAR_FIXTURE_READERS = 1500;
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
    const authorization = `Bearer ${registration.access_token}`;
    const upload = await fetch(`${homeserver}/_matrix/media/v3/upload`, {
      method: "POST",
      headers: { authorization, "content-type": "image/png" },
      body: PNG
    });
    if (!upload.ok) throw new Error("avatar fixture upload failed");
    const { content_uri: uri } = await upload.json();
    if (typeof uri !== "string" || !uri.startsWith("mxc://") || mediaUris.has(uri)) {
      throw new Error("avatar fixture requires distinct media resources");
    }
    mediaUris.add(uri);
    const profile = await fetch(
      `${homeserver}/_matrix/client/v3/profile/${encodeURIComponent(registration.user_id)}/avatar_url`,
      {
        method: "PUT",
        headers: { authorization, "content-type": "application/json" },
        body: JSON.stringify({ avatar_url: uri })
      }
    );
    if (!profile.ok) throw new Error("avatar fixture profile update failed");
    await joinRoom(homeserver, registration.access_token, roomId);
    credentials.push(registration.access_token);
  });
  // Keep the observed message newer than all membership/profile timeline events.
  const { event_id: eventId } = await sendRoomMessage(
    homeserver, ownerAccessToken, roomId, "Avatar demand QA target", `avatar-target-${runId}`
  );
  await inBatches(credentials, (token) => sendReadMarkers(homeserver, token, roomId, eventId));
  return { roomId, eventId, readerCount: AVATAR_FIXTURE_READERS };
}
