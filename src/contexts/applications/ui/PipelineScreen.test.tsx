import { cleanup, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { CommandError, useNav } from "@/shared";

import * as api from "../api/applicationsApi";
import { board } from "../domain/fixtures";
import { PipelineScreen } from "./PipelineScreen";

vi.mock("../api/applicationsApi");
vi.mock("@/shared/bridge", () => ({
  invoke: vi.fn().mockResolvedValue(undefined),
  onDataChanged: () => () => undefined,
  onGraphProgress: () => () => undefined,
}));

const mocked = vi.mocked(api);

beforeEach(() => {
  useNav.setState({ screen: "pipeline", applicationId: undefined, detailFrom: "pipeline" });
  mocked.readBoard.mockResolvedValue(board());
});

afterEach(() => {
  cleanup();
});

describe("the Pipeline", () => {
  it("shows the five columns with their cards, next actions and ages", async () => {
    render(<PipelineScreen />);
    expect(await screen.findByRole("heading", { name: "Pipeline" })).toBeTruthy();
    expect(screen.getByText("3 applications. Stages update from your Gmail replies.")).toBeTruthy();

    const columns = screen.getAllByRole("group").filter((g) => g.id.startsWith("column-"));
    expect(columns.map((c) => c.getAttribute("aria-label"))).toEqual([
      "CV review, 1",
      "Ready to apply, 1",
      "Submitted, 1",
      "Interview, 0",
      "Closed, 0",
    ]);
    const ready = screen.getByRole("group", { name: "Ready to apply, 1" });
    const card = within(ready).getByRole("button", { name: "Globex, SRE: Review & submit, CV ready" });
    expect(within(card).getByText("2 d")).toBeTruthy();
    expect(within(card).getByText("Review & submit")).toBeTruthy();
    expect(within(screen.getByRole("group", { name: "Interview, 0" })).getByText("Nothing here")).toBeTruthy();
  });

  it("opens an application from a card, remembering the Pipeline as where it came from", async () => {
    const user = userEvent.setup();
    render(<PipelineScreen />);
    await user.click(await screen.findByRole("button", { name: "Acme, Backend Engineer: Approve CV, Draft" }));
    expect(useNav.getState().screen).toBe("application");
    expect(useNav.getState().applicationId).toBe(1);
    expect(useNav.getState().detailFrom).toBe("pipeline");
  });

  it("lists the recent activity and opens the application of a row", async () => {
    const user = userEvent.setup();
    render(<PipelineScreen />);
    const list = await screen.findByRole("list", { name: "Recent activity" });
    const rows = within(list).getAllByRole("button");
    expect(rows.map((r) => r.getAttribute("aria-label"))).toEqual(["Initech: moved to Applied, 09:05", "Acme: application created, Yesterday"]);
    await user.click(rows[0] as HTMLElement);
    expect(useNav.getState().applicationId).toBe(3);
  });

  it("explains an empty board and leads to Today", async () => {
    const user = userEvent.setup();
    mocked.readBoard.mockResolvedValue({ count: 0, columns: [], activity: [] });
    render(<PipelineScreen />);
    expect(await screen.findByText("Prepare a CV for a job on Today and it shows up here.")).toBeTruthy();
    expect(screen.getByText("No applications yet.")).toBeTruthy();
    await user.click(screen.getByRole("button", { name: "Go to Today" }));
    expect(useNav.getState().screen).toBe("today");
  });

  it("says what went wrong when the board cannot be read", async () => {
    mocked.readBoard.mockRejectedValue(new CommandError("database is locked"));
    render(<PipelineScreen />);
    await waitFor(() => expect(screen.getByText("database is locked")).toBeTruthy());
    expect(screen.getByText("The pipeline could not be read")).toBeTruthy();
  });
});
