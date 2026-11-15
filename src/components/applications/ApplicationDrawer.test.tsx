import { act, cleanup, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { CommandError } from "@/lib/problem";
import { useNav } from "@/stores/nav";
import { useToasts } from "@/stores/toast";
import { renderWithToasts } from "@/test/render";

import * as api from "@/api/applications";
import { header, idleApply, jd, primary, timeline } from "@/test/fixtures/applications";
import type { ApplicationHeader } from "@/types/applications";
import { ApplicationDrawer } from "./ApplicationDrawer";
import { ApplicationPage } from "@/pages/ApplicationPage";

vi.mock("@/api/applications");
/** Who listens for `shared:data-changed`. */
const changeListeners = vi.hoisted(() => new Set<(scope: string) => void>());
vi.mock("@/api/bridge", () => ({
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
  renderWithToasts(<ApplicationDrawer />);
  return screen.findByRole("dialog", { name: "Acme: Backend Engineer" });
}

beforeEach(() => {
  useNav.setState({
    screen: "jobs",
    stage: "cv",
    drawer: undefined,
    applicationId: undefined,
    settingsFocus: undefined,
  });
  Object.defineProperty(URL, "createObjectURL", {
    value: vi.fn(() => "blob:page"),
    configurable: true,
    writable: true,
  });
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
    expect(within(drawer).getByRole("listitem", { name: "Step 2: CV review" }).getAttribute("aria-current")).toBe(
      "step",
    );
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

    render(<ApplicationPage />);
    expect(await screen.findByRole("heading", { name: "Backend Engineer", level: 1 })).toBeTruthy();
    expect(screen.getByRole("tab", { name: "Apply" }).getAttribute("aria-selected")).toBe("true");
    await waitFor(() => expect(mocked.prepareApply).toHaveBeenCalledTimes(1));
  });

  it("marks the interview round from the usual ones, or another one", async () => {
    const user = userEvent.setup();
    const drawer = await open(INTERVIEW);
    expect(primaryButtons()).toEqual([]);
    const picker = within(drawer).getByRole("region", { name: "Interview round" });
    const chips = within(picker)
      .getAllByRole("button", { pressed: false })
      .concat(within(picker).getAllByRole("button", { pressed: true }));
    expect(chips.map((c) => c.textContent).sort()).toEqual([
      "Final interview",
      "HR screen",
      "System design",
      "Technical interview",
    ]);
    expect(within(picker).getByRole("button", { name: "Technical interview" }).getAttribute("aria-pressed")).toBe(
      "true",
    );

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

  it("reports a round that could not be saved in a toast, and keeps what was typed", async () => {
    const user = userEvent.setup();
    mocked.setRound.mockRejectedValueOnce(new CommandError("the database is locked"));
    const drawer = await open(INTERVIEW);
    const picker = within(drawer).getByRole("region", { name: "Interview round" });
    await user.type(within(picker).getByRole("textbox", { name: "Other round" }), "Onsite{Enter}");
    const toast = await screen.findByRole("alert", { name: "The round could not be saved" });
    expect(within(toast).getByText("the database is locked")).toBeTruthy();
    expect(within(picker).queryByText("the database is locked"), "no banner in the drawer").toBeNull();
    expect((within(picker).getByRole("textbox", { name: "Other round" }) as HTMLInputElement).value).toBe("Onsite");
  });

  it("reports the failure of the last job in a toast with Retry, once", async () => {
    const user = userEvent.setup();
    mocked.retry.mockResolvedValue(undefined);
    mocked.dismissError.mockResolvedValue(undefined);
    await open({
      error: new CommandError("connection refused", {
        kind: "network",
        challenge: null,
        advice: "Check your network and try again.",
        settingsSection: null,
        retryable: true,
      }),
    });
    const toast = await screen.findByRole("alert", { name: "The last job failed" });
    expect(within(toast).getByText("connection refused")).toBeTruthy();
    expect(document.getElementById("job-error")).toBeNull();
    expect(mocked.dismissError, "kept until the toast is closed").not.toHaveBeenCalled();
    await user.click(within(toast).getByRole("button", { name: "Dismiss" }));
    expect(mocked.dismissError).toHaveBeenCalledWith(7);
  });

  it("shows a round the user typed as a selected chip", async () => {
    const drawer = await open({ ...INTERVIEW, round: "Pairing session" });
    expect(within(drawer).getByRole("button", { name: "Pairing session" }).getAttribute("aria-pressed")).toBe("true");
  });

  it("marks what the company answered once confirmed, and reports a failure in a toast", async () => {
    const user = userEvent.setup();
    mocked.mark.mockRejectedValueOnce(new CommandError("an offer cannot go back to an interview"));
    const drawer = await open(INTERVIEW);
    const marks = within(drawer).getByRole("region", { name: "The company answered" });
    const rejected = within(marks).getByRole("button", { name: "Rejected" });
    expect(rejected.id).toBe("mark-rejected");
    expect(rejected.className).toContain("text-danger");
    await user.click(within(marks).getByRole("button", { name: "Offer" }));
    expect(mocked.mark).not.toHaveBeenCalled();
    await user.click(within(marks).getByRole("button", { name: "Confirm" }));
    expect(mocked.mark).toHaveBeenCalledWith(7, "offer");
    const toast = await screen.findByRole("alert", { name: "The answer could not be saved" });
    expect(within(toast).getByText("an offer cannot go back to an interview")).toBeTruthy();
    expect(within(marks).queryByText("an offer cannot go back to an interview"), "no banner in the drawer").toBeNull();
    await user.click(within(marks).getByRole("button", { name: "Rejected" }));
    await user.click(within(marks).getByRole("button", { name: "Confirm" }));
    expect(mocked.mark).toHaveBeenLastCalledWith(7, "rejected");
  });

  it("asks before a mark, since a status never moves back", async () => {
    const user = userEvent.setup();
    const drawer = await open(INTERVIEW);
    const marks = within(drawer).getByRole("region", { name: "The company answered" });

    await user.click(within(marks).getByRole("button", { name: "Offer" }));
    const ask = within(marks).getByRole("group", { name: "Confirm: Offer" });
    expect(within(ask).getByText("Record “Offer”? This can't be undone.")).toBeTruthy();
    const confirm = within(ask).getByRole("button", { name: "Confirm" });
    expect(confirm.id).toBe("mark-confirm");
    expect(confirm.className, "a plain button").not.toContain("text-danger");
    expect(primaryButtons(), "never a second primary").toEqual([]);
    expect(within(marks).queryByRole("button", { name: "Rejected" }), "one question at a time").toBeNull();

    // Cancel: nothing is marked, the marks are back.
    await user.click(within(ask).getByRole("button", { name: "Cancel" }));
    expect(within(marks).queryByRole("group", { name: "Confirm: Offer" })).toBeNull();
    expect(mocked.mark).not.toHaveBeenCalled();

    // Rejected asks with a red confirm.
    await user.click(within(marks).getByRole("button", { name: "Rejected" }));
    const reject = within(marks).getByRole("button", { name: "Confirm" });
    expect(reject.id).toBe("mark-confirm");
    expect(reject.className).toContain("text-danger");
    expect(primaryButtons()).toEqual([]);
    expect(document.getElementById("mark-cancel")).toBeTruthy();
    await user.click(reject);
    expect(mocked.mark).toHaveBeenCalledWith(7, "rejected");
    await waitFor(() => expect(within(marks).queryByRole("group", { name: "Confirm: Rejected" })).toBeNull());
  });

  it("keeps a failed mark to the application it was made on", async () => {
    const user = userEvent.setup();
    mocked.mark.mockRejectedValueOnce(new CommandError("the database is locked"));
    const drawer = await open(INTERVIEW);
    await user.click(within(drawer).getByRole("button", { name: "Offer" }));
    await user.click(within(drawer).getByRole("button", { name: "Confirm" }));
    expect(await screen.findByRole("alert", { name: "The answer could not be saved" })).toBeTruthy();
    act(() => useToasts.getState().clear());

    mocked.readApplication.mockImplementation(async (id) => header({ ...INTERVIEW, id, company: "Globex" }));
    act(() => useNav.setState({ drawer: { kind: "app", id: 8 } }));
    const other = await screen.findByRole("dialog", { name: "Globex: Backend Engineer" });
    expect(await within(other).findByRole("region", { name: "The company answered" })).toBeTruthy();
    expect(screen.queryByRole("alert", { name: "The answer could not be saved" }), "not reported again").toBeNull();
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
    await user.click(within(drawer).getByRole("button", { name: "Confirm" }));
    expect(mocked.mark).toHaveBeenCalledWith(7, "interview");

    // The shell says the application changed: the drawer reads it again, and the page follows.
    mocked.readApplication.mockResolvedValue(header(INTERVIEW));
    applicationsChanged();
    await waitFor(() => expect(useNav.getState().stage).toBe("inProgress"));
    expect(useNav.getState().drawer).toEqual({ kind: "app", id: 7 });
  });

  it("shows the job description in a dialog, read only once opened", async () => {
    const user = userEvent.setup();
    const drawer = await open();
    expect(mocked.readJd).not.toHaveBeenCalled();
    const button = within(drawer).getByRole("button", { name: "Job description" });
    expect(button.id).toBe("drawer-jd");
    expect(button.getAttribute("aria-haspopup")).toBe("dialog");
    expect(button.className, "a plain button").not.toContain("bg-accent");
    const primaries = primaryButtons().length;
    await user.click(button);
    const dialog = await screen.findByRole("dialog", { name: "Job description" });
    expect(await within(dialog).findByText(/Remote friendly/)).toBeTruthy();
    expect(mocked.readJd).toHaveBeenCalledWith(7);
    expect(primaryButtons(), "never a second primary").toHaveLength(primaries);

    // Closing the dialog leaves the drawer open.
    const done = within(dialog)
      .getAllByRole("button", { name: "Close" })
      .find((b) => b.id === "drawer-jd-done");
    await user.click(done ?? dialog);
    await waitFor(() => expect(screen.queryByRole("dialog", { name: "Job description" })).toBeNull());
    expect(screen.getByRole("dialog", { name: "Acme: Backend Engineer" })).toBeTruthy();
  });

  it("closes the job description with Escape, and the drawer stays", async () => {
    const user = userEvent.setup();
    const drawer = await open();
    await user.click(within(drawer).getByRole("button", { name: "Job description" }));
    await screen.findByRole("dialog", { name: "Job description" });
    await user.keyboard("{Escape}");
    await waitFor(() => expect(screen.queryByRole("dialog", { name: "Job description" })).toBeNull());
    expect(screen.getByRole("dialog", { name: "Acme: Backend Engineer" })).toBeTruthy();
    expect(useNav.getState().drawer).toEqual({ kind: "app", id: 7 });
  });
});

describe("dismissing a job in CVs", () => {
  const DISMISSED: Partial<ApplicationHeader> = {
    status: { key: "dismissed", label: "Dismissed", tone: "neutral" },
    primary: null,
    dismissible: false,
    dismissed: { reason: "salary too low", at: new Date(Date.now() - 3 * 3_600_000).toISOString() },
  };

  it("offers a plain Dismiss beside the primary, and dismisses with a reason and the company", async () => {
    const user = userEvent.setup();
    mocked.dismissApplication.mockResolvedValue({ also: 0, kept: 0, notExcluded: null });
    const drawer = await open();
    const dismiss = within(drawer).getByRole("button", { name: "Dismiss" });
    expect(dismiss.classList.contains("bg-accent")).toBe(false);
    expect(primaryButtons()).toHaveLength(1);
    await user.click(dismiss);
    const form = await screen.findByRole("dialog", { name: "Dismiss this job" });
    expect(within(form).getByText("Acme — Backend Engineer")).toBeTruthy();
    await user.type(within(form).getByRole("textbox", { name: "Why dismiss this job" }), "salary too low");
    await user.click(within(form).getByRole("checkbox", { name: "Don't show Acme again" }));
    await user.click(within(form).getByRole("button", { name: "Dismiss job" }));
    expect(mocked.dismissApplication).toHaveBeenCalledWith(7, "salary too low", true);
    expect(await screen.findByRole("status", { name: "Job dismissed" })).toBeTruthy();
    // The job left CVs: the dialog and the drawer close.
    await waitFor(() => expect(useNav.getState().drawer).toBeUndefined());
    expect(screen.queryByRole("dialog", { name: "Dismiss this job" })).toBeNull();
  });

  it("cancels the dialog, and says in it why a dismissal failed", async () => {
    const user = userEvent.setup();
    mocked.dismissApplication.mockRejectedValue(new CommandError("its application form is open: close it first"));
    const drawer = await open();
    await user.click(within(drawer).getByRole("button", { name: "Dismiss" }));
    const form = await screen.findByRole("dialog", { name: "Dismiss this job" });
    await user.click(within(form).getByRole("button", { name: "Dismiss job" }));
    expect((await within(form).findByRole("alert")).textContent).toContain(
      "Not dismissed: its application form is open",
    );
    expect(useNav.getState().drawer).toEqual({ kind: "app", id: 7 });
    await user.click(within(form).getByRole("button", { name: "Cancel" }));
    await waitFor(() => expect(screen.queryByRole("dialog", { name: "Dismiss this job" })).toBeNull());
    // Opened again, the dialog starts afresh.
    await user.click(within(drawer).getByRole("button", { name: "Dismiss" }));
    const again = await screen.findByRole("dialog", { name: "Dismiss this job" });
    expect(within(again).queryByRole("alert")).toBeNull();
  });

  it("says in a toast when the company could not be excluded, and when a restored job's still is", async () => {
    const user = userEvent.setup();
    mocked.dismissApplication.mockResolvedValue({ also: 0, kept: 0, notExcluded: "disk full" });
    const drawer = await open();
    await user.click(within(drawer).getByRole("button", { name: "Dismiss" }));
    const form = await screen.findByRole("dialog", { name: "Dismiss this job" });
    await user.click(within(form).getByRole("button", { name: "Dismiss job" }));
    const failed = await screen.findByRole("alert", { name: "Job dismissed, but the company could not be excluded" });
    expect(within(failed).getByText("disk full")).toBeTruthy();

    // Opened again from Leads > Dismissed: Restore says the company is still excluded.
    mocked.restoreApplication.mockResolvedValue({ hidden: null, excluded: true });
    mocked.readApplication.mockResolvedValue(header(DISMISSED));
    act(() => useNav.setState({ drawer: { kind: "app", id: 7 } }));
    applicationsChanged();
    const reopened = await screen.findByRole("dialog", { name: "Acme: Backend Engineer" });
    const notice = await within(reopened).findByRole("group", { name: "You dismissed this job" });
    mocked.readApplication.mockResolvedValue(header());
    await user.click(within(notice).getByRole("button", { name: "Restore" }));
    const restored = await screen.findByRole("status", { name: "Job restored" });
    expect(within(restored).getByText(/Its company is still excluded/)).toBeTruthy();
  });

  it("offers no Dismiss when the job is not dismissible", async () => {
    const drawer = await open({ dismissible: false });
    expect(within(drawer).queryByRole("button", { name: "Dismiss" })).toBeNull();
  });

  it("shows a dismissed job with its reason and a plain Restore, and no primary", async () => {
    const user = userEvent.setup();
    useNav.setState({ stage: "leads" });
    mocked.restoreApplication.mockResolvedValue({ hidden: null, excluded: false });
    mocked.readApplication.mockResolvedValue(header(DISMISSED));
    useNav.setState({ screen: "jobs", stage: "leads", drawer: { kind: "app", id: 7 } });
    render(<ApplicationDrawer />);
    const drawer = await screen.findByRole("dialog", { name: "Acme: Backend Engineer" });
    const notice = await within(drawer).findByRole("group", { name: "You dismissed this job" });
    expect(within(notice).getByText("Dismissed 3 h ago: salary too low")).toBeTruthy();
    expect(primaryButtons()).toHaveLength(0);
    expect(within(drawer).queryByRole("button", { name: "Dismiss" })).toBeNull();
    // A dismissed job is on no stage: the page stays on Leads.
    expect(useNav.getState().stage).toBe("leads");
    await user.click(within(notice).getByRole("button", { name: "Restore" }));
    expect(mocked.restoreApplication).toHaveBeenCalledWith(7);
  });
});
