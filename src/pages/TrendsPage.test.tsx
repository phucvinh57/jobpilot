import { act, cleanup, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { CommandError } from "@/lib/problem";
import { renderWithToasts as render } from "@/test/render";
import type { GraphProgress } from "@/api/events";

import type { Digest, RunResult, Term } from "@/types/trends";
import { useRuns } from "@/hooks/trends/runs";
import { useToasts } from "@/stores/toast";
import {
  ACME,
  chartFixture,
  challengeFixture,
  deferred,
  GLOBEX,
  globalScope,
  localScope,
  oldChartFixture,
  overviewFixture,
  PARTIAL_COVERAGE,
  reportRowFixture,
} from "@/test/fixtures/trends";
import { TrendsPage } from "./TrendsPage";

const mocks = vi.hoisted(() => ({
  api: {
    overview: vi.fn(),
    runJob: vi.fn(),
    addSource: vi.fn(),
    removeSource: vi.fn(),
    setSourceEnabled: vi.fn(),
    addToPreferences: vi.fn(),
    exportPdf: vi.fn(),
    deleteReport: vi.fn(),
  },
  invoke: vi.fn(),
  progress: new Set<(p: GraphProgress) => void>(),
}));

vi.mock("@/api/trends", () => mocks.api);
vi.mock("@/api/bridge", () => ({
  invoke: mocks.invoke,
  onDataChanged: () => () => undefined,
  onGraphProgress: (handler: (p: GraphProgress) => void) => {
    mocks.progress.add(handler);
    return () => {
      mocks.progress.delete(handler);
    };
  },
}));

const DONE: RunResult = {
  status: "done",
  reportId: 4,
  headline: "Digest ready: 8 point(s) from 6 item(s).",
  details: ["Hacker News: 4 new item(s)", "YC companies: timed out, left out of this run"],
};

function progress(patch: Partial<GraphProgress>) {
  act(() => {
    for (const handler of mocks.progress) {
      handler({ thread: "trends:2026-W46", graph: "trends", kind: "nodeStarted", node: null, text: null, ...patch });
    }
  });
}

function network() {
  return new CommandError("connection refused", {
    kind: "network",
    challenge: null,
    advice: "The connection failed. Check your network and try again.",
    settingsSection: null,
    retryable: true,
  });
}

beforeEach(() => {
  for (const fn of Object.values(mocks.api)) {
    fn.mockReset();
  }
  mocks.invoke.mockReset();
  mocks.invoke.mockResolvedValue(undefined);
  mocks.progress.clear();
  mocks.api.overview.mockResolvedValue(overviewFixture());
  useRuns.setState({ running: undefined, step: undefined, saved: undefined });
});

afterEach(cleanup);

/** The list (the Reports tab). */
async function shown() {
  render(<TrendsPage />);
  await screen.findByRole("region", { name: "Reports" });
  return userEvent.setup();
}

const row = (name: string) => screen.getByRole("row", { name });

/** The newest weekly report, opened from its row. */
async function opened() {
  const user = await shown();
  await user.click(row("Week of 9 November (2026-W46)"));
  await screen.findByRole("heading", { level: 1, name: "Weekly digest 2026-W46" });
  return user;
}

const newResearch = () => screen.getByRole("button", { name: "New research" });
const step = () => document.querySelector("#trends-step")?.textContent;
const primaries = (root: ParentNode = document) =>
  // The kit's primary fill is the only one with `text-on-fill`.
  Array.from(root.querySelectorAll("button")).filter((b) => b.className.includes("text-on-fill"));

/** New research > Collect & build digest, for the period that is chosen. */
async function startResearch(user: ReturnType<typeof userEvent.setup>) {
  await user.click(newResearch());
  const dialog = await screen.findByRole("dialog", { name: "New research" });
  await user.click(within(dialog).getByRole("button", { name: "Collect & build digest" }));
}

describe("the Trends screen", () => {
  it("lists the reports and keeps the digest for the report that is opened", async () => {
    await shown();
    expect(screen.getByRole("heading", { level: 1, name: "Trends" })).toBeTruthy();
    expect(screen.getByText(/Week of 9 November · from 6 postings and articles/)).toBeTruthy();
    expect(mocks.api.overview).toHaveBeenLastCalledWith(undefined, "weekly");
    expect(row("Week of 9 November (2026-W46)")).toBeTruthy();
    // No report content, and one tab at a time.
    expect(document.querySelector("#trends-digest")).toBeNull();
    expect(screen.queryByRole("region", { name: "Skills in demand" })).toBeNull();
    expect(screen.queryByRole("region", { name: "Sources" })).toBeNull();
    expect(screen.queryByRole("button", { name: "Export PDF" })).toBeNull();
    expect(primaries()).toEqual([newResearch()]);
  });

  it("switches between the reports and the sources", async () => {
    const user = await shown();
    const tabs = within(screen.getByRole("tablist", { name: "Trends" }));
    expect(document.querySelector("#trends-tabs [role=tablist]")).toBeTruthy();
    expect(tabs.getByRole("tab", { name: "Reports" }).getAttribute("aria-selected")).toBe("true");

    await user.click(tabs.getByRole("tab", { name: "Sources" }));
    expect(tabs.getByRole("tab", { name: "Sources" }).getAttribute("aria-selected")).toBe("true");
    expect(screen.queryByRole("region", { name: "Reports" })).toBeNull();
    expect(screen.getByRole("switch", { name: "Collect from Hacker News" }).getAttribute("aria-checked")).toBe("true");
    expect(
      screen.getByRole("switch", { name: "Collect from Example engineering blog" }).getAttribute("aria-checked"),
    ).toBe("false");
    // Still one primary: the dialog of a source has its own while it is open.
    expect(primaries()).toEqual([newResearch()]);
    await user.click(screen.getByRole("button", { name: "Web page" }));
    const dialog = await screen.findByRole("dialog", { name: "Add Web page" });
    expect(primaries(dialog)).toEqual([within(dialog).getByRole("button", { name: "Add" })]);
    await user.click(within(dialog).getByRole("button", { name: "Cancel" }));
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());

    await user.click(tabs.getByRole("tab", { name: "Reports" }));
    expect(screen.getByRole("region", { name: "Reports" })).toBeTruthy();
    expect(screen.queryByRole("region", { name: "Sources" })).toBeNull();
  });

  it("opens a report from its row, shows it with its actions, and goes back to the list", async () => {
    const user = await shown();
    const read = deferred<ReturnType<typeof overviewFixture>>();
    mocks.api.overview.mockReturnValueOnce(read.promise);
    await user.click(row("Week of 2 November (2026-W45)"));
    expect(mocks.api.overview).toHaveBeenLastCalledWith(2, "weekly");
    // Until it is read: its name, no digest of another report, nothing to act on.
    expect(screen.getByRole("heading", { level: 1, name: "Weekly digest 2026-W45" })).toBeTruthy();
    expect(screen.getByText("Loading…")).toBeTruthy();
    expect(document.querySelector("#trends-digest")).toBeNull();
    expect(screen.getByRole("button", { name: "Export PDF" }).hasAttribute("disabled")).toBe(true);
    expect(screen.queryByRole("tablist")).toBeNull();

    await act(async () =>
      read.resolve(
        overviewFixture({
          digest: { ...(overviewFixture().digest as Digest), reportId: 2, title: "Weekly digest 2026-W45" },
        }),
      ),
    );
    expect(document.querySelector("#trends-digest")).toBeTruthy();
    expect(screen.getByText("Week of 2 November (2026-W45)")).toBeTruthy();
    for (const name of ["Export PDF", "Rewrite digest", "Rerun"]) {
      expect(screen.getByRole("button", { name }).hasAttribute("disabled")).toBe(false);
    }
    expect(primaries(), "the actions of a report are all secondary").toEqual([]);

    await user.click(screen.getByRole("button", { name: "Back to Trends" }));
    expect(await screen.findByRole("heading", { level: 1, name: "Trends" })).toBeTruthy();
    expect(screen.getByRole("region", { name: "Reports" })).toBeTruthy();
    expect(document.querySelector("#trends-digest")).toBeNull();
    expect(mocks.api.overview).toHaveBeenLastCalledWith(undefined, "weekly");
  });

  it("shows the digest with its citations and the skill chart", async () => {
    await opened();
    expect(screen.getByText("2026-11-14 09:12 · 6 item(s) read")).toBeTruthy();

    // The digest: sections, numbered points, quote, citations.
    for (const title of ["Rising skills", "Active domains", "Hiring signals"]) {
      expect(screen.getByText(title)).toBeTruthy();
    }
    expect(screen.getByText("Rust shows up in more postings and in engineering write-ups about latency.")).toBeTruthy();
    expect(screen.getByText("“cut tail latency in half with Rust”")).toBeTruthy();
    expect(screen.getAllByRole("button", { name: "Source 1" })).toHaveLength(2);
    expect(screen.getAllByRole("button", { name: `Open source: ${GLOBEX}` })).toHaveLength(3);
    expect(screen.getByText("Every point links to the stored items it came from.")).toBeTruthy();
    expect(
      screen.getByText(
        "1 point(s) were left out: no valid source, a quote that is not in the source, or a claim against the app's numbers.",
      ),
    ).toBeTruthy();

    // The skill chart: the local market first (the gap's scope), three rows with share, direction and marks.
    const chart = within(screen.getByRole("region", { name: "Skills in demand" }));
    const rows = within(chart.getByRole("list", { name: "Skill share, Local" }));
    expect(rows.getAllByRole("listitem")).toHaveLength(3);
    expect(
      rows.getByRole("listitem", {
        name: /^Rust: 9\.4% · this period 11\.2% \(8\.6% to 14\.4%\) · rising · in your profile · in your wishes · 56 in total/,
      }),
    ).toBeTruthy();
    expect(rows.getAllByText("rising")).toHaveLength(2);
    // Each bar's share over the window is the bar; the level of the period and its range sit under it.
    expect(rows.getByText("this period 11.2% (8.6% to 14.4%)")).toBeTruthy();
    expect(rows.getAllByText(/^this period /)).toHaveLength(3);
    expect(chart.getByText("In your profile")).toBeTruthy();
    expect(chart.getByText("In your wishes")).toBeTruthy();
    expect(
      chart.getByText(/^Local market: Ho Chi Minh City, Singapore or remote, 593 hiring postings, 2026-06 to 2026-11/),
    ).toBeTruthy();
    expect(chart.getByRole("listitem", { name: "2026-06: 92 postings" })).toBeTruthy();
    expect(chart.getByText(/^Your own leads, for comparison: 26 over the window/)).toBeTruthy();
    // A report that saw everything it could says what it counted in one plain line, not a notice.
    expect(
      chart.getByText(
        (_, el) =>
          el?.id === "trends-coverage" &&
          el.textContent ===
            "Counted 2106 hiring postings over 2026-06 to 2026-11. Not compared with the period before (too few items or too uneven): YC companies.",
      ),
    ).toBeTruthy();
    expect(chart.queryByRole("group", { name: "The counts did not see everything" })).toBeNull();

    // The gap as the app sorted it, next to the model's point; the wishes; the unknown terms.
    const gap = within(screen.getByRole("region", { name: "Your gap: build on, learn next, at risk" }));
    expect(gap.getByText(/^Judged by the app on postings in Ho Chi Minh City/)).toBeTruthy();
    expect(
      within(gap.getByRole("group", { name: "Build on" })).getByRole("listitem", {
        name: "Rust: 9.4% of postings, this period 13.0% (9.0% to 22.0%), rising",
      }),
    ).toBeTruthy();
    expect(
      within(gap.getByRole("group", { name: "Build on" })).getByText("this period 13.0% (9.0% to 22.0%)"),
    ).toBeTruthy();
    expect(within(gap.getByRole("group", { name: "Learn next" })).getByText("Kubernetes")).toBeTruthy();
    expect(
      within(gap.getByRole("group", { name: "At risk" })).getByText("No skill of your profile is falling."),
    ).toBeTruthy();
    expect(gap.getByText("Kubernetes is the skill platform roles ask for most next to Rust and Go.")).toBeTruthy();
    const wishes = within(screen.getByRole("list", { name: "Your wishes, measured" }));
    expect(wishes.getByRole("listitem", { name: "PHP (skill, avoided): falling, 29 postings" })).toBeTruthy();
    expect(wishes.getByText("skill you avoid")).toBeTruthy();
    expect(wishes.getByRole("listitem", { name: "climate tech (domain): no data, no posting names it" })).toBeTruthy();
    // The terms the skill list does not know, by the kind of item, the tested risers marked.
    expect(
      screen.getByText(
        "hiring postings: Temporal (5 items, rising), dbt (4 items); Hacker News stories: vector databases (14 items)",
      ),
    ).toBeTruthy();
    expect(screen.queryByRole("region", { name: "Report search" })).toBeNull();
  });

  it("shows the coverage as a notice when the counts did not see everything", async () => {
    mocks.api.overview.mockResolvedValue(
      overviewFixture({ chart: chartFixture({ coverage: PARTIAL_COVERAGE, partial: true }) }),
    );
    await opened();
    const chart = within(screen.getByRole("region", { name: "Skills in demand" }));
    const notice = within(chart.getByRole("group", { name: "The counts did not see everything" }));
    // Each sentence is a line of its own (its months kept whole when it wraps).
    expect(notice.getAllByRole("listitem").map((li) => li.textContent)).toEqual(PARTIAL_COVERAGE);
    expect(document.querySelector("p#trends-coverage")).toBeNull();
  });

  it("lists the terms of a report saved before the kinds were told apart as it always did", async () => {
    const base = overviewFixture();
    const digest = base.digest;
    if (digest === null || digest.body.kind !== "data") {
      throw new Error("the fixture has a digest with data");
    }
    const terms: Term[] = [
      { term: "Temporal", items: 5, kind: "", rising: false },
      { term: "dbt", items: 4, kind: "", rising: false },
    ];
    mocks.api.overview.mockResolvedValue({
      ...base,
      digest: { ...digest, body: { ...digest.body, uncatalogued: terms } },
    });
    await opened();
    expect(screen.getByText("Temporal (5 items), dbt (4 items)")).toBeTruthy();
  });

  it("switches the chart between the local and the global market", async () => {
    const user = await opened();
    const chart = within(screen.getByRole("region", { name: "Skills in demand" }));
    const local = chart.getByRole("button", { name: "Market scope: Local" });
    const global = chart.getByRole("button", { name: "Market scope: Global" });
    expect(local.getAttribute("aria-pressed")).toBe("true");
    expect(document.querySelector("#trends-scope")).toBeTruthy();

    await user.click(global);
    expect(global.getAttribute("aria-pressed")).toBe("true");
    const rows = within(chart.getByRole("list", { name: "Skill share, Global" }));
    expect(rows.getAllByRole("listitem")).toHaveLength(2);
    expect(
      rows.getByRole("listitem", {
        name: /^PHP: 4\.6% · this period 3\.2% \(2\.4% to 4\.2%\) · falling · in your profile · in your wishes/,
      }),
    ).toBeTruthy();
    expect(chart.getByText(/^Global market: every posting, 2106 hiring postings/)).toBeTruthy();
    expect(chart.getByRole("listitem", { name: "2026-06: 340 postings" })).toBeTruthy();

    await user.click(local);
    expect(chart.getByRole("list", { name: "Skill share, Local" })).toBeTruthy();
  });

  it("starts on the global market when the local one was too thin, and says so on the local one", async () => {
    mocks.api.overview.mockResolvedValue(
      overviewFixture({
        chart: chartFixture({
          scope: "global",
          scopes: [
            localScope({ note: "Too little local data to tell directions: your gap is judged on the global market." }),
            globalScope(),
          ],
        }),
      }),
    );
    const user = await opened();
    const chart = within(screen.getByRole("region", { name: "Skills in demand" }));
    expect(chart.getByRole("button", { name: "Market scope: Global" }).getAttribute("aria-pressed")).toBe("true");
    expect(chart.queryByText(/^Too little local data/)).toBeNull();
    await user.click(chart.getByRole("button", { name: "Market scope: Local" }));
    expect(chart.getByText(/^Too little local data/)).toBeTruthy();
  });

  it("shows a chart saved before the app knew the user's place as it always did", async () => {
    mocks.api.overview.mockResolvedValue(overviewFixture({ chart: oldChartFixture() }));
    await opened();
    const chart = within(screen.getByRole("region", { name: "Skills in demand" }));
    expect(chart.queryByRole("group", { name: "Market scope" })).toBeNull();
    expect(chart.getAllByRole("listitem")).toHaveLength(2);
    expect(chart.getByRole("listitem", { name: /^Python: 29% · 117 in total/ })).toBeTruthy();
    expect(chart.getByText("In your preferences")).toBeTruthy();
    expect(chart.queryByText("In your wishes")).toBeNull();
    expect(chart.queryByText("rising")).toBeNull();
    expect(chart.getByText(/2026-06 to 2026-11 \(398 read\)/)).toBeTruthy();
    // No level, no range and no coverage in a report saved before the tests.
    expect(chart.queryByText(/this period/)).toBeNull();
    expect(document.querySelector("#trends-coverage")).toBeNull();
  });

  it("asks for the period of a new research in a dialog, with what the run does", async () => {
    const user = await shown();
    expect(screen.queryByRole("button", { name: "Collect & build digest" })).toBeNull();
    await user.click(newResearch());
    const dialog = await screen.findByRole("dialog", { name: "New research" });
    expect(dialog.id).toBe("trends-new-dialog");
    const collect = within(dialog).getByRole("button", { name: "Collect & build digest" });
    expect(primaries(dialog), "the one primary of the dialog").toEqual([collect]);
    expect(collect.hasAttribute("disabled")).toBe(false);
    expect(
      within(dialog).getByText(/^2026-W46: collects from your sources, then your AI provider writes the digest/),
    ).toBeTruthy();
    expect(dialog.querySelector("#trends-cadence")).toBeTruthy();
    expect(within(dialog).getByRole("button", { name: "Digest period: Weekly" }).getAttribute("aria-pressed")).toBe(
      "true",
    );

    mocks.api.overview.mockImplementation((_selected: number | undefined, cadence: string) =>
      Promise.resolve(
        overviewFixture(cadence === "quarterly" ? { period: { label: "2026-Q4", title: "Q4 2026" } } : {}),
      ),
    );
    await user.click(within(dialog).getByRole("button", { name: "Digest period: Quarterly" }));
    expect(await within(dialog).findByText(/^2026-Q4: collects from your sources/)).toBeTruthy();
    expect(mocks.api.overview).toHaveBeenLastCalledWith(undefined, "quarterly");

    // Cancel starts nothing.
    await user.click(within(dialog).getByRole("button", { name: "Cancel" }));
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    expect(mocks.api.runJob).not.toHaveBeenCalled();
    expect(primaries()).toEqual([newResearch()]);
  });

  it("opens a cited item with the shell, and only when it has a web address", async () => {
    const user = await opened();
    await user.click(screen.getAllByRole("button", { name: `Open source: ${ACME}` })[0] as HTMLElement);
    expect(mocks.invoke).toHaveBeenCalledWith("shared_open_url", { url: "https://news.example.com/item/1001" });

    mocks.invoke.mockClear();
    // The chip is the same link as the title.
    await user.click(screen.getAllByRole("button", { name: "Source 2" })[0] as HTMLElement);
    expect(mocks.invoke).toHaveBeenCalledWith("shared_open_url", { url: "https://news.example.com/item/1002" });

    // An item with no web address is text: nothing to open.
    mocks.invoke.mockClear();
    expect(screen.queryByRole("button", { name: "Open source: Fintech outlook 2026" })).toBeNull();
    expect(screen.getByText("Fintech outlook 2026")).toBeTruthy();
    await user.click(screen.getByRole("button", { name: "Source 3" }));
    expect(mocks.invoke).not.toHaveBeenCalled();
  });

  it("starts a research from the dialog: it closes at once, the row says the step, a toast the result", async () => {
    const user = await shown();
    const run = deferred<RunResult>();
    mocks.api.runJob.mockReturnValue(run.promise);
    await startResearch(user);

    expect(mocks.api.runJob).toHaveBeenCalledWith({ kind: "collect", period: "2026-W46" });
    // The dialog does not wait for the run.
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    const started = await screen.findByRole("status", { name: "Research started" });
    expect(within(started).getByText("Week of 9 November (2026-W46)")).toBeTruthy();
    expect(newResearch().hasAttribute("disabled")).toBe(true);
    // The row of the period being built says the step in place of when it was updated.
    const building = row("Week of 9 November (2026-W46)");
    expect(within(building).getByRole("status").textContent).toBe("Starting");
    expect(within(building).queryByText("2026-11-14 09:12")).toBeNull();
    expect(within(row("Week of 2 November (2026-W45)")).getByText("2026-11-07 09:00")).toBeTruthy();

    progress({ node: "extract" });
    expect(step()).toBe("Reading the items with your AI provider");
    // Steps of other flows are not this run's.
    progress({ graph: "discovery", node: "plan" });
    progress({ kind: "detail", node: "plan" });
    expect(step()).toBe("Reading the items with your AI provider");

    await act(async () => run.resolve(DONE));
    const result = within(await screen.findByRole("status", { name: "Digest ready: 8 point(s) from 6 item(s)." }));
    expect(
      result.getByText(/Hacker News: 4 new item\(s\) · YC companies: timed out, left out of this run/),
    ).toBeTruthy();
    expect(step()).toBeUndefined();
    // The list stays: the toast and the row say it, the report is opened by the user.
    expect(screen.getByRole("heading", { level: 1, name: "Trends" })).toBeTruthy();
    expect(screen.getByRole("region", { name: "Reports" })).toBeTruthy();
    expect(mocks.api.overview).not.toHaveBeenCalledWith(4, "weekly");
    expect(newResearch().hasAttribute("disabled")).toBe(false);
  });

  it("keeps a dialog of the list open when a run ends behind it", async () => {
    const user = await shown();
    const run = deferred<RunResult>();
    mocks.api.runJob.mockReturnValue(run.promise);
    await startResearch(user);
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());

    await user.click(screen.getByRole("tab", { name: "Sources" }));
    await user.click(screen.getByRole("button", { name: "Web page" }));
    expect(await screen.findByRole("dialog", { name: "Add Web page" })).toBeTruthy();

    await act(async () => run.resolve(DONE));
    // (The screen behind a modal dialog is hidden from the accessibility tree, the toast with it.)
    const name = "Digest ready: 8 point(s) from 6 item(s).";
    expect(await screen.findByRole("status", { name, hidden: true })).toBeTruthy();
    expect(screen.getByRole("dialog", { name: "Add Web page" })).toBeTruthy();
    expect(screen.getByRole("tab", { name: "Sources", hidden: true }).getAttribute("aria-selected")).toBe("true");
    expect(mocks.api.overview).not.toHaveBeenCalledWith(4, "weekly");
  });

  it("says the step above the list while the period being built has no row yet", async () => {
    const user = await shown();
    const run = deferred<RunResult>();
    mocks.api.runJob.mockReturnValue(run.promise);
    mocks.api.overview.mockImplementation((_selected: number | undefined, cadence: string) =>
      Promise.resolve(
        overviewFixture(cadence === "quarterly" ? { period: { label: "2026-Q4", title: "Q4 2026" } } : {}),
      ),
    );
    await user.click(newResearch());
    const dialog = await screen.findByRole("dialog", { name: "New research" });
    await user.click(within(dialog).getByRole("button", { name: "Digest period: Quarterly" }));
    await within(dialog).findByText(/^2026-Q4: collects/);
    await user.click(within(dialog).getByRole("button", { name: "Collect & build digest" }));
    expect(mocks.api.runJob).toHaveBeenCalledWith({ kind: "collect", period: "2026-Q4" });

    // The list follows the period chosen: no quarterly report yet.
    const reports = within(screen.getByRole("region", { name: "Reports" }));
    expect(reports.getByRole("button", { name: "Reports period: Quarterly" }).getAttribute("aria-pressed")).toBe(
      "true",
    );
    expect(step()).toBe("2026-Q4: Starting");
    expect(reports.getByText("No quarterly reports yet. Press New research and choose Quarterly.")).toBeTruthy();
    await act(async () => run.resolve({ ...DONE, reportId: null }));
  });

  it("starts one run at a time", async () => {
    const user = await shown();
    const run = deferred<RunResult>();
    mocks.api.runJob.mockReturnValue(run.promise);
    await startResearch(user);
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    await user.click(newResearch());
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(screen.getByRole("button", { name: /^Rerun .*2026-W45/ }).hasAttribute("disabled")).toBe(true);
    expect(mocks.api.runJob).toHaveBeenCalledTimes(1);
    // The reports can still be filtered while it runs.
    await user.click(screen.getByRole("button", { name: "Reports period: Monthly" }));
    expect(await screen.findByRole("row", { name: "November 2026 (2026-M11)" })).toBeTruthy();
    expect(step()).toBe("2026-W46: Starting");
    await act(async () => run.resolve({ ...DONE, reportId: null }));
    expect(newResearch().hasAttribute("disabled")).toBe(false);
  });

  it("writes the digest of the open report again from the stored items, or collects its period again", async () => {
    const user = await opened();
    const run = deferred<RunResult>();
    mocks.api.runJob.mockReturnValueOnce(run.promise);
    await user.click(screen.getByRole("button", { name: "Rewrite digest" }));
    expect(mocks.api.runJob).toHaveBeenCalledWith({ kind: "regenerate", period: "2026-W46" });
    // No second run meanwhile; saving the report as a PDF is not a run.
    for (const name of ["Rewrite digest", "Rerun"]) {
      expect(screen.getByRole("button", { name }).hasAttribute("disabled"), name).toBe(true);
    }
    expect(screen.getByRole("button", { name: "Export PDF" }).hasAttribute("disabled")).toBe(false);
    expect(step()).toBe("Starting");
    await act(async () => run.resolve(DONE));
    expect(await screen.findByRole("status", { name: "Digest ready: 8 point(s) from 6 item(s)." })).toBeTruthy();
    // The report that replaced the one on screen is shown.
    await waitFor(() => expect(mocks.api.overview).toHaveBeenLastCalledWith(4, "weekly"));

    mocks.api.runJob.mockResolvedValueOnce({ ...DONE, reportId: null });
    await user.click(screen.getByRole("button", { name: "Rerun" }));
    expect(mocks.api.runJob).toHaveBeenLastCalledWith({ kind: "collect", period: "2026-W46" });
  });

  it("says the step in the open report only for a run of its own period", async () => {
    await opened();
    act(() => useRuns.setState({ running: { kind: "collect", period: "2026-M11" }, step: "Counting skills" }));
    expect(step(), "another period is being built").toBeUndefined();
    // Still one run at a time.
    for (const name of ["Rewrite digest", "Rerun"]) {
      expect(screen.getByRole("button", { name }).hasAttribute("disabled"), name).toBe(true);
    }
    expect(screen.getByRole("button", { name: "Export PDF" }).hasAttribute("disabled")).toBe(false);

    act(() => useRuns.setState({ running: { kind: "collect", period: "2026-W46" }, step: "Counting skills" }));
    expect(step()).toBe("Counting skills");
    act(() => useRuns.setState({ running: undefined, step: undefined }));
  });

  it("stays on the open report when a run of another period saves one", async () => {
    await opened();
    act(() => useRuns.setState({ saved: { period: "2026-M11", reportId: 5 } }));
    expect(screen.getByRole("heading", { level: 1, name: "Weekly digest 2026-W46" })).toBeTruthy();
    expect(mocks.api.overview).not.toHaveBeenCalledWith(5, "weekly");
  });

  it("explains a failed run in a toast, offers the fix and Retry carries on where it stopped", async () => {
    const user = await shown();
    mocks.api.runJob.mockRejectedValueOnce(network());
    await startResearch(user);

    const toast = await screen.findByRole("alert", { name: "The run did not finish" });
    expect(within(toast).getByText("connection refused")).toBeTruthy();
    expect(within(toast).getByText("The connection failed. Check your network and try again.")).toBeTruthy();
    expect(screen.getAllByRole("alert", { name: "The run did not finish" }), "told once").toHaveLength(1);

    mocks.api.runJob.mockResolvedValueOnce(DONE);
    await user.click(within(toast).getByRole("button", { name: "Retry" }));
    expect(mocks.api.runJob).toHaveBeenLastCalledWith({ kind: "resume", period: "2026-W46" });
    expect(await screen.findByRole("status", { name: "Digest ready: 8 point(s) from 6 item(s)." })).toBeTruthy();
    expect(screen.queryByRole("alert", { name: "The run did not finish" }), "Retry closes the toast").toBeNull();
  });

  it("retries a failed rewrite as a digest-only run and offers no Retry for a plain error", async () => {
    const user = await opened();
    mocks.api.runJob.mockRejectedValueOnce(network());
    await user.click(screen.getByRole("button", { name: "Rewrite digest" }));
    const toast = await screen.findByRole("alert", { name: "The run did not finish" });
    mocks.api.runJob.mockRejectedValueOnce(new CommandError("the model answered nonsense"));
    await user.click(within(toast).getByRole("button", { name: "Retry" }));
    expect(mocks.api.runJob).toHaveBeenLastCalledWith({ kind: "resumeDigest", period: "2026-W46" });
    const again = await screen.findByText("the model answered nonsense");
    expect(within(again.closest("li") as HTMLElement).queryByRole("button", { name: "Retry" })).toBeNull();
  });

  it("toasts a run that failed while the user is on another screen, there and once, with Retry", async () => {
    const user = await shown();
    const run = deferred<RunResult>();
    mocks.api.runJob.mockReturnValueOnce(run.promise);
    await startResearch(user);
    cleanup();
    act(() => useToasts.getState().clear());

    // On another screen: the failure is said there, at once.
    render(<p>Jobs</p>);
    await act(async () => run.reject(network()));
    const toast = await screen.findByRole("alert", { name: "The run did not finish" });
    expect(within(toast).getByText("connection refused")).toBeTruthy();
    expect(useRuns.getState().running).toBeUndefined();

    // Coming back to Trends does not say it a second time.
    cleanup();
    await shown();
    expect(screen.getAllByRole("alert", { name: "The run did not finish" })).toHaveLength(1);
    cleanup();

    // Retry works from the other screen too.
    render(<p>Jobs</p>);
    mocks.api.runJob.mockResolvedValueOnce(DONE);
    await user.click(
      within(screen.getByRole("alert", { name: "The run did not finish" })).getByRole("button", { name: "Retry" }),
    );
    expect(mocks.api.runJob).toHaveBeenLastCalledWith({ kind: "resume", period: "2026-W46" });
    expect(await screen.findByRole("status", { name: "Digest ready: 8 point(s) from 6 item(s)." })).toBeTruthy();
    expect(screen.queryByRole("alert", { name: "The run did not finish" })).toBeNull();
  });

  it("points to Settings and tells the setup banner when there is no AI provider", async () => {
    const user = await shown();
    mocks.api.runJob.mockRejectedValueOnce(
      new CommandError("no AI provider is configured", {
        kind: "noProvider",
        challenge: null,
        advice: "Add an AI provider in Settings > AI providers and make it the default.",
        settingsSection: "AI providers",
        retryable: false,
      }),
    );
    await startResearch(user);
    const toast = await screen.findByRole("alert", { name: "The run did not finish" });
    expect(within(toast).getByRole("button", { name: "Open Settings > AI providers" })).toBeTruthy();
    expect(within(toast).queryByRole("button", { name: "Retry" })).toBeNull();
    expect(mocks.invoke).toHaveBeenCalledWith("shared_status");
  });

  it("says a source needs the user and lets them deal with it, continue, or build without it", async () => {
    mocks.api.overview.mockResolvedValue(overviewFixture({ challenges: [challengeFixture()] }));
    const user = await shown();
    const banner = screen.getByRole("group", { name: "Example annual report needs you to sign in (2026-W46)" });
    expect(within(banner).getByText(/Open the page, deal with it yourself, then continue/)).toBeTruthy();

    await user.click(within(banner).getByRole("button", { name: "Open page" }));
    expect(mocks.invoke).toHaveBeenCalledWith("shared_open_url", {
      url: "https://reports.example.com/annual-2026.html",
    });

    const waiting = "A source needs you before the digest can be built: see the banner below.";
    mocks.api.runJob.mockResolvedValue({ status: "waiting", reportId: null, headline: waiting, details: [] });
    await user.click(within(banner).getByRole("button", { name: "Continue" }));
    expect(mocks.api.runJob).toHaveBeenLastCalledWith({ kind: "resume", period: "2026-W46" });
    expect(await screen.findByRole("status", { name: waiting })).toBeTruthy();
    // Nothing was saved: the list stays, with its banner.
    expect(screen.getByRole("region", { name: "Reports" })).toBeTruthy();

    await user.click(within(banner).getByRole("button", { name: "Build digest without it" }));
    expect(mocks.api.runJob).toHaveBeenLastCalledWith({ kind: "regenerate", period: "2026-W46" });
  });

  it("keeps a source that needs the user in reach on the Sources tab and in an open report", async () => {
    mocks.api.overview.mockResolvedValue(overviewFixture({ challenges: [challengeFixture()] }));
    const user = await shown();
    const name = "Example annual report needs you to sign in (2026-W46)";
    const waiting = { status: "waiting", reportId: null, headline: "Still waiting.", details: [] };
    mocks.api.runJob.mockResolvedValue(waiting);

    await user.click(screen.getByRole("tab", { name: "Sources" }));
    expect(screen.queryByRole("region", { name: "Reports" })).toBeNull();
    await user.click(within(screen.getByRole("group", { name })).getByRole("button", { name: "Continue" }));
    expect(mocks.api.runJob).toHaveBeenLastCalledWith({ kind: "resume", period: "2026-W46" });
    await screen.findByRole("status", { name: "Still waiting." });

    await user.click(screen.getByRole("tab", { name: "Reports" }));
    await user.click(row("Week of 2 November (2026-W45)"));
    await screen.findByRole("button", { name: "Back to Trends" });
    const banner = await screen.findByRole("group", { name });
    expect(banner.closest("main")?.getAttribute("aria-label")).toBe("Trends report");
    await user.click(within(banner).getByRole("button", { name: "Build digest without it" }));
    expect(mocks.api.runJob).toHaveBeenLastCalledWith({ kind: "regenerate", period: "2026-W46" });
    // One banner, not one per place.
    expect(screen.getAllByRole("group", { name })).toHaveLength(1);
  });

  it("does not offer a page that is not a web address", async () => {
    mocks.api.overview.mockResolvedValue(
      overviewFixture({
        challenges: [challengeFixture({ kind: "blocked", what: "blocks automated access", url: null })],
      }),
    );
    await shown();
    const banner = screen.getByRole("group", { name: "Example annual report blocks automated access (2026-W46)" });
    expect(within(banner).getByRole("button", { name: "Open page" }).hasAttribute("disabled")).toBe(true);
  });

  it("keeps the run going while the user is on another screen, and says how it ended there", async () => {
    const user = await shown();
    const run = deferred<RunResult>();
    mocks.api.runJob.mockReturnValue(run.promise);
    await startResearch(user);
    cleanup();
    act(() => useToasts.getState().clear());

    // On another screen: the result is a toast all the same.
    const elsewhere = render(<p>Jobs</p>);
    expect(useRuns.getState().running).toEqual({ kind: "collect", period: "2026-W46" });
    elsewhere.unmount();

    await shown();
    expect(newResearch().hasAttribute("disabled")).toBe(true);
    expect(step()).toBe("Starting");
    cleanup();

    render(<p>Jobs</p>);
    await act(async () => run.resolve(DONE));
    const result = await screen.findByRole("status", { name: "Digest ready: 8 point(s) from 6 item(s)." });
    expect(within(result).getByText(/YC companies: timed out/)).toBeTruthy();
    expect(useRuns.getState().running).toBeUndefined();
  });

  it("leaves the user on the list when the run they came back to ends", async () => {
    const user = await shown();
    const run = deferred<RunResult>();
    mocks.api.runJob.mockReturnValue(run.promise);
    await startResearch(user);
    cleanup();

    await shown();
    await act(async () => run.resolve(DONE));
    expect(await screen.findByRole("status", { name: "Digest ready: 8 point(s) from 6 item(s)." })).toBeTruthy();
    expect(newResearch().hasAttribute("disabled")).toBe(false);
    expect(screen.getByRole("heading", { level: 1, name: "Trends" })).toBeTruthy();
    expect(mocks.api.overview).not.toHaveBeenCalledWith(4, "weekly");
  });

  it("lists the reports of the chosen cadence", async () => {
    await shown();
    const reports = within(screen.getByRole("region", { name: "Reports" }));
    expect(reports.getByRole("button", { name: "Reports period: Weekly" }).getAttribute("aria-pressed")).toBe("true");
    expect(reports.getByRole("row", { name: "Week of 9 November (2026-W46)" })).toBeTruthy();
    expect(reports.queryByRole("row", { name: "November 2026 (2026-M11)" })).toBeNull();
  });

  it("saves the open report as a PDF", async () => {
    mocks.api.exportPdf.mockResolvedValue("/tmp/jobpilot-market-insight-2026-W46.pdf");
    const user = await opened();
    await user.click(screen.getByRole("button", { name: "Export PDF" }));
    expect(mocks.api.exportPdf).toHaveBeenCalledWith(3, "jobpilot-market-insight-2026-W46.pdf");
    expect(await screen.findByRole("status", { name: "Saved PDF" })).toBeTruthy();
  });

  it("tells a new user what to do first", async () => {
    mocks.api.overview.mockResolvedValue(
      overviewFixture({
        summary: "No digest yet · Week of 9 November is the current period",
        digest: null,
        chart: null,
        history: [],
        sources: [],
      }),
    );
    const user = await shown();
    expect(screen.getByText("No digest yet · Week of 9 November is the current period")).toBeTruthy();
    const reports = within(screen.getByRole("region", { name: "Reports" }));
    expect(reports.getByText("No reports yet")).toBeTruthy();
    expect(reports.getByText("No weekly reports yet. Press New research and choose Weekly.")).toBeTruthy();
    expect(reports.queryByRole("table")).toBeNull();
    // The empty list points at the one primary; it has none of its own.
    expect(primaries()).toEqual([newResearch()]);
    await user.click(reports.getByRole("button", { name: "Reports period: Yearly" }));
    expect(await reports.findByText("No yearly reports yet. Press New research and choose Yearly.")).toBeTruthy();

    await user.click(screen.getByRole("tab", { name: "Sources" }));
    expect(screen.getByText("No sources yet: add Hacker News, the YC companies, a feed or a page.")).toBeTruthy();
  });

  it("shows a report this version cannot read back as its saved text", async () => {
    mocks.api.overview.mockResolvedValue(
      overviewFixture({
        chart: null,
        history: [reportRowFixture({ id: 1, title: "Old digest", period: "2026-X1", periodTitle: "2026-X1" })],
        digest: {
          reportId: 1,
          title: "Old digest",
          when: "2026-01-02 10:00",
          itemsRead: null,
          body: { kind: "plain", lines: ["# Old digest", "- a point"] },
        },
      }),
    );
    const user = await shown();
    await user.click(row("2026-X1"));
    expect(await screen.findByText("# Old digest")).toBeTruthy();
    expect(screen.getByText("- a point")).toBeTruthy();
    expect(screen.getByText("2026-01-02 10:00")).toBeTruthy();
  });

  it("says when the saved data cannot be read, and offers to read again", async () => {
    mocks.api.overview.mockRejectedValue(new CommandError("disk full"));
    render(<TrendsPage />);
    const toast = await screen.findByRole("alert", { name: "The saved data could not be read" });
    expect(within(toast).getByText("disk full")).toBeTruthy();
    expect(screen.getByText("The saved data could not be read.")).toBeTruthy();
    expect(screen.queryByRole("region", { name: "Reports" })).toBeNull();

    // No period is known: a research cannot start.
    const user = userEvent.setup();
    await user.click(newResearch());
    const dialog = await screen.findByRole("dialog", { name: "New research" });
    expect(within(dialog).getByRole("button", { name: "Collect & build digest" }).hasAttribute("disabled")).toBe(true);
    await user.click(within(dialog).getByRole("button", { name: "Cancel" }));
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());

    mocks.api.overview.mockResolvedValue(overviewFixture());
    await user.click(screen.getAllByRole("button", { name: "Retry" })[0] as HTMLElement);
    expect(await screen.findByRole("region", { name: "Reports" })).toBeTruthy();
  });

  it("shows text from feeds as text, never as markup", async () => {
    const evil = '<img src=x onerror="alert(1)"> **bold**';
    const fixture = overviewFixture();
    const first = fixture.digest?.body.kind === "data" ? fixture.digest.body.sections[0]?.points[0] : undefined;
    if (first) {
      first.text = evil;
      first.citations = [{ number: 1, title: evil, url: "https://news.example.com/x" }];
    }
    mocks.api.overview.mockResolvedValue(fixture);
    const user = userEvent.setup();
    const { container } = render(<TrendsPage />);
    await user.click(await screen.findByRole("row", { name: "Week of 9 November (2026-W46)" }));
    expect((await screen.findAllByText(evil)).length).toBeGreaterThan(0);
    expect(container.querySelector("img")).toBeNull();
  });
});
