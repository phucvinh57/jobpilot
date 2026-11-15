// Settings > Job sources: the job board behind a careers page, offered as a source to add instead.
// The shell finds it (one plain read of the page); nothing here reads a page or adds a source.

import type { BoardHint } from "@/types/discovery";

/** The kind whose address is a careers page: the only one that gets board suggestions. */
export const CAREER_PAGE = "career_page";

/** How long the address must rest before the page is checked (leaving the field checks at once;
 *  typing never waits for it). Long enough that a pause mid-word does not send a request. */
export const SUGGEST_DELAY_MS = 1500;

/** Most suggestions shown under the form. */
export const MAX_SUGGESTIONS = 3;

/** The address to check for boards, or `undefined` when there is nothing to check: another kind, or an
 *  address that is not yet a web address with a whole host (`acme.co`, not `acme` or `acme.c`). */
export function suggestTarget(kind: string, url: string): string | undefined {
  const address = url.trim();
  if (kind !== CAREER_PAGE || address === "" || /\s/.test(address)) {
    return undefined;
  }
  let parsed: URL;
  try {
    parsed = new URL(address.includes("://") ? address : `https://${address}`);
  } catch {
    return undefined;
  }
  const labels = parsed.hostname.split(".");
  const top = labels[labels.length - 1] ?? "";
  const web = parsed.protocol === "https:" || parsed.protocol === "http:";
  return web && labels.length >= 2 && labels.every((l) => l !== "") && /^[a-z]{2,}$/i.test(top) ? address : undefined;
}

/** The suggestion's words: `This page lists its jobs on Greenhouse (boards.greenhouse.io/acme). Add it as a Greenhouse source instead?` */
export function suggestionText(hint: Pick<BoardHint, "label" | "input">): string {
  return `This page lists its jobs on ${hint.label} (${hint.input}). Add it as a ${hint.label} source instead?`;
}

/** The suggestions to show: at most [`MAX_SUGGESTIONS`], each with ids that stay unique when two boards
 *  have the same kind: banner `board-suggestion-<kind>-<n>`, button `suggest-board-<kind>-<n>`. */
export function shownSuggestions(
  hints: readonly BoardHint[],
): { hint: BoardHint; bannerId: string; buttonId: string }[] {
  return hints.slice(0, MAX_SUGGESTIONS).map((hint, n) => {
    const kind = hint.kind.replace(/[^a-z0-9_-]/gi, "");
    return { hint, bannerId: `board-suggestion-${kind}-${n}`, buttonId: `suggest-board-${kind}-${n}` };
  });
}
