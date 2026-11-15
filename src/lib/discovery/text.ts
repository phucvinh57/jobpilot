// Small text rules of Today: links, meta lines, the sentences about a search and a dismissal.

import { ago } from "@/lib/display";

import type { Dismissed, RunSummary, Source } from "@/types/discovery";

/** A link the system browser may open: http(s) only. */
export function isWebLink(url: string): boolean {
  const lower = url.trim().toLowerCase();
  return lower.startsWith("https://") || lower.startsWith("http://");
}

/** The line under a lead's name: location and source. */
export function metaLine(location: string, source?: string | null): string {
  const place = location.trim();
  const parts = [place === "" ? undefined : place, source ?? undefined].filter((p): p is string => p !== undefined);
  return parts.length === 0 ? "location not stated" : parts.join(" · ");
}

/** `text` cut to `max` characters, with an ellipsis when something was cut (a long link in a banner). */
export function shorten(text: string, max: number): string {
  const chars = Array.from(text);
  if (chars.length <= max) {
    return text;
  }
  return `${chars.slice(0, Math.max(0, max - 1)).join("")}…`;
}

/** `skills` -> `Skills`. */
export function capitalize(text: string): string {
  const chars = Array.from(text);
  const first = chars[0];
  return first === undefined ? "" : first.toUpperCase() + chars.slice(1).join("");
}

/** `Company — Role`: how a lead or an application is named in a banner. */
export function leadTitle(company: string, role: string): string {
  return `${company} — ${role}`;
}

/** The reasons under a lead's name: `skills 90: Rust and Postgres match · location 100: remote`. */
export function reasonsLine(reasons: readonly { criterion: string; score: number; text: string }[]): string {
  return reasons.map((r) => `${r.criterion} ${r.score}: ${r.text}`).join(" · ");
}

/** `Saturday, 14 November`: the date above the page title. */
export function todayLabel(date: Date): string {
  const weekday = date.toLocaleDateString("en-US", { weekday: "long" });
  const month = date.toLocaleDateString("en-US", { month: "long" });
  return `${weekday}, ${date.getDate()} ${month}`;
}

/** `Found 3 new leads, 1 source waiting for you` for the header and the note after a search. */
export function summaryLine(s: Pick<RunSummary, "saved" | "waiting" | "failed">): string {
  const plural = (n: number, one: string, many: string) => `${n} ${n === 1 ? one : many}`;
  const parts = [`found ${plural(s.saved, "new lead", "new leads")}`];
  if (s.waiting > 0) {
    parts.push(`${plural(s.waiting, "source", "sources")} waiting for you`);
  }
  if (s.failed > 0) {
    parts.push(`${plural(s.failed, "source", "sources")} failed`);
  }
  return capitalize(parts.join(", "));
}

/** The header's last-search line: `Last search 5 min ago: Found 3 new leads`. */
export function lastSearchLine(run: RunSummary, now: Date): string {
  const finished = new Date(run.finishedAt);
  const when = Number.isNaN(finished.getTime()) ? "earlier" : ago(now, finished);
  return `Last search ${when}: ${summaryLine(run)}`;
}

/** What to tell the user after "Dismiss". */
export function dismissedNote(out: Dismissed): string {
  if (out.notExcluded !== null) {
    return `Lead dismissed, but the company could not be excluded: ${out.notExcluded}`;
  }
  const more = (n: number) => (n === 1 ? "1 more job" : `${n} more jobs`);
  let text = out.also === 0 ? "Lead dismissed." : `Lead dismissed, and ${more(out.also)} from the same company.`;
  if (out.kept > 0) {
    const verb = out.kept === 1 ? "was" : "were";
    text += ` ${more(out.kept)} from that company ${verb} kept: the CV is being prepared.`;
  }
  return text;
}

// ------------------------------------------------------------ sources

/** The source's name, or its kind when it has none. */
export function sourceName(s: Pick<Source, "name" | "kind">): string {
  const name = s.name.trim();
  return name === "" ? s.kind : name;
}

/** How the last run of a source ended, in words: `not searched yet`, `ok, 5 min ago`, `needs you`. */
export function sourceStatus(s: Pick<Source, "lastStatus" | "lastRunAt">, now: Date): string {
  const raw = s.lastStatus?.trim() ?? "";
  if (raw === "") {
    return "not searched yet";
  }
  const status = raw === "needs_human" ? "needs you" : raw.replaceAll("_", " ");
  const at = s.lastRunAt === null ? undefined : new Date(s.lastRunAt);
  return at === undefined || Number.isNaN(at.getTime()) ? status : `${status}, ${ago(now, at)}`;
}

/** Does this status line need the user's attention (colour it)? */
export function sourceNeedsAttention(s: Pick<Source, "lastStatus">): boolean {
  return s.lastStatus !== null && (s.lastStatus === "needs_human" || s.lastStatus.startsWith("failed"));
}

/** `3 waiting` next to the spinner; `undefined` when nothing waits. */
export function waitingLabel(waiting: number): string | undefined {
  return waiting > 0 ? `${waiting} waiting` : undefined;
}
