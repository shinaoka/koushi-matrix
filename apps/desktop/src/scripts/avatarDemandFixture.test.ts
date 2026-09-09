import { crc32, inflateSync } from "node:zlib";
import { afterEach, expect, test, vi } from "vitest";
import {
  AVATAR_FIXTURE_READERS,
  seedAvatarDemandFixture
} from "../../../../scripts/lib/avatar-demand-fixture.mjs";

afterEach(() => vi.unstubAllGlobals());

test("seeds 1500 distinct avatars and receipts with bounded traffic and no returned credentials", async () => {
  let active = 0;
  let peak = 0;
  let registered = 0;
  let uploaded = 0;
  const uploadedImages: Buffer[] = [];
  let joined = 0;
  let receipts = 0;
  const profiles = new Set<string>();
  vi.stubGlobal("fetch", vi.fn(async (url: string, options: RequestInit) => {
    active += 1;
    peak = Math.max(peak, active);
    await Promise.resolve();
    try {
      const headers = options.headers as Record<string, string>;
      const token = headers?.authorization;
      let body: object;
      if (url.endsWith("/createRoom")) {
        body = { room_id: "!fixture:example.invalid" };
      } else if (url.endsWith("/register")) {
        registered += 1;
        body = { user_id: `@reader${registered}:example.invalid`, access_token: `private-${registered}` };
      } else if (url.endsWith("/upload")) {
        uploaded += 1;
        expect(options.body).toBeInstanceOf(Uint8Array);
        if (uploaded === 1) uploadedImages.push(Buffer.from(options.body as Uint8Array));
        body = { content_uri: `mxc://example.invalid/avatar-${uploaded}` };
      } else if (url.endsWith("/avatar_url")) {
        profiles.add(token);
        body = {};
      } else if (url.includes("/join/")) {
        expect(profiles.has(token)).toBe(true);
        joined += 1;
        body = {};
      } else if (url.includes("/send/")) {
        expect(joined).toBe(AVATAR_FIXTURE_READERS);
        body = { event_id: "$target" };
      } else if (url.endsWith("/read_markers")) {
        expect(JSON.parse(options.body as string)).toEqual({ fully_read: "$target", "m.read": "$target" });
        receipts += 1;
        body = {};
      } else {
        throw new Error("unexpected fixture endpoint");
      }
      return new Response(JSON.stringify(body), { status: 200 });
    } finally {
      active -= 1;
    }
  }));
  const result = await seedAvatarDemandFixture({
    homeserver: "http://example.invalid", ownerAccessToken: "owner-secret", runId: "unit"
  });
  expect(result).toEqual({ roomId: "!fixture:example.invalid", eventId: "$target", readerCount: 1500 });
  expect([registered, uploaded, joined, receipts]).toEqual([1500, 1500, 1500, 1500]);
  expect(peak).toBeLessThanOrEqual(8);
  const png = uploadedImages[0];
  expect(png.subarray(0, 8)).toEqual(Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]));
  const pixels: Buffer[] = [];
  let offset = 8;
  while (offset < png.length) {
    const length = png.readUInt32BE(offset);
    const end = offset + 8 + length;
    const kind = png.toString("ascii", offset + 4, offset + 8);
    expect(crc32(png.subarray(offset + 4, end)), `${kind} checksum`).toBe(png.readUInt32BE(end));
    if (kind === "IDAT") pixels.push(png.subarray(offset + 8, end));
    offset = end + 4;
  }
  expect(offset).toBe(png.length);
  expect([...inflateSync(Buffer.concat(pixels))]).toEqual([1, 255, 255]);
  expect(JSON.stringify(result)).not.toMatch(/private-|secret|mxc:/);
});

test("rejects shared upload identities rather than weakening the distinct-resource fixture", async () => {
  const fetchMock = vi.fn(async (url: string) => {
    const body = url.endsWith("/createRoom") ? { room_id: "!fixture:example.invalid" }
      : url.endsWith("/register") ? { user_id: "@reader:example.invalid", access_token: "private" }
      : url.endsWith("/upload") ? { content_uri: "mxc://example.invalid/shared" }
      : {};
    return new Response(JSON.stringify(body), { status: 200 });
  });
  vi.stubGlobal("fetch", fetchMock);
  await expect(seedAvatarDemandFixture({
    homeserver: "http://example.invalid", ownerAccessToken: "owner-secret", runId: "unit"
  })).rejects.toThrow(/^avatar fixture batch failed$/);
  expect(fetchMock.mock.calls.some(([url]) => url.includes("/send/"))).toBe(false);
});

test("settles a failed batch and does not expose registration identity or continue seeding", async () => {
  const fetchMock = vi.fn(async (url: string) => new Response(
    JSON.stringify(url.endsWith("/createRoom") ? { room_id: "!fixture:example.invalid" } : {}),
    { status: url.endsWith("/createRoom") ? 200 : 403 }
  ));
  vi.stubGlobal("fetch", fetchMock);
  await expect(seedAvatarDemandFixture({
    homeserver: "http://example.invalid", ownerAccessToken: "owner-secret", runId: "private-run"
  })).rejects.toThrow(/^avatar fixture batch failed$/);
  expect(fetchMock).toHaveBeenCalledTimes(9);
});
