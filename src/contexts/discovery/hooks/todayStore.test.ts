import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { CommandError, useNav, type Problem } from "@/shared";

import * as api from "../api/discoveryApi";
import type { AddedLead, Lead, PrepareOutcome, Queue } from "../domain/types";
import {
  addDropped,
  addInput,
  afterDismissed,
  applyPrepared,
  continueSource,
  enter,
  forgetFailure,
  onProgress,
  onSearchFinished,
  openBrowser,
  openedLeadId,
  prepare,
  resetToday,
  searchNow,
  selectLead,
  setChecked,
  setFilter,
  skipSource,
  submitLink,
  useTodayStore,
} from "./todayStore";

vi.mock("../api/discoveryApi");

const mocked = vi.mocked(api);

const NO_ADVICE: Problem = { kind: "other", challenge: null, advice: null, settingsSection: null, retryable: false };

function lead(id: number, extra: Partial<Lead> = {}): Lead {
  return {
    id,
    sourceId: null,
    url: `https://a.test/${id}`,
    company: "Acme",
    role: `Role ${id}`,
    location: "",
    score: 80 - id,
    foundAt: "2026-11-14T09:00:00+00:00",
    reasons: [],
    criteria: null,
    insight: null,
    ...extra,
  };
}

function queue(extra: Partial<Queue> = {}): Queue {
  return { items: [lead(1), lead(2), lead(3)], failed: [], sources: [], waiting: [], lastRun: null, searching: false, ...extra };
}

/** The lead the drawer asks for (`undefined`: closed or showing something else). */
const drawerLead = () => {
  const drawer = useNav.getState().drawer;
  return drawer?.kind === "lead" ? drawer.id : undefined;
};

/** Let pending promise callbacks run. */
const settle = () => new Promise<void>((resolve) => setTimeout(resolve, 0));

/** A promise settled from outside, to hold a command open. */
function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason: unknown) => void;
  const promise = new Promise<T>((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

const ready = (leadId: number, applicationId: number): PrepareOutcome => ({ leadId, applicationId, label: `Acme — Role ${leadId}`, error: null });
const failed = (leadId: number, message: string): PrepareOutcome => ({ leadId, applicationId: null, label: null, error: new CommandError(message) });

beforeEach(() => {
  resetToday();
  useNav.setState({ screen: "jobs", stage: "leads", drawer: undefined, needsYou: 0, droppedFile: undefined, applicationId: undefined });
  mocked.readQueue.mockResolvedValue(queue());
  mocked.readProfileSkills.mockResolvedValue(["Rust"]);
  mocked.onChanged.mockReturnValue(() => undefined);
  mocked.onProgress.mockReturnValue(() => undefined);
  mocked.onPrepared.mockReturnValue(() => undefined);
  mocked.onSearchFinished.mockReturnValue(() => undefined);
});

afterEach(() => {
  vi.clearAllMocks();
});

describe("reading the queue", () => {
  it("reads the queue and the profile's skills when the Jobs page is shown", async () => {
    enter();
    await settle();
    const s = useTodayStore.getState();
    expect(s.queue?.items.map((l) => l.id)).toEqual([1, 2, 3]);
    expect(s.profileSkills).toEqual(["Rust"]);
    expect(s.loadError).toBeUndefined();
  });

  it("keeps the old queue and says why the read failed", async () => {
    enter();
    await settle();
    mocked.readQueue.mockRejectedValue(new CommandError("database is locked"));
    onProgress({ thread: "t", graph: "source_run", kind: "done", node: null, text: null });
    await settle();
    const s = useTodayStore.getState();
    expect(s.queue?.items).toHaveLength(3);
    expect(s.loadError?.message).toBe("database is locked");
  });

  it("opens the lead the drawer asks for when it is listed, and ticks only leads that are listed", async () => {
    enter();
    await settle();
    expect(openedLeadId(useTodayStore.getState()), "the drawer is closed: no lead is open").toBeUndefined();
    selectLead("best");
    expect(openedLeadId(useTodayStore.getState())).toBe(1);
    selectLead(2);
    expect(openedLeadId(useTodayStore.getState())).toBe(2);
    expect(useNav.getState().drawer).toEqual({ kind: "lead", id: 2 });
    selectLead(99);
    expect(openedLeadId(useTodayStore.getState()), "not listed").toBeUndefined();
    useNav.getState().openAppDrawer(5);
    expect(openedLeadId(useTodayStore.getState()), "the drawer shows an application").toBeUndefined();

    setChecked(2, true);
    setChecked(3, true);
    setChecked(2, false);
    expect(useTodayStore.getState().checked).toEqual([3]);
    mocked.readQueue.mockResolvedValue(queue({ items: [lead(1)] }));
    onProgress({ thread: "t", graph: "source_run", kind: "done", node: null, text: null });
    await settle();
    expect(useTodayStore.getState().checked, "a ticked lead that left the list is dropped").toEqual([]);
  });

  it("the filters narrow what is shown", async () => {
    enter();
    await settle();
    setFilter({ minScore: 78 });
    expect(useTodayStore.getState().filter.minScore).toBe(78);
    selectLead(3);
    expect(openedLeadId(useTodayStore.getState()), "lead 3 (score 77) is filtered out: no lead is open").toBeUndefined();
    selectLead("best");
    expect(openedLeadId(useTodayStore.getState()), "the best shown").toBe(1);
  });
});

describe("the sidebar badge", () => {
  it("counts sources waiting, failed CVs and CVs to read", async () => {
    mocked.readQueue.mockResolvedValue(
      queue({
        waiting: [{ thread: "discover:1:3", sourceId: 3, sourceName: "Acme", reason: "login", url: "https://a.test/login" }],
        failed: [{ leadId: 1, company: "Acme", role: "Role 1", message: "boom", problem: NO_ADVICE, applicationId: 5 }],
      }),
    );
    enter();
    await settle();
    expect(useNav.getState().needsYou).toBe(2);

    mocked.prepareCvs.mockResolvedValue([ready(2, 9)]);
    await prepare([2]);
    expect(useNav.getState().needsYou, "a CV to read counts too").toBe(3);
  });
});

describe("Prepare CV", () => {
  it("queues the leads, shows each CV as it is ready and opens the next lead", async () => {
    enter();
    await settle();
    selectLead(1);
    setChecked(1, true);
    const batch = deferred<PrepareOutcome[]>();
    mocked.prepareCvs.mockReturnValue(batch.promise);

    const done = prepare([1, 2]);
    let s = useTodayStore.getState();
    expect(s.jobs).toEqual({ 1: { state: "queued" }, 2: { state: "queued" } });
    expect(s.checked, "the ticks are used up").toEqual([]);

    // The pipeline reports progress for lead 1, then its outcome arrives as an event.
    onProgress({ thread: "lead:1", graph: "prepare_cv", kind: "nodeStarted", node: "generate", text: null });
    expect(useTodayStore.getState().jobs[1]).toEqual({ state: "running", label: "Writing the CV" });

    mocked.readQueue.mockResolvedValue(queue({ items: [lead(2), lead(3)] }));
    applyPrepared(ready(1, 11));
    s = useTodayStore.getState();
    expect(s.jobs).toEqual({ 2: { state: "queued" } });
    expect(s.prepared).toEqual([{ applicationId: 11, label: "Acme — Role 1" }]);
    expect(drawerLead(), "the lead that was open left: the one after it opens").toBe(2);

    applyPrepared(ready(2, 12));
    batch.resolve([ready(1, 11), ready(2, 12)]);
    await done;
    s = useTodayStore.getState();
    expect(s.jobs).toEqual({});
    expect(s.prepared.map((p) => p.applicationId), "an outcome seen twice is shown once").toEqual([11, 12]);
  });

  it("does not move the user to the Jobs page when a CV finishes elsewhere", async () => {
    enter();
    await settle();
    selectLead(1);
    useNav.getState().show("trends");
    mocked.prepareCvs.mockResolvedValue([]);
    void prepare([1]);
    applyPrepared(ready(1, 5));
    expect(useNav.getState().screen).toBe("trends");
    expect(drawerLead(), "its drawer moves on to the next lead, out of sight").toBe(2);
    await settle();
  });

  it("opens no lead when the one prepared was not the one open", async () => {
    enter();
    await settle();
    mocked.prepareCvs.mockResolvedValue([]);
    // The drawer is closed.
    void prepare([1]);
    applyPrepared(ready(1, 5));
    expect(useNav.getState().drawer).toBeUndefined();

    // The drawer shows another lead, then an application.
    selectLead(3);
    void prepare([2]);
    applyPrepared(ready(2, 6));
    expect(drawerLead()).toBe(3);
    useNav.getState().openAppDrawer(6, "cv");
    void prepare([3]);
    applyPrepared(ready(3, 7));
    expect(useNav.getState().drawer).toEqual({ kind: "app", id: 6 });
    await settle();
  });

  it("closes the drawer when the last lead leaves", async () => {
    mocked.readQueue.mockResolvedValue(queue({ items: [lead(1)] }));
    enter();
    await settle();
    selectLead("best");
    mocked.prepareCvs.mockResolvedValue([]);
    void prepare([1]);
    applyPrepared(ready(1, 5));
    expect(useNav.getState().drawer).toBeUndefined();
    await settle();
  });

  it("shows a failed CV with what to do about it, and forgets it on request", async () => {
    enter();
    await settle();
    const batch = deferred<PrepareOutcome[]>();
    mocked.prepareCvs.mockReturnValue(batch.promise);
    const done = prepare([1]);
    applyPrepared(failed(1, "no AI provider is configured"));
    batch.resolve([]);
    await done;
    expect(useTodayStore.getState().failed[1]?.message).toBe("no AI provider is configured");
    expect(useTodayStore.getState().jobs).toEqual({});

    mocked.forgetFailure.mockResolvedValue(undefined);
    await forgetFailure(1);
    expect(useTodayStore.getState().failed).toEqual({});
    expect(mocked.forgetFailure).toHaveBeenCalledWith(1);
  });

  it("skips leads that are already being prepared", async () => {
    enter();
    await settle();
    const batch = deferred<PrepareOutcome[]>();
    mocked.prepareCvs.mockReturnValue(batch.promise);
    const first = prepare([1]);
    await prepare([1]);
    expect(mocked.prepareCvs).toHaveBeenCalledTimes(1);
    batch.resolve([ready(1, 3)]);
    await first;
  });

  it("turns jobs that never reported into failures that can be retried", async () => {
    enter();
    await settle();
    mocked.prepareCvs.mockRejectedValue(new CommandError("the task was cancelled"));
    await prepare([1, 2]);
    const s = useTodayStore.getState();
    expect(s.jobs).toEqual({});
    expect(Object.keys(s.failed).sort()).toEqual(["1", "2"]);
    expect(s.failed[1]?.message).toContain("Try again");

    // Retrying works: the lead is a job again.
    mocked.prepareCvs.mockResolvedValue([ready(1, 4)]);
    await prepare([1]);
    expect(useTodayStore.getState().failed[1]).toBeUndefined();
    expect(useTodayStore.getState().prepared).toHaveLength(1);
  });

  it("uses the final list when an event was missed", async () => {
    enter();
    await settle();
    mocked.prepareCvs.mockResolvedValue([ready(1, 8)]);
    await prepare([1]);
    expect(useTodayStore.getState().prepared).toEqual([{ applicationId: 8, label: "Acme — Role 1" }]);
    expect(useTodayStore.getState().jobs).toEqual({});
  });
});

describe("the link box", () => {
  const url = { kind: "url", value: "https://a.test/jobs/9" } as const;

  it("reads a posting and opens the new lead whatever the filters were", async () => {
    enter();
    await settle();
    setFilter({ minScore: 99, location: "Mars" });
    mocked.parseInput.mockResolvedValue(url);
    const read = deferred<AddedLead>();
    mocked.addLead.mockReturnValue(read.promise);

    expect(await submitLink(" https://a.test/jobs/9 ")).toBe(true);
    expect(useTodayStore.getState().adding).toEqual(url);
    read.resolve({ leadId: 3, state: "new", applicationId: null });
    await settle();
    const s = useTodayStore.getState();
    expect(s.adding).toBeUndefined();
    expect(s.filter).toEqual({ sourceId: null, minScore: null, location: "" });
    expect(useNav.getState().drawer).toEqual({ kind: "lead", id: 3 });
  });

  it("does nothing for a blank box", async () => {
    mocked.parseInput.mockResolvedValue(null);
    expect(await submitLink("   ")).toBe(false);
    expect(mocked.addLead).not.toHaveBeenCalled();
  });

  it("queues what is added while one is read, and reads them in order", async () => {
    const first = deferred<AddedLead>();
    mocked.addLead.mockReturnValueOnce(first.promise).mockResolvedValue({ leadId: 5, state: "new", applicationId: null });
    const b = { kind: "url", value: "https://b.test/1" } as const;
    expect(addInput(url)).toBe(true);
    expect(addInput(b)).toBe(true);
    expect(useTodayStore.getState().waiting).toEqual([b]);
    // The same posting again is not read twice, and the box says so.
    expect(addInput(b)).toBe(true);
    expect(useTodayStore.getState().linkNote).toBe("Already reading this posting");
    expect(useTodayStore.getState().waiting).toHaveLength(1);

    first.resolve({ leadId: 4, state: "new", applicationId: null });
    await settle();
    expect(mocked.addLead).toHaveBeenCalledTimes(2);
    expect(mocked.addLead).toHaveBeenLastCalledWith(b);
    expect(useTodayStore.getState().waiting).toEqual([]);
    expect(useTodayStore.getState().adding).toBeUndefined();
  });

  it("keeps why a posting failed, with its own Try again, and carries on with the next", async () => {
    const problem: Problem = { kind: "network", challenge: null, advice: "The connection failed.", settingsSection: null, retryable: true };
    mocked.addLead.mockRejectedValueOnce(new CommandError("connection refused", problem)).mockResolvedValue({ leadId: 6, state: "new", applicationId: null });
    addInput(url);
    await settle();
    let s = useTodayStore.getState();
    expect(s.adding).toBeUndefined();
    expect(s.addFailures).toEqual([{ message: "connection refused", input: url, problem }]);

    // Try again: its failure goes while it is read again.
    addInput(url);
    s = useTodayStore.getState();
    expect(s.addFailures).toEqual([]);
    await settle();
    expect(useTodayStore.getState().addFailures).toEqual([]);
  });

  it("refuses a full queue and says so", () => {
    mocked.addLead.mockReturnValue(new Promise(() => undefined));
    addInput(url);
    for (let n = 0; n < 10; n += 1) {
      expect(addInput({ kind: "text", value: `posting ${n}` })).toBe(true);
    }
    expect(addInput({ kind: "text", value: "one too many" })).toBe(false);
    expect(useTodayStore.getState().addFailures[0]?.message).toContain("10 postings are already waiting");
  });

  it("says when a job was dismissed before, and shows the application of one that has its CV", async () => {
    mocked.addLead.mockResolvedValueOnce({ leadId: 8, state: "dismissed", applicationId: null });
    addInput(url);
    await settle();
    expect(useTodayStore.getState().note).toEqual({ ok: false, text: "You dismissed this job before, so it is not in the list." });

    mocked.addLead.mockResolvedValueOnce({ leadId: 9, state: "promoted", applicationId: 21 });
    addInput({ kind: "url", value: "https://a.test/jobs/10" });
    await settle();
    // The application's drawer moves the page to its stage once it has read it.
    const nav = useNav.getState();
    expect(nav.screen).toBe("jobs");
    expect(nav.drawer).toEqual({ kind: "app", id: 21 });
  });

  it("a dropped file is a lead like a pasted link, or the reason it was refused", async () => {
    mocked.fileInput.mockResolvedValueOnce({ kind: "file", value: "/tmp/jd.pdf" });
    mocked.addLead.mockResolvedValue({ leadId: 7, state: "new", applicationId: null });
    await addDropped("/tmp/jd.pdf");
    expect(mocked.addLead).toHaveBeenCalledWith({ kind: "file", value: "/tmp/jd.pdf" });

    mocked.fileInput.mockRejectedValueOnce(new CommandError("c.png: .png files are not supported (use .md, .txt, .html or .pdf)."));
    await addDropped("/tmp/c.png");
    const failure = useTodayStore.getState().addFailures[0];
    expect(failure?.message).toContain(".png");
    expect(failure?.input, "a refusal has nothing to retry").toBeUndefined();
  });
});

describe("searching", () => {
  it("shows what a search found", async () => {
    const run = deferred<{ finishedAt: string; saved: number; waiting: number; failed: number }>();
    mocked.searchNow.mockReturnValue(run.promise);
    const done = searchNow();
    expect(useTodayStore.getState().searching).toBe("Searching your sources");
    void searchNow();
    expect(mocked.searchNow, "one search at a time").toHaveBeenCalledTimes(1);
    run.resolve({ finishedAt: "2026-11-14T10:00:00Z", saved: 2, waiting: 1, failed: 0 });
    await done;
    const s = useTodayStore.getState();
    expect(s.searching).toBeUndefined();
    expect(s.note).toEqual({ ok: true, text: "Found 2 new leads, 1 source waiting for you" });
  });

  it("shows why a search failed", async () => {
    mocked.searchNow.mockRejectedValue(new CommandError("no AI provider is configured", { ...NO_ADVICE, kind: "noProvider", advice: "Add one.", settingsSection: "AI providers" }));
    await searchNow();
    const note = useTodayStore.getState().note;
    expect(note?.ok).toBe(false);
    expect(note?.text).toBe("no AI provider is configured");
    expect(note?.problem?.settingsSection).toBe("AI providers");
  });

  it("shows how the daily search ended", () => {
    onSearchFinished({ summary: { finishedAt: "x", saved: 1, waiting: 0, failed: 0 }, error: null });
    expect(useTodayStore.getState().note).toEqual({ ok: true, text: "Found 1 new lead" });
    onSearchFinished({ summary: null, error: new CommandError("connection refused") });
    expect(useTodayStore.getState().note?.ok).toBe(false);
  });
});

describe("paused sources", () => {
  it("opens the browser once at a time and keeps why it could not", async () => {
    const opened = deferred<void>();
    mocked.openBrowser.mockReturnValue(opened.promise);
    const first = openBrowser("discover:1:3");
    void openBrowser("discover:1:3");
    expect(mocked.openBrowser).toHaveBeenCalledTimes(1);
    expect(useTodayStore.getState().opening).toEqual(["discover:1:3"]);
    opened.reject(new CommandError("no Chrome found"));
    await first;
    const s = useTodayStore.getState();
    expect(s.opening).toEqual([]);
    expect(s.openErrors["discover:1:3"]?.message).toBe("no Chrome found");

    // Continue clears it.
    mocked.resumeSource.mockResolvedValue(undefined);
    await continueSource("discover:1:3");
    expect(useTodayStore.getState().openErrors).toEqual({});
    expect(useTodayStore.getState().continuing).toEqual([]);
  });

  it("says why a source could not continue, and skips one", async () => {
    mocked.resumeSource.mockRejectedValue(new CommandError("that source is running right now"));
    await continueSource("discover:1:4");
    expect(useTodayStore.getState().note).toMatchObject({ ok: false, text: "that source is running right now" });

    mocked.skipSource.mockResolvedValue(undefined);
    await skipSource("discover:1:4");
    expect(mocked.skipSource).toHaveBeenCalledWith("discover:1:4");
  });
});

describe("Dismiss", () => {
  it("leaves the list, opens the next lead and says what went", async () => {
    enter();
    await settle();
    selectLead(2);
    setChecked(2, true);
    mocked.readQueue.mockResolvedValue(queue({ items: [lead(1), lead(3)] }));
    afterDismissed(2, { also: 1, kept: 0, notExcluded: null });
    const s = useTodayStore.getState();
    expect(drawerLead()).toBe(3);
    expect(s.checked).toEqual([]);
    expect(s.note).toEqual({ ok: true, text: "Lead dismissed, and 1 more job from the same company." });
    await settle();
    expect(useTodayStore.getState().queue?.items.map((l) => l.id)).toEqual([1, 3]);
  });

  it("opens nothing when the drawer was closed", async () => {
    enter();
    await settle();
    afterDismissed(2, { also: 0, kept: 0, notExcluded: null });
    expect(useNav.getState().drawer).toBeUndefined();
  });

  it("flags a company that could not be excluded", () => {
    afterDismissed(1, { also: 0, kept: 0, notExcluded: "disk full" });
    expect(useTodayStore.getState().note?.ok).toBe(false);
  });
});
