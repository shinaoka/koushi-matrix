import { describe, expect, it } from "vitest";
import {
  assistantDraftOverlayDecision,
  overlayIsShown
} from "./assistantDraftAdoption";
import { parseComposerDraftRevision } from "./composerDraftRevision";
import { documentFromText } from "./composerDocument";

const revision = (value: string) => parseComposerDraftRevision(value);
const marker = (value: string) => ({ client_name: "claude-code", revision: revision(value) });

describe("assistantDraftOverlayDecision (#1644)", () => {
  it("shows the Rust document when the renderer has no overlay", () => {
    expect(assistantDraftOverlayDecision(null, marker("7"))).toBe("snapshot");
  });

  it("keeps the overlay when nothing was staged", () => {
    const overlay = { document: documentFromText("mine"), revision: revision("3") };
    expect(assistantDraftOverlayDecision(overlay, null)).toBe("overlay");
  });

  it("adopts a staged draft over an older empty overlay", () => {
    const overlay = { document: documentFromText(""), revision: revision("6") };
    const decision = assistantDraftOverlayDecision(overlay, marker("7"));
    expect(decision).toBe("adopt");
    expect(overlayIsShown(decision)).toBe(false);
  });

  it("never replaces text the user typed before the draft landed", () => {
    // The renderer minted 7 locally while Rust staged the draft at 7: Rust
    // refuses the user's write, so the renderer must re-persist it.
    const raced = { document: documentFromText("typing…"), revision: revision("7") };
    expect(assistantDraftOverlayDecision(raced, marker("7"))).toBe("user_wins");
    const unpersisted = { document: documentFromText("typing…"), revision: null };
    const decision = assistantDraftOverlayDecision(unpersisted, marker("7"));
    expect(decision).toBe("user_wins");
    expect(overlayIsShown(decision)).toBe(true);
  });

  it("keeps the user's edits of the staged draft", () => {
    const overlay = { document: documentFromText("edited draft"), revision: revision("8") };
    expect(assistantDraftOverlayDecision(overlay, marker("7"))).toBe("overlay");
  });
});
