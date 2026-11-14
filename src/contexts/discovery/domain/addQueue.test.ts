import { describe, expect, it } from "vitest";

import { clearFailureOf, enqueue, fullMessage, MAX_FAILURES, MAX_WAITING, pushFailure, queueBehind, sameInput, type AddFailure } from "./addQueue";
import { waitingLabel } from "./text";

const failure = (message: string, input?: string): AddFailure<string> => ({ message, input });
const messages = (failures: readonly AddFailure<string>[]) => failures.map((f) => f.message);

describe("the add queue", () => {
  it("postings added while one is read wait in order up to a limit", () => {
    let queue: number[] = [];
    expect(waitingLabel(queue.length), "nothing waits, nothing is said").toBeUndefined();
    for (let n = 0; n < MAX_WAITING; n += 1) {
      const next = enqueue<number>(undefined, queue, n);
      expect(next.queued).toBe("waiting");
      queue = next.queue;
    }
    expect(enqueue<number>(undefined, queue, 99).queued, "a full queue refuses the next one").toBe("full");
    expect(queue).toHaveLength(MAX_WAITING);
    // first in, first read
    expect(queue.shift()).toBe(0);
    expect(queue.shift()).toBe(1);
    const again = enqueue<number>(undefined, queue, 99);
    expect(again.queued, "room again after one is taken").toBe("waiting");
    expect(again.queue.at(-1)).toBe(99);
    expect(waitingLabel(1)).toBe("1 waiting");
    expect(waitingLabel(again.queue.length)).toBe("9 waiting");
  });

  it("an input already running or waiting is not queued again", () => {
    expect(enqueue("a", [], "a").queued, "the one being read").toBe("duplicate");
    const b = enqueue("a", [], "b");
    expect(b.queued).toBe("waiting");
    expect(enqueue("a", b.queue, "b").queued, "the one already waiting").toBe("duplicate");
    const a = enqueue<string>(undefined, b.queue, "a");
    expect(a.queued, "nothing is running: it is a new one").toBe("waiting");
    expect(a.queue).toEqual(["b", "a"]);
    // A duplicate is a duplicate even when the queue is full.
    const full = Array.from({ length: MAX_WAITING }, (_, i) => i);
    expect(enqueue<number>(undefined, full, 3).queued).toBe("duplicate");
  });

  it("failures are kept per input, newest first, and each is cleared on its own", () => {
    let failures: AddFailure<string>[] = [];
    failures = pushFailure(failures, failure("a failed", "a"));
    failures = pushFailure(failures, failure("b failed", "b"));
    failures = pushFailure(failures, failure("drop refused"));
    expect(messages(failures), "a batch keeps every failure, newest first").toEqual(["drop refused", "b failed", "a failed"]);

    // Reading `a` again clears its failure only.
    failures = clearFailureOf(failures, "a");
    expect(messages(failures)).toEqual(["drop refused", "b failed"]);
    expect(clearFailureOf(failures, "c"), "an input with no failure changes nothing").toHaveLength(2);

    // The same input failing again replaces its old message and moves to the front.
    failures = pushFailure(failures, failure("b failed again", "b"));
    expect(messages(failures)).toEqual(["b failed again", "drop refused"]);
    // Without an input, the same message is shown once.
    failures = pushFailure(failures, failure("drop refused"));
    expect(messages(failures)).toEqual(["drop refused", "b failed again"]);
  });

  it("a failure goes only when its input is really queued again", () => {
    // `a` failed first, then `b`: the newest comes first.
    const failures = [failure("b failed", "b"), failure("a failed", "a")];

    // `b` is being read: asking for `b` again changes nothing, and `a` keeps its failure.
    const dup = queueBehind("b", [], failures, "b");
    expect(dup.queued).toBe("duplicate");
    expect(messages(dup.failures), "a duplicate clears nothing").toEqual(["b failed", "a failed"]);
    expect(dup.waiting).toEqual([]);

    // `a`'s Retry queues it behind `b`: its failure goes, `b`'s stays.
    const queued = queueBehind("b", [], failures, "a");
    expect(queued.queued).toBe("waiting");
    expect(messages(queued.failures)).toEqual(["b failed"]);
    // `a` again while it waits: a duplicate.
    expect(queueBehind("b", queued.waiting, queued.failures, "a").queued).toBe("duplicate");

    // A full queue refuses and says so, keeping the failure of the refused input.
    const waiting = Array.from({ length: MAX_WAITING }, (_, i) => `w${i + 1}`);
    const full = queueBehind("x", waiting, failures, "b");
    expect(full.queued).toBe("full");
    expect(full.failures).toHaveLength(3);
    expect(full.failures[0], "the refusal is the newest line").toEqual({ message: fullMessage(), input: undefined });
    expect(full.failures.some((f) => f.input === "b"), "b still has its own failure and Retry").toBe(true);
    expect(fullMessage()).toContain(`${MAX_WAITING} postings are already waiting`);
  });

  it("only the newest failures are kept", () => {
    let failures: AddFailure<string>[] = [];
    for (const input of ["a", "b", "c", "d", "e", "f", "g"]) {
      failures = pushFailure(failures, failure(input, input));
    }
    expect(failures).toHaveLength(MAX_FAILURES);
    expect(failures[0]?.message).toBe("g");
    expect(failures.at(-1)?.message).toBe("c");
  });

  it("two inputs are the same posting when their kind and value are", () => {
    expect(sameInput({ kind: "url", value: "https://a.test/1" }, { kind: "url", value: "https://a.test/1" })).toBe(true);
    expect(sameInput({ kind: "url", value: "x" }, { kind: "text", value: "x" })).toBe(false);
    expect(sameInput({ kind: "file", value: "/a" }, { kind: "file", value: "/b" })).toBe(false);
  });
});
