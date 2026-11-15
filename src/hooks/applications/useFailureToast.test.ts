import { act, renderHook } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import { CommandError } from "@/lib/problem";
import { useToasts } from "@/stores/toast";

import { useKeptFailureToast, useSameFailure } from "./useFailureToast";

describe("a failure the shell keeps", () => {
  it("keeps the object it had when it was first seen while it is the same failure", () => {
    const first = new CommandError("timed out");
    const { result, rerender } = renderHook(({ error }) => useSameFailure(error), {
      initialProps: { error: first as CommandError | null },
    });
    expect(result.current).toBe(first);
    rerender({ error: new CommandError("timed out") });
    expect(result.current, "read again").toBe(first);
    const other = new CommandError("refused");
    rerender({ error: other });
    expect(result.current).toBe(other);
    rerender({ error: null });
    expect(result.current).toBeUndefined();
  });

  it("is reported once per failure, not with every read that still carries it", () => {
    const clear = vi.fn();
    const { rerender } = renderHook(({ error }) => useKeptFailureToast(error, { title: "It failed", clear }), {
      initialProps: { error: new CommandError("timed out") as CommandError | null },
    });
    const ids = () => useToasts.getState().toasts.map((t) => t.id);
    const reported = ids();
    expect(reported).toHaveLength(1);
    expect(clear).toHaveBeenCalledTimes(1);

    rerender({ error: new CommandError("timed out") });
    expect(ids(), "the same toast, not pushed again").toEqual(reported);
    expect(clear).toHaveBeenCalledTimes(1);

    // Gone, then the same failure again (a second attempt failed the same way): reported again.
    act(() => useToasts.getState().clear());
    rerender({ error: null });
    rerender({ error: new CommandError("timed out") });
    expect(ids()).toHaveLength(1);
    expect(clear).toHaveBeenCalledTimes(2);
  });
});
