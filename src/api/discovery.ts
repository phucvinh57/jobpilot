// Typed functions over the `discovery_*` commands and the
// events they send. A function that changes stored data tells the other screens through
// `notifyChanged` once it succeeded.

import { toCommandError } from "@/lib/problem";
import type { GraphProgress } from "./bridge";
import { invoke, listen, notifyChanged, onDataChanged, onGraphProgress, type Scope } from "./bridge";

import type {
  AddedLead,
  BoardHint,
  Dismissed,
  JobSources,
  LeadInput,
  PrepareOutcome,
  Queue,
  RunSummary,
  SearchFinished,
  Source,
} from "@/types/discovery";

// ------------------------------------------------------------------ Leads: reading

/** Everything the Leads stage shows. */
export function readQueue(): Promise<Queue> {
  return invoke<Queue>("discovery_queue");
}

/** The profile's skill names (empty when it cannot be read). */
export function readProfileSkills(): Promise<string[]> {
  return invoke<string[]>("discovery_profile_skills");
}

// ------------------------------------------------------------------ Leads: the link box

/** What a pasted link, path or text is; `null` for a blank box. */
export function parseInput(raw: string): Promise<LeadInput | null> {
  return invoke<LeadInput | null>("discovery_parse_input", { raw });
}

/** A file dropped on the window: a lead input, or a rejection whose message says why it is refused. */
export function fileInput(path: string): Promise<LeadInput> {
  return invoke<LeadInput>("discovery_file_input", { path });
}

/** Read the posting behind `input`, score it and add it as a Lead. Takes seconds to about a minute. */
export async function addLead(input: LeadInput): Promise<AddedLead> {
  const added = await invoke<AddedLead>("discovery_add_lead", { input });
  notifyChanged("leads");
  return added;
}

// ------------------------------------------------------------------ Leads: acting on leads

interface PrepareOutcomeDto {
  leadId: number;
  applicationId: number | null;
  label: string | null;
  error: { problem: unknown; message: string } | null;
}

function toOutcome(dto: PrepareOutcomeDto): PrepareOutcome {
  return { ...dto, error: dto.error === null ? null : toCommandError(dto.error) };
}

/** "Prepare CV" on `leadIds`, one after the other. Each outcome also arrives through [`onPrepared`]
 *  as soon as it is known; this resolves with all of them when the batch is done. */
export async function prepareCvs(leadIds: number[]): Promise<PrepareOutcome[]> {
  const outcomes = await invoke<PrepareOutcomeDto[]>("discovery_prepare_cvs", { leadIds });
  notifyChanged("applications", "leads");
  return outcomes.map(toOutcome);
}

/** Forget a failed "Prepare CV" (its application stays as a Draft). */
export async function forgetFailure(leadId: number): Promise<void> {
  await invoke<void>("discovery_forget_failure", { leadId });
  notifyChanged("leads");
}

/** "Dismiss", with an optional reason; `excludeCompany` keeps the company out of later searches. */
export async function dismissLead(leadId: number, reason: string, excludeCompany: boolean): Promise<Dismissed> {
  const out = await invoke<Dismissed>("discovery_dismiss_lead", { leadId, reason, excludeCompany });
  notifyChanged("leads");
  return out;
}

// ------------------------------------------------------------------ Leads: searching

/** "Search now": every enabled job source. Resolves when each is done, paused or failed. */
export async function searchNow(): Promise<RunSummary> {
  try {
    return await invoke<RunSummary>("discovery_search_now");
  } finally {
    // Sources and leads change even when a source failed on the way.
    notifyChanged("leads", "sources");
  }
}

/** "Open browser" on the page a source is paused at (the user deals with it; the app never does). */
export function openBrowser(thread: string): Promise<void> {
  return invoke<void>("discovery_open_browser", { thread });
}

/** "Continue" a paused source, or retry a failed one. */
export async function resumeSource(thread: string): Promise<void> {
  try {
    await invoke<void>("discovery_resume", { thread });
  } finally {
    notifyChanged("leads", "sources");
  }
}

/** "Skip" a paused source for this run. */
export async function skipSource(thread: string): Promise<void> {
  try {
    await invoke<void>("discovery_skip", { thread });
  } finally {
    notifyChanged("leads", "sources");
  }
}

// ------------------------------------------------------------------ events

/** One lead's "Prepare CV" ended (while its batch goes on). Returns the unsubscribe. */
export function onPrepared(handler: (outcome: PrepareOutcome) => void): () => void {
  return listen<PrepareOutcomeDto>("discovery:prepared", (dto) => {
    notifyChanged("applications", "leads");
    handler(toOutcome(dto));
  });
}

/** The daily search ended. Returns the unsubscribe. */
export function onSearchFinished(handler: (finished: SearchFinished) => void): () => void {
  return listen<{ summary: RunSummary | null; error: { problem: unknown; message: string } | null }>(
    "discovery:search-finished",
    (dto) => {
      handler({ summary: dto.summary, error: dto.error === null ? null : toCommandError(dto.error) });
    },
  );
}

/** Stored data changed (any screen). Returns the unsubscribe. */
export function onChanged(handler: (scope: Scope) => void): () => void {
  return onDataChanged(handler);
}

/** A graph run moved. Returns the unsubscribe. */
export function onProgress(handler: (progress: GraphProgress) => void): () => void {
  return onGraphProgress(handler);
}

export type { Scope };

// ------------------------------------------------------------------ Settings > Job sources

export function readJobSources(): Promise<JobSources> {
  return invoke<JobSources>("discovery_job_sources");
}

/** Add a job source. The adapter of `kind` must accept `url`; a blank `name` becomes the board name. */
export async function addSource(kind: string, name: string, url: string): Promise<Source> {
  const source = await invoke<Source>("discovery_add_source", { kind, name, url });
  notifyChanged("sources");
  return source;
}

/** The job boards a careers page points to (the shell reads the page once; nothing is added). */
export function suggestBoards(url: string): Promise<BoardHint[]> {
  return invoke<BoardHint[]>("discovery_suggest_boards", { url });
}

export async function setSourceEnabled(id: number, enabled: boolean): Promise<void> {
  await invoke<void>("discovery_set_source_enabled", { id, enabled });
  notifyChanged("sources");
}

/** Bind (or with `null` unbind) the browser session a source is read in. */
export async function setSourceSession(id: number, sessionId: number | null): Promise<void> {
  await invoke<void>("discovery_set_source_session", { id, sessionId });
  notifyChanged("sources");
}

/** Remove a source (the leads it found stay). */
export async function removeSource(id: number): Promise<void> {
  await invoke<void>("discovery_remove_source", { id });
  notifyChanged("sources", "leads");
}

/** "Search daily while JobPilot is open". */
export async function setDaily(on: boolean): Promise<void> {
  await invoke<void>("discovery_set_daily", { on });
  notifyChanged("sources");
}
