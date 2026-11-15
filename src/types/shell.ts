// What the shell reports about itself (`shared_status`): the health line, what blocks the main
// flow, and the counts behind the navigation badges.

import type { Problem } from "@/lib/problem";

export interface HealthLine {
  ok: boolean;
  text: string;
  /** The Settings section that helps with the first problem. */
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
  /** A profile was read from the user's documents (Settings > About you). */
  profileReady: boolean;
  newLeads: number;
  applications: number;
}
