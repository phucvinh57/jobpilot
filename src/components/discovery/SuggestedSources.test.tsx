import { act, cleanup, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { GraphProgress } from "@/api/events";
import { CommandError } from "@/lib/problem";

import * as api from "@/api/discovery";
import { jobSources, SOURCE, suggestions } from "@/test/fixtures/discovery";
import { renderWithToasts } from "@/test/render";
import type { SuggestRun } from "@/types/discovery";
import { JobSourcesSection } from "./JobSourcesSection";

vi.mock("@/api/discovery");

/** The graph progress handlers the screen subscribed to. */
const progress = new Set<(p: GraphProgress) => void>();
vi.mock("@/api/events", () => ({
  onDataChanged: () => () => {},
  onGraphProgress: (handler: (p: GraphProgress) => void) => {
    progress.add(handler);
    return () => progress.delete(handler);
  },
}));

const mocked = vi.mocked(api);

function emit(text: string) {
  act(() => {
    for (const handler of progress) {
      handler({ thread: "source_suggest", graph: "source_suggest", kind: "detail", node: "search", text });
    }
  });
}

beforeEach(() => {
  mocked.readJobSources.mockResolvedValue(jobSources({ suggestions: suggestions() }));
  mocked.suggestBoards.mockResolvedValue([]);
  mocked.dismissSuggestion.mockResolvedValue(undefined);
  mocked.cancelSuggest.mockResolvedValue(undefined);
  mocked.openSuggestBrowser.mockResolvedValue(undefined);
});

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
  progress.clear();
});

describe("Settings > Job sources: suggested sources", () => {
  it("lists the suggestions with their kind, host and how many jobs fit", async () => {
    renderWithToasts(<JobSourcesSection />);
    const block = await screen.findByRole("region", { name: "Suggested sources" });
    expect(
      within(block).getByText(
        "JobPilot searches the web in your Chrome for job boards and pages that fit your roles, and checks each one.",
      ),
    ).toBeTruthy();
    const rows = await within(block).findAllByRole("listitem");
    expect(rows.map((r) => r.getAttribute("aria-label"))).toEqual([
      "Demo Robotics: Greenhouse · boards.greenhouse.io · 3 of 14 jobs fit your roles",
      "Demo Health: Lever · jobs.lever.co · 2 of 6 jobs fit your roles",
      "example.com: Job page · example.com · 4 jobs",
    ]);
    const add = within(rows[0] as HTMLElement).getByRole("button", { name: "Add Demo Robotics" });
    expect(add.id).toBe("suggestion-add-11");
    expect(within(rows[0] as HTMLElement).getByRole("button", { name: "Dismiss Demo Robotics" }).id).toBe(
      "suggestion-dismiss-11",
    );
    expect(within(block).getByRole("button", { name: "Add all" }).id).toBe("suggestion-add-all");
    // Every button of the block is plain: the screen keeps one primary.
    for (const button of within(block).getAllByRole("button")) {
      expect(button.className).not.toContain("bg-accent");
    }
  });

  it("shows no Add all for a single suggestion", async () => {
    mocked.readJobSources.mockResolvedValue(jobSources({ suggestions: suggestions().slice(0, 1) }));
    renderWithToasts(<JobSourcesSection />);
    expect(await screen.findByRole("button", { name: "Add Demo Robotics" })).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Add all" })).toBeNull();
  });

  it("adds a suggestion and says so", async () => {
    const user = userEvent.setup();
    mocked.addSuggestion.mockResolvedValue({ ...SOURCE, id: 9, name: "Demo Robotics" });
    renderWithToasts(<JobSourcesSection />);
    await user.click(await screen.findByRole("button", { name: "Add Demo Robotics" }));
    expect(mocked.addSuggestion).toHaveBeenCalledWith(11);
    const toast = await screen.findByRole("status", { name: "Source added" });
    expect(within(toast).getByText("Demo Robotics")).toBeTruthy();
  });

  it("reports an Add that failed as a toast", async () => {
    const user = userEvent.setup();
    mocked.addSuggestion.mockRejectedValue(new CommandError("this suggestion was already added"));
    renderWithToasts(<JobSourcesSection />);
    await user.click(await screen.findByRole("button", { name: "Add Demo Health" }));
    const toast = await screen.findByRole("alert", { name: "Not added" });
    expect(within(toast).getByText("this suggestion was already added")).toBeTruthy();
  });

  it("adds all of them and counts them", async () => {
    const user = userEvent.setup();
    mocked.addAllSuggestions.mockResolvedValue({ added: 3, failed: 0, error: null });
    renderWithToasts(<JobSourcesSection />);
    await user.click(await screen.findByRole("button", { name: "Add all" }));
    expect(mocked.addAllSuggestions).toHaveBeenCalledTimes(1);
    expect(await screen.findByRole("status", { name: "3 sources added" })).toBeTruthy();
    expect(screen.queryByRole("alert")).toBeNull();
  });

  it("says which of Add all failed, next to the count of the others", async () => {
    const user = userEvent.setup();
    mocked.addAllSuggestions.mockResolvedValue({
      added: 2,
      failed: 1,
      error: new CommandError("that board address is not valid"),
    });
    renderWithToasts(<JobSourcesSection />);
    await user.click(await screen.findByRole("button", { name: "Add all" }));
    expect(await screen.findByRole("status", { name: "2 sources added" })).toBeTruthy();
    const toast = await screen.findByRole("alert", { name: "1 source was not added" });
    expect(within(toast).getByText("that board address is not valid")).toBeTruthy();
  });

  it("dismisses a suggestion quietly", async () => {
    const user = userEvent.setup();
    renderWithToasts(<JobSourcesSection />);
    await user.click(await screen.findByRole("button", { name: "Dismiss Demo Health" }));
    expect(mocked.dismissSuggestion).toHaveBeenCalledWith(12);
    await waitFor(() => expect(mocked.dismissSuggestion).toHaveBeenCalledTimes(1));
    expect(screen.queryByRole("status", { name: /./ })).toBeNull();
  });

  it("disables Suggest sources while the search runs and shows how far it is", async () => {
    const user = userEvent.setup();
    let finish: (run: SuggestRun) => void = () => {};
    mocked.suggestSources.mockReturnValue(
      new Promise((resolve) => {
        finish = resolve;
      }),
    );
    renderWithToasts(<JobSourcesSection />);
    const button = await screen.findByRole("button", { name: "Suggest sources" });
    expect(button.id).toBe("suggest-sources");
    expect(button.className).not.toContain("bg-accent");
    await user.click(button);
    expect(mocked.suggestSources).toHaveBeenCalledTimes(1);
    const running = screen.getByRole("button", { name: "Searching…" });
    expect((running as HTMLButtonElement).disabled).toBe(true);
    expect(screen.getByText("Starting the search…")).toBeTruthy();

    emit("Searching 3 of 8");
    expect(screen.getByText("Searching 3 of 8")).toBeTruthy();
    // A step of another graph does not move the line.
    act(() => {
      for (const handler of progress) {
        handler({ thread: "discovery", graph: "discovery", kind: "detail", node: "fetch", text: "3 of 9" });
      }
    });
    expect(screen.getByText("Searching 3 of 8")).toBeTruthy();
    emit("Checking 5 sources");
    expect(screen.getByText("Checking 5 sources")).toBeTruthy();

    await act(async () => finish({ suggestions: suggestions(), waiting: false }));
    const again = await screen.findByRole("button", { name: "Suggest sources" });
    expect((again as HTMLButtonElement).disabled).toBe(false);
    expect(screen.queryByText("Checking 5 sources")).toBeNull();
  });

  it("says when a search found nothing new", async () => {
    const user = userEvent.setup();
    mocked.readJobSources.mockResolvedValue(jobSources());
    mocked.suggestSources.mockResolvedValue({ suggestions: [], waiting: false });
    renderWithToasts(<JobSourcesSection />);
    expect(screen.queryByText(/No new sources found/)).toBeNull();
    await user.click(await screen.findByRole("button", { name: "Suggest sources" }));
    expect(await screen.findByText("No new sources found. Try again later or add one yourself.")).toBeTruthy();
  });

  it("reports a search that failed as a toast with a Retry", async () => {
    const user = userEvent.setup();
    mocked.suggestSources.mockRejectedValue(new CommandError("network is unreachable"));
    renderWithToasts(<JobSourcesSection />);
    await user.click(await screen.findByRole("button", { name: "Suggest sources" }));
    const toast = await screen.findByRole("alert", { name: "The search for sources failed" });
    expect(within(toast).getByText("network is unreachable")).toBeTruthy();
  });

  it("keeps Suggest sources disabled for a search started before the screen was shown", async () => {
    mocked.readJobSources.mockResolvedValue(jobSources({ suggestRunning: true }));
    renderWithToasts(<JobSourcesSection />);
    const button = (await screen.findByRole("button", { name: /Search/ })) as HTMLButtonElement;
    expect(button.id).toBe("suggest-sources");
    expect(button.disabled).toBe(true);
    expect(mocked.suggestSources).not.toHaveBeenCalled();
  });

  it("shows a paused search with Open browser, Continue and Stop", async () => {
    const user = userEvent.setup();
    mocked.readJobSources.mockResolvedValue(jobSources({ suggestWaiting: true }));
    mocked.resumeSuggest.mockResolvedValue({ suggestions: suggestions(), waiting: false });
    renderWithToasts(<JobSourcesSection />);
    const banner = await screen.findByRole("group", {
      name: "The search is paused: the search page asks you to confirm in Chrome.",
    });
    expect((screen.getByRole("button", { name: "Suggest sources" }) as HTMLButtonElement).disabled).toBe(true);

    await user.click(within(banner).getByRole("button", { name: "Open browser" }));
    expect(mocked.openSuggestBrowser).toHaveBeenCalledTimes(1);
    expect(within(banner).getByRole("button", { name: "Open browser" }).id).toBe("suggest-open-browser");
    expect(within(banner).getByRole("button", { name: "Stop" }).id).toBe("suggest-cancel");

    const resume = within(banner).getByRole("button", { name: "Continue" });
    expect(resume.id).toBe("suggest-resume");
    await user.click(resume);
    expect(mocked.resumeSuggest).toHaveBeenCalledTimes(1);
  });

  it("stops a paused search", async () => {
    const user = userEvent.setup();
    mocked.readJobSources.mockResolvedValue(jobSources({ suggestWaiting: true }));
    renderWithToasts(<JobSourcesSection />);
    await user.click(await screen.findByRole("button", { name: "Stop" }));
    expect(mocked.cancelSuggest).toHaveBeenCalledTimes(1);
  });
});
