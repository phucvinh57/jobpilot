// Typed functions over the `trends_*` commands. A function that changes stored data
// tells the other screens.

import { invoke, notifyChanged } from "./bridge";

import type { Job, JobKind } from "@/lib/trends/jobs";
import type { Added, Cadence, Overview, PrefField, RunResult, SourceKind } from "@/types/trends";

/** Everything the screen shows. `selected`: the report to show (the newest when `undefined`);
 *  `cadence`: the period a new run would cover. */
export function overview(selected: number | undefined, cadence: Cadence): Promise<Overview> {
  return invoke<Overview>("trends_overview", { selected: selected ?? null, cadence });
}

const RUN_COMMAND: Record<JobKind, string> = {
  collect: "trends_collect",
  regenerate: "trends_regenerate",
  resume: "trends_resume",
  resumeDigest: "trends_resume_digest",
};

/** Run `job` and resolve when it ended. A failed run rejects with its `Problem`. Progress goes out
 *  as `shared:graph-progress` events (graphs `trends` and `trends_digest`). */
export async function runJob(job: Job): Promise<RunResult> {
  try {
    return await invoke<RunResult>(RUN_COMMAND[job.kind], { period: job.period });
  } finally {
    // Even a run that failed or paused may have stored items and moved a source's status.
    notifyChanged("trends");
  }
}

/** Add a source. Hacker News and YC need nothing more; a feed or page needs `url`. */
export async function addSource(kind: SourceKind, name: string, url: string): Promise<void> {
  await invoke<void>("trends_add_source", { kind, name, url });
  notifyChanged("trends");
}

export async function removeSource(id: number): Promise<void> {
  await invoke<void>("trends_remove_source", { id });
  notifyChanged("trends");
}

export async function setSourceEnabled(id: number, enabled: boolean): Promise<void> {
  await invoke<void>("trends_set_source_enabled", { id, enabled });
  notifyChanged("trends");
}

/** Put a skill or domain of the digest into the saved preferences. */
export async function addToPreferences(field: PrefField, topic: string): Promise<Added> {
  const added = await invoke<Added>("trends_add_to_preferences", { field, topic });
  // The Profile screen and the setup banner follow the saved preferences.
  notifyChanged("profile");
  return added;
}
