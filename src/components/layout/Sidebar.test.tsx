import { act, cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
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

const indicator = () => document.getElementById("activity") as HTMLElement;
const list = () => screen.queryByRole("dialog", { name: "Background jobs" });

/** Press the indicator (what a click, Enter or Space on it does) and return its list. */
function openList(): HTMLElement {
  fireEvent.click(indicator());
  return screen.getByRole("dialog", { name: "Background jobs" });
}

beforeEach(() => {
  useActivity.setState(NO_ACTIVITY);
  useNav.setState({ screen: "profile", settingsFocus: undefined });
});

afterEach(() => {
  cleanup();
  vi.useRealTimers();
});

describe("Sidebar background jobs", () => {
  it("shows nothing while no job runs", () => {
    render(<Sidebar />);
    expect(document.getElementById("activity")).toBeNull();
    expect(screen.getByRole("heading", { name: "JobPilot" })).toBeTruthy();
  });

  it("counts the jobs and lists each with its status", () => {
    render(<Sidebar />);
    jobs(SEARCH, RESEARCH);
    const button = screen.getByRole("button", { name: "2 background jobs" });
    expect(button.id).toBe("activity");
    expect(button.textContent).toBe("2");
    expect(button.className).not.toContain("text-warn");
    expect(button.getAttribute("aria-haspopup")).toBe("dialog");
    expect(button.getAttribute("aria-expanded")).toBe("false");

    const card = openList();
    expect(card.id).toBe("activity-list");
    expect(button.getAttribute("aria-expanded")).toBe("true");
    expect(button.getAttribute("aria-controls")).toBe("activity-list");
    const rows = within(card).getAllByRole("button");
    expect(rows.map((r) => r.id)).toEqual(["activity-item-1", "activity-item-2"]);
    expect(within(rows[0]!).getByText("Job search")).toBeTruthy();
    expect(within(rows[0]!).getByText("Searching the sources")).toBeTruthy();
    expect(within(rows[1]!).getByText("Market research: Week of 9 November")).toBeTruthy();
    expect(within(rows[1]!).getByText("Reading the sources")).toBeTruthy();
  });

  it("turns to the warn colour when a job waits for the user", () => {
    render(<Sidebar />);
    jobs(SEARCH, SUGGEST);
    const button = screen.getByRole("button", { name: "2 background jobs, 1 waiting for you" });
    expect(button.className).toContain("text-warn");
  });

  it("disappears when the last job ends", () => {
    render(<Sidebar />);
    jobs(SEARCH);
    expect(screen.getByRole("button", { name: "1 background job" })).toBeTruthy();
    jobs();
    expect(document.getElementById("activity")).toBeNull();
  });

  it("opens the screen of the job a row stands for, and closes the list", () => {
    render(<Sidebar />);
    jobs(SEARCH, RESEARCH, SUGGEST);
    fireEvent.click(within(openList()).getByRole("button", { name: /Market research/ }));
    expect(useNav.getState().screen).toBe("trends");
    expect(list()).toBeNull();
    expect(document.activeElement).toBe(indicator());

    fireEvent.click(within(openList()).getByRole("button", { name: /Job search/ }));
    expect(useNav.getState().screen).toBe("jobs");

    fireEvent.click(within(openList()).getByRole("button", { name: /Finding job sources/ }));
    expect(useNav.getState().screen).toBe("settings");
    expect(useNav.getState().settingsFocus).toBe("Job sources");
  });

  it("toggles the list when the indicator is pressed, and goes nowhere", () => {
    render(<Sidebar />);
    jobs(RESEARCH, SUGGEST);
    fireEvent.click(indicator());
    expect(list()).not.toBeNull();
    expect(useNav.getState().screen).toBe("profile");
    fireEvent.click(indicator());
    expect(list()).toBeNull();
    expect(useNav.getState().screen).toBe("profile");
  });

  it("is reachable from the keyboard: a press moves the focus into the list, Escape gives it back", async () => {
    const user = userEvent.setup();
    render(<Sidebar />);
    jobs(SEARCH, RESEARCH);
    indicator().focus();
    await user.keyboard("{Enter}");
    const card = screen.getByRole("dialog", { name: "Background jobs" });
    expect(document.activeElement).toBe(card);
    // The rows are in the tab order.
    await user.tab();
    expect(document.activeElement?.id).toBe("activity-item-1");
    await user.tab();
    expect(document.activeElement?.id).toBe("activity-item-2");

    await user.keyboard("{Escape}");
    expect(list()).toBeNull();
    expect(document.activeElement).toBe(indicator());

    // Space opens it too, and Enter on a row opens its screen.
    await user.keyboard(" ");
    await user.tab();
    await user.tab();
    await user.keyboard("{Enter}");
    expect(useNav.getState().screen).toBe("trends");
    expect(list()).toBeNull();
  });

  it("opens under the pointer after a moment and closes once it left the indicator and the list", () => {
    vi.useFakeTimers();
    render(<Sidebar />);
    jobs(SEARCH);
    fireEvent.pointerEnter(indicator());
    expect(list()).toBeNull();
    act(() => void vi.advanceTimersByTime(120));
    const card = list() as HTMLElement;
    expect(card).not.toBeNull();
    // The focus stays where it was.
    expect(document.activeElement).not.toBe(card);

    // From the indicator onto the list: it stays.
    fireEvent.pointerLeave(indicator());
    act(() => void vi.advanceTimersByTime(100));
    fireEvent.pointerEnter(card);
    act(() => void vi.advanceTimersByTime(1000));
    expect(list()).not.toBeNull();

    fireEvent.pointerLeave(card);
    act(() => void vi.advanceTimersByTime(150));
    expect(list()).toBeNull();
  });

  it("stays open without the pointer once it was pressed", () => {
    vi.useFakeTimers();
    render(<Sidebar />);
    jobs(SEARCH);
    fireEvent.pointerEnter(indicator());
    act(() => void vi.advanceTimersByTime(120));
    fireEvent.click(indicator());
    expect(document.activeElement).toBe(list());
    fireEvent.pointerLeave(indicator());
    act(() => void vi.advanceTimersByTime(1000));
    expect(list()).not.toBeNull();
  });
});
