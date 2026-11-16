import { cleanup, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { CommandError, type Problem } from "@/lib/problem";
import { useNav } from "@/stores/nav";

import * as api from "@/api/discovery";
import { FAILED, INSIGHT, lead, queue, SOURCE, WAITING } from "@/test/fixtures/discovery";
import { renderWithToasts } from "@/test/render";
import type { DismissedJob } from "@/types/discovery";
import { resetToday } from "@/stores/today";
import { LeadDrawer } from "./LeadDrawer";
import { LeadsHeaderLine, SearchNowButton, SourcesLine } from "./JobsHeader";
import { LeadsStage } from "./LeadsStage";
import { LinkBox } from "./LinkBox";
import { NeedsYou } from "./NeedsYou";
import { useLeadsLifecycle, useLeadsSummary } from "@/hooks/discovery/useToday";

vi.mock("@/api/discovery");

const mocked = vi.mocked(api);

/** The Leads stage as the Jobs page composes it. */
function JobsPage({ otherDrawerOpen = false }: { otherDrawerOpen?: boolean }) {
  useLeadsLifecycle();
  const summary = useLeadsSummary();
  return (
    <main aria-label="Jobs">
      <h1>Jobs</h1>
      <LeadsHeaderLine />
      <SearchNowButton otherDrawerOpen={otherDrawerOpen} />
      <LinkBox />
      <SourcesLine />
      <NeedsYou />
      <p>{`Leads, ${summary.newLeads}`}</p>
      <LeadsStage otherDrawerOpen={otherDrawerOpen} />
      <LeadDrawer />
    </main>
  );
}

beforeEach(() => {
  resetToday();
  useNav.setState({
    screen: "jobs",
    stage: "leads",
    drawer: undefined,
    needsYou: 0,
    droppedFile: undefined,
    applicationId: undefined,
  });
  mocked.readQueue.mockResolvedValue(queue());
  mocked.readProfileSkills.mockResolvedValue(["Rust", "Kubernetes (EKS)"]);
  mocked.readDismissed.mockResolvedValue([]);
  mocked.onChanged.mockReturnValue(() => undefined);
  mocked.onProgress.mockReturnValue(() => undefined);
  mocked.onPrepared.mockReturnValue(() => undefined);
  mocked.onSearchFinished.mockReturnValue(() => undefined);
});

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});

/** The one primary button (it carries the accent fill; plain buttons never do). */
const primaryButtons = () => Array.from(document.querySelectorAll("button.bg-accent"));

/** The page once the list is read. */
async function showList() {
  renderWithToasts(<JobsPage />);
  return screen.findByRole("table", { name: "Leads" });
}

/** Open the lead of the row named `name`; resolves to its drawer. */
async function openLead(user: ReturnType<typeof userEvent.setup>, name: string) {
  await user.click(screen.getByRole("row", { name }));
  return screen.findByRole("dialog", { name: name.replace(/, score .*$/, "") });
}

describe("the list of leads", () => {
  it("shows the leads best first, with no drawer and Search now as a plain button", async () => {
    const table = await showList();
    const rows = within(table).getAllByRole("row", { name: /Acme: Backend Engineer/ });
    expect(rows.map((r) => r.getAttribute("aria-label"))).toEqual([
      "Acme: Backend Engineer 1, score 80",
      "Acme: Backend Engineer 2, score 70",
      "Acme: Backend Engineer 3, score –",
    ]);
    expect(rows.every((r) => r.getAttribute("aria-current") === null)).toBe(true);
    expect(within(rows[0] as HTMLElement).getByText("Acme careers")).toBeTruthy();
    expect(within(rows[0] as HTMLElement).getByText("Acme · Remote")).toBeTruthy();
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(screen.getByText("Leads, 3")).toBeTruthy();

    // The header and the sources.
    expect(screen.getByText(/Last search 5 min ago: Found 3 new leads/)).toBeTruthy();
    expect(screen.getByText("Acme careers: ok, 1 h ago")).toBeTruthy();
    expect(screen.getByText("3 of 3 shown")).toBeTruthy();
    // Nothing is open or ticked: no primary button at all.
    expect((screen.getByRole("button", { name: "Search now" }) as HTMLButtonElement).className).not.toContain(
      "bg-accent",
    );
    expect(primaryButtons()).toEqual([]);
  });

  it("opens a lead in the drawer with its brief, and the drawer holds the one primary button", async () => {
    const user = userEvent.setup();
    await showList();
    const drawer = await openLead(user, "Acme: Backend Engineer 1, score 80");
    expect(useNav.getState().drawer).toEqual({ kind: "lead", id: 1 });
    expect(screen.getByRole("row", { name: "Acme: Backend Engineer 1, score 80" }).getAttribute("aria-current")).toBe(
      "true",
    );

    const prepare = within(drawer).getByRole("button", { name: "Prepare CV" });
    expect(primaryButtons()).toEqual([prepare]);

    // Why it fits, skills against the profile, company and interview.
    const breakdown = within(drawer).getByRole("group", { name: "Score breakdown" });
    expect(within(breakdown).getByText("Skills 35%")).toBeTruthy();
    expect(within(breakdown).getByText("Not judged")).toBeTruthy();
    expect(within(breakdown).getByText("Weighted by your preferences.")).toBeTruthy();
    const skills = await within(drawer).findByRole("group", {
      name: "Skills the posting asks for. In your profile: Rust, Kubernetes. Not in your profile: PostgreSQL, Kafka.",
    });
    expect(within(skills).getByText("Rust")).toBeTruthy();
    expect(within(drawer).getByText(INSIGHT.domain)).toBeTruthy();
    expect(within(drawer).getByText("Recruiter call")).toBeTruthy();
    expect(within(drawer).getByLabelText("Fit score 80")).toBeTruthy();
    expect(within(drawer).getByRole("button", { name: "Posting: https://jobs.acme.test/1" })).toBeTruthy();
    expect(within(drawer).getByRole("button", { name: "Open posting" })).toBeTruthy();
    expect(within(drawer).getByRole("button", { name: "Dismiss" })).toBeTruthy();
  });

  it("switches the drawer to another row, and offers no posting link for a pasted lead", async () => {
    const user = userEvent.setup();
    await showList();
    await openLead(user, "Acme: Backend Engineer 1, score 80");
    const drawer = await openLead(user, "Acme: Backend Engineer 3, score –");
    expect((within(drawer).getByRole("button", { name: "Open posting" }) as HTMLButtonElement).disabled).toBe(true);
    expect(within(drawer).queryByRole("group", { name: "Score breakdown" })).toBeNull();
    expect(within(drawer).getByText("No company brief for this lead.")).toBeTruthy();
    expect(useNav.getState().drawer).toEqual({ kind: "lead", id: 3 });
    expect(screen.getAllByRole("dialog")).toHaveLength(1);
  });

  it("opens a row with Enter, and closes the drawer with Escape or its close button", async () => {
    const user = userEvent.setup();
    await showList();
    screen.getByRole("row", { name: "Acme: Backend Engineer 2, score 70" }).focus();
    await user.keyboard("{Enter}");
    expect(await screen.findByRole("dialog", { name: "Acme: Backend Engineer 2" })).toBeTruthy();
    await user.keyboard("{Escape}");
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    expect(useNav.getState().drawer).toBeUndefined();

    const drawer = await openLead(user, "Acme: Backend Engineer 2, score 70");
    await user.click(within(drawer).getByRole("button", { name: "Close" }));
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    expect(useNav.getState().drawer).toBeUndefined();
    expect(primaryButtons()).toEqual([]);
  });

  it("opens the best lead when asked to", async () => {
    await showList();
    useNav.getState().openLead("best");
    expect(await screen.findByRole("dialog", { name: "Acme: Backend Engineer 1" })).toBeTruthy();
  });

  it("prepares the CV of the open lead", async () => {
    const user = userEvent.setup();
    mocked.prepareCvs.mockReturnValue(new Promise(() => undefined));
    await showList();
    const drawer = await openLead(user, "Acme: Backend Engineer 1, score 80");
    await user.click(within(drawer).getByRole("button", { name: "Prepare CV" }));
    expect(mocked.prepareCvs).toHaveBeenCalledWith([1]);
    // Lead 1 is being prepared: its row says so, and Dismiss is off.
    expect(await screen.findAllByText("Waiting for its turn")).toHaveLength(1);
    expect((within(drawer).getByRole("button", { name: "Dismiss" }) as HTMLButtonElement).disabled).toBe(true);
    expect((within(drawer).getByRole("button", { name: "Prepare CV" }) as HTMLButtonElement).disabled).toBe(true);
  });

  it("prepares the ticked leads in queue order from the bulk bar, without opening them", async () => {
    const user = userEvent.setup();
    mocked.prepareCvs.mockReturnValue(new Promise(() => undefined));
    await showList();
    await user.click(screen.getByRole("checkbox", { name: "Select Acme: Backend Engineer 2" }));
    await user.click(screen.getByRole("checkbox", { name: "Select Acme: Backend Engineer 1" }));
    expect(screen.queryByRole("dialog"), "a tick never opens the row").toBeNull();
    const bar = screen.getByRole("group", { name: "2 selected" });
    const bulk = within(bar).getByRole("button", { name: "Prepare 2 CVs" });
    expect(primaryButtons()).toEqual([bulk]);
    expect(within(bar).getByText("2 leads selected, their CVs are written one after the other")).toBeTruthy();
    await user.click(bulk);
    expect(mocked.prepareCvs).toHaveBeenCalledWith([1, 2]);
    expect(screen.queryByRole("group", { name: "2 selected" }), "the ticks are used up").toBeNull();
  });

  it("clears the ticks, and leaves the primary to the drawer while it is open", async () => {
    const user = userEvent.setup();
    await showList();
    await user.click(screen.getByRole("checkbox", { name: "Select Acme: Backend Engineer 2" }));
    expect(screen.getByRole("group", { name: "1 selected" })).toBeTruthy();

    const drawer = await openLead(user, "Acme: Backend Engineer 1, score 80");
    expect(screen.queryByRole("group", { name: "1 selected" })).toBeNull();
    expect(primaryButtons()).toEqual([within(drawer).getByRole("button", { name: "Prepare CV" })]);
    await user.click(within(drawer).getByRole("button", { name: "Close" }));
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());

    await user.click(within(screen.getByRole("group", { name: "1 selected" })).getByRole("button", { name: "Clear" }));
    expect(screen.queryByRole("group", { name: "1 selected" })).toBeNull();
    expect(primaryButtons()).toEqual([]);
  });

  it("prepares only the lead shown in the drawer, whatever is ticked", async () => {
    const user = userEvent.setup();
    mocked.prepareCvs.mockReturnValue(new Promise(() => undefined));
    await showList();
    await user.click(screen.getByRole("checkbox", { name: "Select Acme: Backend Engineer 1" }));
    const drawer = await openLead(user, "Acme: Backend Engineer 2, score 70");
    expect(within(drawer).queryByText(/leads selected/), "no hint about the ticked leads").toBeNull();
    await user.click(within(drawer).getByRole("button", { name: "Prepare CV" }));
    expect(mocked.prepareCvs).toHaveBeenCalledWith([2]);
    // Lead 1 is still ticked, for the bulk bar.
    expect(
      (screen.getByRole("checkbox", { name: "Select Acme: Backend Engineer 1" }) as HTMLInputElement).checked,
    ).toBe(true);
  });

  it("closes the drawer when a filter hides its lead", async () => {
    const user = userEvent.setup();
    await showList();
    await openLead(user, "Acme: Backend Engineer 2, score 70");
    await user.click(screen.getByRole("button", { name: "85+" }));
    await waitFor(() => expect(useNav.getState().drawer).toBeUndefined());
    expect(screen.queryByRole("dialog")).toBeNull();
    // Loosening the filter again does not bring it back by itself.
    await user.click(screen.getByRole("button", { name: "70+" }));
    expect(useNav.getState().drawer).toBeUndefined();
  });

  it("closes the drawer asked for the best lead of an empty list", async () => {
    mocked.readQueue.mockResolvedValue(queue({ items: [] }));
    useNav.setState({ drawer: { kind: "lead", id: "best" } });
    renderWithToasts(<JobsPage />);
    expect(await screen.findByRole("group", { name: "No jobs yet" })).toBeTruthy();
    await waitFor(() => expect(useNav.getState().drawer).toBeUndefined());
  });

  it("narrows the list with the filters", async () => {
    const user = userEvent.setup();
    await showList();
    await user.click(screen.getByRole("button", { name: "70+" }));
    expect(screen.getByText("2 of 3 shown")).toBeTruthy();
    await user.click(screen.getByRole("button", { name: "85+" }));
    expect(screen.getByText("0 of 3 shown")).toBeTruthy();
    expect(screen.getByText("No lead matches the filters")).toBeTruthy();
    expect(primaryButtons()).toHaveLength(0);
  });

  it("dismisses the open lead with a reason, and the company too if asked", async () => {
    const user = userEvent.setup();
    mocked.dismissLead.mockResolvedValue({ also: 0, kept: 0, notExcluded: null });
    await showList();
    const drawer = await openLead(user, "Acme: Backend Engineer 1, score 80");
    await user.click(within(drawer).getByRole("button", { name: "Dismiss" }));
    const form = await screen.findByRole("dialog", { name: "Dismiss this lead" });
    expect(within(form).getByText("Acme — Backend Engineer 1")).toBeTruthy();
    await user.type(within(form).getByRole("textbox", { name: "Why dismiss this lead" }), "too junior");
    await user.click(within(form).getByRole("checkbox", { name: "Don't show Acme again" }));
    await user.click(within(form).getByRole("button", { name: "Dismiss lead" }));
    expect(mocked.dismissLead).toHaveBeenCalledWith(1, "too junior", true);
    expect(await screen.findByRole("status", { name: "Lead dismissed" })).toBeTruthy();
    // The dialog is gone, and the next lead opens in its place.
    await waitFor(() => expect(screen.queryByRole("dialog", { name: "Dismiss this lead" })).toBeNull());
    expect(await screen.findByRole("dialog", { name: "Acme: Backend Engineer 2" })).toBeTruthy();
  });

  it("dismisses with Enter, and says in a toast when the company could not be excluded", async () => {
    const user = userEvent.setup();
    mocked.dismissLead.mockResolvedValue({ also: 1, kept: 0, notExcluded: "disk full" });
    await showList();
    const drawer = await openLead(user, "Acme: Backend Engineer 1, score 80");
    await user.click(within(drawer).getByRole("button", { name: "Dismiss" }));
    const form = await screen.findByRole("dialog", { name: "Dismiss this lead" });
    await user.click(within(form).getByRole("checkbox", { name: "Don't show Acme again" }));
    await user.type(within(form).getByRole("textbox", { name: "Why dismiss this lead" }), "no{Enter}");
    expect(mocked.dismissLead).toHaveBeenCalledWith(1, "no", true);
    const toast = await screen.findByRole("alert", { name: "Lead dismissed, but the company could not be excluded" });
    expect(within(toast).getByText("disk full")).toBeTruthy();
  });

  it("shows in the dialog why a dismissal failed, and cancels it", async () => {
    const user = userEvent.setup();
    mocked.dismissLead.mockRejectedValue(new CommandError("its CV is being prepared: wait for it to finish first"));
    await showList();
    const drawer = await openLead(user, "Acme: Backend Engineer 1, score 80");
    await user.click(within(drawer).getByRole("button", { name: "Dismiss" }));
    const form = await screen.findByRole("dialog", { name: "Dismiss this lead" });
    await user.type(within(form).getByRole("textbox", { name: "Why dismiss this lead" }), "meh");
    await user.click(within(form).getByRole("button", { name: "Dismiss lead" }));
    expect((await within(form).findByRole("alert")).textContent).toContain("Not dismissed: its CV is being prepared");
    await user.click(within(form).getByRole("button", { name: "Cancel" }));
    await waitFor(() => expect(screen.queryByRole("dialog", { name: "Dismiss this lead" })).toBeNull());
    // The drawer stays; opened again, the dialog starts afresh.
    await user.click(within(drawer).getByRole("button", { name: "Dismiss" }));
    const again = await screen.findByRole("dialog", { name: "Dismiss this lead" });
    expect(within(again).queryByRole("alert")).toBeNull();
    expect((within(again).getByRole("textbox", { name: "Why dismiss this lead" }) as HTMLInputElement).value).toBe("");

    // Escape closes the dialog only, not the drawer under it.
    await user.keyboard("{Escape}");
    await waitFor(() => expect(screen.queryByRole("dialog", { name: "Dismiss this lead" })).toBeNull());
    expect(screen.getByRole("dialog", { name: "Acme: Backend Engineer 1" })).toBeTruthy();
    expect(useNav.getState().drawer).toEqual({ kind: "lead", id: 1 });
  });
});

describe("Needs you", () => {
  it("shows a paused source with Open browser, Continue and Skip, and counts it on the badge", async () => {
    const user = userEvent.setup();
    mocked.readQueue.mockResolvedValue(queue({ waiting: [WAITING] }));
    mocked.openBrowser.mockResolvedValue(undefined);
    mocked.resumeSource.mockResolvedValue(undefined);
    mocked.skipSource.mockResolvedValue(undefined);
    renderWithToasts(<JobsPage />);
    const group = await screen.findByRole("group", { name: "Needs you" });
    expect(within(group).getByText("NEEDS YOU · 1")).toBeTruthy();
    const banner = within(group).getByRole("group", { name: "Beta jobs needs you to sign in" });
    expect(within(banner).getByText("https://beta.test/login")).toBeTruthy();
    await waitFor(() => expect(useNav.getState().needsYou).toBe(1));

    await user.click(within(banner).getByRole("button", { name: "Open browser" }));
    expect(mocked.openBrowser).toHaveBeenCalledWith("discover:1:2");
    await user.click(within(banner).getByRole("button", { name: "Continue" }));
    expect(mocked.resumeSource).toHaveBeenCalledWith("discover:1:2");
    await user.click(within(banner).getByRole("button", { name: "Skip" }));
    expect(mocked.skipSource).toHaveBeenCalledWith("discover:1:2");
  });

  it("says why the browser could not be opened, with what to do about it", async () => {
    const user = userEvent.setup();
    const problem: Problem = {
      kind: "noBrowser",
      challenge: null,
      advice: "Connect the JobPilot extension: open Chrome with the extension and pair it in Settings > Browser.",
      settingsSection: "Browser",
      callToAction: "Connect the JobPilot extension",
      retryable: false,
    };
    mocked.readQueue.mockResolvedValue(queue({ waiting: [{ ...WAITING, reason: "captcha", url: null }] }));
    mocked.openBrowser.mockRejectedValue(new CommandError("the JobPilot browser extension is not connected", problem));
    renderWithToasts(<JobsPage />);
    const banner = await screen.findByRole("group", { name: "Beta jobs shows a captcha" });
    expect(within(banner).getByText("Deal with the page in the browser, then press Continue.")).toBeTruthy();
    await user.click(within(banner).getByRole("button", { name: "Open browser" }));
    const toast = await screen.findByRole("alert", { name: "The browser could not be opened" });
    expect(within(toast).getByText("the JobPilot browser extension is not connected")).toBeTruthy();
    expect(within(toast).getByText(/^Connect the JobPilot extension: open Chrome/)).toBeTruthy();
    expect(within(toast).getByRole("button", { name: "Connect the JobPilot extension" })).toBeTruthy();
    expect(within(banner).queryByText(/not connected/), "nothing red under the banner").toBeNull();
  });

  it("shows a CV that could not be written, with Try again, Open application and Forget", async () => {
    const user = userEvent.setup();
    mocked.readQueue.mockResolvedValue(queue({ failed: [FAILED] }));
    mocked.prepareCvs.mockReturnValue(new Promise(() => undefined));
    mocked.forgetFailure.mockResolvedValue(undefined);
    renderWithToasts(<JobsPage />);
    const banner = await screen.findByRole("group", { name: "The CV for Gamma — SRE could not be written" });
    expect(within(banner).getByText("the fact guard refused the CV")).toBeTruthy();
    // More than one line: the whole text sits under the banner, in plain (not red) text.
    const text = document.getElementById("failure-text-7");
    expect(text?.textContent).toContain("line two");
    expect(text?.className).not.toContain("danger");
    await user.click(within(banner).getByRole("button", { name: "Open application" }));
    const nav = useNav.getState();
    // The application's drawer moves the page to its stage once it has read it.
    expect([nav.screen, nav.drawer]).toEqual(["jobs", { kind: "app", id: 31 }]);

    await user.click(screen.getByRole("button", { name: "Forget" }));
    expect(mocked.forgetFailure).toHaveBeenCalledWith(7);
  });

  it("shows a CV that is ready, opening its application in the drawer", async () => {
    const user = userEvent.setup();
    mocked.prepareCvs.mockResolvedValue([
      { leadId: 1, applicationId: 12, label: "Acme — Backend Engineer 1", error: null },
    ]);
    await showList();
    const drawer = await openLead(user, "Acme: Backend Engineer 1, score 80");
    await user.click(within(drawer).getByRole("button", { name: "Prepare CV" }));
    const banner = await screen.findByRole("group", { name: "CV ready for Acme — Backend Engineer 1" });
    await user.click(within(banner).getByRole("button", { name: "Open application" }));
    const nav = useNav.getState();
    expect(nav.drawer).toEqual({ kind: "app", id: 12 });
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
  });

  it("shows nothing when nothing needs the user", async () => {
    await showList();
    expect(screen.queryByRole("group", { name: "Needs you" })).toBeNull();
  });
});

describe("the link box", () => {
  it("adds a job from what is pasted, and opens it", async () => {
    const user = userEvent.setup();
    mocked.parseInput.mockResolvedValue({ kind: "url", value: "https://acme.test/jobs/9" });
    mocked.addLead.mockResolvedValue({ leadId: 2, state: "new", applicationId: null });
    await showList();
    const add = screen.getByRole("button", { name: "Add job" }) as HTMLButtonElement;
    expect(add.disabled).toBe(true);
    const box = screen.getByRole("textbox", { name: "Job link or posting text" }) as HTMLInputElement;
    await user.type(box, "acme.test/jobs/9{Enter}");
    expect(mocked.parseInput).toHaveBeenCalledWith("acme.test/jobs/9");
    expect(mocked.addLead).toHaveBeenCalledWith({ kind: "url", value: "https://acme.test/jobs/9" });
    await waitFor(() => expect(box.value).toBe(""));
    expect(await screen.findByRole("dialog", { name: "Acme: Backend Engineer 2" })).toBeTruthy();
  });

  it("says why a job could not be added, with Try again", async () => {
    const user = userEvent.setup();
    mocked.parseInput.mockResolvedValue({ kind: "text", value: "We are hiring" });
    mocked.addLead
      .mockRejectedValueOnce(new CommandError("text that has no job title"))
      .mockReturnValue(new Promise(() => undefined));
    await showList();
    await user.type(screen.getByRole("textbox", { name: "Job link or posting text" }), "We are hiring");
    await user.click(screen.getByRole("button", { name: "Add job" }));
    const failure = await screen.findByRole("group", {
      name: "The posting could not be added: text that has no job title",
    });
    // A form error of the link box, not a toast.
    expect(within(failure).getByRole("alert").textContent).toContain("text that has no job title");
    expect(document.querySelector("[data-toast-region] li"), "no toast").toBeNull();
    await user.click(within(failure).getByRole("button", { name: "Try again" }));
    expect(mocked.addLead).toHaveBeenCalledTimes(2);
    expect(await screen.findByRole("status", { name: "Reading the posting" })).toBeTruthy();
  });

  it("turns a dropped file into a lead, or says why it was refused", async () => {
    mocked.fileInput.mockRejectedValue(
      new CommandError("c.png: .png files are not supported (use .md, .txt, .html or .pdf)."),
    );
    await showList();
    useNav.getState().dropFile("/tmp/c.png");
    expect(await screen.findByText(/c\.png: \.png files are not supported/)).toBeTruthy();
    expect(mocked.fileInput).toHaveBeenCalledWith("/tmp/c.png");
    expect(useNav.getState().droppedFile, "the file is read once").toBeUndefined();
  });
});

describe("no leads yet", () => {
  it("offers Search now as the one primary button", async () => {
    const user = userEvent.setup();
    mocked.readQueue.mockResolvedValue(queue({ items: [], lastRun: null }));
    mocked.searchNow.mockResolvedValue({ finishedAt: "x", saved: 0, waiting: 0, failed: 0 });
    renderWithToasts(<JobsPage />);
    await screen.findByRole("group", { name: "No jobs yet" });
    expect(await screen.findByText("No jobs yet")).toBeTruthy();
    const search = screen.getByRole("button", { name: "Search now" });
    expect(search.id).toBe("queue-search");
    expect(primaryButtons()).toEqual([search]);
    expect(screen.getAllByRole("button", { name: "Search now" })).toHaveLength(1);
    expect(screen.getByText("No search yet.")).toBeTruthy();
    expect(screen.getByText("Leads, 0")).toBeTruthy();
    await user.click(search);
    expect(mocked.searchNow).toHaveBeenCalledTimes(1);
    const toast = await screen.findByRole("status", { name: "Search finished" });
    expect(within(toast).getByText("Found 0 new leads")).toBeTruthy();
  });

  it("says in a toast why a search failed, with what to do about it", async () => {
    const user = userEvent.setup();
    const problem: Problem = {
      kind: "noProvider",
      challenge: null,
      advice: "Add one.",
      settingsSection: "AI providers",
      retryable: false,
    };
    mocked.readQueue.mockResolvedValue(queue({ items: [], lastRun: null }));
    mocked.searchNow.mockRejectedValue(new CommandError("no AI provider is configured", problem));
    renderWithToasts(<JobsPage />);
    await user.click(await screen.findByRole("button", { name: "Search now" }));
    const toast = await screen.findByRole("alert", { name: "The search failed" });
    expect(within(toast).getByText("no AI provider is configured")).toBeTruthy();
    expect(within(toast).getByRole("button", { name: "Open Settings > AI providers" })).toBeTruthy();
    expect(document.getElementById("queue-note"), "no red line under the header").toBeNull();
  });

  it("leaves Search now plain while another drawer holds the primary", async () => {
    mocked.readQueue.mockResolvedValue(queue({ items: [] }));
    renderWithToasts(<JobsPage otherDrawerOpen />);
    expect(await screen.findByText("No jobs yet")).toBeTruthy();
    expect(primaryButtons()).toEqual([]);
  });

  it("says there is no source to search yet", async () => {
    mocked.readQueue.mockResolvedValue(queue({ items: [], sources: [] }));
    renderWithToasts(<JobsPage />);
    expect(await screen.findByText("No job source yet: add one in Settings, then search.")).toBeTruthy();
    expect((screen.getByRole("button", { name: "Search now" }) as HTMLButtonElement).disabled).toBe(true);
  });

  it("shows a search that runs in the background", async () => {
    mocked.readQueue.mockResolvedValue(queue({ items: [], searching: true }));
    renderWithToasts(<JobsPage />);
    expect((await screen.findAllByText("Searching your sources")).length).toBeGreaterThan(0);
  });

  it("says when the job list could not be read", async () => {
    const user = userEvent.setup();
    mocked.readQueue.mockRejectedValueOnce(new CommandError("database is locked"));
    renderWithToasts(<JobsPage />);
    const toast = await screen.findByRole("alert", { name: "The job list could not be read" });
    expect(within(toast).getByText("database is locked")).toBeTruthy();
    expect(screen.getByText("Loading the job list")).toBeTruthy();
    const quiet = document.getElementById("queue-load-error") as HTMLElement;
    expect(within(quiet).getByText("The job list could not be read.")).toBeTruthy();
    await user.click(within(quiet).getByRole("button", { name: "Retry" }));
    expect(await screen.findByRole("table", { name: "Leads" })).toBeTruthy();
    expect(document.getElementById("queue-load-error")).toBeNull();
  });
});

describe("several sources", () => {
  it("filters by source", async () => {
    const user = userEvent.setup();
    mocked.readQueue.mockResolvedValue(
      queue({ sources: [SOURCE, { ...SOURCE, id: 2, name: "Beta jobs" }], items: [lead(1), lead(2, { sourceId: 2 })] }),
    );
    await showList();
    await user.click(screen.getByRole("button", { name: "Beta jobs" }));
    expect(screen.getByText("1 of 2 shown")).toBeTruthy();
    await user.click(screen.getByRole("button", { name: "All" }));
    expect(screen.getByText("2 of 2 shown")).toBeTruthy();
  });
});

describe("dismissed jobs", () => {
  const LEAD: DismissedJob = {
    kind: "lead",
    id: 9,
    company: "Initech",
    role: "Platform Engineer",
    url: "https://initech.test/jobs/9",
    reason: "excluded company: Initech",
    dismissedAt: new Date(Date.now() - 5 * 60_000).toISOString(),
  };
  const CV: DismissedJob = {
    kind: "application",
    id: 31,
    company: "Globex",
    role: "SRE",
    url: null,
    reason: null,
    dismissedAt: new Date(Date.now() - 2 * 3_600_000).toISOString(),
  };

  beforeEach(() => {
    mocked.readQueue.mockResolvedValue(queue({ dismissed: 2 }));
    mocked.readDismissed.mockResolvedValue([LEAD, CV]);
  });

  it("lists every dismissed job, newest first, without the lead filters", async () => {
    const user = userEvent.setup();
    await showList();
    expect(screen.getByRole("button", { name: "Open" }).getAttribute("aria-pressed")).toBe("true");
    await user.click(screen.getByRole("button", { name: "Dismissed 2" }));
    const table = await screen.findByRole("table", { name: "Dismissed jobs" });
    const rows = within(table).getAllByRole("row").slice(1);
    expect(rows.map((r) => r.getAttribute("aria-label"))).toEqual([
      "Initech: Platform Engineer, dismissed lead",
      "Globex: SRE, dismissed CV",
    ]);
    expect(within(rows[0]!).getByText("Lead")).toBeTruthy();
    expect(within(rows[0]!).getByText("excluded company: Initech")).toBeTruthy();
    expect(within(rows[0]!).getByText("5 min ago")).toBeTruthy();
    expect(within(rows[1]!).getByText("CV")).toBeTruthy();
    expect(within(rows[1]!).getByText("No reason given")).toBeTruthy();
    expect(within(rows[0]!).getByRole("button", { name: "Open posting: Initech: Platform Engineer" })).toBeTruthy();
    expect(within(rows[1]!).queryByRole("button", { name: /Open posting/ })).toBeNull();
    expect(screen.queryByRole("button", { name: "70+" })).toBeNull();
    expect(screen.queryByRole("table", { name: "Leads" })).toBeNull();
    expect(primaryButtons()).toHaveLength(0);
  });

  it("searches the dismissed jobs by company or role", async () => {
    const user = userEvent.setup();
    await showList();
    await user.click(screen.getByRole("button", { name: "Dismissed 2" }));
    await screen.findByRole("table", { name: "Dismissed jobs" });
    const search = screen.getByRole("textbox", { name: "Company or role contains" });
    await user.type(search, "sre");
    expect(screen.getByText("1 of 2 shown")).toBeTruthy();
    await user.clear(search);
    await user.type(search, "nobody");
    expect(screen.getByText("No dismissed job matches the search")).toBeTruthy();
  });

  it("restores a job and says where it went", async () => {
    const user = userEvent.setup();
    mocked.restore.mockResolvedValue({ hidden: null, excluded: true });
    await showList();
    await user.click(screen.getByRole("button", { name: "Dismissed 2" }));
    await screen.findByRole("table", { name: "Dismissed jobs" });
    await user.click(screen.getByRole("button", { name: "Restore Initech: Platform Engineer" }));
    expect(mocked.restore).toHaveBeenCalledWith("lead", 9);
    expect(await screen.findByText(/is back in Leads\. The company is still excluded/)).toBeTruthy();
  });

  it("drops the ticks when Dismissed is picked, and offers them again on Open", async () => {
    const user = userEvent.setup();
    await showList();
    await user.click(screen.getByRole("checkbox", { name: "Select Acme: Backend Engineer 1" }));
    expect(screen.getByRole("group", { name: "1 selected" })).toBeTruthy();
    await user.click(screen.getByRole("button", { name: "Dismissed 2" }));
    await screen.findByRole("table", { name: "Dismissed jobs" });
    expect(screen.queryByRole("group", { name: "1 selected" })).toBeNull();
    await user.click(screen.getByRole("button", { name: "Open" }));
    expect(screen.queryByRole("group", { name: "1 selected" })).toBeNull();
    expect(
      (screen.getByRole("checkbox", { name: "Select Acme: Backend Engineer 1" }) as HTMLInputElement).checked,
    ).toBe(false);
  });

  it("opens a dismissed CV in the application drawer, on the Leads stage", async () => {
    const user = userEvent.setup();
    await showList();
    await user.click(screen.getByRole("button", { name: "Dismissed 2" }));
    const table = await screen.findByRole("table", { name: "Dismissed jobs" });
    await user.click(within(table).getByRole("row", { name: "Globex: SRE, dismissed CV" }));
    expect(useNav.getState().drawer).toEqual({ kind: "app", id: 31 });
    expect(useNav.getState().stage).toBe("leads");
  });

  it("offers the dismissed jobs when no lead is left", async () => {
    const user = userEvent.setup();
    mocked.readQueue.mockResolvedValue(queue({ items: [], dismissed: 2 }));
    renderWithToasts(<JobsPage />);
    await user.click(await screen.findByRole("button", { name: "Dismissed 2" }));
    expect(await screen.findByRole("table", { name: "Dismissed jobs" })).toBeTruthy();
  });
});
