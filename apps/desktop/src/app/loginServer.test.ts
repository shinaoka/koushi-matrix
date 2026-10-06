import { describe, expect, it } from "vitest";
import {
  DEFAULT_LOGIN_SERVER,
  effectiveLoginServer,
  loginServerDisplayName,
  matrixIdServerName
} from "./loginServer";

describe("login server derivation (#1101)", () => {
  it("takes the server name from a full Matrix ID", () => {
    expect(matrixIdServerName("@alice:example.org")).toBe("example.org");
    expect(matrixIdServerName("  @alice:example.org:8448 ")).toBe("example.org:8448");
  });

  it("does not treat a bare or incomplete username as a Matrix ID", () => {
    expect(matrixIdServerName("alice")).toBeNull();
    expect(matrixIdServerName("alice:example.org")).toBeNull();
    expect(matrixIdServerName("@alice")).toBeNull();
    expect(matrixIdServerName("@alice:")).toBeNull();
    expect(matrixIdServerName("@:example.org")).toBeNull();
  });

  it("prefers an explicit server, then the Matrix ID, then the default", () => {
    expect(effectiveLoginServer("@alice:example.org", "https://hs.example.net")).toBe(
      "https://hs.example.net"
    );
    expect(effectiveLoginServer("@alice:example.org", null)).toBe("example.org");
    expect(effectiveLoginServer("alice", null)).toBe(DEFAULT_LOGIN_SERVER);
    expect(effectiveLoginServer("alice", "  ")).toBe("");
  });

  it("shows a server without its scheme or trailing slash", () => {
    expect(loginServerDisplayName("https://matrix.example.org/")).toBe("matrix.example.org");
    expect(loginServerDisplayName("example.org")).toBe("example.org");
  });
});
