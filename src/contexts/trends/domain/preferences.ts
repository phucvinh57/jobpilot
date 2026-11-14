// "Add to preferences": what the button of a topic shows and does.

import type { PrefField, Presence } from "./types";

/** Longest list entry, in characters (the same cut the shell makes). */
const MAX_ITEM_CHARS = 80;

/** What two entries are compared by: trimmed, cut to 80 characters, lower case. */
export function itemKey(text: string): string {
  return Array.from(text.trim()).slice(0, MAX_ITEM_CHARS).join("").toLowerCase();
}

/** The key of a topic of one list: a second click on it waits for the first save. */
export function topicKey(field: PrefField, topic: string): string {
  return `${field}:${itemKey(topic)}`;
}

export type AddButton =
  /** Ready to add. */
  | "add"
  /** The save is running: pressing again does nothing. */
  | "adding"
  /** In the wanted lists (skills, domains). */
  | "added"
  /** In the avoid list: adding it would contradict the preferences. */
  | "avoided";

export function addButton(presence: Presence, pending: boolean): AddButton {
  switch (presence) {
    case "added":
      return "added";
    case "avoided":
      return "avoided";
    case "absent":
      return pending ? "adding" : "add";
  }
}

export function canAdd(state: AddButton): boolean {
  return state === "add";
}

export function addLabel(state: AddButton): string {
  return state === "added" ? "Added" : "Add to preferences";
}

export function addTooltip(state: AddButton): string {
  switch (state) {
    case "add":
    case "adding":
      return "Add to preferences";
    case "added":
      return "Added to your preferences";
    case "avoided":
      return "In your avoid list";
  }
}

export function addAccessibleName(state: AddButton, topic: string): string {
  switch (state) {
    case "add":
    case "adding":
      return `Add ${topic} to preferences`;
    case "added":
      return `${topic} is in your preferences`;
    case "avoided":
      return `${topic} is in your avoid list`;
  }
}
