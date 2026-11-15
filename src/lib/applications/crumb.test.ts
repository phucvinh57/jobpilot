import { describe, expect, it } from "vitest";

import { crumbFor } from "./crumb";

describe("the breadcrumb", () => {
  it("leads back to the stage of the Jobs page the user left", () => {
    expect(crumbFor("cv")).toEqual({ id: "breadcrumb-jobs", label: "Jobs › CVs", aria: "Back to Jobs, CVs" });
    expect(crumbFor("inProgress").label).toBe("Jobs › In progress");
    expect(crumbFor("leads").aria).toBe("Back to Jobs, Leads");
  });
});
