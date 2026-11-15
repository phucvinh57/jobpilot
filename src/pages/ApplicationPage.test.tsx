import { act, cleanup, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { CommandError } from "@/lib/problem";
import { useNav } from "@/stores/nav";
import { useShell } from "@/stores/shell";
import { renderWithToasts } from "@/test/render";

import * as api from "@/api/applications";
import { cv, header, idleApply, jd, primary, timeline } from "@/test/fixtures/applications";
import type { ApplicationHeader, JobProgress } from "@/types/applications";
import { ApplicationPage } from "./ApplicationPage";

const bridge = vi.hoisted(() => ({ invoke: vi.fn() }));

vi.mock("@/api/applications");
vi.mock("@/api/bridge", () => ({
  invoke: bridge.invoke,
  onDataChanged: () => () => undefined,
  onGraphProgress: () => () => undefined,
}));
vi.mock("@/components/applications/CvEditor", async () => {
  const { createElement } = await import("react");
  return {
    CvEditor: ({ doc, readOnly }: { doc?: { text: string }; readOnly: boolean }) =>
      createElement("textarea", { "aria-label": "cv.yaml", readOnly, defaultValue: doc?.text ?? "" }),
  };
});

const mocked = vi.mocked(api);
let progress: ((p: JobProgress) => void) | undefined;

function open(over: Partial<ApplicationHeader> = {}) {
  mocked.readApplication.mockResolvedValue(header(over));
  renderWithToasts(<ApplicationPage />);
}

/** The one primary button (it carries the accent fill; plain buttons never do). */
const primaryButtons = () => Array.from(document.querySelectorAll("button.bg-accent"));

beforeEach(() => {
  bridge.invoke.mockReset();
  bridge.invoke.mockResolvedValue(undefined);
  useNav.setState({
    screen: "application",
    applicationId: 7,
    stage: "cv",
    drawer: { kind: "app", id: 7 },
    settingsFocus: undefined,
  });
  useShell.setState({ status: undefined });
  Object.defineProperty(URL, "createObjectURL", {
    value: vi.fn(() => "blob:page"),
    configurable: true,
    writable: true,
  });
  Object.defineProperty(URL, "revokeObjectURL", { value: vi.fn(), configurable: true, writable: true });
  progress = undefined;
  mocked.onProgress.mockImplementation((handler) => {
    progress = handler;
    return () => undefined;
  });
  mocked.onApplyProgress.mockReturnValue(() => undefined);
  mocked.readApply.mockResolvedValue(idleApply());
  mocked.readJd.mockResolvedValue(jd());
  mocked.readCv.mockResolvedValue(cv());
  mocked.readCvPage.mockResolvedValue(new Blob(["png"], { type: "image/png" }));
  mocked.readTimeline.mockResolvedValue(timeline());
  mocked.generate.mockResolvedValue(undefined);
  mocked.approveCv.mockResolvedValue(undefined);
  mocked.prepareApply.mockResolvedValue(undefined);
  mocked.confirmApply.mockResolvedValue(undefined);
  mocked.retry.mockResolvedValue(undefined);
  mocked.dismissError.mockResolvedValue(undefined);
  mocked.syncApply.mockResolvedValue(null);
});

afterEach(() => {
  cleanup();
});

describe("the Application screen header", () => {
  it("shows the breadcrumb, role, company, status, steps and tabs, with exactly one primary button", async () => {
    open();
    expect(await screen.findByRole("heading", { name: "Backend Engineer", level: 1 })).toBeTruthy();
    expect(screen.getByRole("button", { name: "Back to Jobs, CVs" }).textContent).toBe("Jobs › CVs");
    expect(screen.getByText("Acme · Berlin")).toBeTruthy();
    expect(screen.getByText("Draft")).toBeTruthy();
    expect(screen.getByRole("list", { name: "Steps" })).toBeTruthy();
    expect(screen.getByRole("listitem", { name: "Step 2: CV review" }).getAttribute("aria-current")).toBe("step");
    expect(screen.getAllByRole("tab").map((t) => t.textContent)).toEqual(["JD", "CV", "Apply", "Timeline"]);
    expect(screen.getByRole("tab", { name: "CV" }).getAttribute("aria-selected")).toBe("true");

    const approve = screen.getByRole("button", { name: "Approve CV" });
    expect(primaryButtons()).toEqual([approve]);
    expect(approve.hasAttribute("disabled")).toBe(false);
  });

  it("leads back to the Jobs page as it was left: its stage and its drawer", async () => {
    const user = userEvent.setup();
    useNav.setState({ stage: "inProgress", drawer: { kind: "app", id: 7 } });
    open();
    await user.click(await screen.findByRole("button", { name: "Back to Jobs, In progress" }));
    const nav = useNav.getState();
    expect([nav.screen, nav.stage, nav.drawer]).toEqual(["jobs", "inProgress", { kind: "app", id: 7 }]);
  });

  it("disables the primary button and says why", async () => {
    open({
      primary: primary({ action: "submit", label: "Submit", enabled: false, hint: "2 required field(s) still empty" }),
    });
    const button = await screen.findByRole("button", { name: "Submit" });
    expect(button.hasAttribute("disabled")).toBe(true);
    expect(screen.getByText("2 required field(s) still empty")).toBeTruthy();
  });

  it("points to the Settings section that fixes what blocks the button", async () => {
    const user = userEvent.setup();
    open({
      primary: primary({
        action: "generate",
        label: "Generate",
        enabled: false,
        hint: "No AI provider",
        settings: "AI providers",
      }),
    });
    await user.click(await screen.findByRole("button", { name: "Open Settings > AI providers" }));
    expect(useNav.getState().screen).toBe("settings");
    expect(useNav.getState().settingsFocus).toBe("AI providers");
  });

  it("has no primary button when the application was sent, and offers Next job instead", async () => {
    const user = userEvent.setup();
    open({ primary: null, showNextJob: true, status: { key: "applied", label: "Applied", tone: "ok" }, stepIndex: 4 });
    await screen.findByRole("heading", { name: "Backend Engineer" });
    expect(primaryButtons()).toEqual([]);
    await user.click(screen.getByRole("button", { name: "Next job" }));
    const nav = useNav.getState();
    expect([nav.screen, nav.stage, nav.drawer]).toEqual(["jobs", "leads", { kind: "lead", id: "best" }]);
  });

  it("shows the progress line of a running job, as it moves", async () => {
    open({
      job: "Writing the CV...",
      primary: primary({ action: "generate", label: "Generate", enabled: false, loading: true }),
    });
    expect(await screen.findByText("Writing the CV...")).toBeTruthy();
    act(() => progress?.({ appId: 7, label: "Rendering..." }));
    expect(screen.getByText("Rendering...")).toBeTruthy();
    act(() => progress?.({ appId: 8, label: "Someone else" }));
    expect(screen.queryByText("Someone else")).toBeNull();
  });
});

describe("the primary action", () => {
  it("generates a CV", async () => {
    const user = userEvent.setup();
    open({ hasCv: false, primary: primary({ action: "generate", label: "Generate" }) });
    await user.click(await screen.findByRole("button", { name: "Generate" }));
    expect(mocked.generate).toHaveBeenCalledWith(7);
  });

  it("approves the CV where the user is", async () => {
    const user = userEvent.setup();
    open();
    await user.click(await screen.findByRole("button", { name: "Approve CV" }));
    expect(mocked.approveCv).toHaveBeenCalledWith(7);
    expect(screen.getByRole("tab", { name: "CV" }).getAttribute("aria-selected")).toBe("true");
  });

  it("opens the Apply tab and starts the form for Review & Submit", async () => {
    const user = userEvent.setup();
    open({ primary: primary({ action: "reviewAndSubmit", label: "Review & Submit" }) });
    await user.click(await screen.findByRole("button", { name: "Review & Submit" }));
    expect(screen.getByRole("tab", { name: "Apply" }).getAttribute("aria-selected")).toBe("true");
    expect(mocked.prepareApply).toHaveBeenCalledWith(7);
  });

  it("only brings the open review into view when Submit is pressed from another tab", async () => {
    const user = userEvent.setup();
    open({ primary: primary({ action: "submit", label: "Submit" }) });
    await user.click(await screen.findByRole("button", { name: "Submit" }));
    expect(screen.getByRole("tab", { name: "Apply" }).getAttribute("aria-selected")).toBe("true");
    expect(mocked.confirmApply).not.toHaveBeenCalled();
    expect(mocked.prepareApply).not.toHaveBeenCalled();
  });

  it("asks the shell to check the setup again when a prerequisite is missing", async () => {
    const user = userEvent.setup();
    mocked.generate.mockRejectedValue(
      new CommandError("no AI provider", {
        kind: "noProvider",
        challenge: null,
        advice: "Add a provider.",
        settingsSection: "AI providers",
        retryable: false,
      }),
    );
    open({ hasCv: false, primary: primary({ action: "generate", label: "Generate" }) });
    await user.click(await screen.findByRole("button", { name: "Generate" }));
    await waitFor(() => expect(bridge.invoke).toHaveBeenCalledWith("shared_status"));
  });
});

describe("dismissing the job", () => {
  it("offers a plain Dismiss while the job is in CVs, and says where it went", async () => {
    const user = userEvent.setup();
    mocked.dismissApplication.mockResolvedValue({ also: 0, kept: 0, notExcluded: null });
    open();
    await screen.findByRole("heading", { name: "Backend Engineer", level: 1 });
    expect(primaryButtons()).toHaveLength(1);
    await user.click(document.getElementById("app-dismiss-open") as HTMLElement);
    const form = await screen.findByRole("dialog", { name: "Dismiss this job" });
    await user.click(within(form).getByRole("button", { name: "Dismiss job" }));
    expect(mocked.dismissApplication).toHaveBeenCalledWith(7, "", false);
    const done = await screen.findByRole("status", { name: "Job dismissed" });
    expect(within(done).getByText("Find it under Leads > Dismissed.")).toBeTruthy();
  });

  it("shows a dismissed job with a plain Restore and no primary", async () => {
    const user = userEvent.setup();
    mocked.restoreApplication.mockResolvedValue({ hidden: null, excluded: true });
    open({
      status: { key: "dismissed", label: "Dismissed", tone: "neutral" },
      primary: null,
      dismissible: false,
      dismissed: { reason: null, at: null },
    });
    const notice = await screen.findByRole("group", { name: "You dismissed this job" });
    expect(within(notice).getByText("Dismissed, no reason given.")).toBeTruthy();
    expect(primaryButtons()).toHaveLength(0);
    expect(document.getElementById("app-dismiss-open")).toBeNull();
    await user.click(within(notice).getByRole("button", { name: "Restore" }));
    expect(mocked.restoreApplication).toHaveBeenCalledWith(7);
  });
});

describe("the failure of a job", () => {
  it("reports it in a toast with Retry that stays, and dismisses it in the shell when it is closed", async () => {
    const user = userEvent.setup();
    open({
      error: new CommandError("connection refused", {
        kind: "network",
        challenge: null,
        advice: "The connection failed. Check your network and try again.",
        settingsSection: null,
        retryable: true,
      }),
    });
    const toast = await screen.findByRole("alert", { name: "The last job failed" });
    expect(within(toast).getByText("connection refused")).toBeTruthy();
    expect(within(toast).getByText("The connection failed. Check your network and try again.")).toBeTruthy();
    expect(document.getElementById("job-error"), "no banner on the page").toBeNull();
    expect(mocked.dismissError, "kept in the shell while the toast is shown").not.toHaveBeenCalled();
    await user.click(within(toast).getByRole("button", { name: "Retry" }));
    expect(mocked.retry).toHaveBeenCalledWith(7);
    // Closed: the shell forgets it, so it is not reported again on the next read.
    expect(mocked.dismissError).toHaveBeenCalledWith(7);
    expect(screen.queryByRole("alert", { name: "The last job failed" }), "Retry closes the toast").toBeNull();
  });
});

describe("the tabs", () => {
  it("shows the JD, the Apply step and the Timeline", async () => {
    const user = userEvent.setup();
    open();
    await screen.findByRole("heading", { name: "Backend Engineer" });
    await user.click(screen.getByRole("tab", { name: "JD" }));
    expect(await screen.findByRole("heading", { name: "Job description" })).toBeTruthy();
    await user.click(screen.getByRole("tab", { name: "Apply" }));
    expect(await screen.findByText("Greenhouse")).toBeTruthy();
    await user.click(screen.getByRole("tab", { name: "Timeline" }));
    expect(await screen.findByRole("heading", { name: "History" })).toBeTruthy();
  });
});

describe("with nothing open", () => {
  it("explains how to get an application when there is none", async () => {
    useNav.setState({ applicationId: undefined });
    useShell.setState({
      status: {
        health: { ok: true, text: "", focus: null },
        blockers: [],
        prefsSaved: true,
        profileReady: true,
        newLeads: 0,
        applications: 0,
      },
    });
    render(<ApplicationPage />);
    expect(screen.getByText("No applications yet")).toBeTruthy();
    expect(screen.getByText(/paste the link of a job posting/)).toBeTruthy();
  });

  it("asks to pick one when there are some", async () => {
    useNav.setState({ applicationId: undefined });
    useShell.setState({
      status: {
        health: { ok: true, text: "", focus: null },
        blockers: [],
        prefsSaved: true,
        profileReady: true,
        newLeads: 0,
        applications: 3,
      },
    });
    render(<ApplicationPage />);
    expect(screen.getByText("Nothing selected")).toBeTruthy();
  });

  it("shows the same when the application is gone", async () => {
    useShell.setState({
      status: {
        health: { ok: true, text: "", focus: null },
        blockers: [],
        prefsSaved: true,
        profileReady: true,
        newLeads: 0,
        applications: 3,
      },
    });
    mocked.readApplication.mockResolvedValue(null);
    render(<ApplicationPage />);
    expect(await screen.findByText("Nothing selected")).toBeTruthy();
  });
});
