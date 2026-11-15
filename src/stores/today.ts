// State of the Leads stage: the queue, filters, ticks, CVs being written, postings being read, notes
// and banners. CVs finish and sources pause whether or not the Jobs page is shown, and the sidebar
// badge must follow, so this store outlives the page; `startWatching` (once per app run) keeps it
// current. Which lead is open is the nav store's drawer.

import { create } from "zustand";

import { CommandError, needsRecheck, toCommandError } from "@/lib/problem";
import { useNav, type DrawerTarget } from "./nav";
import { useShell } from "./shell";
import type { GraphProgress } from "@/api/events";

import * as api from "@/api/discovery";
import {
  ALREADY_READING,
  clearFailureOf,
  pushFailure,
  queueBehind,
  sameInput,
  type AddFailure,
} from "@/lib/discovery/addQueue";
import { parsePrepareThread, movesTheQueue, prepareLabel, PREPARE_GRAPH } from "@/lib/discovery/progress";
import {
  dropUnreported,
  failureRows,
  needsYouCount,
  neighbourOf,
  NO_FILTER,
  filterItems,
  openLeadId,
  STOPPED,
  type QueueFilter,
  type QueueShow,
  type SessionFailure,
} from "@/lib/discovery/queue";
import { dismissedNote, restoredNote, summaryLine } from "@/lib/discovery/text";
import type {
  Dismissed,
  DismissedJob,
  Job,
  Lead,
  LeadInput,
  PrepareOutcome,
  Queue,
  SearchFinished,
} from "@/types/discovery";

/** A CV that was written: the way into the application. */
export interface Prepared {
  applicationId: number;
  label: string;
}

/** An inline message under the header. */
export interface Note {
  ok: boolean;
  text: string;
  /** What the user can do about a failure. */
  problem?: CommandError["problem"];
}

export interface TodayState {
  /** The queue as last read; `undefined` until the first answer. */
  queue: Queue | undefined;
  loadError: CommandError | undefined;
  /** The profile's skill names (empty when it cannot be read). */
  profileSkills: string[];
  filter: QueueFilter;
  /** Ticked leads (several CVs at once). */
  checked: number[];
  /** Leads whose CV is queued or being written in this session. */
  jobs: Record<number, Job>;
  /** Failures of this session that left no thread behind (no provider, ...), by lead. */
  failed: Record<number, SessionFailure>;
  prepared: Prepared[];
  /** A search started here is running: its progress line. */
  searching: string | undefined;
  note: Note | undefined;
  /** The posting being read ("Reading the posting..."), if any. */
  adding: LeadInput | undefined;
  /** Links pasted and files dropped while one is being read: read one after another. */
  waiting: LeadInput[];
  /** Why the last inputs were not added, newest first (each has its own "Try again"). */
  addFailures: AddFailure[];
  /** A short remark under the link box; the next paste or typing clears it. */
  linkNote: string | undefined;
  /** Threads of paused sources being continued. */
  continuing: string[];
  /** Threads whose browser window is being opened. */
  opening: string[];
  /** Why "Open browser" failed, by thread (cleared by the next try, Continue or Skip). */
  openErrors: Record<string, CommandError>;
  /** The dismissed jobs as last read (read while "Show: Dismissed" is picked); `undefined` until then. */
  dismissed: DismissedJob[] | undefined;
  dismissedError: CommandError | undefined;
  /** Dismissed jobs being restored, by `dismissedKey`. */
  restoring: string[];
}

const INITIAL: TodayState = {
  queue: undefined,
  loadError: undefined,
  profileSkills: [],
  filter: NO_FILTER,
  checked: [],
  jobs: {},
  failed: {},
  prepared: [],
  searching: undefined,
  note: undefined,
  adding: undefined,
  waiting: [],
  addFailures: [],
  linkNote: undefined,
  continuing: [],
  opening: [],
  openErrors: {},
  dismissed: undefined,
  dismissedError: undefined,
  restoring: [],
};

export const useTodayStore = create<TodayState>(() => INITIAL);

/** Back to the start (tests). */
export function resetToday(): void {
  reads = 0;
  dismissedReads = 0;
  latestRead = Promise.resolve();
  jobBatch.clear();
  useTodayStore.setState(INITIAL, true);
}

const get = () => useTodayStore.getState();
const set = (patch: Partial<TodayState>) => useTodayStore.setState(patch);

// ------------------------------------------------------------ selection

/** The leads that pass the filters. */
export function shownLeads(state: TodayState): Lead[] {
  return filterItems(state.queue?.items ?? [], state.filter);
}

/** The lead the drawer asks for (`undefined`: the drawer is closed or shows something else). */
export function requestedLead(drawer: DrawerTarget | undefined): number | "best" | undefined {
  return drawer?.kind === "lead" ? drawer.id : undefined;
}

/** The lead whose drawer is open: the one navigation asked for, when it is listed. */
export function openedLeadId(state: TodayState): number | undefined {
  return openLeadId(shownLeads(state), requestedLead(useNav.getState().drawer));
}

/** Open lead `id` in the drawer of the Leads stage (`"best"`: the best one shown). */
export function selectLead(id: number | "best"): void {
  useNav.getState().openLead(id);
}

/** After lead `id` left the list, its drawer shows the lead next to it, or closes. Only when that
 *  lead's drawer was the one open; the screen shown does not change. */
function openNeighbourOf(id: number): void {
  const state = get();
  if (openedLeadId(state) !== id) {
    return;
  }
  const next = neighbourOf(shownLeads(state), id);
  if (next === undefined) {
    useNav.getState().closeDrawer();
  } else {
    useNav.setState({ drawer: { kind: "lead", id: next } });
  }
}

/** A failure that says a prerequisite is missing: the setup state is read again, so the banner is right. */
function recheck(error: CommandError): void {
  if (needsRecheck(error.problem)) {
    useShell.getState().refresh();
  }
}

// ------------------------------------------------------------ reading

let reads = 0;
/** The latest read of the queue. */
let latestRead: Promise<void> = Promise.resolve();

/** Read the queue (off the UI thread). Only the latest answer is kept. Resolves once the queue
 *  shown is at least as new as this read (a later read that took over has answered too). */
function load(): Promise<void> {
  const read = ++reads;
  const done = api.readQueue().then(
    (queue) => {
      if (read !== reads) {
        return latestRead;
      }
      set({ queue, loadError: undefined, checked: get().checked.filter((id) => queue.items.some((l) => l.id === id)) });
      return undefined;
    },
    (thrown: unknown) => {
      if (read !== reads) {
        return latestRead;
      }
      set({ loadError: toCommandError(thrown) });
      return undefined;
    },
  );
  latestRead = done;
  return done;
}

let dismissedReads = 0;

/** Read the dismissed jobs (off the UI thread). Only the latest answer is kept. */
function loadDismissed(): Promise<void> {
  const read = ++dismissedReads;
  return api.readDismissed().then(
    (dismissed) => {
      if (read === dismissedReads) {
        set({ dismissed, dismissedError: undefined });
      }
    },
    (thrown: unknown) => {
      if (read === dismissedReads) {
        set({ dismissedError: toCommandError(thrown) });
      }
    },
  );
}

/** Read the dismissed jobs again (a failed read has a Retry). */
export function reloadDismissed(): void {
  void loadDismissed();
}

/** The profile changes only when the user reads their documents again, so its skills are read when
 *  the Jobs page is shown, not on every refresh. */
function loadSkills(): void {
  api.readProfileSkills().then(
    (profileSkills) => set({ profileSkills }),
    (thrown: unknown) => console.warn("reading the profile's skills failed", thrown),
  );
}

/** Read the queue again (a failed read has a Retry). */
export function reload(): void {
  load();
}

/** The Jobs page is shown: read everything again. */
export function enter(): void {
  startWatching();
  load();
  loadSkills();
}

// ------------------------------------------------------------ filters and ticks

export function setFilter(patch: Partial<QueueFilter>): void {
  set({ filter: { ...get().filter, ...patch } });
}

/** "Show: Open | Dismissed". The dismissed jobs are read each time they are picked (and again when
 *  leads or applications change while they are shown); the open lead's drawer closes and the ticks
 *  go (neither is in that list, and the bulk bar must not hold the primary button there). */
export function setShow(show: QueueShow): void {
  if (show === get().filter.show) {
    return;
  }
  setFilter({ show });
  if (show === "dismissed") {
    set({ checked: [] });
    if (useNav.getState().drawer?.kind === "lead") {
      useNav.getState().closeDrawer();
    }
    void loadDismissed();
  }
}

/** The key of a dismissed job in `restoring` (and of its Restore button). */
export function dismissedKey(job: Pick<DismissedJob, "kind" | "id">): string {
  return `${job.kind}-${job.id}`;
}

/** "Restore" a dismissed job: a lead goes back to the list, a job from CVs back to the status it had.
 *  The note says where it went. */
export async function restoreJob(job: DismissedJob): Promise<void> {
  const key = dismissedKey(job);
  if (get().restoring.includes(key)) {
    return;
  }
  set({ restoring: [...get().restoring, key] });
  try {
    // The shell says leads and applications changed: the queue and this list are read again then.
    const out = await api.restore(job.kind, job.id);
    set({ note: { ok: out.hidden === null, text: restoredNote(job, out) } });
  } catch (thrown) {
    const error = toCommandError(thrown);
    set({ note: { ok: false, text: error.message, problem: error.problem } });
  } finally {
    set({ restoring: without(get().restoring, key) });
  }
}

export function setChecked(id: number, on: boolean): void {
  const rest = get().checked.filter((c) => c !== id);
  set({ checked: on ? [...rest, id] : rest });
}

/** Untick every lead (the bulk bar's "Clear"). */
export function clearChecked(): void {
  set({ checked: [] });
}

export function setNote(note: Note | undefined): void {
  set({ note });
}

export function closePrepared(index: number): void {
  set({ prepared: get().prepared.filter((_, i) => i !== index) });
}

// ------------------------------------------------------------ the link box

/** Typing again clears the "could not add" messages and the remark. */
export function clearAddFeedback(): void {
  const { addFailures, linkNote } = get();
  if (addFailures.length > 0 || linkNote !== undefined) {
    set({ addFailures: [], linkNote: undefined });
  }
}

/** "Add job" or Enter in the link box. Resolves to `true` when the box should be emptied (what was
 *  typed is taken), `false` when it should stay (blank, unreadable, or too many waiting). */
export async function submitLink(raw: string): Promise<boolean> {
  let input: LeadInput | null;
  try {
    input = await api.parseInput(raw);
  } catch (thrown) {
    const error = toCommandError(thrown);
    set({
      addFailures: pushFailure(
        get().addFailures,
        { message: error.message, input: undefined, problem: error.problem },
        sameInput,
      ),
    });
    return false;
  }
  return input !== null && addInput(input);
}

/** Read the posting behind `input`, score it and add it to the list as a Lead (never an
 *  Application: Prepare CV does that). While another posting is being read this one waits its turn;
 *  `false` only when too many are waiting already. */
export function addInput(input: LeadInput): boolean {
  set({ linkNote: undefined });
  const state = get();
  if (state.adding !== undefined) {
    const queued = queueBehind(state.adding, state.waiting, state.addFailures, input, sameInput);
    set({
      waiting: queued.waiting,
      addFailures: queued.failures,
      linkNote: queued.queued === "duplicate" ? ALREADY_READING : undefined,
    });
    return queued.queued !== "full";
  }
  set({ addFailures: clearFailureOf(state.addFailures, input, sameInput) });
  startAdd(input);
  return true;
}

/** Start reading `input` (nothing else is being read). */
function startAdd(input: LeadInput): void {
  set({ adding: input });
  // Where the user was when they added it: the new lead opens only if they are still there.
  const before = useNav.getState();
  const was = { screen: before.screen, stage: before.stage, drawer: before.drawer };
  api.addLead(input).then(
    (added) => {
      set({ addFailures: clearFailureOf(get().addFailures, input, sameInput) });
      const read = load();
      if (added.state === "promoted" && added.applicationId !== null) {
        // The same job was found before and its CV exists: that is where it is now (the drawer
        // moves the page to its stage).
        useNav.getState().openAppDrawer(added.applicationId);
      } else if (added.state === "dismissed") {
        set({ note: { ok: false, text: "You dismissed this job before: find it under Show: Dismissed." } });
      } else {
        // Show the new lead, whatever the filters were, once the list holds it (the drawer closes
        // on a lead that is not listed).
        set({ filter: NO_FILTER });
        void read.then(() => {
          const now = useNav.getState();
          const moved = now.screen !== was.screen || now.stage !== was.stage || now.drawer !== was.drawer;
          const listed = get().queue?.items.some((lead) => lead.id === added.leadId) ?? false;
          // The user went elsewhere meanwhile, or the list could not be read: leave the page as it is.
          if (!moved && listed) {
            selectLead(added.leadId);
          }
        });
      }
      readNext();
    },
    (thrown: unknown) => {
      const error = toCommandError(thrown);
      recheck(error);
      set({
        addFailures: pushFailure(
          get().addFailures,
          { message: error.message, input, problem: error.problem },
          sameInput,
        ),
      });
      readNext();
    },
  );
}

/** The next posting that was waiting; a failure stays on screen meanwhile. */
function readNext(): void {
  const [next, ...rest] = get().waiting;
  set({ adding: undefined, waiting: rest });
  if (next !== undefined) {
    startAdd(next);
  }
}

/** A file dropped on the window: a lead like a pasted link, or the reason it was refused. */
export async function addDropped(path: string): Promise<void> {
  try {
    addInput(await api.fileInput(path));
  } catch (thrown) {
    const error = toCommandError(thrown);
    set({
      addFailures: pushFailure(
        get().addFailures,
        { message: error.message, input: undefined, problem: error.problem },
        sameInput,
      ),
    });
  }
}

// ------------------------------------------------------------ searching

/** "Search now". */
export async function searchNow(): Promise<void> {
  if (get().searching !== undefined) {
    return;
  }
  set({ searching: "Searching your sources", note: undefined });
  try {
    const run = await api.searchNow();
    set({ note: { ok: true, text: summaryLine(run) } });
  } catch (thrown) {
    const error = toCommandError(thrown);
    recheck(error);
    set({ note: { ok: false, text: error.message, problem: error.problem } });
  } finally {
    set({ searching: undefined });
    load();
  }
}

/** The daily search ended (it runs in the shell, whichever screen is shown). */
export function onSearchFinished(finished: SearchFinished): void {
  if (finished.error !== null) {
    recheck(finished.error);
    set({ note: { ok: false, text: finished.error.message, problem: finished.error.problem } });
  } else if (finished.summary !== null) {
    set({ note: { ok: true, text: summaryLine(finished.summary) } });
  }
  load();
}

// ------------------------------------------------------------ Prepare CV

/** The batch each running job belongs to, so a result that never arrived is told apart from the
 *  result of a later run of the same lead. */
const jobBatch = new Map<number, number>();
let batches = 0;

/** Write the CVs of `requested`, one after the other (the core queues across clicks too). */
export async function prepare(requested: number[]): Promise<void> {
  const state = get();
  const ids = requested.filter((id) => state.jobs[id] === undefined);
  if (ids.length === 0) {
    return;
  }
  const batch = ++batches;
  const jobs = { ...state.jobs };
  const failed = { ...state.failed };
  for (const id of ids) {
    jobs[id] = { state: "queued" };
    delete failed[id];
    jobBatch.set(id, batch);
  }
  // The leads asked for are no longer ticked (also one hidden by a filter or already running);
  // the other ticks stay.
  set({ jobs, failed, checked: state.checked.filter((id) => !requested.includes(id)) });
  try {
    // Each outcome arrives as an event while the batch goes on; the final list is the safety net.
    for (const outcome of await api.prepareCvs(ids)) {
      if (jobBatch.get(outcome.leadId) === batch) {
        applyPrepared(outcome);
      }
    }
  } catch (thrown) {
    console.warn("preparing the CVs stopped early", thrown);
  }
  // The task ended without reporting these (it was cancelled or failed): they are not running any
  // more, whatever the list says.
  const mine = ids.filter((id) => jobBatch.get(id) === batch && get().jobs[id] !== undefined);
  if (mine.length > 0) {
    const dropped = dropUnreported(get().jobs, get().failed, mine, new CommandError(STOPPED).problem);
    set(dropped);
    load();
  }
}

/** One lead's "Prepare CV" ended: the lead leaves the list, and the CV is ready or failed. Does
 *  nothing for a lead that is not being prepared here (an outcome seen twice). */
export function applyPrepared(outcome: PrepareOutcome): void {
  const state = get();
  if (state.jobs[outcome.leadId] === undefined) {
    return;
  }
  const jobs = { ...state.jobs };
  delete jobs[outcome.leadId];
  jobBatch.delete(outcome.leadId);
  // The lead left the queue: open the one next to it.
  openNeighbourOf(outcome.leadId);
  if (outcome.error === null && outcome.applicationId !== null) {
    set({
      jobs,
      prepared: [
        ...state.prepared,
        { applicationId: outcome.applicationId, label: outcome.label ?? `Application ${outcome.applicationId}` },
      ],
    });
  } else {
    const error = outcome.error ?? new CommandError(STOPPED);
    recheck(error);
    set({ jobs, failed: { ...state.failed, [outcome.leadId]: { message: error.message, problem: error.problem } } });
  }
  load();
}

/** Forget a failed "Prepare CV" (its application stays as a Draft). */
export async function forgetFailure(leadId: number): Promise<void> {
  const failed = { ...get().failed };
  delete failed[leadId];
  set({ failed });
  try {
    await api.forgetFailure(leadId);
  } catch (thrown) {
    console.warn("forgetting a failed CV failed", thrown);
  }
  load();
}

/** A lead was dismissed: it leaves the list, and the note says what else went. */
export function afterDismissed(leadId: number, out: Dismissed): void {
  openNeighbourOf(leadId);
  set({
    checked: get().checked.filter((id) => id !== leadId),
    note: { ok: out.notExcluded === null, text: dismissedNote(out) },
  });
  load();
}

// ------------------------------------------------------------ paused sources

function without<T>(list: readonly T[], item: T): T[] {
  return list.filter((x) => x !== item);
}

function withoutKey(record: Readonly<Record<string, CommandError>>, key: string): Record<string, CommandError> {
  const next = { ...record };
  delete next[key];
  return next;
}

/** "Open browser" for a source that waits at a login, captcha or block page: a visible browser on that
 *  page. The user deals with the page; "Continue" goes on. */
export async function openBrowser(thread: string): Promise<void> {
  if (get().opening.includes(thread)) {
    return;
  }
  set({ opening: [...get().opening, thread], openErrors: withoutKey(get().openErrors, thread) });
  try {
    await api.openBrowser(thread);
  } catch (thrown) {
    set({ openErrors: { ...get().openErrors, [thread]: toCommandError(thrown) } });
  } finally {
    set({ opening: without(get().opening, thread) });
  }
}

/** Continue a source that waited for the user (a login, a captcha) or retry a failed one. */
export async function continueSource(thread: string): Promise<void> {
  if (get().continuing.includes(thread)) {
    return;
  }
  set({ continuing: [...get().continuing, thread], openErrors: withoutKey(get().openErrors, thread) });
  try {
    await api.resumeSource(thread);
  } catch (thrown) {
    const error = toCommandError(thrown);
    set({ note: { ok: false, text: error.message, problem: error.problem } });
  } finally {
    set({ continuing: without(get().continuing, thread) });
    load();
  }
}

/** Skip a paused source for this run. */
export async function skipSource(thread: string): Promise<void> {
  set({ openErrors: withoutKey(get().openErrors, thread) });
  try {
    await api.skipSource(thread);
  } catch (thrown) {
    set({ note: { ok: false, text: toCommandError(thrown).message } });
  }
  load();
}

// ------------------------------------------------------------ events

/** A step of a graph run: the line under a lead whose CV is being written, or a source that ended
 *  (its leads are saved by then). */
export function onProgress(p: GraphProgress): void {
  if (p.graph === PREPARE_GRAPH) {
    const id = parsePrepareThread(p.thread);
    const label = prepareLabel(p);
    if (id !== undefined && label !== undefined && get().jobs[id] !== undefined) {
      set({ jobs: { ...get().jobs, [id]: { state: "running", label } } });
    }
  } else if (movesTheQueue(p)) {
    load();
  }
}

/** What waits for the user, written to the sidebar badge so the two never differ. */
function syncNeedsYou(): void {
  const s = get();
  const rows = failureRows(s.queue?.failed ?? [], s.queue?.items ?? [], s.jobs, s.failed);
  useNav.getState().setNeedsYou(needsYouCount(s.queue, rows, s.prepared.length));
}

/** Stored data changed (any screen): what the Leads stage shows is read again. */
export function onChanged(scope: api.Scope): void {
  // The sidebar badge follows every change the list has, shown or not.
  if (scope === "leads" || scope === "sources" || scope === "applications") {
    load();
  }
  // The dismissed jobs only change with leads and applications, and are read only while shown
  // (not on every queue read, such as each step of a search).
  if ((scope === "leads" || scope === "applications") && get().filter.show === "dismissed") {
    void loadDismissed();
  }
}

let watching = false;

/** Follow the shell for as long as the app runs, whether or not the Jobs page is shown: the badge, the CVs
 *  being written and the sources that pause all change on their own. Safe to call again. */
export function startWatching(): void {
  if (watching) {
    return;
  }
  watching = true;
  api.onChanged(onChanged);
  api.onProgress(onProgress);
  api.onPrepared(applyPrepared);
  api.onSearchFinished(onSearchFinished);
  useTodayStore.subscribe(syncNeedsYou);
  syncNeedsYou();
}
