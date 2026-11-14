import { cleanup, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import * as apps from "@/contexts/applications/api/applicationsApi";
import { board, header, timeline } from "@/contexts/applications/domain/fixtures";
import type { ApplicationHeader } from "@/contexts/applications/domain/types";
import * as leads from "@/contexts/discovery/api/discoveryApi";
import { queue } from "@/contexts/discovery/domain/fixtures";
import { resetToday } from "@/contexts/discovery/hooks/todayStore";
import { useNav } from "@/shared";

import { JobsScreen } from "./JobsScreen";

vi.mock("@/contexts/applications/api/applicationsApi");
vi.mock("@/contexts/discovery/api/discoveryApi");
vi.mock("@/shared/bridge", () => ({
  invoke: vi.fn().mockResolvedValue(undefined),
  onDataChanged: () => () => undefined,
  onGraphProgress: () => () => undefined,
}));

const appsApi = vi.mocked(apps);
const leadsApi = vi.mocked(leads);

/** The one primary button (it carries the accent fill; plain buttons never do). */
const primaryButtons = () => Array.from(document.querySelectorAll("button.bg-accent"));

/** Application `id` as the drawer reads it: the board's card, at its stage. */
function headerOf(id: number): ApplicationHeader {
  const stage = board().stages.find((s) => s.cards.some((c) => c.id === id));
  const card = stage?.cards.find((c) => c.id === id);
  const base = header({ id, company: card?.subtitle ?? "", role: card?.title ?? "", stage: stage?.key ?? "cv" });
  if (stage?.key !== "inProgress") {
    return base;
  }
  return {
    ...base,
    status: { key: "interview", label: "Interview", tone: "warn" },
    primary: null,
    round: card?.round ?? null,
    marks: [
      { key: "offer", label: "Offer" },
      { key: "rejected", label: "Rejected" },
    ],
  };
}

beforeEach(() => {
  resetToday();
  useNav.setState({ screen: "jobs", stage: "leads", drawer: undefined, applicationId: undefined, needsYou: 0, droppedFile: undefined });
  leadsApi.readQueue.mockResolvedValue(queue());
  leadsApi.readProfileSkills.mockResolvedValue([]);
  leadsApi.onChanged.mockReturnValue(() => undefined);
  leadsApi.onProgress.mockReturnValue(() => undefined);
  leadsApi.onPrepared.mockReturnValue(() => undefined);
  leadsApi.onSearchFinished.mockReturnValue(() => undefined);
  appsApi.readBoard.mockResolvedValue(board());
  appsApi.readApplication.mockImplementation(async (id) => headerOf(id));
  appsApi.readTimeline.mockResolvedValue(timeline());
  appsApi.onProgress.mockReturnValue(() => undefined);
  appsApi.onApplyProgress.mockReturnValue(() => undefined);
  appsApi.setRound.mockImplementation(async (id, round) => ({ ...headerOf(id), round }));
});

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});

const stageNames = () =>
  within(screen.getByRole("tablist", { name: "Stages" }))
    .getAllByRole("tab")
    .map((t) => t.getAttribute("aria-label"));

describe("the Jobs page", () => {
  it("counts every stage in the stage bar, starting on the Leads", async () => {
    render(<JobsScreen />);
    expect(screen.getByRole("heading", { name: "Jobs", level: 1 })).toBeTruthy();
    await screen.findByRole("table", { name: "Leads" });
    await waitFor(() => expect(stageNames()).toEqual(["Leads, 3", "CVs, 2", "Applied, 1", "In progress, 1", "Done, 2"]));
    expect(screen.getByRole("tab", { name: "Leads, 3" }).getAttribute("aria-selected")).toBe("true");
  });

  it("switches to an application stage and shows its table", async () => {
    const user = userEvent.setup();
    render(<JobsScreen />);
    await screen.findByRole("table", { name: "Leads" });
    await user.click(await screen.findByRole("tab", { name: "Applied, 1" }));
    expect(useNav.getState().stage).toBe("applied");
    const table = await screen.findByRole("table", { name: "Applied" });
    expect(within(table).getByRole("row", { name: "Initech, Platform Engineer: No reply yet, Applied" })).toBeTruthy();
    expect(screen.queryByRole("table", { name: "Leads" })).toBeNull();
  });

  it("opens an application in the drawer, which holds the one primary button, and expands it", async () => {
    const user = userEvent.setup();
    useNav.setState({ stage: "cv" });
    const { container } = render(<JobsScreen />);
    await user.click(await screen.findByRole("row", { name: "Acme, Backend Engineer: Approve CV, Draft" }));
    const drawer = await screen.findByRole("dialog", { name: "Acme: Backend Engineer" });
    const action = within(drawer).getByRole("button", { name: "Approve CV" });
    expect(action.id).toBe("primary-action");
    expect(primaryButtons()).toEqual([action]);
    // The list stays visible next to the drawer on wide screens.
    expect((container.firstElementChild as HTMLElement).className).toContain("2xl:pr-[760px]");

    await user.click(within(drawer).getByRole("button", { name: "Open full view" }));
    expect([useNav.getState().screen, useNav.getState().applicationId]).toEqual(["application", 1]);
  });

  it("leaves Search now plain while an application's drawer is open on an empty Leads list", async () => {
    leadsApi.readQueue.mockResolvedValue(queue({ items: [] }));
    useNav.setState({ stage: "cv", drawer: { kind: "app", id: 1 } });
    render(<JobsScreen />);
    const drawer = await screen.findByRole("dialog", { name: "Acme: Backend Engineer" });
    await within(drawer).findByRole("button", { name: "Approve CV" });
    expect(screen.getByRole("button", { name: "Search now" }).className).not.toContain("bg-accent");
    expect(primaryButtons()).toEqual([within(drawer).getByRole("button", { name: "Approve CV" })]);
  });

  it("marks the round of an interview from the drawer", async () => {
    const user = userEvent.setup();
    useNav.setState({ stage: "inProgress" });
    render(<JobsScreen />);
    await user.click(await screen.findByRole("row", { name: /^Umbrella, Data Engineer/ }));
    const drawer = await screen.findByRole("dialog", { name: "Umbrella: Data Engineer" });
    const rounds = within(drawer).getByRole("group", { name: "Rounds" });
    expect(within(rounds).getByRole("button", { name: "Technical interview" }).getAttribute("aria-pressed")).toBe("true");
    await user.click(within(rounds).getByRole("button", { name: "System design" }));
    expect(appsApi.setRound).toHaveBeenCalledWith(4, "System design");
    expect(primaryButtons()).toEqual([]);
  });

  it("swaps the lead drawer for an application's", async () => {
    const user = userEvent.setup();
    render(<JobsScreen />);
    await user.click(await screen.findByRole("row", { name: "Acme: Backend Engineer 1, score 80" }));
    expect(await screen.findByRole("dialog", { name: "Acme: Backend Engineer 1" })).toBeTruthy();
    await user.click(screen.getByRole("tab", { name: /^CVs/ }));
    expect(useNav.getState().drawer).toBeUndefined();
    await user.click(await screen.findByRole("row", { name: "Globex, SRE: Review & submit, CV ready" }));
    expect(await screen.findByRole("dialog", { name: "Globex: SRE" })).toBeTruthy();
    await waitFor(() => expect(screen.getAllByRole("dialog")).toHaveLength(1));
  });
});
