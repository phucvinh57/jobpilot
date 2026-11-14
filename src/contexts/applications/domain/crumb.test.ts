import { describe, expect, it } from "vitest";

import { crumbFor } from "./crumb";

describe("the breadcrumb", () => {
  it("leads back to where the application was opened from", () => {
    const today = crumbFor("today");
    expect([today.label, today.target, today.id]).toEqual(["Today", "today", "breadcrumb-today"]);
    const pipeline = crumbFor("pipeline");
    expect([pipeline.label, pipeline.target, pipeline.id]).toEqual(["Pipeline", "pipeline", "breadcrumb-pipeline"]);
  });

  it("says where it leads for assistive technology", () => {
    expect(crumbFor("today").aria).toBe("Back to Today");
    expect(crumbFor("pipeline").aria).toBe("Back to the Pipeline");
  });
});
