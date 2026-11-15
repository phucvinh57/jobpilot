// What Settings > About you shows (the shell's `AboutDto`) and the rules only the screen needs:
// the status line, document sizes, the summary lines, when a note can be saved. The profile is
// read-only here: the user changes a document and reads again.

import { ago } from "@/lib/display";
import type { About, Education, Role, Source } from "@/types/profile";

/** The file types a document can be (the shell checks again). */
export const DOCUMENT_EXTENSIONS = ["pdf", "md", "txt"] as const;

// ------------------------------------------------------------------ text

/** `820 characters`, `4.2k characters`, `20k characters`. */
export function sizeLabel(chars: number): string {
  if (chars < 1000) {
    return `${chars} ${chars === 1 ? "character" : "characters"}`;
  }
  const k = chars / 1000;
  return `${k >= 10 ? Math.round(k) : Math.round(k * 10) / 10}k characters`;
}

/** The line under a document's name: `File · 4.2k characters`, `Note · 300 characters`. */
export function sourceLine(source: Source): string {
  return `${source.kind === "file" ? "File" : "Note"} · ${sizeLabel(source.chars)}`;
}

/** The note of a document whose text was cut. */
export function truncatedNote(maxChars: number): string {
  return `Only the first ${sizeLabel(maxChars)} were kept.`;
}

/** The status line: where the reading stands, and whether Re-read is offered. */
export interface StatusLine {
  text: string;
  tone: "muted" | "warn" | "ok";
  reread: boolean;
}

export function statusLine(about: About, reading: boolean, now: Date): StatusLine {
  if (reading) {
    return { text: "Reading your documents…", tone: "muted", reread: false };
  }
  switch (about.status) {
    case "empty":
      return { text: "No documents yet.", tone: "muted", reread: false };
    case "unread":
      return { text: "Not read yet.", tone: "warn", reread: true };
    case "stale":
      return { text: "Out of date: your documents changed since the last read.", tone: "warn", reread: true };
    case "ready": {
      const at = about.builtAt === null ? undefined : new Date(about.builtAt);
      const when = at === undefined || Number.isNaN(at.getTime()) ? "" : ` ${ago(now, at)}`;
      return { text: `Read${when}.`, tone: "ok", reread: true };
    }
  }
}

/** `Position — Company · dates`, leaving out what is missing. */
export function roleLine(role: Role): string {
  const head = [role.position, role.company].filter((s) => s !== "").join(" — ");
  return role.dates === "" ? head : `${head} · ${role.dates}`;
}

/** `Degree — Institution · dates`, leaving out what is missing. */
export function educationLine(education: Education): string {
  return roleLine({ position: education.degree, company: education.institution, dates: education.dates });
}

/** All documents' characters together. */
export function totalChars(sources: readonly Source[]): number {
  return sources.reduce((sum, s) => sum + s.chars, 0);
}

/** Why the note cannot be saved, or `null` when it can. */
export function noteProblem(body: string, about: About): string | null {
  const chars = Array.from(body.trim()).length;
  if (chars === 0) {
    return "Write something about your work first.";
  }
  if (chars > about.limits.maxChars) {
    return `Keep a note under ${sizeLabel(about.limits.maxChars)} (this one has ${chars}).`;
  }
  if (totalChars(about.sources) + chars > about.limits.maxTotalChars) {
    return `All documents together can hold ${sizeLabel(about.limits.maxTotalChars)}: remove one first.`;
  }
  return null;
}
