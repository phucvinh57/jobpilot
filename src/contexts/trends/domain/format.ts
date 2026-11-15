// The sentences of the screen that are made of numbers from the shell.

import type { Chart, ChartScope, ScopeKey, Term } from "./types";

/** Under the title of the digest: when it was made and how much it read. */
export function digestHint(when: string, itemsRead: number | null): string {
  return itemsRead === null ? when : `${when} · ${itemsRead} item(s) read`;
}

/** Under the "Skills in demand" title: which postings the bars are a share of. A report saved
 *  before the app knew the user's place counted postings and leads together. */
export function chartHint(
  chart: Pick<Chart, "fromMonth" | "toMonth" | "marks">,
  scope: Pick<ChartScope, "label" | "covers" | "documents">,
): string {
  const window = `${chart.fromMonth} to ${chart.toMonth}`;
  if (!chart.marks) {
    return (
      `Job postings and leads naming each skill, ${window} (${scope.documents} read), as a % of them. ` +
      "Counted by the app, not by the AI. Hover a skill for its trend."
    );
  }
  return (
    `${scope.label} market: ${scope.covers}, ${scope.documents} hiring postings, ${window}. ` +
    "Each bar is the % of postings naming the skill, counted by the app, not by the AI. Hover a skill for its trend."
  );
}

/** The scope the chart shows: the one picked, else the one the gap was judged on, else the first. */
export function pickScope(chart: Pick<Chart, "scopes" | "scope">, picked: ScopeKey | undefined): ChartScope | undefined {
  const find = (key: ScopeKey | undefined) => chart.scopes.find((s) => s.key === key);
  return find(picked) ?? find(chart.scope) ?? chart.scopes[0];
}

/** The accessible name and tooltip of a chart row: the share, the direction, the marks and the counts. */
export function skillRowName(skill: {
  name: string;
  share: string;
  description: string;
  direction: { label: string } | null;
  inProfile: boolean;
  wished: boolean;
}): string {
  const parts = [skill.share];
  if (skill.direction) {
    parts.push(skill.direction.label);
  }
  if (skill.inProfile) {
    parts.push("in your profile");
  }
  if (skill.wished) {
    parts.push("in your wishes");
  }
  return `${skill.name}: ${parts.join(" · ")} · ${skill.description}`;
}

/** The terms the skill list does not know, with how many items name each. */
export function termsLine(terms: Term[]): string {
  return terms.map((t) => `${t.term} (${t.items} item${t.items === 1 ? "" : "s"})`).join(", ");
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
