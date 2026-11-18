// What the shell reports about itself (`shared_status`): the health line, what blocks the main
// flow, and the counts behind the navigation badges.

import type { Problem } from "@/lib/problem";

export interface HealthLine {
  ok: boolean;
  text: string;
  /** The section that helps with the first problem (Settings, or Profile for "About you"). */
  focus: string | null;
}

export interface Blocker {
  id: string;
  title: string;
  problem: Problem;
}

export interface ShellStatus {
  health: HealthLine;
  blockers: Blocker[];
  prefsSaved: boolean;
  /** A profile was read from the user's documents (Profile > About you). */
  profileReady: boolean;
  newLeads: number;
  applications: number;
}

/** A background job (`shared_activity`): a graph run that is running or waiting for the user. */
export interface Activity {
  /** The graph thread: progress events of this thread belong to this job. */
  id: string;
  title: string;
  /** What it does now, or what it needs from the user. */
  status: string;
  state: "running" | "waiting";
  /** The screen that shows it (`settings`: Job sources). */
  screen: "jobs" | "trends" | "settings";
}
