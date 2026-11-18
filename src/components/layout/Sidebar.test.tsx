import { act, cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { NO_ACTIVITY } from "@/lib/activity";
import { useActivity } from "@/stores/activity";
import { useNav } from "@/stores/nav";
import type { Activity } from "@/types/shell";
import { Sidebar } from "./Sidebar";

vi.mock("@/api/shell");
vi.mock("@/api/events");

const SEARCH: Activity = {
  id: "discover:r1",
  title: "Job search",
  status: "Searching the sources",
  state: "running",
  screen: "jobs",
};
const RESEARCH: Activity = {
  id: "trends:2026-W46",
  title: "Market research: Week of 9 November",
  status: "Reading the sources",
  state: "running",
  screen: "trends",
};
const SUGGEST: Activity = {
  id: "source_suggest",
  title: "Finding job sources",
  status: "Waiting for you: captcha",
  state: "waiting",
  screen: "settings",
};

function jobs(...items: Activity[]) {
  act(() => useActivity.setState({ items, details: {} }));
}

/** Focus the indicator (as the keyboard does) and wait for its list. */
async function openList(): Promise<HTMLElement> {
  const indicator = document.getElementById("activity");
  expect(indicator).not.toBeNull();
  act(() => indicator?.focus());
  return screen.findByRole("group", { name: "Background jobs" });
}

beforeEach(() => {
  useActivity.setState(NO_ACTIVITY);
  useNav.setState({ screen: "profile", settingsFocus: undefined });
});

afterEach(cleanup);

describe("Sidebar background jobs", () => {
  it("shows nothing while no job runs", () => {
    render(<Sidebar />);
    expect(document.getElementById("activity")).toBeNull();
    expect(screen.getByRole("heading", { name: "JobPilot" })).toBeTruthy();
  });

  it("counts the jobs and lists each with its status", async () => {
    render(<Sidebar />);
    jobs(SEARCH, RESEARCH);
    const indicator = screen.getByRole("button", { name: "2 background jobs" });
    expect(indicator.id).toBe("activity");
    expect(indicator.textContent).toBe("2");
    expect(indicator.className).not.toContain("text-warn");

    const list = await openList();
    expect(list.id).toBe("activity-list");
    const rows = within(list).getAllByRole("button");
    expect(rows.map((r) => r.id)).toEqual(["activity-item-1", "activity-item-2"]);
    expect(within(rows[0]!).getByText("Job search")).toBeTruthy();
    expect(within(rows[0]!).getByText("Searching the sources")).toBeTruthy();
    expect(within(rows[1]!).getByText("Market research: Week of 9 November")).toBeTruthy();
    expect(within(rows[1]!).getByText("Reading the sources")).toBeTruthy();
  });

  it("turns to the warn colour when a job waits for the user", () => {
    render(<Sidebar />);
    jobs(SEARCH, SUGGEST);
    const indicator = screen.getByRole("button", { name: "2 background jobs, 1 waiting for you" });
    expect(indicator.className).toContain("text-warn");
  });

  it("disappears when the last job ends", () => {
    render(<Sidebar />);
    jobs(SEARCH);
    expect(screen.getByRole("button", { name: "1 background job" })).toBeTruthy();
    jobs();
    expect(document.getElementById("activity")).toBeNull();
  });

  it("opens the screen of the job a row stands for", async () => {
    render(<Sidebar />);
    jobs(SEARCH, RESEARCH, SUGGEST);
    const list = await openList();
    fireEvent.click(within(list).getByRole("button", { name: /Market research/ }));
    expect(useNav.getState().screen).toBe("trends");

    fireEvent.click(within(list).getByRole("button", { name: /Job search/ }));
    expect(useNav.getState().screen).toBe("jobs");

    fireEvent.click(within(list).getByRole("button", { name: /Finding job sources/ }));
    expect(useNav.getState().screen).toBe("settings");
    expect(useNav.getState().settingsFocus).toBe("Job sources");
  });

  it("opens the job that waits for the user when the indicator itself is pressed", () => {
    render(<Sidebar />);
    jobs(SEARCH, RESEARCH);
    fireEvent.click(screen.getByRole("button", { name: "2 background jobs" }));
    expect(useNav.getState().screen).toBe("jobs");

    jobs(RESEARCH, SUGGEST);
    fireEvent.click(document.getElementById("activity")!);
    expect(useNav.getState().screen).toBe("settings");
  });
});
