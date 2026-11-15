import { act, cleanup, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { CommandError, type GraphProgress } from "@/shared";

import type { RunResult } from "../domain/types";
import { useRuns } from "../hooks";
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
} from "../testing";
import { TrendsScreen } from "./TrendsScreen";

const mocks = vi.hoisted(() => ({
  api: {
    overview: vi.fn(),
    runJob: vi.fn(),
    addSource: vi.fn(),
    removeSource: vi.fn(),
    setSourceEnabled: vi.fn(),
    saveKey: vi.fn(),
    clearKey: vi.fn(),
    addToPreferences: vi.fn(),
  },
  invoke: vi.fn(),
  progress: new Set<(p: GraphProgress) => void>(),
}));

vi.mock("../api", () => mocks.api);
vi.mock("@/shared/bridge", () => ({
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
  useRuns.setState({ running: undefined, step: undefined, notice: undefined });
});

afterEach(cleanup);

async function shown() {
  render(<TrendsScreen />);
  await screen.findByRole("heading", { name: "Weekly digest 2026-W46" });
  return userEvent.setup();
}

const collect = () => screen.getByRole("button", { name: "Collect & build digest" });

describe("the Trends screen", () => {
  it("shows the digest with its citations, the skill chart, the sources and report search", async () => {
    await shown();
    expect(screen.getByRole("heading", { level: 1, name: "Trends" })).toBeTruthy();
    expect(screen.getByText(/Week of 9 November · from 6 postings and articles/)).toBeTruthy();
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
      rows.getByRole("listitem", { name: /^Rust: 9\.4% · rising · in your profile · in your wishes · 56 in total/ }),
    ).toBeTruthy();
    expect(rows.getAllByText("rising")).toHaveLength(2);
    expect(chart.getByText("In your profile")).toBeTruthy();
    expect(chart.getByText("In your wishes")).toBeTruthy();
    expect(
      chart.getByText(/^Local market: Ho Chi Minh City, Singapore or remote, 593 hiring postings, 2026-06 to 2026-11/),
    ).toBeTruthy();
    expect(chart.getByRole("listitem", { name: "2026-06: 92 postings" })).toBeTruthy();
    expect(chart.getByText(/^Your own leads, for comparison: 26 over the window/)).toBeTruthy();

    // The gap as the app sorted it, next to the model's point; the wishes; the unknown terms.
    const gap = within(screen.getByRole("region", { name: "Your gap: build on, learn next, at risk" }));
    expect(gap.getByText(/^Judged by the app on postings in Ho Chi Minh City/)).toBeTruthy();
    expect(
      within(gap.getByRole("group", { name: "Build on" })).getByRole("listitem", {
        name: "Rust: 9.4% of postings, rising",
      }),
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
    expect(screen.getByText("Temporal (5 items), dbt (4 items)")).toBeTruthy();

    // Sources and report search.
    expect(screen.getByRole("switch", { name: "Collect from Hacker News" }).getAttribute("aria-checked")).toBe("true");
    expect(
      screen.getByRole("switch", { name: "Collect from Example engineering blog" }).getAttribute("aria-checked"),
    ).toBe("false");
    expect(screen.getByText("Report search is on.")).toBeTruthy();
  });

  it("switches the chart between the local and the global market", async () => {
    const user = await shown();
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
      rows.getByRole("listitem", { name: /^PHP: 4\.6% · falling · in your profile · in your wishes/ }),
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
    const user = await shown();
    const chart = within(screen.getByRole("region", { name: "Skills in demand" }));
    expect(chart.getByRole("button", { name: "Market scope: Global" }).getAttribute("aria-pressed")).toBe("true");
    expect(chart.queryByText(/^Too little local data/)).toBeNull();
    await user.click(chart.getByRole("button", { name: "Market scope: Local" }));
    expect(chart.getByText(/^Too little local data/)).toBeTruthy();
  });

  it("shows a chart saved before the app knew the user's place as it always did", async () => {
    mocks.api.overview.mockResolvedValue(overviewFixture({ chart: oldChartFixture() }));
    await shown();
    const chart = within(screen.getByRole("region", { name: "Skills in demand" }));
    expect(chart.queryByRole("group", { name: "Market scope" })).toBeNull();
    expect(chart.getAllByRole("listitem")).toHaveLength(2);
    expect(chart.getByRole("listitem", { name: /^Python: 29% · 117 in total/ })).toBeTruthy();
    expect(chart.getByText("In your preferences")).toBeTruthy();
    expect(chart.queryByText("In your wishes")).toBeNull();
    expect(chart.queryByText("rising")).toBeNull();
    expect(chart.getByText(/2026-06 to 2026-11 \(398 read\)/)).toBeTruthy();
  });

  it("has one primary action, the period choice and the caption of what a run does", async () => {
    const user = await shown();
    expect(collect().hasAttribute("disabled")).toBe(false);
    // Exactly one primary button on the screen (the kit's primary fill is the only one with `text-on-fill`),
    // also with the form of a source open.
    const primaries = () =>
      Array.from(document.querySelectorAll("button")).filter((b) => b.className.includes("text-on-fill"));
    expect(primaries()).toEqual([collect()]);
    await user.click(screen.getByRole("button", { name: "Web page" }));
    expect(primaries()).toEqual([collect()]);
    await user.click(screen.getByRole("button", { name: "Web page" }));
    expect(
      screen.getByText(/^2026-W46: collects from your sources, then your AI provider writes the digest/),
    ).toBeTruthy();
    expect(screen.getByRole("button", { name: "Digest period: Weekly" }).getAttribute("aria-pressed")).toBe("true");

    mocks.api.overview.mockImplementation((_selected: number | undefined, cadence: string) =>
      Promise.resolve(
        overviewFixture(cadence === "quarterly" ? { period: { label: "2026-Q4", title: "Q4 2026" } } : {}),
      ),
    );
    await user.click(screen.getByRole("button", { name: "Digest period: Quarterly" }));
    expect(await screen.findByText(/^2026-Q4: collects from your sources/)).toBeTruthy();
    expect(mocks.api.overview).toHaveBeenLastCalledWith(undefined, "quarterly");
  });

  it("opens a cited item with the shell, and only when it has a web address", async () => {
    const user = await shown();
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

  it("collects and builds the digest: busy with its step, then the summary of the run", async () => {
    const user = await shown();
    const run = deferred<RunResult>();
    mocks.api.runJob.mockReturnValue(run.promise);
    await user.click(collect());

    expect(mocks.api.runJob).toHaveBeenCalledWith({ kind: "collect", period: "2026-W46" });
    expect(collect().hasAttribute("disabled")).toBe(true);
    expect(screen.getByRole("button", { name: "Regenerate from stored items" }).hasAttribute("disabled")).toBe(true);
    expect(screen.getByRole("status").textContent).toBe("Starting");

    progress({ node: "extract" });
    expect(screen.getByRole("status").textContent).toBe("Reading the items with your AI provider");
    // Steps of other flows are not this run's.
    progress({ graph: "discovery", node: "plan" });
    progress({ kind: "detail", node: "plan" });
    expect(screen.getByRole("status").textContent).toBe("Reading the items with your AI provider");

    await act(async () => run.resolve(DONE));
    const result = within(screen.getByRole("group", { name: "Result" }));
    expect(result.getByText("Digest ready: 8 point(s) from 6 item(s).")).toBeTruthy();
    expect(result.getByText("YC companies: timed out, left out of this run")).toBeTruthy();
    expect(screen.queryByRole("status")).toBeNull();
    expect(collect().hasAttribute("disabled")).toBe(false);
    // The new digest is the one shown.
    await waitFor(() => expect(mocks.api.overview).toHaveBeenLastCalledWith(4, "weekly"));
  });

  it("starts one run at a time", async () => {
    const user = await shown();
    const run = deferred<RunResult>();
    mocks.api.runJob.mockReturnValue(run.promise);
    await user.click(collect());
    await user.click(collect());
    expect(mocks.api.runJob).toHaveBeenCalledTimes(1);
    // The period waits for the run to finish.
    await user.click(screen.getByRole("button", { name: "Digest period: Quarterly" }));
    expect(mocks.api.overview).not.toHaveBeenCalledWith(undefined, "quarterly");
    await act(async () => run.resolve(DONE));
  });

  it("writes the digest again from the stored items", async () => {
    const user = await shown();
    mocks.api.runJob.mockResolvedValue(DONE);
    await user.click(screen.getByRole("button", { name: "Regenerate from stored items" }));
    expect(mocks.api.runJob).toHaveBeenCalledWith({ kind: "regenerate", period: "2026-W46" });
    expect(await screen.findByText("Digest ready: 8 point(s) from 6 item(s).")).toBeTruthy();
  });

  it("explains a failed run, offers the fix and Retry carries on where it stopped", async () => {
    const user = await shown();
    mocks.api.runJob.mockRejectedValueOnce(network());
    await user.click(collect());

    const banner = await screen.findByRole("group", { name: "The run did not finish" });
    expect(within(banner).getByText("connection refused")).toBeTruthy();
    expect(within(banner).getByText("The connection failed. Check your network and try again.")).toBeTruthy();

    mocks.api.runJob.mockResolvedValueOnce(DONE);
    await user.click(within(banner).getByRole("button", { name: "Retry" }));
    expect(mocks.api.runJob).toHaveBeenLastCalledWith({ kind: "resume", period: "2026-W46" });
    expect(await screen.findByText("Digest ready: 8 point(s) from 6 item(s).")).toBeTruthy();
  });

  it("retries a failed refresh as a digest-only run and offers no Retry for a plain error", async () => {
    const user = await shown();
    mocks.api.runJob.mockRejectedValueOnce(network());
    await user.click(screen.getByRole("button", { name: "Regenerate from stored items" }));
    const banner = await screen.findByRole("group", { name: "The run did not finish" });
    mocks.api.runJob.mockRejectedValueOnce(new CommandError("the model answered nonsense"));
    await user.click(within(banner).getByRole("button", { name: "Retry" }));
    expect(mocks.api.runJob).toHaveBeenLastCalledWith({ kind: "resumeDigest", period: "2026-W46" });
    const again = await screen.findByText("the model answered nonsense");
    expect(within(again.closest("[role=group]") as HTMLElement).queryByRole("button", { name: "Retry" })).toBeNull();
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
    await user.click(collect());
    const banner = await screen.findByRole("group", { name: "The run did not finish" });
    expect(within(banner).getByRole("button", { name: "Open Settings > AI providers" })).toBeTruthy();
    expect(within(banner).queryByRole("button", { name: "Retry" })).toBeNull();
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

    mocks.api.runJob.mockResolvedValue({
      status: "waiting",
      reportId: null,
      headline: "A source needs you before the digest can be built: see the banner below.",
      details: [],
    });
    await user.click(within(banner).getByRole("button", { name: "Continue" }));
    expect(mocks.api.runJob).toHaveBeenLastCalledWith({ kind: "resume", period: "2026-W46" });
    expect(await screen.findByText(/A source needs you before the digest can be built/)).toBeTruthy();

    await user.click(within(banner).getByRole("button", { name: "Build digest without it" }));
    expect(mocks.api.runJob).toHaveBeenLastCalledWith({ kind: "regenerate", period: "2026-W46" });
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

  it("keeps the run going while the user is on another screen", async () => {
    const user = await shown();
    const run = deferred<RunResult>();
    mocks.api.runJob.mockReturnValue(run.promise);
    await user.click(collect());
    cleanup();

    render(<TrendsScreen />);
    await screen.findByRole("heading", { name: "Weekly digest 2026-W46" });
    expect(collect().hasAttribute("disabled")).toBe(true);
    expect(screen.getByRole("status").textContent).toBe("Starting");
    await act(async () => run.resolve(DONE));
    expect(await screen.findByText("Digest ready: 8 point(s) from 6 item(s).")).toBeTruthy();
  });

  it("shows earlier digests on demand and reads the one picked", async () => {
    const user = await shown();
    expect(screen.queryByRole("region", { name: "Earlier digests" })).toBeNull();
    const toggle = screen.getByRole("button", { name: "Earlier digests" });
    expect(toggle.getAttribute("aria-pressed")).toBe("false");
    await user.click(toggle);
    expect(toggle.getAttribute("aria-pressed")).toBe("true");

    const history = within(screen.getByRole("region", { name: "Earlier digests" }));
    expect(
      history.getByRole("button", { name: "Weekly digest 2026-W46, 2026-11-14 09:12" }).getAttribute("aria-current"),
    ).toBe("true");
    await user.click(history.getByRole("button", { name: "Weekly digest 2026-W45, 2026-11-07 09:00" }));
    expect(mocks.api.overview).toHaveBeenLastCalledWith(2, "weekly");
  });

  it("tells a new user what to do first", async () => {
    mocks.api.overview.mockResolvedValue(
      overviewFixture({
        summary: "No digest yet · Week of 9 November is the current period",
        digest: null,
        chart: null,
        history: [],
        sources: [],
        searchKeys: [
          { provider: "brave", label: "Brave Search", saved: false },
          { provider: "tavily", label: "Tavily", saved: false },
        ],
      }),
    );
    const user = userEvent.setup();
    render(<TrendsScreen />);
    expect(await screen.findByText("No digest yet")).toBeTruthy();
    expect(screen.getByText(/Add sources, then press "Collect & build digest"/)).toBeTruthy();
    expect(screen.getByText("No digest yet · Week of 9 November is the current period")).toBeTruthy();
    expect(screen.getByText("No sources yet: add Hacker News, the YC companies, a feed or a page.")).toBeTruthy();
    expect(screen.getByText("Report search is off: no key saved.")).toBeTruthy();
    expect(screen.queryByRole("region", { name: "Skills in demand" })).toBeNull();

    await user.click(screen.getByRole("button", { name: "Earlier digests" }));
    expect(within(screen.getByRole("region", { name: "Earlier digests" })).getByText("No digests yet.")).toBeTruthy();
  });

  it("shows a report this version cannot read back as its saved text", async () => {
    mocks.api.overview.mockResolvedValue(
      overviewFixture({
        chart: null,
        digest: {
          reportId: 1,
          title: "Old digest",
          when: "2026-01-02 10:00",
          itemsRead: null,
          body: { kind: "plain", lines: ["# Old digest", "- a point"] },
        },
      }),
    );
    render(<TrendsScreen />);
    expect(await screen.findByText("# Old digest")).toBeTruthy();
    expect(screen.getByText("- a point")).toBeTruthy();
    expect(screen.getByText("2026-01-02 10:00")).toBeTruthy();
  });

  it("says when the saved data cannot be read", async () => {
    mocks.api.overview.mockRejectedValue(new CommandError("disk full"));
    render(<TrendsScreen />);
    expect(await screen.findByText("Could not read the saved data: disk full")).toBeTruthy();
    expect(collect().hasAttribute("disabled")).toBe(true);
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
    const { container } = render(<TrendsScreen />);
    expect((await screen.findAllByText(evil)).length).toBeGreaterThan(0);
    expect(container.querySelector("img")).toBeNull();
  });
});
