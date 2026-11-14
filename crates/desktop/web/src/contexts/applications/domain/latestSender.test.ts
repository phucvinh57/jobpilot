import { describe, expect, it } from "vitest";

import { LatestSender } from "./latestSender";

function gate() {
  let open: () => void = () => undefined;
  const opened = new Promise<void>((resolve) => {
    open = resolve;
  });
  return { opened, open };
}

describe("the latest-wins sender", () => {
  it("sends a lone value at once", async () => {
    const sent: number[] = [];
    const sender = new LatestSender<number>(async (v) => {
      sent.push(v);
    });
    sender.push(1);
    await sender.idle();
    expect(sent).toEqual([1]);
  });

  it("skips the values that were replaced while a send was running", async () => {
    const sent: number[] = [];
    const first = gate();
    const sender = new LatestSender<number>(async (v) => {
      sent.push(v);
      if (v === 1) {
        await first.opened;
      }
    });
    sender.push(1);
    sender.push(2);
    sender.push(3);
    first.open();
    await sender.idle();
    expect(sent).toEqual([1, 3]);
  });

  it("keeps going after a failed send", async () => {
    const sent: number[] = [];
    const sender = new LatestSender<number>(async (v) => {
      sent.push(v);
      if (v === 1) {
        throw new Error("boom");
      }
    });
    sender.push(1);
    await sender.idle();
    sender.push(2);
    await sender.idle();
    expect(sent).toEqual([1, 2]);
  });

  it("is idle when nothing was pushed", async () => {
    await new LatestSender<number>(async () => undefined).idle();
  });
});
