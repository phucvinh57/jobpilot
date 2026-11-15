import { describe, expect, it } from "vitest";

import { CommandError } from "@/lib/problem";
import { MAX_TOASTS, toast, toastDuration, useToasts } from "./toast";

const titles = () => useToasts.getState().toasts.map((t) => t.title);

describe("toast", () => {
  it("adds ok, info and error toasts in order", () => {
    toast.ok("Saved", "Local");
    toast.info("Nothing new");
    toast.error(new CommandError("HTTP 500"), { title: "Could not save" });
    expect(titles()).toEqual(["Saved", "Nothing new", "Could not save"]);
    expect(useToasts.getState().toasts[2]).toMatchObject({ kind: "error", error: { message: "HTTP 500" } });
  });

  it("gives an error a default title", () => {
    toast.error(new CommandError("boom"));
    expect(titles()).toEqual(["Something went wrong"]);
  });

  it("replaces an identical toast instead of stacking it", () => {
    toast.error(new CommandError("offline"), { title: "Could not sync" });
    toast.ok("Saved");
    toast.error(new CommandError("offline"), { title: "Could not sync" });
    expect(titles()).toEqual(["Saved", "Could not sync"]);
  });

  it(`keeps at most ${MAX_TOASTS}, dropping the oldest`, () => {
    for (let i = 0; i < MAX_TOASTS + 2; i++) {
      toast.ok(`Saved ${i}`);
    }
    expect(titles()).toEqual(["Saved 2", "Saved 3", "Saved 4", "Saved 5"]);
  });

  it("dismisses one by id", () => {
    const id = toast.ok("Saved");
    toast.ok("Deleted");
    useToasts.getState().dismiss(id);
    expect(titles()).toEqual(["Deleted"]);
  });

  it("keeps an error longer than a success", () => {
    const ok = { id: 1, kind: "ok", title: "Saved" } as const;
    const error = { id: 2, kind: "error", title: "Failed", error: new CommandError("x") } as const;
    expect(toastDuration(error)).toBeGreaterThan(toastDuration(ok));
  });
});
