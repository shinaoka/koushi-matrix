import { describe, expect, it } from "vitest";
import { createComposerSubmissionController } from "./composerSubmission";

describe("composer submission controller", () => {
  it("admits one submission synchronously", () => {
    const controller = createComposerSubmissionController(() => "submission-1");
    expect(controller.begin()).toBe("submission-1");
    expect(controller.begin()).toBeNull();
  });

  it("preserves the guard after acceptance until matching terminal release", () => {
    const controller = createComposerSubmissionController(() => "submission-1");
    const id = controller.begin()!;
    controller.accept(id);
    expect(controller.begin()).toBeNull();
    controller.releaseTerminal("other");
    expect(controller.begin()).toBeNull();
    controller.releaseTerminal(id);
    expect(controller.begin()).toBe("submission-1");
  });

  it("releases immediately on matching rejection", () => {
    const controller = createComposerSubmissionController(() => "submission-1");
    controller.reject(controller.begin()!);
    expect(controller.begin()).toBe("submission-1");
  });
});
