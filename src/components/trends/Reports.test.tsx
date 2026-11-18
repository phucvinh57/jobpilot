import { act, cleanup, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { CommandError } from "@/lib/problem";
import { renderWithToasts } from "@/test/render";

import { useRuns } from "@/hooks/trends/runs";
import { overviewFixture, reportRowFixture } from "@/test/fixtures/trends";
import { TrendsPage } from "@/pages/TrendsPage";

const mocks = vi.hoisted(() => ({
  api: {
    overview: vi.fn(),
    runJob: vi.fn(),
    addSource: vi.fn(),
    removeSource: vi.fn(),
    setSourceEnabled: vi.fn(),
    exportPdf: vi.fn(),
    deleteReport: vi.fn(),
  },
  invoke: vi.fn(),
}));

vi.mock("@/api/trends", () => mocks.api);
vi.mock("@/api/bridge", () => ({
  invoke: mocks.invoke,
  onDataChanged: () => () => undefined,
  onGraphProgress: () => () => undefined,
}));

beforeEach(() => {
  for (const fn of Object.values(mocks.api)) {
    fn.mockReset();
    fn.mockResolvedValue(undefined);
  }
  mocks.api.overview.mockImplementation((_selected: number | undefined, cadence: string) =>
    Promise.resolve(
      overviewFixture({
        period: cadence === "monthly" ? { label: "2026-M11", title: "November 2026" } : overviewFixture().period,
        history: [
          reportRowFixture({ id: 9, title: "Yearly digest 2026", period: "2026", points: 12 }),
          reportRowFixture({ id: 5, title: "Monthly digest 2026-M11", period: "2026-M11", points: 8 }),
          reportRowFixture(),
          reportRowFixture({ id: 2, title: "Weekly digest 2026-W45", when: "2026-11-07 09:00", period: "2026-W45" }),
          reportRowFixture({ id: 1, title: "Old digest", period: "2026-X1", periodTitle: "2026-X1", rerun: false }),
        ],
      }),
    ),
  );
  mocks.api.runJob.mockResolvedValue({ status: "done", reportId: 5, headline: "Digest ready.", details: [] });
  useRuns.setState({ running: undefined, step: undefined, notice: undefined });
});

afterEach(cleanup);

async function shown() {
  renderWithToasts(<TrendsPage />);
  await screen.findByRole("region", { name: "Reports" });
  return userEvent.setup();
}

const reports = () => within(screen.getByRole("region", { name: "Reports" }));
const rowNames = () =>
  reports()
    .queryAllByRole("row")
    .map((row) => row.getAttribute("aria-label"))
    .filter((name) => name !== null);

describe("Reports", () => {
  it("shows the reports of one cadence, and no Open button: the row opens the report", async () => {
    const user = await shown();
    expect(rowNames()).toEqual(["Week of 9 November (2026-W46)", "Week of 2 November (2026-W45)", "2026-X1"]);
    expect(reports().queryByRole("button", { name: /^Open/ })).toBeNull();
    expect(reports().getAllByRole("button", { name: /^(Rerun|Export PDF of|Delete) / })).toHaveLength(9);

    await user.click(reports().getByRole("button", { name: "Reports period: Yearly" }));
    await waitFor(() => expect(rowNames()).toEqual(["2026"]));
    expect(mocks.api.overview).toHaveBeenLastCalledWith(undefined, "yearly");
    expect(reports().getByText("12")).toBeTruthy();
    await user.click(reports().getByRole("button", { name: "Reports period: Quarterly" }));
    expect(rowNames()).toEqual([]);
    expect(reports().getByText("No quarterly reports yet. Press New research and choose Quarterly.")).toBeTruthy();

    // The period chosen for a new research moves it.
    await user.click(screen.getByRole("button", { name: "New research" }));
    const dialog = within(await screen.findByRole("dialog", { name: "New research" }));
    await user.click(dialog.getByRole("button", { name: "Digest period: Monthly" }));
    await user.click(dialog.getByRole("button", { name: "Cancel" }));
    await waitFor(() => expect(rowNames()).toEqual(["November 2026 (2026-M11)"]));
    expect(reports().getByRole("button", { name: "Reports period: Monthly" }).getAttribute("aria-pressed")).toBe(
      "true",
    );
  });

  it("opens a report by a click on its row or Enter on it, not by its buttons", async () => {
    const user = await shown();
    reports().getByRole("row", { name: "Week of 2 November (2026-W45)" }).focus();
    await user.keyboard("{Enter}");
    expect(mocks.api.overview).toHaveBeenLastCalledWith(2, "weekly");
    expect(await screen.findByRole("button", { name: "Back to Trends" })).toBeTruthy();
  });

  it("says the step of a run in the row of its period, in place of when it was updated", async () => {
    await shown();
    act(() => useRuns.setState({ running: { kind: "collect", period: "2026-W45" }, step: "Writing the digest" }));
    const building = within(reports().getByRole("row", { name: "Week of 2 November (2026-W45)" }));
    expect(building.getByRole("status").textContent).toBe("Writing the digest");
    expect(building.getByRole("status").id).toBe("trends-step");
    expect(building.queryByText("2026-11-07 09:00")).toBeNull();
    // The other rows keep theirs, and none can be rerun meanwhile.
    expect(reports().getAllByRole("status")).toHaveLength(1);
    expect(reports().getAllByText("2026-11-14 09:12").length).toBeGreaterThan(0);
    expect(
      reports()
        .getAllByRole("button", { name: /^Rerun/ })
        .every((b) => b.hasAttribute("disabled")),
    ).toBe(true);

    // A period with no row in the list: the step is said above the table.
    act(() => useRuns.setState({ running: { kind: "collect", period: "2026-Q4" }, step: "Counting skills" }));
    expect(reports().getByRole("status").textContent).toBe("2026-Q4: Counting skills");
    expect(reports().getByRole("status").closest("tr")).toBeNull();

    act(() => useRuns.setState({ running: undefined, step: undefined }));
    expect(reports().queryByRole("status")).toBeNull();
  });

  it("reruns a report for its own period, one run at a time", async () => {
    const user = await shown();
    await user.click(reports().getByRole("button", { name: "Reports period: Monthly" }));
    await user.click(reports().getByRole("button", { name: "Rerun November 2026 (2026-M11)" }));
    expect(mocks.api.runJob).toHaveBeenCalledWith({ kind: "collect", period: "2026-M11" });
    // The run saved report 5: it is opened.
    await waitFor(() => expect(mocks.api.overview).toHaveBeenLastCalledWith(5, "monthly"));
    await user.click(await screen.findByRole("button", { name: "Back to Trends" }));

    act(() => useRuns.setState({ running: { kind: "collect", period: "2026-W46" }, step: "Starting" }));
    expect(
      (await reports().findByRole("button", { name: "Rerun November 2026 (2026-M11)" })).hasAttribute("disabled"),
    ).toBe(true);
  });

  it("offers no rerun for a report whose period cannot be read", async () => {
    await shown();
    expect(
      reports()
        .getByRole("button", { name: /^Rerun .*2026-W45/ })
        .hasAttribute("disabled"),
    ).toBe(false);
    const rerun = reports()
      .getAllByRole("button", { name: /^Rerun/ })
      .find((b) => b.id === "trends-report-1-rerun");
    expect(rerun?.hasAttribute("disabled")).toBe(true);
  });

  it("saves a report as a PDF, and says nothing when the save was cancelled", async () => {
    const user = await shown();
    mocks.api.exportPdf.mockResolvedValueOnce(null);
    await user.click(reports().getByRole("button", { name: "Export PDF of Week of 2 November (2026-W45)" }));
    expect(mocks.api.exportPdf).toHaveBeenCalledWith(2, "jobpilot-market-insight-2026-W45.pdf");
    await waitFor(() =>
      expect(
        reports().getByRole("button", { name: "Export PDF of Week of 2 November (2026-W45)" }).hasAttribute("disabled"),
      ).toBe(false),
    );
    expect(screen.queryByRole("status", { name: "Saved PDF" })).toBeNull();

    // The user renamed it in the dialog: the toast names the file written.
    mocks.api.exportPdf.mockResolvedValueOnce("/home/me/Docs/early-november.pdf");
    await user.click(reports().getByRole("button", { name: "Export PDF of Week of 2 November (2026-W45)" }));
    const note = await screen.findByRole("status", { name: "Saved PDF" });
    expect(within(note).getByText("early-november.pdf")).toBeTruthy();
  });

  it("says in a toast why a PDF could not be saved", async () => {
    mocks.api.exportPdf.mockRejectedValueOnce(new CommandError("typst not found"));
    const user = await shown();
    await user.click(reports().getByRole("button", { name: "Export PDF of Week of 9 November (2026-W46)" }));
    const toast = await screen.findByRole("alert", { name: "The report could not be saved as PDF" });
    expect(within(toast).getByText("typst not found")).toBeTruthy();
  });

  it("deletes a report after asking, and says so", async () => {
    const user = await shown();
    await user.click(reports().getByRole("button", { name: "Delete Week of 2 November (2026-W45)" }));
    const dialog = within(await screen.findByRole("dialog", { name: "Delete this report?" }));
    expect(mocks.api.deleteReport).not.toHaveBeenCalled();
    await user.click(dialog.getByRole("button", { name: "Delete" }));
    expect(mocks.api.deleteReport).toHaveBeenCalledWith(2);
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    expect(await screen.findByRole("status", { name: "Report deleted" })).toBeTruthy();
    // Still the list: a button of a row does not open its report.
    expect(mocks.api.overview).not.toHaveBeenCalledWith(2, "weekly");
  });

  it("keeps the report when the delete is cancelled", async () => {
    const user = await shown();
    await user.click(reports().getByRole("button", { name: "Delete Week of 9 November (2026-W46)" }));
    const dialog = within(await screen.findByRole("dialog", { name: "Delete this report?" }));
    await user.click(dialog.getByRole("button", { name: "Cancel" }));
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    expect(mocks.api.deleteReport).not.toHaveBeenCalled();
  });
});
