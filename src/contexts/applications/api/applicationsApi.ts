// The port of this context to the shell: typed functions over the `applications_*` commands and the
// events they send. A function that changes stored data tells the other screens through
// `notifyChanged` once it ended, whether it succeeded or not (a failed job still moved the header).

import { toCommandError, type CommandError, type Problem } from "@/shared";
import { invoke, listen, notifyChanged } from "@/shared/bridge";

import type {
  Apply,
  ApplyProgress,
  ApplicationHeader,
  Board,
  Cv,
  Jd,
  JobProgress,
  MarkKey,
  Primary,
  Rendered,
  ReviewEdits,
  ShotKind,
  Timeline,
} from "../domain/types";

/** How a failure comes over the bridge (`CommandError` of the shell). */
interface RawError {
  message: string;
  problem: Problem;
}

type HeaderDto = Omit<ApplicationHeader, "error"> & { error: RawError | null };
type ApplyDto = Omit<Apply, "error"> & { error: RawError | null };

function failure(raw: RawError | null): CommandError | null {
  return raw === null ? null : toCommandError(raw);
}

/** Bytes from the shell as an image the webview can show (through an object URL). */
function png(bytes: ArrayBuffer): Blob {
  return new Blob([bytes], { type: "image/png" });
}

// ------------------------------------------------------------------ Stages of the Jobs page

/** The application stages with their cards, and the latest events across all applications. */
export function readBoard(): Promise<Board> {
  return invoke<Board>("applications_board");
}

// ------------------------------------------------------------------ Application screen

/** The header of one application; `null` when it does not exist (any more). */
export async function readApplication(id: number): Promise<ApplicationHeader | null> {
  const dto = await invoke<HeaderDto | null>("applications_get", { id });
  return dto === null ? null : header(dto);
}

function header(dto: HeaderDto): ApplicationHeader {
  return { ...dto, error: failure(dto.error) };
}

/** Run `job`, then tell the other screens. A failure is read back from the header. */
async function job(run: () => Promise<void>): Promise<void> {
  try {
    await run();
  } finally {
    notifyChanged("applications");
  }
}

/** "Generate": write the CV of a Draft that has none. Resolves when the job ended. */
export function generate(id: number): Promise<void> {
  return job(() => invoke<void>("applications_generate", { id }));
}

/** "Ask AI": a targeted change to the CV. `yaml` is the editor text, written to `cv.yaml` first. */
export function revise(id: number, instruction: string, yaml: string | null): Promise<void> {
  return job(() => invoke<void>("applications_revise", { id, instruction, yaml }));
}

/** Run the failed job again. */
export function retry(id: number): Promise<void> {
  return job(() => invoke<void>("applications_retry", { id }));
}

/** Hide the failure of the last job. */
export function dismissError(id: number): Promise<void> {
  return job(() => invoke<void>("applications_dismiss_error", { id }));
}

/** "Approve CV": Draft -> CV ready. */
export function approveCv(id: number): Promise<void> {
  return job(() => invoke<void>("applications_approve_cv", { id }));
}

/** Mark the interview round the application is at (`null`: clear it). Only while it is at
 *  Interview. Resolves to the header as it is now. */
export async function setRound(id: number, round: string | null): Promise<ApplicationHeader> {
  try {
    return header(await invoke<HeaderDto>("applications_set_round", { id, round }));
  } finally {
    notifyChanged("applications");
  }
}

/** Mark what the company answered by hand (interview, offer, rejected). Resolves to the header. */
export async function mark(id: number, mark: MarkKey): Promise<ApplicationHeader> {
  try {
    return header(await invoke<HeaderDto>("applications_mark", { id, mark }));
  } finally {
    notifyChanged("applications");
  }
}

// ------------------------------------------------------------------ JD tab

export function readJd(id: number): Promise<Jd> {
  return invoke<Jd>("applications_jd", { id });
}

// ------------------------------------------------------------------ CV tab

export function readCv(id: number): Promise<Cv> {
  return invoke<Cv>("applications_cv", { id });
}

/** Save `yaml`, then parse, fact-check and render it. Rejects when the CV does not parse or compile
 *  (the edit is saved anyway); fact-guard findings come back as warnings. */
export function saveCv(id: number, yaml: string): Promise<Rendered> {
  return invoke<Rendered>("applications_cv_save", { id, yaml }).finally(() => notifyChanged("applications"));
}

/** Preview page `page` (0-based). */
export async function readCvPage(id: number, page: number): Promise<Blob> {
  return png(await invoke<ArrayBuffer>("applications_cv_page", { id, page }));
}

/** Open `profile.yaml` in the user's editor. */
export function openProfile(): Promise<void> {
  return invoke<void>("applications_open_profile");
}

// ------------------------------------------------------------------ Timeline tab

export function readTimeline(id: number): Promise<Timeline> {
  return invoke<Timeline>("applications_timeline", { id });
}

/** Accept (apply the suggested status) or dismiss an email the classifier was unsure about. */
export function resolveEmail(emailId: string, accept: boolean): Promise<void> {
  return job(() => invoke<void>("applications_resolve_email", { emailId, accept }));
}

// ------------------------------------------------------------------ Apply tab

export async function readApply(id: number): Promise<Apply> {
  const dto = await invoke<ApplyDto>("applications_apply_state", { id });
  return { ...dto, error: failure(dto.error) };
}

/** The "Dry run" box. */
export function setDryRun(on: boolean): Promise<void> {
  return invoke<void>("applications_apply_set_dry_run", { on });
}

/** What the user typed so far (a full snapshot). Resolves to the primary action as it is now. */
export function syncApply(id: number, edits: ReviewEdits): Promise<Primary | null> {
  return invoke<Primary | null>("applications_apply_sync", { id, typed: edits.values, remember: edits.remember });
}

/** "Review & Submit": open and fill the form for a review. Nothing is sent. Resolves when it is ready. */
export function prepareApply(id: number): Promise<void> {
  return job(() => invoke<void>("applications_apply_prepare", { id }));
}

/** The explicit confirmation: Submit (or Continue on a form with more steps). */
export function confirmApply(id: number, edits: ReviewEdits): Promise<void> {
  return job(() => invoke<void>("applications_apply_confirm", { id, typed: edits.values, remember: edits.remember }));
}

/** Close the browser and drop the review without submitting. */
export function discardApply(id: number): Promise<void> {
  return job(() => invoke<void>("applications_apply_discard", { id }));
}

/** After a handoff: the same profile in a visible window at the form. */
export function openVisible(id: number): Promise<void> {
  return job(() => invoke<void>("applications_apply_open_visible", { id }));
}

/** "Mark as applied / failed" after finishing by hand. */
export function markApply(id: number, applied: boolean): Promise<void> {
  return job(() => invoke<void>("applications_apply_mark", { id, applied }));
}

/** Save the apply URL the user typed. */
export function saveApplyUrl(id: number, url: string): Promise<void> {
  return job(() => invoke<void>("applications_apply_save_url", { id, url }));
}

/** A screenshot of the run. */
export async function readShot(id: number, kind: ShotKind, index: number): Promise<Blob> {
  return png(await invoke<ArrayBuffer>("applications_apply_image", { id, kind, index }));
}

// ------------------------------------------------------------------ events

/** The generate / revise job of an application moved. Returns the unsubscribe. */
export function onProgress(handler: (progress: JobProgress) => void): () => void {
  return listen<JobProgress>("applications:progress", handler);
}

/** The form run of an application moved. Returns the unsubscribe. */
export function onApplyProgress(handler: (progress: ApplyProgress) => void): () => void {
  return listen<ApplyProgress>("applications:apply-progress", handler);
}
