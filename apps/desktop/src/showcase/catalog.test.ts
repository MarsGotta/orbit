import { describe, expect, it } from "vitest";
import { composeView } from "../state/app-view";
import { applyFrame } from "../state/feature-frame";
import { baseViews, bootstrapOf, framesOf } from "./base";
import { scenarios } from "./catalog";

// The showcase paints the app from these states; the types keep their shape,
// this keeps each one something the app's store accepts as it is.
describe("the showcase's scenarios", () => {
  it("each has its own id, usable as a file name", () => {
    const ids = scenarios.map((scenario) => scenario.id);
    expect(new Set(ids).size).toBe(ids.length);
    for (const id of ids) expect(id).toMatch(/^[a-z0-9-]+$/);
  });

  it("each composes into the app's view, and what it publishes later applies to it", () => {
    for (const scenario of scenarios) {
      const views = (scenario.views ?? ((base) => base))(baseViews());
      const bootstrap = bootstrapOf(views);
      let view = composeView(bootstrap.frames, bootstrap.revision);
      for (const frame of framesOf(scenario.later?.(views) ?? {}, 2))
        view = applyFrame(view, frame);
      expect(view.settings.hub, scenario.id).toBeDefined();
    }
  });
});
