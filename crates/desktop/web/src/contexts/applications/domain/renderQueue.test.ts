import { describe, expect, it } from "vitest";

import { Debounce, EDIT_DEBOUNCE, RenderQueue } from "./renderQueue";

describe("the edit debounce", () => {
  it("waits 800 ms after the last keystroke", () => {
    expect(EDIT_DEBOUNCE).toBe(800);
  });

  it("lets only the last edit survive", () => {
    const d = new Debounce();
    const first = d.touch();
    const second = d.touch();
    expect(d.isLatest(first)).toBe(false);
    expect(d.isLatest(second)).toBe(true);
    d.cancel();
    expect(d.isLatest(second)).toBe(false);
    expect(d.isLatest(d.touch())).toBe(true);
  });
});

describe("the render queue", () => {
  it("causes one more render for the edits that arrive during a render", () => {
    const q = new RenderQueue();
    expect(q.request()).toBe(true); // idle: start
    expect(q.request()).toBe(false); // busy: queued
    expect(q.request()).toBe(false); // busy: still one
    expect(q.finish()).toBe(true); // one repeat
    expect(q.isRunning()).toBe(true);
    expect(q.finish()).toBe(false); // done
    expect(q.isRunning()).toBe(false);
    expect(q.request()).toBe(true);
  });

  it("starts over after a reset", () => {
    const q = new RenderQueue();
    q.request();
    q.request();
    q.reset();
    expect(q.isRunning()).toBe(false);
    expect(q.request()).toBe(true);
    expect(q.finish()).toBe(false);
  });
});
