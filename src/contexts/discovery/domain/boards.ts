// Settings > Job sources: the job board behind a careers page, offered as a source to add instead.
// The shell finds it (one plain read of the page); nothing here reads a page or adds a source.

import type { BoardHint } from "./types";

/** The kind whose address is a careers page: the only one that gets board suggestions. */
export const CAREER_PAGE = "career_page";

/** How long the address must rest before the page is checked (typing never waits for it). */
export const SUGGEST_DELAY_MS = 700;

/** The address to check for boards, or `undefined` when there is nothing to check: another kind, or a
 *  blank or too-short address (no dot, so no host yet). */
export function suggestTarget(kind: string, url: string): string | undefined {
  const address = url.trim();
  return kind === CAREER_PAGE && address.length >= 4 && address.includes(".") ? address : undefined;
}

/** The suggestion's words: `This page lists its jobs on Greenhouse (boards.greenhouse.io/acme). Add it as a Greenhouse source instead?` */
export function suggestionText(hint: Pick<BoardHint, "label" | "input">): string {
  return `This page lists its jobs on ${hint.label} (${hint.input}). Add it as a ${hint.label} source instead?`;
}

/** The id of the suggestion's button: `suggest-board-greenhouse`. */
export function suggestButtonId(hint: Pick<BoardHint, "kind">): string {
  return `suggest-board-${hint.kind.replace(/[^a-z0-9_-]/gi, "")}`;
}
