import { cleanup, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { CommandError } from "@/lib/problem";

import { useRuns } from "@/hooks/trends/runs";
import { overviewFixture } from "@/test/fixtures/trends";
import { TrendsPage } from "@/pages/TrendsPage";

const mocks = vi.hoisted(() => ({
  api: {
    overview: vi.fn(),
    addSource: vi.fn(),
    removeSource: vi.fn(),
    setSourceEnabled: vi.fn(),
    saveKey: vi.fn(),
    clearKey: vi.fn(),
  },
  invoke: vi.fn(),
}));

vi.mock("@/api/trends", () => mocks.api);
vi.mock("@/api/bridge", () => ({
  invoke: mocks.invoke,
  onDataChanged: () => () => undefined,
  onGraphProgress: () => () => undefined,
}));

beforeEach(() => {
  for (const fn of Object.values(mocks.api)) {
    fn.mockReset();
    if (fn !== mocks.api.overview) {
      fn.mockResolvedValue(undefined);
    }
  }
  mocks.api.overview.mockResolvedValue(overviewFixture());
  useRuns.setState({ running: undefined, step: undefined, notice: undefined });
});

afterEach(cleanup);

async function shown() {
  render(<TrendsPage />);
  await screen.findByRole("heading", { name: "Weekly digest 2026-W46" });
  return userEvent.setup();
}

const sources = () => within(screen.getByRole("region", { name: "Sources" }));
const search = () => within(screen.getByRole("region", { name: "Report search" }));

describe("Sources", () => {
  it("lists each source with its kind, host and how its last run went", async () => {
    await shown();
    const list = sources();
    expect(list.getByText("Hacker News", { selector: "span" })).toBeTruthy();
    expect(list.getByText("ok")).toBeTruthy();
    expect(list.getByText("last run: ok")).toBeTruthy();
    expect(list.getByText("RSS / Atom feed · blog.example.com")).toBeTruthy();
    expect(list.getByText("not run")).toBeTruthy();
    expect(list.getByText("not collected yet")).toBeTruthy();
  });

  it("adds Hacker News or the YC companies at once, once", async () => {
    const user = await shown();
    // Hacker News is a source already.
    expect(sources().getByRole("button", { name: "Hacker News" }).hasAttribute("disabled")).toBe(true);
    await user.click(sources().getByRole("button", { name: "YC companies" }));
    expect(mocks.api.addSource).toHaveBeenCalledWith("yc", "", "");
  });

  it("opens a form for a feed, adds it with its address and closes the form", async () => {
    const user = await shown();
    const toggle = sources().getByRole("button", { name: "RSS / Atom feed" });
    expect(toggle.getAttribute("aria-pressed")).toBe("false");
    await user.click(toggle);
    expect(toggle.getAttribute("aria-pressed")).toBe("true");

    await user.type(sources().getByLabelText("Name (optional)"), "Another blog");
    await user.type(sources().getByLabelText("Address"), "https://another.example.com/feed.xml");
    await user.click(sources().getByRole("button", { name: "Add RSS / Atom feed" }));
    expect(mocks.api.addSource).toHaveBeenCalledWith("rss", "Another blog", "https://another.example.com/feed.xml");
    expect(sources().queryByLabelText("Address")).toBeNull();
    expect(toggle.getAttribute("aria-pressed")).toBe("false");

    // Reopened, the form is empty again.
    await user.click(sources().getByRole("button", { name: "Web page" }));
    expect((sources().getByLabelText("Address") as HTMLInputElement).value).toBe("");
  });

  it("adds a feed with Enter and keeps the form open with the reason when it is refused", async () => {
    mocks.api.addSource.mockRejectedValueOnce(new CommandError("RSS / Atom feed needs an http(s) address"));
    const user = await shown();
    await user.click(sources().getByRole("button", { name: "RSS / Atom feed" }));
    await user.type(sources().getByLabelText("Address"), "ftp://nope{Enter}");
    expect(mocks.api.addSource).toHaveBeenCalledWith("rss", "", "ftp://nope");
    expect((await sources().findByRole("alert")).textContent).toBe("RSS / Atom feed needs an http(s) address");
    expect((sources().getByLabelText("Address") as HTMLInputElement).value).toBe("ftp://nope");

    // Cancel closes the form and clears the reason.
    await user.click(sources().getByRole("button", { name: "Cancel" }));
    expect(sources().queryByLabelText("Address")).toBeNull();
    expect(sources().queryByRole("alert")).toBeNull();
  });

  it("turns a source on or off and removes it", async () => {
    const user = await shown();
    await user.click(screen.getByRole("switch", { name: "Collect from Example engineering blog" }));
    expect(mocks.api.setSourceEnabled).toHaveBeenCalledWith(3, true);
    await user.click(screen.getByRole("switch", { name: "Collect from Hacker News" }));
    expect(mocks.api.setSourceEnabled).toHaveBeenCalledWith(1, false);
    await user.click(screen.getByRole("button", { name: "Remove Example engineering blog" }));
    expect(mocks.api.removeSource).toHaveBeenCalledWith(3);
  });

  it("says why a source could not be changed", async () => {
    mocks.api.removeSource.mockRejectedValueOnce(new CommandError("trend source 3 not found"));
    const user = await shown();
    await user.click(screen.getByRole("button", { name: "Remove Example engineering blog" }));
    expect((await sources().findByRole("alert")).textContent).toBe("trend source 3 not found");
    // The next change clears it.
    await user.click(screen.getByRole("switch", { name: "Collect from Hacker News" }));
    expect(sources().queryByRole("alert")).toBeNull();
  });
});

describe("Report search", () => {
  it("shows which keys are saved and whether search is on", async () => {
    await shown();
    expect(search().getByText("Report search is on.")).toBeTruthy();
    expect(search().getByLabelText("Brave Search · no key")).toBeTruthy();
    expect(search().getByLabelText("Tavily · key saved")).toBeTruthy();
    expect(search().getByRole("button", { name: "Save the Brave Search API key" }).textContent).toBe("Save key");
    expect(search().getByRole("button", { name: "Save the Tavily API key" }).textContent).toBe("Replace key");
    expect(search().queryByRole("button", { name: "Remove the Brave Search API key" })).toBeNull();
    expect(search().getByRole("button", { name: "Remove the Tavily API key" })).toBeTruthy();
  });

  it("saves a typed key through the shell, clears the field and never shows it", async () => {
    const user = await shown();
    const field = search().getByLabelText("Brave Search · no key") as HTMLInputElement;
    // Masked, with a toggle to check what was typed.
    expect(field.type).toBe("password");
    await user.type(field, "  BSA-secret-key ");
    await user.click(search().getByRole("button", { name: "Show the Brave Search API key" }));
    expect(field.type).toBe("text");
    await user.click(search().getByRole("button", { name: "Hide the Brave Search API key" }));
    expect(field.type).toBe("password");

    await user.click(search().getByRole("button", { name: "Save the Brave Search API key" }));
    expect(mocks.api.saveKey).toHaveBeenCalledWith("brave", "BSA-secret-key");
    expect(field.value).toBe("");
    expect(document.body.textContent).not.toContain("BSA-secret-key");
  });

  it("asks for the key first when the field is empty", async () => {
    const user = await shown();
    await user.click(search().getByRole("button", { name: "Save the Brave Search API key" }));
    expect((await search().findByRole("alert")).textContent).toBe("Enter the Brave Search API key first.");
    expect(mocks.api.saveKey).not.toHaveBeenCalled();
    // Typing clears the reminder.
    await user.type(search().getByLabelText("Brave Search · no key"), "k");
    expect(search().queryByRole("alert")).toBeNull();
  });

  it("keeps the typed key when saving fails, and says why", async () => {
    mocks.api.saveKey.mockRejectedValueOnce(new CommandError("the keyring is locked"));
    const user = await shown();
    const field = search().getByLabelText("Brave Search · no key") as HTMLInputElement;
    await user.type(field, "abc");
    await user.click(search().getByRole("button", { name: "Save the Brave Search API key" }));
    expect((await search().findByRole("alert")).textContent).toBe("the keyring is locked");
    expect(field.value).toBe("abc");
  });

  it("removes a saved key", async () => {
    const user = await shown();
    await user.click(search().getByRole("button", { name: "Remove the Tavily API key" }));
    expect(mocks.api.clearKey).toHaveBeenCalledWith("tavily");
  });
});
