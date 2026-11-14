import { cleanup, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { CommandError, useNav, type Problem } from "@/shared";

import * as api from "../api/discoveryApi";
import { FAILED, INSIGHT, lead, queue, SOURCE, WAITING } from "../domain/fixtures";
import { resetToday } from "../hooks/todayStore";
import { TodayScreen } from "./TodayScreen";

vi.mock("../api/discoveryApi");

const mocked = vi.mocked(api);

beforeEach(() => {
  resetToday();
  useNav.setState({ screen: "today", leadId: undefined, needsYou: 0, droppedFile: undefined, applicationId: undefined });
  mocked.readQueue.mockResolvedValue(queue());
  mocked.readProfileSkills.mockResolvedValue(["Rust", "Kubernetes (EKS)"]);
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

describe("Today with leads", () => {
  it("shows the leads best first, the open lead's brief and exactly one primary button", async () => {
    render(<TodayScreen />);
    const prepare = await screen.findByRole("button", { name: "Prepare CV" });
    expect(primaryButtons()).toEqual([prepare]);

    expect(screen.getByRole("heading", { name: "Today" })).toBeTruthy();
    const list = screen.getByRole("list", { name: "Leads" });
    const rows = within(list).getAllByRole("button", { name: /Acme: Backend Engineer/ });
    expect(rows.map((r) => r.getAttribute("aria-label"))).toEqual([
      "Acme: Backend Engineer 1, score 80",
      "Acme: Backend Engineer 2, score 70",
      "Acme: Backend Engineer 3, score –",
    ]);
    expect(rows[0]?.getAttribute("aria-current")).toBe("true");

    // The open lead: title, why it fits, skills against the profile, company and interview.
    const panel = screen.getByRole("group", { name: "Acme: Backend Engineer 1" });
    expect(within(panel).getByRole("heading", { name: "Backend Engineer 1" })).toBeTruthy();
    const breakdown = within(panel).getByRole("group", { name: "Score breakdown" });
    expect(within(breakdown).getByText("Skills 35%")).toBeTruthy();
    expect(within(breakdown).getByText("Not judged")).toBeTruthy();
    expect(within(breakdown).getByText("Weighted by your preferences.")).toBeTruthy();
    const skills = await within(panel).findByRole("group", {
      name: "Skills the posting asks for. In your profile: Rust, Kubernetes. Not in your profile: PostgreSQL, Kafka.",
    });
    expect(within(skills).getByText("Rust")).toBeTruthy();
    expect(within(panel).getByText(INSIGHT.domain)).toBeTruthy();
    expect(within(panel).getByText("Recruiter call")).toBeTruthy();
    expect(within(panel).getByRole("button", { name: "Posting: https://jobs.acme.test/1" })).toBeTruthy();
    expect(within(panel).getByRole("button", { name: "Open posting" })).toBeTruthy();
    expect(within(panel).getByRole("button", { name: "Dismiss" })).toBeTruthy();

    // The header and the sources.
    expect(screen.getByText(/Last search 5 min ago: Found 3 new leads/)).toBeTruthy();
    expect(screen.getByText("Acme careers: ok, 1 h ago")).toBeTruthy();
    expect(screen.getByText("3 of 3 shown")).toBeTruthy();
    expect(screen.getByRole("button", { name: "Search now" })).toBeTruthy();
  });

  it("prepares the CV of the open lead, and of every ticked lead", async () => {
    const user = userEvent.setup();
    mocked.prepareCvs.mockReturnValue(new Promise(() => undefined));
    render(<TodayScreen />);
    await user.click(await screen.findByRole("button", { name: "Prepare CV" }));
    expect(mocked.prepareCvs).toHaveBeenCalledWith([1]);
    // Lead 1 is being prepared: its row says so, and Dismiss is off.
    expect(await screen.findAllByText("Waiting for its turn")).toHaveLength(1);
    expect((screen.getByRole("button", { name: "Dismiss" }) as HTMLButtonElement).disabled).toBe(true);
    expect((screen.getByRole("button", { name: "Prepare CV" }) as HTMLButtonElement).disabled).toBe(true);
  });

  it("prepares the ticked leads in queue order", async () => {
    const user = userEvent.setup();
    mocked.prepareCvs.mockReturnValue(new Promise(() => undefined));
    render(<TodayScreen />);
    await screen.findByRole("button", { name: "Prepare CV" });
    await user.click(screen.getByRole("checkbox", { name: "Select Acme: Backend Engineer 2" }));
    await user.click(screen.getByRole("checkbox", { name: "Select Acme: Backend Engineer 1" }));
    expect(await screen.findByText("2 leads selected, their CVs are written one after the other")).toBeTruthy();
    await user.click(screen.getByRole("button", { name: "Prepare CV" }));
    expect(mocked.prepareCvs).toHaveBeenCalledWith([1, 2]);
  });

  it("opens another lead when its card is pressed, and offers no posting link for a pasted one", async () => {
    const user = userEvent.setup();
    render(<TodayScreen />);
    await screen.findByRole("button", { name: "Prepare CV" });
    await user.click(screen.getByRole("button", { name: "Acme: Backend Engineer 3, score –" }));
    const panel = screen.getByRole("group", { name: "Acme: Backend Engineer 3" });
    expect((within(panel).getByRole("button", { name: "Open posting" }) as HTMLButtonElement).disabled).toBe(true);
    expect(within(panel).queryByRole("group", { name: "Score breakdown" })).toBeNull();
    expect(within(panel).getByText("No company brief for this lead.")).toBeTruthy();
    expect(useNav.getState().leadId).toBe(3);
  });

  it("narrows the list with the filters", async () => {
    const user = userEvent.setup();
    render(<TodayScreen />);
    await screen.findByRole("button", { name: "Prepare CV" });
    await user.click(screen.getByRole("button", { name: "70+" }));
    expect(screen.getByText("2 of 3 shown")).toBeTruthy();
    await user.click(screen.getByRole("button", { name: "85+" }));
    expect(screen.getByText("0 of 3 shown")).toBeTruthy();
    expect(screen.getByText("No lead matches the filters")).toBeTruthy();
    expect((screen.getByRole("button", { name: "Prepare CV" }) as HTMLButtonElement).disabled).toBe(true);
    expect(primaryButtons()).toHaveLength(1);
  });

  it("dismisses the open lead with a reason, and the company too if asked", async () => {
    const user = userEvent.setup();
    mocked.dismissLead.mockResolvedValue({ also: 0, kept: 0, notExcluded: null });
    render(<TodayScreen />);
    await user.click(await screen.findByRole("button", { name: "Dismiss" }));
    const form = screen.getByRole("group", { name: "Dismiss this lead" });
    await user.type(within(form).getByRole("textbox", { name: "Why dismiss this lead" }), "too junior");
    await user.click(within(form).getByRole("checkbox", { name: "Don't show Acme again" }));
    await user.click(within(form).getByRole("button", { name: "Dismiss lead" }));
    expect(mocked.dismissLead).toHaveBeenCalledWith(1, "too junior", true);
    expect(await screen.findByText("Lead dismissed.")).toBeTruthy();
  });

  it("cancels the dismiss form, and shows why a dismissal failed", async () => {
    const user = userEvent.setup();
    mocked.dismissLead.mockRejectedValue(new CommandError("its CV is being prepared: wait for it to finish first"));
    render(<TodayScreen />);
    await user.click(await screen.findByRole("button", { name: "Dismiss" }));
    await user.click(screen.getByRole("button", { name: "Dismiss lead" }));
    expect((await screen.findByRole("alert")).textContent).toContain("its CV is being prepared");
    await user.click(screen.getByRole("button", { name: "Cancel" }));
    expect(screen.queryByRole("group", { name: "Dismiss this lead" })).toBeNull();
  });
});

describe("Needs you", () => {
  it("shows a paused source with Open browser, Continue and Skip, and counts it on the badge", async () => {
    const user = userEvent.setup();
    mocked.readQueue.mockResolvedValue(queue({ waiting: [WAITING] }));
    mocked.openBrowser.mockResolvedValue(undefined);
    mocked.resumeSource.mockResolvedValue(undefined);
    mocked.skipSource.mockResolvedValue(undefined);
    render(<TodayScreen />);
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
    const problem: Problem = { kind: "noChrome", challenge: null, advice: "Chrome is needed.", settingsSection: "Browser sessions", retryable: false };
    mocked.readQueue.mockResolvedValue(queue({ waiting: [{ ...WAITING, reason: "captcha", url: null }] }));
    mocked.openBrowser.mockRejectedValue(new CommandError("no Chrome found", problem));
    render(<TodayScreen />);
    const banner = await screen.findByRole("group", { name: "Beta jobs shows a captcha" });
    expect(within(banner).getByText("Deal with the page in the browser, then press Continue.")).toBeTruthy();
    await user.click(within(banner).getByRole("button", { name: "Open browser" }));
    const error = await screen.findByRole("group", { name: "The browser could not be opened: no Chrome found" });
    expect(within(error).getByText("Chrome is needed.")).toBeTruthy();
    expect(within(error).getByRole("button", { name: "Open Settings > Browser sessions" })).toBeTruthy();
  });

  it("shows a CV that could not be written, with Try again, Open application and Forget", async () => {
    const user = userEvent.setup();
    mocked.readQueue.mockResolvedValue(queue({ failed: [FAILED] }));
    mocked.prepareCvs.mockReturnValue(new Promise(() => undefined));
    mocked.forgetFailure.mockResolvedValue(undefined);
    render(<TodayScreen />);
    const banner = await screen.findByRole("group", { name: "The CV for Gamma — SRE could not be written" });
    expect(within(banner).getByText("the fact guard refused the CV")).toBeTruthy();
    // More than one line: the whole text sits under the banner.
    expect(document.getElementById("failure-text-7")?.textContent).toContain("line two");
    await user.click(within(banner).getByRole("button", { name: "Open application" }));
    expect(useNav.getState().applicationId).toBe(31);

    useNav.setState({ screen: "today" });
    await user.click(screen.getByRole("button", { name: "Forget" }));
    expect(mocked.forgetFailure).toHaveBeenCalledWith(7);
  });

  it("shows nothing when nothing needs the user", async () => {
    render(<TodayScreen />);
    await screen.findByRole("button", { name: "Prepare CV" });
    expect(screen.queryByRole("group", { name: "Needs you" })).toBeNull();
  });
});

describe("the link box", () => {
  it("adds a job from what is pasted, after it is told apart from text", async () => {
    const user = userEvent.setup();
    mocked.parseInput.mockResolvedValue({ kind: "url", value: "https://acme.test/jobs/9" });
    mocked.addLead.mockResolvedValue({ leadId: 9, state: "new", applicationId: null });
    render(<TodayScreen />);
    await screen.findByRole("button", { name: "Prepare CV" });
    const add = screen.getByRole("button", { name: "Add job" }) as HTMLButtonElement;
    expect(add.disabled).toBe(true);
    const box = screen.getByRole("textbox", { name: "Job link or posting text" }) as HTMLInputElement;
    await user.type(box, "acme.test/jobs/9{Enter}");
    expect(mocked.parseInput).toHaveBeenCalledWith("acme.test/jobs/9");
    expect(mocked.addLead).toHaveBeenCalledWith({ kind: "url", value: "https://acme.test/jobs/9" });
    await waitFor(() => expect(box.value).toBe(""));
  });

  it("says why a job could not be added, with Try again", async () => {
    const user = userEvent.setup();
    mocked.parseInput.mockResolvedValue({ kind: "text", value: "We are hiring" });
    mocked.addLead.mockRejectedValueOnce(new CommandError("text that has no job title")).mockReturnValue(new Promise(() => undefined));
    render(<TodayScreen />);
    await screen.findByRole("button", { name: "Prepare CV" });
    await user.type(screen.getByRole("textbox", { name: "Job link or posting text" }), "We are hiring");
    await user.click(screen.getByRole("button", { name: "Add job" }));
    const failure = await screen.findByRole("group", { name: "The posting could not be added: text that has no job title" });
    await user.click(within(failure).getByRole("button", { name: "Try again" }));
    expect(mocked.addLead).toHaveBeenCalledTimes(2);
    expect(await screen.findByRole("status", { name: "Reading the posting" })).toBeTruthy();
  });

  it("turns a dropped file into a lead, or says why it was refused", async () => {
    mocked.fileInput.mockRejectedValue(new CommandError("c.png: .png files are not supported (use .md, .txt, .html or .pdf)."));
    render(<TodayScreen />);
    await screen.findByRole("button", { name: "Prepare CV" });
    useNav.getState().dropFile("/tmp/c.png");
    expect(await screen.findByText(/c\.png: \.png files are not supported/)).toBeTruthy();
    expect(mocked.fileInput).toHaveBeenCalledWith("/tmp/c.png");
    expect(useNav.getState().droppedFile, "the file is read once").toBeUndefined();
  });
});

describe("Today without leads", () => {
  it("offers Search now as the one primary button", async () => {
    const user = userEvent.setup();
    mocked.readQueue.mockResolvedValue(queue({ items: [], lastRun: null }));
    mocked.searchNow.mockResolvedValue({ finishedAt: "x", saved: 0, waiting: 0, failed: 0 });
    render(<TodayScreen />);
    const empty = await screen.findByRole("group", { name: "No jobs yet" });
    const search = within(empty).getByRole("button", { name: "Search now" });
    expect(primaryButtons()).toEqual([search]);
    expect(screen.getByText("No search yet.")).toBeTruthy();
    // The header has no second "Search now" while the list is empty.
    expect(screen.getAllByRole("button", { name: "Search now" })).toHaveLength(1);
    await user.click(search);
    expect(mocked.searchNow).toHaveBeenCalledTimes(1);
    expect(await screen.findByText("Found 0 new leads")).toBeTruthy();
  });

  it("says there is no source to search yet", async () => {
    mocked.readQueue.mockResolvedValue(queue({ items: [], sources: [] }));
    render(<TodayScreen />);
    expect(await screen.findByText("No job source yet: add one in Settings, then search.")).toBeTruthy();
    expect((screen.getByRole("button", { name: "Search now" }) as HTMLButtonElement).disabled).toBe(true);
  });

  it("shows a search that runs in the background", async () => {
    mocked.readQueue.mockResolvedValue(queue({ items: [], searching: true }));
    render(<TodayScreen />);
    expect((await screen.findAllByText("Searching your sources")).length).toBeGreaterThan(0);
  });

  it("says when the job list could not be read", async () => {
    mocked.readQueue.mockRejectedValue(new CommandError("database is locked"));
    render(<TodayScreen />);
    const banner = await screen.findByRole("group", { name: "The job list could not be read" });
    expect(within(banner).getByText("database is locked")).toBeTruthy();
    expect(screen.getByText("Loading the job list")).toBeTruthy();
  });
});

describe("Today with sources", () => {
  it("filters by source when there are several", async () => {
    const user = userEvent.setup();
    mocked.readQueue.mockResolvedValue(
      queue({ sources: [SOURCE, { ...SOURCE, id: 2, name: "Beta jobs" }], items: [lead(1), lead(2, { sourceId: 2 })] }),
    );
    render(<TodayScreen />);
    await screen.findByRole("button", { name: "Prepare CV" });
    await user.click(screen.getByRole("button", { name: "Beta jobs" }));
    expect(screen.getByText("1 of 2 shown")).toBeTruthy();
    await user.click(screen.getByRole("button", { name: "All" }));
    expect(screen.getByText("2 of 2 shown")).toBeTruthy();
  });
});
