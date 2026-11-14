// Pure rules of the design kit (no React): how full a bar is, what tone a score has, where a
// step stands.

import type { Tone } from "../theme/tokens";

/** How full a bar is: `value / max` clamped to 0..=1 (an empty scale is 0). */
export function barFraction(value: number, max: number): number {
  if (max <= 0 || Number.isNaN(value)) {
    return 0;
  }
  return Math.min(1, Math.max(0, value / max));
}

/** The tone of a match score: high is good, mid needs a look, low is bad, none is neutral. */
export function scoreTone(score: number | null | undefined): Tone {
  if (score === null || score === undefined) {
    return "neutral";
  }
  if (score >= 75) {
    return "ok";
  }
  if (score >= 50) {
    return "warn";
  }
  return "danger";
}

/** The score as shown: a rounded number, or a dash when the Lead was never scored. */
export function scoreLabel(score: number | null | undefined): string {
  return score === null || score === undefined ? "–" : score.toFixed(0);
}

export type StepState = "done" | "current" | "todo";

/** The state of step `index` when `current` is the step in progress (`current == steps.length`:
 *  every step is done). */
export function stepState(index: number, current: number): StepState {
  if (index < current) {
    return "done";
  }
  return index === current ? "current" : "todo";
}

/** `just now`, `5 min ago`, `3 h ago`, `2 d ago`. */
export function ago(now: Date, then: Date): string {
  const minutes = Math.floor((now.getTime() - then.getTime()) / 60_000);
  if (minutes < 1) {
    return "just now";
  }
  if (minutes < 60) {
    return `${minutes} min ago`;
  }
  if (minutes < 60 * 24) {
    return `${Math.floor(minutes / 60)} h ago`;
  }
  return `${Math.floor(minutes / (60 * 24))} d ago`;
}
