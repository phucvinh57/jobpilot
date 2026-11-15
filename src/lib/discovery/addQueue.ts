// The link box takes one posting at a time: what is pasted or dropped while one is being read waits
// its turn, and what could not be added is shown under the box with its own "Try again".
// Everything here is immutable: each function returns the new lists.

import type { Problem } from "@/lib/problem";

import type { LeadInput } from "@/types/discovery";

/** Most postings that may wait behind the one being read (links pasted or files dropped meanwhile). */
export const MAX_WAITING = 10;

/** Most failures kept under the link box: the oldest fall off. */
export const MAX_FAILURES = 5;

/** What the link box says when a posting is pasted that is being read or already waits. */
export const ALREADY_READING = "Already reading this posting";

/** What became of an input put behind the posting that is being read. */
export type Queued = "waiting" | "duplicate" | "full";

/** Two inputs are the same posting: same kind, same value. */
export function sameInput(a: LeadInput, b: LeadInput): boolean {
  return a.kind === b.kind && a.value === b.value;
}

type Same<T> = (a: T, b: T) => boolean;
const identical = <T>(a: T, b: T) => a === b;

/** An input (link, file, text) that did not become a lead, with what "Try again" runs. */
export interface AddFailure<T = LeadInput> {
  message: string;
  /** What "Try again" runs; `undefined` for a refusal with nothing to retry (a drop that was refused,
   *  a full queue). */
  input: T | undefined;
  /** What the user can do about it (advice, Settings shortcut, Retry). */
  problem?: Problem;
}

/** Put `item` at the end of `queue`, unless it is the `running` one or one already waiting (a Retry
 *  or a second paste of the same link would read the same posting twice), or [`MAX_WAITING`] wait
 *  already. */
export function enqueue<T>(
  running: T | undefined,
  queue: readonly T[],
  item: T,
  same: Same<T> = identical,
): { queued: Queued; queue: T[] } {
  if ((running !== undefined && same(running, item)) || queue.some((q) => same(q, item))) {
    return { queued: "duplicate", queue: [...queue] };
  }
  if (queue.length >= MAX_WAITING) {
    return { queued: "full", queue: [...queue] };
  }
  return { queued: "waiting", queue: [...queue, item] };
}

/** Show `failure` first. It replaces the one of the same input (or, without an input, the one with
 *  the same message); at most [`MAX_FAILURES`] are kept. */
export function pushFailure<T>(
  failures: readonly AddFailure<T>[],
  failure: AddFailure<T>,
  same: Same<T> = identical,
): AddFailure<T>[] {
  const kept = failures.filter((f) => {
    if (f.input !== undefined && failure.input !== undefined) {
      return !same(f.input, failure.input);
    }
    if (f.input === undefined && failure.input === undefined) {
      return f.message !== failure.message;
    }
    return true;
  });
  return [failure, ...kept].slice(0, MAX_FAILURES);
}

/** Forget the failure of `input` (it is being read again, or was read): the others stay. */
export function clearFailureOf<T>(
  failures: readonly AddFailure<T>[],
  input: T,
  same: Same<T> = identical,
): AddFailure<T>[] {
  return failures.filter((f) => f.input === undefined || !same(f.input, input));
}

/** The message a refused input (a full queue) shows. */
export function fullMessage(): string {
  return `${MAX_WAITING} postings are already waiting: add this one when they have been read.`;
}

/** Put `input` behind the posting being read (`running`), and keep the failures honest about it: the
 *  failure of `input` goes only when it is really queued again; a duplicate or a full queue leaves
 *  every failure as it was (a full queue adds its own, with nothing to retry). */
export function queueBehind<T>(
  running: T | undefined,
  waiting: readonly T[],
  failures: readonly AddFailure<T>[],
  input: T,
  same: Same<T> = identical,
): { queued: Queued; waiting: T[]; failures: AddFailure<T>[] } {
  const { queued, queue } = enqueue(running, waiting, input, same);
  switch (queued) {
    case "waiting":
      return { queued, waiting: queue, failures: clearFailureOf(failures, input, same) };
    case "duplicate":
      return { queued, waiting: queue, failures: [...failures] };
    case "full":
      return {
        queued,
        waiting: queue,
        failures: pushFailure(failures, { message: fullMessage(), input: undefined }, same),
      };
  }
}
