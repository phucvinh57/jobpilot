import { cleanup, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { CommandError, useNav, type Problem } from "@/shared";

import * as api from "../api/applicationsApi";
import { board } from "../domain/fixtures";
import type { ActivityItem, StageKey } from "../domain/types";
import { useBoard } from "../hooks/useBoard";
import { ApplicationsStage as Stage } from "./ApplicationsStage";

/** The stage as the Jobs page shows it: with the page's one read of the board. */
function ApplicationsStage({ stage }: { stage: StageKey }) {
  return <Stage stage={stage} board={useBoard()} />;
}

vi.mock("../api/applicationsApi");
vi.mock("@/shared/bridge", () => ({
  invoke: vi.fn().mockResolvedValue(undefined),
  onDataChanged: () => () => undefined,
  onGraphProgress: () => () => undefined,
}));

const mocked = vi.mocked(api);

beforeEach(() => {
  useNav.setState({ screen: "jobs", stage: "cv", drawer: undefined, applicationId: undefined });
  mocked.readBoard.mockResolvedValue(board());
});

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});

const rowNames = (table: HTMLElement) =>
  within(table)
    .getAllByRole("row")
    .map((r) => r.getAttribute("aria-label"))
    .filter((name) => name !== null);

describe("an application stage", () => {
  it("lists the CVs with their role, company, next step and age", async () => {
    render(<ApplicationsStage stage="cv" />);
    const table = await screen.findByRole("table", { name: "CVs" });
    expect(rowNames(table)).toEqual([
      "Acme, Backend Engineer: Approve CV, Draft",
      "Globex, SRE: Review & submit, CV ready",
    ]);
    const row = within(table).getByRole("row", { name: "Globex, SRE: Review & submit, CV ready" });
    expect(within(row).getByText("SRE")).toBeTruthy();
    expect(within(row).getByText("Globex")).toBeTruthy();
    expect(within(row).getByText("Review & submit")).toBeTruthy();
    expect(within(row).getByText("2 d")).toBeTruthy();
    expect(document.querySelectorAll("button.bg-accent")).toHaveLength(0);
  });

  it("shows the round of an interview and how a finished application ended", async () => {
    const { unmount } = render(<ApplicationsStage stage="inProgress" />);
    const progress = await screen.findByRole("table", { name: "In progress" });
    expect(within(progress).getByText("Technical interview")).toBeTruthy();
    unmount();

    render(<ApplicationsStage stage="done" />);
    const done = await screen.findByRole("table", { name: "Done" });
    expect(within(within(done).getByRole("row", { name: /^Hooli/ })).getByText("Offer")).toBeTruthy();
    expect(within(within(done).getByRole("row", { name: /^Vandelay/ })).getByText("Rejected").className).toContain(
      "danger",
    );
  });

  it("shows what the shell says happens next, for every card", async () => {
    const data = board();
    const done = data.stages.find((s) => s.key === "done");
    const card = done?.cards[1];
    if (card) {
      card.next = { label: "Closed", tone: "neutral" };
    }
    mocked.readBoard.mockResolvedValue(data);
    render(<ApplicationsStage stage="done" />);
    const table = await screen.findByRole("table", { name: "Done" });
    const row = within(table).getByRole("row", { name: /^Vandelay/ });
    expect(within(row).getByText("Closed")).toBeTruthy();
    expect(within(row).queryByText("Rejected", { selector: "span" })).toBeNull();
  });

  it("opens a row in the drawer by click or Enter, and marks the open row", async () => {
    const user = userEvent.setup();
    render(<ApplicationsStage stage="cv" />);
    await user.click(await screen.findByRole("row", { name: "Acme, Backend Engineer: Approve CV, Draft" }));
    expect(useNav.getState().drawer).toEqual({ kind: "app", id: 1 });
    expect(
      screen.getByRole("row", { name: "Acme, Backend Engineer: Approve CV, Draft" }).getAttribute("aria-current"),
    ).toBe("true");

    screen.getByRole("row", { name: "Globex, SRE: Review & submit, CV ready" }).focus();
    await user.keyboard("{Enter}");
    expect(useNav.getState().drawer).toEqual({ kind: "app", id: 2 });
    expect(useNav.getState().screen).toBe("jobs");
  });

  it("explains an empty stage, and leads from an empty CVs stage to the Leads", async () => {
    const user = userEvent.setup();
    mocked.readBoard.mockResolvedValue({ count: 0, stages: [], activity: [] });
    render(<ApplicationsStage stage="cv" />);
    expect(await screen.findByText("Nothing here yet")).toBeTruthy();
    await user.click(screen.getByRole("button", { name: "Pick a lead" }));
    expect(useNav.getState().stage).toBe("leads");
    cleanup();

    render(<ApplicationsStage stage="done" />);
    expect(await screen.findByText("Offers and rejections end up here.")).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Pick a lead" })).toBeNull();
  });

  it("lists the recent activity, and opens a row's application at its stage", async () => {
    const user = userEvent.setup();
    render(<ApplicationsStage stage="cv" />);
    const list = await screen.findByRole("list", { name: "Recent activity" });
    const rows = within(list).getAllByRole("button");
    expect(rows.map((r) => r.getAttribute("aria-label"))).toEqual([
      "Initech: moved to Applied, 09:05",
      "Acme: application created, Yesterday",
    ]);
    expect(screen.queryByRole("button", { name: /Show all/ })).toBeNull();
    await user.click(rows[0] as HTMLElement);
    const nav = useNav.getState();
    expect([nav.stage, nav.drawer]).toEqual(["applied", { kind: "app", id: 3 }]);
  });

  it("folds a long activity feed", async () => {
    const user = userEvent.setup();
    const activity: ActivityItem[] = Array.from({ length: 12 }, (_, i) => ({
      id: i,
      appId: 1,
      text: `Acme: event ${i}`,
      when: "09:00",
      tone: "neutral",
    }));
    mocked.readBoard.mockResolvedValue({ ...board(), activity });
    render(<ApplicationsStage stage="cv" />);
    const list = await screen.findByRole("list", { name: "Recent activity" });
    expect(within(list).getAllByRole("button")).toHaveLength(8);
    await user.click(screen.getByRole("button", { name: "Show all 12" }));
    expect(within(list).getAllByRole("button")).toHaveLength(12);
    await user.click(screen.getByRole("button", { name: "Show less" }));
    expect(within(list).getAllByRole("button")).toHaveLength(8);
  });

  it("says when the applications could not be read, with Retry", async () => {
    const user = userEvent.setup();
    const problem: Problem = { kind: "network", challenge: null, advice: null, settingsSection: null, retryable: true };
    mocked.readBoard.mockRejectedValueOnce(new CommandError("database is locked", problem));
    render(<ApplicationsStage stage="cv" />);
    const banner = await screen.findByRole("group", { name: "The applications could not be read" });
    expect(within(banner).getByText("database is locked")).toBeTruthy();
    await user.click(within(banner).getByRole("button", { name: "Retry" }));
    expect(await screen.findByRole("table", { name: "CVs" })).toBeTruthy();
    await waitFor(() => expect(mocked.readBoard).toHaveBeenCalledTimes(2));
  });
});
