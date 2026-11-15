import { act, cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { CommandError, type Problem } from "@/lib/problem";
import { toast } from "@/stores/toast";
import { Toaster } from "./Toaster";

afterEach(() => {
  cleanup();
  vi.useRealTimers();
});

const NETWORK: Problem = {
  kind: "network",
  challenge: null,
  advice: "Check your connection.",
  settingsSection: null,
  retryable: true,
};

describe("Toaster", () => {
  it("shows an error with its advice and a Retry that runs it again and closes the toast", () => {
    const onRetry = vi.fn();
    render(<Toaster />);
    act(() => void toast.error(new CommandError("timed out", NETWORK), { title: "Could not sync", onRetry }));
    const shown = screen.getByRole("alert", { name: "Could not sync" });
    expect(within(shown).getByText("timed out")).toBeTruthy();
    expect(within(shown).getByText("Check your connection.")).toBeTruthy();
    fireEvent.click(within(shown).getByRole("button", { name: "Retry" }));
    expect(onRetry).toHaveBeenCalledTimes(1);
    expect(screen.queryByRole("alert")).toBeNull();
  });

  it("closes on Dismiss", () => {
    render(<Toaster />);
    act(() => void toast.ok("Saved", "Local"));
    const shown = screen.getByRole("status", { name: "Saved" });
    expect(within(shown).getByText("Local")).toBeTruthy();
    fireEvent.click(within(shown).getByRole("button", { name: "Dismiss" }));
    expect(screen.queryByRole("status", { name: "Saved" })).toBeNull();
  });

  it("leaves on its own after a while, but not while the pointer is on it", () => {
    vi.useFakeTimers();
    render(<Toaster />);
    act(() => void toast.ok("Saved"));
    const shown = screen.getByRole("status", { name: "Saved" });
    fireEvent.mouseEnter(shown);
    act(() => void vi.advanceTimersByTime(60_000));
    expect(screen.getByRole("status", { name: "Saved" })).toBeTruthy();
    fireEvent.mouseLeave(shown);
    act(() => void vi.advanceTimersByTime(5_000));
    expect(screen.queryByRole("status", { name: "Saved" })).toBeNull();
  });
});
