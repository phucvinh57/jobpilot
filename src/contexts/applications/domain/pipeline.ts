// The application stages of the Jobs page (CVs | Applied | In progress | Done): counts, cards,
// tags and wording.

import type { ActivityItem, Board, BoardCard, StageKey } from "./types";

/** The application stages in order, with the labels the shell gives them. */
export const APP_STAGES: readonly { key: StageKey; label: string }[] = [
  { key: "cv", label: "CVs" },
  { key: "applied", label: "Applied" },
  { key: "inProgress", label: "In progress" },
  { key: "done", label: "Done" },
];

/** How many activity rows show before "Show all". */
export const ACTIVITY_PREVIEW = 8;

/** Every application stage with its label and how many cards it holds (0 until the board is read). */
export function appStages(board: Board | undefined): { key: StageKey; label: string; count: number }[] {
  return APP_STAGES.map(({ key, label }) => {
    const stage = board?.stages.find((s) => s.key === key);
    return { key, label: stage?.label ?? label, count: stage?.cards.length ?? 0 };
  });
}

/** The label of stage `key`. */
export function stageLabel(key: StageKey): string {
  return APP_STAGES.find((s) => s.key === key)?.label ?? key;
}

/** The cards of stage `key` (none when the board has no such stage). */
export function stageCards(board: Board, key: StageKey): BoardCard[] {
  return board.stages.find((s) => s.key === key)?.cards ?? [];
}

/** The stage application `id` is in (`undefined`: not on the board). */
export function stageOf(board: Board, id: number): StageKey | undefined {
  return board.stages.find((s) => s.cards.some((c) => c.id === id))?.key;
}

/** What an empty stage says. */
export function emptyHint(key: StageKey): string {
  switch (key) {
    case "cv":
      return "Pick a lead and press Prepare CV: its tailored CV waits here for your review.";
    case "applied":
      return "Applications you sent wait here for a reply.";
    case "inProgress":
      return "Applications with an interview show up here, from your Gmail replies or marked by hand.";
    case "done":
      return "Offers and rejections end up here.";
  }
}

/** The activity rows shown: the first few, or all of them. */
export function shownActivity(items: readonly ActivityItem[], all: boolean): readonly ActivityItem[] {
  return all ? items : items.slice(0, ACTIVITY_PREVIEW);
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
