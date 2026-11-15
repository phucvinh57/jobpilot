import { cleanup, render, screen, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { Insight } from "@/components/InsightBrief";

import * as api from "@/api/applications";
import { jd } from "@/test/fixtures/applications";
import { JdTab } from "./JdTab";

vi.mock("@/api/applications");
vi.mock("@/api/bridge", () => ({
  invoke: vi.fn().mockResolvedValue(undefined),
  onDataChanged: () => () => undefined,
  onGraphProgress: () => () => undefined,
}));

const mocked = vi.mocked(api);

const INSIGHT: Insight = {
  domain: "Payments",
  businessSummary: "Acme moves money.",
  facts: "Founded 2015 · 120 people",
  requiredSkills: ["Rust"],
  preferredSkills: ["Kafka"],
  interviewSteps: ["Recruiter call", "Take-home"],
  interviewNotes: null,
  sources: ["https://acme.test/about"],
};

beforeEach(() => {
  mocked.readJd.mockResolvedValue(jd());
});

afterEach(() => {
  cleanup();
});

describe("the JD tab", () => {
  it("keeps the line structure of the posting and shows it as plain text", async () => {
    mocked.readJd.mockResolvedValue(jd({ text: "Title\n* one\n* two\n\n<b>Body</b>" }));
    render(<JdTab id={7} />);
    const text = await screen.findByText(/Title/);
    expect(text.textContent).toBe("Title\n* one\n* two\n\n<b>Body</b>");
    expect(text.className).toContain("whitespace-pre-wrap");
    expect(text.querySelector("b")).toBeNull();
  });

  it("shows the key requirements when there is no company brief", async () => {
    render(<JdTab id={7} />);
    const card = await screen.findByRole("heading", { name: "Key requirements" });
    expect(card).toBeTruthy();
    expect(screen.getByText("Must have")).toBeTruthy();
    expect(screen.getByText("Terraform")).toBeTruthy();
    expect(screen.queryByRole("heading", { name: "Company brief" })).toBeNull();
  });

  it("shows the company brief with the shared blocks, instead of the requirements", async () => {
    mocked.readJd.mockResolvedValue(jd({ insight: INSIGHT }));
    render(<JdTab id={7} />);
    await screen.findByRole("heading", { name: "Company brief" });
    expect(screen.getByText("Payments")).toBeTruthy();
    expect(screen.getByText("Acme moves money.")).toBeTruthy();
    expect(screen.getByText("Recruiter call")).toBeTruthy();
    const brief = document.getElementById("jd-brief") as HTMLElement;
    expect(within(brief).getByText("Required")).toBeTruthy();
    expect(within(brief).getByText("Kafka")).toBeTruthy();
    expect(screen.queryByRole("heading", { name: "Key requirements" })).toBeNull();
  });

  it("says so when there is no posting text, and shows a source that is not a link as text", async () => {
    mocked.readJd.mockResolvedValue(jd({ text: null, source: "/home/me/jd.pdf", sourceIsLink: false }));
    render(<JdTab id={7} />);
    expect(await screen.findByText("No JD text yet.")).toBeTruthy();
    expect(screen.getByText("/home/me/jd.pdf").tagName).toBe("P");
    expect(screen.queryByRole("button", { name: "/home/me/jd.pdf" })).toBeNull();
  });

  it("offers a web source as a link", async () => {
    render(<JdTab id={7} />);
    expect(await screen.findByRole("button", { name: "https://jobs.acme.test/1" })).toBeTruthy();
  });
});
