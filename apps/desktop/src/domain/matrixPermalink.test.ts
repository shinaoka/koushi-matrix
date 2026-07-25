import { describe, expect, it } from "vitest";

import {
  normalizeMatrixTargetInput,
  parseMatrixPermalink,
  serverNameFromMatrixId
} from "./matrixPermalink";

describe("parseMatrixPermalink", () => {
  it("parses a percent-encoded room alias", () => {
    // The acceptance-criteria link from the issue.
    expect(
      parseMatrixPermalink("https://matrix.to/#/%23cqmp%3Amatrix.gull-group.org")
    ).toEqual({
      kind: "room",
      roomIdOrAlias: "#cqmp:matrix.gull-group.org",
      viaServers: []
    });
  });

  it("parses an unencoded alias and a room id alike", () => {
    expect(parseMatrixPermalink("https://matrix.to/#/#room:example.org")).toEqual({
      kind: "room",
      roomIdOrAlias: "#room:example.org",
      viaServers: []
    });
    expect(parseMatrixPermalink("https://matrix.to/#/!abc:example.org")).toEqual({
      kind: "room",
      roomIdOrAlias: "!abc:example.org",
      viaServers: []
    });
  });

  it("keeps the event id and every via server", () => {
    expect(
      parseMatrixPermalink("https://matrix.to/#/!room:server/$event:server?via=a.org&via=b.org")
    ).toEqual({
      kind: "room",
      roomIdOrAlias: "!room:server",
      eventId: "$event:server",
      viaServers: ["a.org", "b.org"]
    });
  });

  it("deduplicates and drops blank via servers", () => {
    const target = parseMatrixPermalink(
      "https://matrix.to/#/!room:server?via=a.org&via=a.org&via=%20"
    );
    expect(target).toMatchObject({ viaServers: ["a.org"] });
  });

  it("parses a user permalink", () => {
    expect(parseMatrixPermalink("https://matrix.to/#/@alice:example.org")).toEqual({
      kind: "user",
      userId: "@alice:example.org"
    });
  });

  it("accepts the www host and http scheme", () => {
    expect(parseMatrixPermalink("http://www.matrix.to/#/#room:example.org")).toMatchObject({
      kind: "room"
    });
  });

  it("rejects links that are not actionable Matrix targets", () => {
    for (const rawUrl of [
      "",
      "not a url",
      "https://example.org/#/#room:example.org",
      "https://matrix.to/",
      "https://matrix.to/#/",
      // No sigil, no server part, or empty parts.
      "https://matrix.to/#/room:example.org",
      "https://matrix.to/#/#room",
      "https://matrix.to/#/#:example.org",
      "https://matrix.to/#/#room:",
      // A malformed escape must not be treated as literal text.
      "https://matrix.to/#/%ZZ",
      // A user permalink cannot carry an event.
      "https://matrix.to/#/@alice:example.org/$event:server",
      // A trailing segment that is not an event id.
      "https://matrix.to/#/!room:server/notanevent",
      "matrix:r/room:example.org"
    ]) {
      expect(parseMatrixPermalink(rawUrl), rawUrl).toBeNull();
    }
  });
});

describe("normalizeMatrixTargetInput", () => {
  it("turns a pasted permalink into the entity a user would have typed", () => {
    expect(
      normalizeMatrixTargetInput("  https://matrix.to/#/%23cqmp%3Amatrix.gull-group.org  ")
    ).toBe("#cqmp:matrix.gull-group.org");
    expect(normalizeMatrixTargetInput("https://matrix.to/#/@alice:example.org")).toBe(
      "@alice:example.org"
    );
  });

  it("leaves ordinary input untouched", () => {
    expect(normalizeMatrixTargetInput("  #room:example.org ")).toBe("#room:example.org");
    expect(normalizeMatrixTargetInput("public rooms")).toBe("public rooms");
  });
});

describe("serverNameFromMatrixId", () => {
  it("reads the server part used as a via hint", () => {
    expect(serverNameFromMatrixId("#room:example.org")).toBe("example.org");
    expect(serverNameFromMatrixId("!abc:matrix.org")).toBe("matrix.org");
  });

  it("returns null when there is no usable server part", () => {
    expect(serverNameFromMatrixId("#room")).toBeNull();
    expect(serverNameFromMatrixId("#room:")).toBeNull();
    expect(serverNameFromMatrixId(":example.org")).toBeNull();
  });
});
