import { cleanup, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";

import { NewResearchDialog } from "./NewResearchDialog";

afterEach(cleanup);

function shown(patch: Partial<Parameters<typeof NewResearchDialog>[0]> = {}) {
  const props = {
    open: true,
    cadence: "weekly" as const,
    onCadence: vi.fn(),
    periodLabel: "2026-W46" as string | undefined,
    onStart: vi.fn(),
    onClose: vi.fn(),
    ...patch,
  };
  render(<NewResearchDialog {...props} />);
  return { props, user: userEvent.setup(), dialog: within(screen.getByRole("dialog", { name: "New research" })) };
}

describe("NewResearchDialog", () => {
  it("offers the four periods, says what the run does and has one primary", () => {
    const { dialog } = shown({ cadence: "monthly", periodLabel: "2026-M11" });
    expect(screen.getByRole("dialog").id).toBe("trends-new-dialog");
    const periods = within(dialog.getByRole("group", { name: "Digest period" }));
    expect(periods.getAllByRole("button").map((b) => b.textContent)).toEqual([
      "Weekly",
      "Monthly",
      "Quarterly",
      "Yearly",
    ]);
    expect(periods.getByRole("button", { name: "Digest period: Monthly" }).getAttribute("aria-pressed")).toBe("true");
    expect(document.querySelector("#trends-cadence")).toBeTruthy();
    expect(dialog.getByText(/^2026-M11: collects from your sources, then your AI provider writes/)).toBeTruthy();
    const collect = dialog.getByRole("button", { name: "Collect & build digest" });
    expect(collect.id).toBe("trends-collect");
    expect(
      dialog.getAllByRole("button").filter((b) => b.className.includes("text-on-fill")),
      "one primary",
    ).toEqual([collect]);
  });

  it("picks a period, starts the run and cancels", async () => {
    const { dialog, props, user } = shown();
    await user.click(dialog.getByRole("button", { name: "Digest period: Yearly" }));
    expect(props.onCadence).toHaveBeenCalledWith("yearly");

    await user.click(dialog.getByRole("button", { name: "Collect & build digest" }));
    expect(props.onStart).toHaveBeenCalledTimes(1);
    expect(props.onClose).not.toHaveBeenCalled();

    await user.click(dialog.getByRole("button", { name: "Cancel" }));
    expect(props.onClose).toHaveBeenCalledTimes(1);
    expect(props.onStart).toHaveBeenCalledTimes(1);
  });

  it("cannot start while the period is not known", async () => {
    const { dialog, props, user } = shown({ periodLabel: undefined });
    const collect = dialog.getByRole("button", { name: "Collect & build digest" });
    expect(collect.hasAttribute("disabled")).toBe(true);
    expect(dialog.getByText("Reading the period…")).toBeTruthy();
    await user.click(collect);
    expect(props.onStart).not.toHaveBeenCalled();
  });

  it("shows nothing while closed", () => {
    render(
      <NewResearchDialog
        open={false}
        cadence="weekly"
        onCadence={vi.fn()}
        periodLabel="2026-W46"
        onStart={vi.fn()}
        onClose={vi.fn()}
      />,
    );
    expect(screen.queryByRole("dialog")).toBeNull();
  });
});
