import { cleanup, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { CommandError } from "@/lib/problem";
import { renderWithToasts } from "@/test/render";

import * as api from "@/api/applications";
import { timeline } from "@/test/fixtures/applications";
import { TimelineTab } from "./TimelineTab";

vi.mock("@/api/applications");
vi.mock("@/api/bridge", () => ({
  invoke: vi.fn().mockResolvedValue(undefined),
  onDataChanged: () => () => undefined,
  onGraphProgress: () => () => undefined,
}));

const mocked = vi.mocked(api);

const REVIEW = {
  id: "m1",
  title: "Rejection (60% sure): Your application",
  body: "From hr@acme.test - sounds like a no",
};

beforeEach(() => {
  mocked.readTimeline.mockResolvedValue(timeline());
  mocked.resolveEmail.mockResolvedValue(undefined);
});

afterEach(() => {
  cleanup();
});

describe("the Timeline tab", () => {
  it("lists the events, newest first, with their times", async () => {
    render(<TimelineTab id={7} />);
    expect(await screen.findByRole("heading", { name: "History" })).toBeTruthy();
    const rows = screen.getAllByText(/moved to Applied|application created/).map((n) => n.textContent);
    expect(rows).toEqual(["moved to Applied", "application created"]);
    expect(screen.getByText("2026-11-02 09:05")).toBeTruthy();
  });

  it("carries the whole text of a cut event as its tooltip", async () => {
    mocked.readTimeline.mockResolvedValue(
      timeline({
        events: [
          {
            id: 5,
            tone: "danger",
            text: "the form could not be filled...",
            full: "the form could not be filled\nfield: email",
            when: "2026-11-03 10:00",
          },
        ],
      }),
    );
    render(<TimelineTab id={7} />);
    await screen.findByText("the form could not be filled...");
    expect(document.getElementById("event-5")?.getAttribute("title")).toBe(
      "the form could not be filled\nfield: email",
    );
  });

  it("says when nothing has happened", async () => {
    mocked.readTimeline.mockResolvedValue(timeline({ events: [] }));
    render(<TimelineTab id={7} />);
    expect(await screen.findByText("Nothing has happened yet.")).toBeTruthy();
  });

  it("accepts or dismisses an email that waits for a review", async () => {
    const user = userEvent.setup();
    mocked.readTimeline.mockResolvedValue(timeline({ reviews: [REVIEW] }));
    render(<TimelineTab id={7} />);
    const banner = await screen.findByRole("group", { name: REVIEW.title });
    expect(within(banner).getByText(REVIEW.body)).toBeTruthy();
    await user.click(within(banner).getByRole("button", { name: "Accept" }));
    expect(mocked.resolveEmail).toHaveBeenCalledWith("m1", true);
    await waitFor(() =>
      expect(within(banner).getByRole("button", { name: "Dismiss" }).hasAttribute("disabled")).toBe(false),
    );
    await user.click(within(banner).getByRole("button", { name: "Dismiss" }));
    expect(mocked.resolveEmail).toHaveBeenLastCalledWith("m1", false);
  });

  it("reports an email that could not be handled in a toast, each time", async () => {
    const user = userEvent.setup();
    mocked.readTimeline.mockResolvedValue(timeline({ reviews: [REVIEW] }));
    mocked.resolveEmail.mockImplementation(async () => {
      throw new CommandError("the email is gone");
    });
    renderWithToasts(<TimelineTab id={7} />);
    await user.click(await screen.findByRole("button", { name: "Accept" }));
    const toast = await screen.findByRole("alert", { name: "That email could not be handled" });
    expect(within(toast).getByText("the email is gone")).toBeTruthy();
    expect(within(document.getElementById("timeline") as HTMLElement).queryByText("the email is gone")).toBeNull();

    // Reported and forgotten: the same failure again is reported again.
    await user.click(within(toast).getByRole("button", { name: "Dismiss" }));
    await waitFor(() => expect(screen.queryByRole("alert", { name: "That email could not be handled" })).toBeNull());
    await waitFor(() => expect(screen.getByRole("button", { name: "Accept" }).hasAttribute("disabled")).toBe(false));
    await user.click(screen.getByRole("button", { name: "Accept" }));
    expect(await screen.findByRole("alert", { name: "That email could not be handled" })).toBeTruthy();
  });

  it("reports a timeline that could not be read in a toast, with Retry in its place", async () => {
    const user = userEvent.setup();
    mocked.readTimeline.mockRejectedValueOnce(new CommandError("database is locked"));
    mocked.readTimeline.mockResolvedValue(timeline());
    renderWithToasts(<TimelineTab id={7} />);
    const toast = await screen.findByRole("alert", { name: "The timeline could not be read" });
    expect(within(toast).getByText("database is locked")).toBeTruthy();
    await user.click(screen.getByRole("button", { name: "Retry" }));
    expect(await screen.findByText("History")).toBeTruthy();
  });
});
