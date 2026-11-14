// The sentences of the screen that are made of numbers from the shell.

import type { Chart } from "./types";

/** Under the title of the digest: when it was made and how much it read. */
export function digestHint(when: string, itemsRead: number | null): string {
  return itemsRead === null ? when : `${when} · ${itemsRead} item(s) read`;
}

/** Under the "Skills in demand" title. */
export function chartHint(chart: Pick<Chart, "fromMonth" | "toMonth" | "documents">): string {
  return (
    `Job postings and leads naming each skill, ${chart.fromMonth} to ${chart.toMonth} (${chart.documents} read). ` +
    "Counted by the app, not by the AI. Hover a skill for its trend."
  );
}

/** Under the primary button: what a run for `periodLabel` does. */
export function collectCaption(periodLabel: string): string {
  return `${periodLabel}: collects from your sources, then your AI provider writes the digest. Every point links to the items it rests on.`;
}

/** The accessible name of a row of the history. */
export function reportRowName(title: string, when: string): string {
  return `${title}, ${when}`;
}

/** The notice when the saved data cannot be read. */
export function readFailure(message: string): string {
  return `Could not read the saved data: ${message}`;
}

/** The notice when a topic could not be saved. */
export function addFailure(topic: string, message: string): string {
  return `Could not add "${topic}" to your preferences: ${message}`;
}
