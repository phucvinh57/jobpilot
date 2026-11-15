import { cleanup, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { CommandError } from "@/shared";

import * as api from "../api/discoveryApi";
import { jobSources, SOURCE } from "../domain/fixtures";
import { JobSourcesSection } from "./JobSourcesSection";

vi.mock("../api/discoveryApi");

const mocked = vi.mocked(api);

beforeEach(() => {
  mocked.readJobSources.mockResolvedValue(jobSources());
  mocked.setSourceEnabled.mockResolvedValue(undefined);
  mocked.setSourceSession.mockResolvedValue(undefined);
  mocked.removeSource.mockResolvedValue(undefined);
  mocked.setDaily.mockResolvedValue(undefined);
  mocked.suggestBoards.mockResolvedValue([]);
});

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});

describe("Settings > Job sources", () => {
  it("lists the sources with how their last run ended, and the sessions they can be read in", async () => {
    render(<JobSourcesSection />);
    const first = await screen.findByRole("listitem", { name: /^Acme careers: ok - 1h ago$/ });
    expect(within(first).getByText("Greenhouse boards.greenhouse.io")).toBeTruthy();
    expect(within(first).getByText("ok")).toBeTruthy();
    expect(within(first).getByRole("switch", { name: "Search Acme careers" }).getAttribute("aria-checked")).toBe(
      "true",
    );
    // No session bound: "No session" is the one that is on.
    expect(within(first).getByRole("button", { name: "No session" }).getAttribute("aria-pressed")).toBe("true");
    expect(within(first).getByRole("button", { name: "LinkedIn" }).getAttribute("aria-pressed")).toBe("false");

    // A source with no name shows its kind; a paused one needs the user, whatever its status says.
    const second = screen.getByRole("listitem", { name: "Career page: needs you: continue it from Jobs" });
    expect(within(second).getByText("needs you")).toBeTruthy();
    expect(within(second).getByRole("switch", { name: "Search Career page" }).getAttribute("aria-checked")).toBe(
      "false",
    );
    expect(within(second).getByRole("button", { name: "LinkedIn" }).getAttribute("aria-pressed")).toBe("true");
  });

  it("turns a source off, binds a session and removes a source after a second press", async () => {
    const user = userEvent.setup();
    render(<JobSourcesSection />);
    const first = await screen.findByRole("listitem", { name: /^Acme careers/ });
    await user.click(within(first).getByRole("switch", { name: "Search Acme careers" }));
    expect(mocked.setSourceEnabled).toHaveBeenCalledWith(1, false);

    await user.click(within(first).getByRole("button", { name: "Work account" }));
    expect(mocked.setSourceSession).toHaveBeenCalledWith(1, 6);
    const second = screen.getByRole("listitem", { name: /^Career page/ });
    await user.click(within(second).getByRole("button", { name: "No session" }));
    expect(mocked.setSourceSession).toHaveBeenCalledWith(2, null);

    await user.click(within(first).getByRole("button", { name: "Remove" }));
    expect(mocked.removeSource, "the first press only asks").not.toHaveBeenCalled();
    await user.click(within(first).getByRole("button", { name: "Sure?" }));
    expect(mocked.removeSource).toHaveBeenCalledWith(1);
  });

  it("forgets a pending 'Sure?' when something else is changed", async () => {
    const user = userEvent.setup();
    render(<JobSourcesSection />);
    const first = await screen.findByRole("listitem", { name: /^Acme careers/ });
    await user.click(within(first).getByRole("button", { name: "Remove" }));
    expect(within(first).getByRole("button", { name: "Sure?" })).toBeTruthy();
    await user.click(within(first).getByRole("switch", { name: "Search Acme careers" }));
    expect(within(first).getByRole("button", { name: "Remove" })).toBeTruthy();
  });

  it("switches the daily search at once and keeps it when the write succeeds", async () => {
    const user = userEvent.setup();
    render(<JobSourcesSection />);
    const daily = await screen.findByRole("switch", { name: "Search daily while JobPilot is open" });
    expect(daily.getAttribute("aria-checked")).toBe("false");
    await user.click(daily);
    expect(daily.getAttribute("aria-checked")).toBe("true");
    expect(mocked.setDaily).toHaveBeenCalledWith(true);
  });

  it("puts the daily switch back when it could not be saved, and says why", async () => {
    const user = userEvent.setup();
    mocked.setDaily.mockRejectedValue(new CommandError("database is locked"));
    render(<JobSourcesSection />);
    const daily = await screen.findByRole("switch", { name: "Search daily while JobPilot is open" });
    await user.click(daily);
    expect(await screen.findByText("database is locked")).toBeTruthy();
    await waitFor(() => expect(daily.getAttribute("aria-checked")).toBe("false"));
  });

  it("adds a source of the chosen kind", async () => {
    const user = userEvent.setup();
    mocked.addSource.mockResolvedValue({ ...SOURCE, id: 3, name: "Beta", kind: "career_page" });
    render(<JobSourcesSection />);
    await user.click(await screen.findByRole("button", { name: "Add source" }));
    expect(screen.queryByRole("button", { name: "Add source" }), "the button gives way to the form").toBeNull();
    expect(screen.getByText("Board URL or name, e.g. boards.greenhouse.io/acme")).toBeTruthy();

    await user.click(screen.getByRole("button", { name: "Career page" }));
    expect(screen.getByText("Careers page, e.g. https://acme.com/careers")).toBeTruthy();
    await user.type(screen.getByLabelText("Name"), "Beta");
    await user.type(screen.getByLabelText("Board name or address"), "beta.test/careers");
    await user.click(screen.getByRole("button", { name: "Add" }));
    expect(mocked.addSource).toHaveBeenCalledWith("career_page", "Beta", "beta.test/careers");
    await waitFor(() => expect(screen.getByRole("button", { name: "Add source" })).toBeTruthy());
  });

  it("offers the job board behind a careers page with a plain button", async () => {
    const user = userEvent.setup();
    const board = { kind: "greenhouse", label: "Greenhouse", input: "boards.greenhouse.io/beta" };
    mocked.suggestBoards.mockImplementation((url) => Promise.resolve(url === "beta.test/careers" ? [board] : []));
    mocked.addSource.mockResolvedValue({ ...SOURCE, id: 3, name: "beta", kind: "greenhouse" });
    render(<JobSourcesSection />);
    await user.click(await screen.findByRole("button", { name: "Add source" }));
    await user.click(screen.getByRole("button", { name: "Career page" }));
    const address = screen.getByLabelText("Board name or address");
    await user.type(address, "beta.test/careers");
    await user.tab();

    const text =
      "This page lists its jobs on Greenhouse (boards.greenhouse.io/beta). Add it as a Greenhouse source instead?";
    const banner = await screen.findByRole("group", { name: text });
    expect(mocked.suggestBoards).toHaveBeenCalledWith("beta.test/careers");
    const offer = within(banner).getByRole("button", { name: "Add Greenhouse source" });
    expect(offer.id).toBe("suggest-board-greenhouse-0");
    // A plain button: the suggestion never takes the form's main action.
    expect(offer.className).not.toContain("bg-accent");

    await user.click(offer);
    expect(mocked.addSource).toHaveBeenCalledWith("greenhouse", "", "boards.greenhouse.io/beta");
    await waitFor(() => expect(screen.getByRole("button", { name: "Add source" })).toBeTruthy());
  });

  it("shows no suggestion without a board, or for another kind", async () => {
    const user = userEvent.setup();
    render(<JobSourcesSection />);
    await user.click(await screen.findByRole("button", { name: "Add source" }));
    await user.type(screen.getByLabelText("Board name or address"), "boards.greenhouse.io/acme");
    await user.tab();
    expect(mocked.suggestBoards, "a board kind is not a careers page").not.toHaveBeenCalled();

    await user.click(screen.getByRole("button", { name: "Career page" }));
    await user.click(screen.getByLabelText("Board name or address"));
    await user.tab();
    await waitFor(() => expect(mocked.suggestBoards).toHaveBeenCalledTimes(1));
    expect(screen.queryByRole("group", { name: /lists its jobs on/ })).toBeNull();
  });

  it("shows nothing when the page cannot be read, and asks again later", async () => {
    const user = userEvent.setup();
    mocked.suggestBoards.mockRejectedValue(new CommandError("network is unreachable"));
    render(<JobSourcesSection />);
    await user.click(await screen.findByRole("button", { name: "Add source" }));
    await user.click(screen.getByRole("button", { name: "Career page" }));
    await user.type(screen.getByLabelText("Board name or address"), "beta.test/careers");
    await user.tab();
    await waitFor(() => expect(mocked.suggestBoards).toHaveBeenCalled());
    expect(screen.queryByRole("group", { name: /lists its jobs on/ })).toBeNull();
    expect(screen.queryByText("network is unreachable"), "not an error the user must act on").toBeNull();

    // The failure is not remembered: leaving the field again asks again.
    mocked.suggestBoards.mockResolvedValue([{ kind: "lever", label: "Lever", input: "jobs.lever.co/beta" }]);
    await user.click(screen.getByLabelText("Board name or address"));
    await user.tab();
    expect(await screen.findByRole("group", { name: /lists its jobs on Lever/ })).toBeTruthy();
    expect(mocked.suggestBoards).toHaveBeenCalledTimes(2);
  });

  it("keeps the form open and says why a source was refused", async () => {
    const user = userEvent.setup();
    mocked.addSource.mockRejectedValue(new CommandError("that board address is not valid"));
    render(<JobSourcesSection />);
    await user.click(await screen.findByRole("button", { name: "Add source" }));
    await user.click(screen.getByRole("button", { name: "Add" }));
    expect(await screen.findByText("that board address is not valid")).toBeTruthy();
    expect(screen.getByRole("button", { name: "Add" })).toBeTruthy();
    await user.click(screen.getByRole("button", { name: "Cancel" }));
    expect(screen.queryByText("that board address is not valid")).toBeNull();
    expect(screen.getByRole("button", { name: "Add source" })).toBeTruthy();
  });

  it("says there are no sources yet, once the list has been read", async () => {
    mocked.readJobSources.mockResolvedValue(jobSources({ sources: [], waiting: [] }));
    render(<JobSourcesSection />);
    expect(await screen.findByText("No sources yet. Add a company board, a careers page or Hacker News.")).toBeTruthy();
  });

  it("says when the list could not be read", async () => {
    mocked.readJobSources.mockRejectedValue(new CommandError("database is locked"));
    render(<JobSourcesSection />);
    expect(await screen.findByText("database is locked")).toBeTruthy();
    expect(screen.queryByText(/No sources yet/)).toBeNull();
  });
});
