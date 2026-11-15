// Settings > Job sources: how a source's last run reads, and the tag that says it at a glance.

import type { Tone } from "@/lib/tokens";

import type { Source, SourceKind } from "@/types/discovery";

/** Longest status text shown on a row (a failure note can be long; a "links to a board" note fits). */
export const STATUS_CHARS = 120;

/** How the last run went, for the colour of the status tag. `boardHint`: a careers page with no
 *  postings that points to a job board (the status says which one to add). */
export type Outcome = "never" | "ok" | "needsYou" | "failed" | "boardHint" | "other";

/** How the runner's "no postings, but it links to a board" status starts. */
const BOARD_HINT_STATUS = "No postings on the page. It links to a ";

/** `just now`, `5m ago`, `3h ago`, `2d ago`. */
export function compactAgo(now: Date, then: Date): string {
  const minutes = Math.trunc((now.getTime() - then.getTime()) / 60_000);
  if (minutes < 1) {
    return "just now";
  }
  const hours = Math.trunc(minutes / 60);
  if (hours < 1) {
    return `${minutes}m ago`;
  }
  const days = Math.trunc(hours / 24);
  return days < 1 ? `${hours}h ago` : `${days}d ago`;
}

/** The status of a source as one line and its outcome: when it last ran and how it ended. */
export function statusLine(
  source: Pick<Source, "lastRunAt" | "lastStatus">,
  now: Date,
): { line: string; outcome: Outcome } {
  const status = source.lastStatus?.trim() ?? "";
  const at = source.lastRunAt === null ? undefined : new Date(source.lastRunAt);
  const ran = at !== undefined && !Number.isNaN(at.getTime()) ? at : undefined;
  if (ran === undefined && status === "") {
    return { line: "never run", outcome: "never" };
  }
  if (status === "needs_human") {
    return { line: "needs you: continue it from Jobs", outcome: "needsYou" };
  }
  const outcome: Outcome =
    status === "ok"
      ? "ok"
      : status.startsWith("failed")
        ? "failed"
        : status.startsWith(BOARD_HINT_STATUS)
          ? "boardHint"
          : "other";
  const chars = Array.from(status);
  const shown = chars.length > STATUS_CHARS ? `${chars.slice(0, STATUS_CHARS).join("").trimEnd()}...` : status;
  if (ran === undefined) {
    return { line: shown, outcome };
  }
  const when = compactAgo(now, ran);
  return { line: shown === "" ? `ran ${when}` : `${shown} - ${when}`, outcome };
}

/** The tag of a source row: its words and tone. A source paused for the user always "needs you". */
export function sourceTag(outcome: Outcome, waiting: boolean): { text: string; tone: Tone } {
  if (waiting || outcome === "needsYou") {
    return { text: "needs you", tone: "warn" };
  }
  switch (outcome) {
    case "ok":
      return { text: "ok", tone: "ok" };
    case "failed":
      return { text: "failed", tone: "danger" };
    case "never":
      return { text: "new", tone: "neutral" };
    case "boardHint":
      return { text: "see note", tone: "accent" };
    default:
      return { text: "idle", tone: "neutral" };
  }
}

/** The label of a source kind; an unknown kind shows as it is. */
export function kindLabel(kinds: readonly SourceKind[], kind: string): string {
  return kinds.find((k) => k.kind === kind)?.label ?? kind;
}

/** The name of a source row: its own name, else the label of its kind. */
export function rowName(source: Pick<Source, "name" | "kind">, kinds: readonly SourceKind[]): string {
  const name = source.name.trim();
  return name === "" ? kindLabel(kinds, source.kind) : name;
}

/** The line under the name: `Greenhouse boards.greenhouse.io`. */
export function rowAddress(source: Pick<Source, "kind" | "host">, kinds: readonly SourceKind[]): string {
  return `${kindLabel(kinds, source.kind)} ${source.host.trim()}`.trim();
}
