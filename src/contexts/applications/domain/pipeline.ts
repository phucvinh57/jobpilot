// Wording of the Pipeline screen.

import type { ActivityItem, BoardCard } from "./types";

/** The line under the title: how many applications there are. */
export function pipelineSummary(count: number): string {
  switch (count) {
    case 0:
      return "No applications yet.";
    case 1:
      return "1 application. Stages update from your Gmail replies.";
    default:
      return `${count} applications. Stages update from your Gmail replies.`;
  }
}

/** The accessible name of a card: company, role, what happens next and the status. */
export function cardLabel(card: BoardCard): string {
  const head = card.subtitle === "" ? card.title : `${card.subtitle}, ${card.title}`;
  return `${head}: ${card.next.label}, ${card.status}`;
}

/** The accessible name of an activity row. */
export function activityLabel(item: ActivityItem): string {
  return `${item.text}, ${item.when}`;
}

/** The accessible name of a board column: its title and how many cards it holds. */
export function columnLabel(label: string, count: number): string {
  return `${label}, ${count}`;
}
