import { act, cleanup, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { CommandError, useNav } from "@/shared";

import * as api from "../api/applicationsApi";
import { header, idleApply, jd, primary, timeline } from "../domain/fixtures";
import type { ApplicationHeader } from "../domain/types";
import { ApplicationDrawer } from "./ApplicationDrawer";
import { ApplicationScreen } from "./ApplicationScreen";

vi.mock("../api/applicationsApi");
/** Who listens for `shared:data-changed`. */
const changeListeners = vi.hoisted(() => new Set<(scope: string) => void>());
vi.mock("@/shared/bridge", () => ({
  invoke: vi.fn().mockResolvedValue(undefined),
  onDataChanged: (handler: (scope: string) => void) => {
    changeListeners.add(handler);
    return () => changeListeners.delete(handler);
  },
  onGraphProgress: () => () => undefined,
}));

/** The shell says stored applications changed. */
const applicationsChanged = () => act(() => changeListeners.forEach((handler) => handler("applications")));
vi.mock("./CvEditor", async () => {
  const { createElement } = await import("react");
  return { CvEditor: () => createElement("textarea", { "aria-label": "cv.yaml" }) };
});

const mocked = vi.mocked(api);

/** The one primary button (it carries the accent fill; plain buttons never do). */
const primaryButtons = () => Array.from(document.querySelectorAll("button.bg-accent"));

const INTERVIEW: Partial<ApplicationHeader> = {
  status: { key: "interview", label: "Interview", tone: "warn" },
  stage: "inProgress",
  primary: null,
  round: "Technical interview",
  marks: [
    { key: "offer", label: "Offer" },
    { key: "rejected", label: "Rejected" },
  ],
};

/** Show the drawer of application 7 with header `over`; resolves to the drawer. */
async function open(over: Partial<ApplicationHeader> = {}) {
  mocked.readApplication.mockResolvedValue(header(over));
  useNav.setState({ screen: "jobs", stage: over.stage ?? "cv", drawer: { kind: "app", id: 7 } });
  render(<ApplicationDrawer />);
  return screen.findByRole("dialog", { name: "Acme: Backend Engineer" });
}

beforeEach(() => {
  useNav.setState({ screen: "jobs", stage: "cv", drawer: undefined, applicationId: undefined, settingsFocus: undefined });
  Object.defineProperty(URL, "createObjectURL", { value: vi.fn(() => "blob:page"), configurable: true, writable: true });
  Object.defineProperty(URL, "revokeObjectURL", { value: vi.fn(), configurable: true, writable: true });
  mocked.onProgress.mockReturnValue(() => undefined);
  mocked.onApplyProgress.mockReturnValue(() => undefined);
  mocked.readTimeline.mockResolvedValue(timeline());
  mocked.readJd.mockResolvedValue(jd());
  mocked.readApply.mockResolvedValue(idleApply());
  mocked.approveCv.mockResolvedValue(undefined);
  mocked.prepareApply.mockResolvedValue(undefined);
  mocked.setRound.mockImplementation(async (_id, round) => header({ ...INTERVIEW, round }));
  mocked.mark.mockResolvedValue(header());
});

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
  changeListeners.clear();
});

describe("the application drawer", () => {
  it("is closed unless navigation shows an application", () => {
    render(<ApplicationDrawer />);
    expect(screen.queryByRole("dialog")).toBeNull();
    act(() => useNav.setState({ drawer: { kind: "lead", id: 1 } }));
    expect(screen.queryByRole("dialog")).toBeNull();
  });

  it("shows the status, the steps, the timeline and the one primary action", async () => {
    const user = userEvent.setup();
    const drawer = await open();
    expect(within(drawer).getByText("Draft")).toBeTruthy();
    expect(within(drawer).getByText("Acme · Berlin")).toBeTruthy();
    expect(within(drawer).getByRole("listitem", { name: "Step 2: CV review" }).getAttribute("aria-current")).toBe("step");
    expect(await within(drawer).findByText("moved to Applied")).toBeTruthy();
    const approve = within(drawer).getByRole("button", { name: "Approve CV" });
    expect(approve.id).toBe("primary-action");
    expect(primaryButtons()).toEqual([approve]);
    // Nothing to mark before the application was sent, and no round picker.
    expect(within(drawer).queryByRole("region", { name: "Interview round" })).toBeNull();
    expect(within(drawer).queryByRole("region", { name: "The company answered" })).toBeNull();

    await user.click(approve);
    expect(mocked.approveCv).toHaveBeenCalledWith(7);
  });

  it("opens the full view, and closes", async () => {
    const user = userEvent.setup();
    const drawer = await open();
    await user.click(within(drawer).getByRole("button", { name: "Open full view" }));
    expect([useNav.getState().screen, useNav.getState().applicationId]).toEqual(["application", 7]);

    act(() => useNav.setState({ screen: "jobs" }));
    await user.click(within(screen.getByRole("dialog")).getByRole("button", { name: "Close" }));
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    expect(useNav.getState().drawer).toBeUndefined();
  });

  it("starts Review & Submit on the full view's Apply tab, where the review shows", async () => {
    const user = userEvent.setup();
    const drawer = await open({ primary: primary({ action: "reviewAndSubmit", label: "Review & Submit" }) });
    await user.click(within(drawer).getByRole("button", { name: "Review & Submit" }));
    expect([useNav.getState().screen, useNav.getState().applicationId]).toEqual(["application", 7]);
    expect(mocked.prepareApply).not.toHaveBeenCalled();
    cleanup();

    render(<ApplicationScreen />);
    expect(await screen.findByRole("heading", { name: "Backend Engineer", level: 1 })).toBeTruthy();
    expect(screen.getByRole("tab", { name: "Apply" }).getAttribute("aria-selected")).toBe("true");
    await waitFor(() => expect(mocked.prepareApply).toHaveBeenCalledTimes(1));
  });

  it("marks the interview round from the usual ones, or another one", async () => {
    const user = userEvent.setup();
    const drawer = await open(INTERVIEW);
    expect(primaryButtons()).toEqual([]);
    const picker = within(drawer).getByRole("region", { name: "Interview round" });
    const chips = within(picker).getAllByRole("button", { pressed: false }).concat(within(picker).getAllByRole("button", { pressed: true }));
    expect(chips.map((c) => c.textContent).sort()).toEqual(["Final interview", "HR screen", "System design", "Technical interview"]);
    expect(within(picker).getByRole("button", { name: "Technical interview" }).getAttribute("aria-pressed")).toBe("true");

    await user.click(within(picker).getByRole("button", { name: "System design" }));
    expect(mocked.setRound).toHaveBeenCalledWith(7, "System design");

    const other = within(picker).getByRole("textbox", { name: "Other round" });
    expect(other.id).toBe("round-other");
    expect((within(picker).getByRole("button", { name: "Save" }) as HTMLButtonElement).disabled).toBe(true);
    await user.type(other, "Onsite with the team{Enter}");
    expect(mocked.setRound).toHaveBeenCalledWith(7, "Onsite with the team");
    await waitFor(() => expect((other as HTMLInputElement).value).toBe(""));

    await user.click(within(picker).getByRole("button", { name: "Clear" }));
    expect(mocked.setRound).toHaveBeenLastCalledWith(7, null);
  });

  it("shows a round the user typed as a selected chip", async () => {
    const drawer = await open({ ...INTERVIEW, round: "Pairing session" });
    expect(within(drawer).getByRole("button", { name: "Pairing session" }).getAttribute("aria-pressed")).toBe("true");
  });

  it("marks what the company answered once confirmed, and says when that failed", async () => {
    const user = userEvent.setup();
    mocked.mark.mockRejectedValueOnce(new CommandError("an offer cannot go back to an interview"));
    const drawer = await open(INTERVIEW);
    const marks = within(drawer).getByRole("region", { name: "The company answered" });
    const rejected = within(marks).getByRole("button", { name: "Rejected" });
    expect(rejected.id).toBe("mark-rejected");
    expect(rejected.className).toContain("text-danger");
    await user.click(within(marks).getByRole("button", { name: "Offer" }));
    expect(mocked.mark).not.toHaveBeenCalled();
    await user.click(within(marks).getByRole("button", { name: "Mark as Offer" }));
    expect(mocked.mark).toHaveBeenCalledWith(7, "offer");
    expect(await within(marks).findByText("an offer cannot go back to an interview")).toBeTruthy();
    await user.click(within(marks).getByRole("button", { name: "Rejected" }));
    await user.click(within(marks).getByRole("button", { name: "Mark as Rejected" }));
    expect(mocked.mark).toHaveBeenLastCalledWith(7, "rejected");
  });

  it("asks before a mark, since a status never moves back", async () => {
    const user = userEvent.setup();
    const drawer = await open(INTERVIEW);
    const marks = within(drawer).getByRole("region", { name: "The company answered" });

    await user.click(within(marks).getByRole("button", { name: "Offer" }));
    const ask = within(marks).getByRole("group", { name: "Mark as Offer" });
    expect(within(ask).getByText("Mark as Offer? This can't be undone.")).toBeTruthy();
    const confirm = within(ask).getByRole("button", { name: "Mark as Offer" });
    expect(confirm.id).toBe("mark-confirm");
    expect(confirm.className, "a plain button").not.toContain("text-danger");
    expect(primaryButtons(), "never a second primary").toEqual([]);
    expect(within(marks).queryByRole("button", { name: "Rejected" }), "one question at a time").toBeNull();

    // Cancel: nothing is marked, the marks are back.
    await user.click(within(ask).getByRole("button", { name: "Cancel" }));
    expect(within(marks).queryByRole("group", { name: "Mark as Offer" })).toBeNull();
    expect(mocked.mark).not.toHaveBeenCalled();

    // Rejected asks with a red confirm.
    await user.click(within(marks).getByRole("button", { name: "Rejected" }));
    const reject = within(marks).getByRole("button", { name: "Mark as Rejected" });
    expect(reject.id).toBe("mark-confirm");
    expect(reject.className).toContain("text-danger");
    expect(primaryButtons()).toEqual([]);
    expect(document.getElementById("mark-cancel")).toBeTruthy();
    await user.click(reject);
    expect(mocked.mark).toHaveBeenCalledWith(7, "rejected");
    await waitFor(() => expect(within(marks).queryByRole("group", { name: "Mark as Rejected" })).toBeNull());
  });

  it("keeps a failed mark to the application it was made on", async () => {
    const user = userEvent.setup();
    mocked.mark.mockRejectedValueOnce(new CommandError("the database is locked"));
    const drawer = await open(INTERVIEW);
    await user.click(within(drawer).getByRole("button", { name: "Offer" }));
    await user.click(within(drawer).getByRole("button", { name: "Mark as Offer" }));
    expect(await within(drawer).findByText("the database is locked")).toBeTruthy();

    mocked.readApplication.mockImplementation(async (id) => header({ ...INTERVIEW, id, company: "Globex" }));
    act(() => useNav.setState({ drawer: { kind: "app", id: 8 } }));
    const other = await screen.findByRole("dialog", { name: "Globex: Backend Engineer" });
    expect(await within(other).findByRole("region", { name: "The company answered" })).toBeTruthy();
    expect(within(other).queryByText("the database is locked")).toBeNull();
  });

  it("shows the stage the application is in, without closing", async () => {
    // Opened from elsewhere (the Leads) without a stage: the page moves to the application's.
    mocked.readApplication.mockResolvedValue(header({ stage: "applied" }));
    useNav.setState({ screen: "jobs", stage: "leads", drawer: { kind: "app", id: 7 } });
    render(<ApplicationDrawer />);
    await screen.findByRole("dialog", { name: "Acme: Backend Engineer" });
    await waitFor(() => expect(useNav.getState().stage).toBe("applied"));
    expect(useNav.getState().drawer).toEqual({ kind: "app", id: 7 });
  });

  it("follows the application when a mark moves it to another stage", async () => {
    const user = userEvent.setup();
    const applied: Partial<ApplicationHeader> = {
      status: { key: "applied", label: "Applied", tone: "accent" },
      stage: "applied",
      primary: null,
      marks: [
        { key: "interview", label: "Got an interview" },
        { key: "offer", label: "Offer" },
        { key: "rejected", label: "Rejected" },
      ],
    };
    const drawer = await open(applied);
    expect(useNav.getState().stage).toBe("applied");
    mocked.mark.mockResolvedValue(header(INTERVIEW));
    await user.click(within(drawer).getByRole("button", { name: "Got an interview" }));
    await user.click(within(drawer).getByRole("button", { name: "Mark as Got an interview" }));
    expect(mocked.mark).toHaveBeenCalledWith(7, "interview");

    // The shell says the application changed: the drawer reads it again, and the page follows.
    mocked.readApplication.mockResolvedValue(header(INTERVIEW));
    applicationsChanged();
    await waitFor(() => expect(useNav.getState().stage).toBe("inProgress"));
    expect(useNav.getState().drawer).toEqual({ kind: "app", id: 7 });
  });

  it("reads the job description only when it is unfolded", async () => {
    const user = userEvent.setup();
    const drawer = await open();
    expect(mocked.readJd).not.toHaveBeenCalled();
    await user.click(within(drawer).getByText("Job description", { selector: "summary" }));
    expect(await within(drawer).findByText(/Remote friendly/)).toBeTruthy();
    expect(mocked.readJd).toHaveBeenCalledWith(7);
  });
});
