import { act, cleanup, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { CommandError } from "@/shared";

import type { Added } from "../domain/types";
import { useRuns } from "../hooks";
import { deferred, overviewFixture } from "../testing";
import { TrendsScreen } from "./TrendsScreen";

const mocks = vi.hoisted(() => ({
  api: { overview: vi.fn(), addToPreferences: vi.fn() },
  invoke: vi.fn(),
}));

vi.mock("../api", () => mocks.api);
vi.mock("@/shared/bridge", () => ({
  invoke: mocks.invoke,
  onDataChanged: () => () => undefined,
  onGraphProgress: () => () => undefined,
}));

beforeEach(() => {
  for (const fn of Object.values(mocks.api)) {
    fn.mockReset();
  }
  mocks.invoke.mockReset();
  mocks.invoke.mockResolvedValue(undefined);
  mocks.api.overview.mockResolvedValue(overviewFixture());
  useRuns.setState({ running: undefined, step: undefined, notice: undefined });
});

afterEach(cleanup);

async function shown() {
  render(<TrendsScreen />);
  await screen.findByRole("heading", { name: "Weekly digest 2026-W46" });
  return userEvent.setup();
}

describe("Add to preferences", () => {
  it("reads where each topic stands: ready, already added, in the avoid list", async () => {
    await shown();
    // Rust is not in the preferences: the point and the chart row both offer it.
    expect(screen.getAllByRole("button", { name: "Add Rust to preferences" })).toHaveLength(2);
    expect(screen.getAllByRole("button", { name: "Add Rust to preferences" }).every((b) => !b.hasAttribute("disabled"))).toBe(true);
    // Kubernetes is in the preferences already.
    const added = screen.getAllByRole("button", { name: "Kubernetes is in your preferences" });
    expect(added).toHaveLength(2);
    expect(added.every((b) => b.hasAttribute("disabled"))).toBe(true);
    expect(screen.getAllByText("Added")).toHaveLength(1);
    // Fintech is in the avoid list: adding it would contradict the preferences.
    const avoided = screen.getByRole("button", { name: "fintech is in your avoid list" });
    expect(avoided.hasAttribute("disabled")).toBe(true);
    expect(avoided.getAttribute("title")).toBe("In your avoid list");
    // Only the skills and domains of a point can be added: the hiring signal has no button.
    const hiring = screen.getByText("Remote Rust roles are posted in the monthly hiring thread.").closest("li") as HTMLElement;
    expect(within(hiring).queryByRole("button", { name: /preferences/ })).toBeNull();
  });

  it("adds a skill at once, shows a spinner while it saves, then reads Added everywhere it appears", async () => {
    const user = await shown();
    const save = deferred<Added>();
    mocks.api.addToPreferences.mockReturnValue(save.promise);

    await user.click(screen.getAllByRole("button", { name: "Add Rust to preferences" })[0] as HTMLElement);
    expect(mocks.api.addToPreferences).toHaveBeenCalledWith("skills", "Rust");
    const pending = screen.getAllByRole("button", { name: "Add Rust to preferences" });
    expect(pending.every((b) => b.hasAttribute("disabled") && b.getAttribute("aria-busy") === "true")).toBe(true);

    // More clicks while it saves do nothing.
    await user.click(pending[0] as HTMLElement);
    await user.click(pending[1] as HTMLElement);
    expect(mocks.api.addToPreferences).toHaveBeenCalledTimes(1);

    await act(async () => save.resolve({ presence: "added", message: null }));
    expect(screen.getAllByRole("button", { name: "Rust is in your preferences" })).toHaveLength(2);
    expect(screen.queryByRole("button", { name: "Add Rust to preferences" })).toBeNull();
    expect(screen.getAllByText("Added")).toHaveLength(2);
    // The digest stays as it was: no notice, nothing cancelled.
    expect(screen.queryByRole("group", { name: "Result" })).toBeNull();
  });

  it("adds a domain to the domains", async () => {
    const fixture = overviewFixture();
    if (fixture.digest?.body.kind === "data") {
      const fintech = fixture.digest.body.sections[1]?.points[0];
      if (fintech?.suggest) {
        fintech.suggest.presence = "absent";
      }
    }
    mocks.api.overview.mockResolvedValue(fixture);
    mocks.api.addToPreferences.mockResolvedValue({ presence: "added", message: null });
    const user = await shown();
    await user.click(screen.getByRole("button", { name: "Add fintech to preferences" }));
    expect(mocks.api.addToPreferences).toHaveBeenCalledWith("domains", "fintech");
  });

  it("does not add what is in the avoid list or in the preferences already", async () => {
    const user = await shown();
    await user.click(screen.getByRole("button", { name: "fintech is in your avoid list" }));
    await user.click(screen.getAllByRole("button", { name: "Kubernetes is in your preferences" })[0] as HTMLElement);
    expect(mocks.api.addToPreferences).not.toHaveBeenCalled();
  });

  it("says why a topic was not added", async () => {
    mocks.api.addToPreferences.mockResolvedValue({ presence: "avoided", message: '"Rust" is in your avoid list, so it was not added.' });
    const user = await shown();
    await user.click(screen.getAllByRole("button", { name: "Add Rust to preferences" })[0] as HTMLElement);
    const notice = within(await screen.findByRole("group", { name: "Result" }));
    expect(notice.getByText('"Rust" is in your avoid list, so it was not added.')).toBeTruthy();
    // It now reads as in the avoid list, not as added.
    expect(screen.getAllByRole("button", { name: "Rust is in your avoid list" }).length).toBeGreaterThan(0);
  });

  it("says when saving failed and lets the user try again", async () => {
    mocks.api.addToPreferences.mockRejectedValueOnce(new CommandError("database is locked"));
    const user = await shown();
    await user.click(screen.getAllByRole("button", { name: "Add Rust to preferences" })[0] as HTMLElement);
    expect(await screen.findByText('Could not add "Rust" to your preferences: database is locked')).toBeTruthy();

    mocks.api.addToPreferences.mockResolvedValueOnce({ presence: "added", message: null });
    await user.click(screen.getAllByRole("button", { name: "Add Rust to preferences" })[0] as HTMLElement);
    expect(mocks.api.addToPreferences).toHaveBeenCalledTimes(2);
  });
});
