import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { CommandError } from "@/lib/problem";

import * as api from "@/api/profile";
import type { About } from "@/types/profile";
import { AboutYouSection } from "./AboutYouSection";

vi.mock("@/api/profile");

function about(patch: Partial<About> = {}): About {
  return {
    sources: [
      { id: 1, name: "cv.pdf", kind: "file", chars: 4200, truncated: true },
      { id: 2, name: "On-call", kind: "text", chars: 300, truncated: false },
    ],
    status: "ready",
    summary: {
      name: "Minh Tran",
      headline: "Senior backend engineer",
      roles: [{ position: "Senior Backend Engineer", company: "Lotus Pay", dates: "2021 – Present" }],
      education: [{ degree: "BSc Computer Science", institution: "HCMUT", dates: "2014 – 2018" }],
      skills: ["Rust", "Go"],
      languages: ["English (Professional)", "Vietnamese (Native)"],
      projects: ["tinyqueue"],
      certifications: [],
    },
    leftOut: ["Certification “CKA”"],
    builtAt: new Date().toISOString(),
    limits: { maxChars: 20000, maxTotalChars: 60000 },
    ...patch,
  };
}

const EMPTY: Partial<About> = { sources: [], status: "empty", summary: null, leftOut: [], builtAt: null };

beforeEach(() => {
  vi.mocked(api.readAbout).mockResolvedValue(about());
});

afterEach(cleanup);

describe("AboutYouSection", () => {
  it("lists the documents and shows what was understood, with no primary button", async () => {
    render(<AboutYouSection />);
    expect(screen.getByRole("heading", { name: "About you" })).toBeTruthy();
    const list = await screen.findByRole("list", { name: "Your documents" });
    expect(within(list).getByText("cv.pdf")).toBeTruthy();
    expect(within(list).getByText(/File · 4.2k characters/)).toBeTruthy();
    expect(within(list).getByText(/Only the first 20k characters were kept/)).toBeTruthy();
    expect(screen.getByRole("button", { name: "Remove On-call" })).toBeTruthy();

    expect(screen.getByRole("status").textContent).toMatch(/^Read/);
    expect(screen.getByText("Minh Tran")).toBeTruthy();
    expect(screen.getByText("Senior Backend Engineer — Lotus Pay · 2021 – Present")).toBeTruthy();
    expect(screen.getByText("BSc Computer Science — HCMUT · 2014 – 2018")).toBeTruthy();
    expect(within(screen.getByRole("list", { name: "Skills" })).getByText("Rust")).toBeTruthy();
    expect(screen.getByText("English (Professional), Vietnamese (Native)")).toBeTruthy();
    expect(screen.queryByRole("group", { name: "Certifications" })).toBeNull();
    expect(screen.getByText("Certification “CKA”")).toBeTruthy();
    expect(screen.getByText("Add a document that states it, then read again.")).toBeTruthy();

    for (const button of screen.getAllByRole("button")) {
      expect(button.className).not.toContain("bg-accent text-on-fill");
    }
  });

  it("shows the empty state without a status line", async () => {
    vi.mocked(api.readAbout).mockResolvedValue(about(EMPTY));
    render(<AboutYouSection />);
    expect(await screen.findByText("No documents yet")).toBeTruthy();
    expect(screen.queryByRole("status")).toBeNull();
    expect(screen.queryByRole("button", { name: "Re-read" })).toBeNull();
  });

  it("adds a note, then reads the documents and says so meanwhile", async () => {
    vi.mocked(api.readAbout).mockResolvedValue(about(EMPTY));
    const added = about({
      ...EMPTY,
      sources: [{ id: 3, name: "Side projects", kind: "text", chars: 20, truncated: false }],
      status: "unread",
    });
    vi.mocked(api.addText).mockResolvedValue(added);
    let finish: (value: About) => void = () => undefined;
    vi.mocked(api.readDocuments).mockReturnValue(new Promise((resolve) => (finish = resolve)));
    render(<AboutYouSection />);

    fireEvent.click(await screen.findByRole("button", { name: "Add text" }));
    const save = screen.getByRole("button", { name: "Save" });
    expect(save).toHaveProperty("disabled", true);
    fireEvent.change(screen.getByRole("textbox", { name: "Title" }), { target: { value: "Side projects" } });
    fireEvent.change(screen.getByRole("textbox", { name: "Text" }), { target: { value: "I built tinyqueue." } });
    expect(save).toHaveProperty("disabled", false);
    fireEvent.click(save);

    expect(api.addText).toHaveBeenCalledWith("Side projects", "I built tinyqueue.");
    await waitFor(() => expect(api.readDocuments).toHaveBeenCalled());
    expect((await screen.findByRole("status")).textContent).toBe("Reading your documents…");
    expect(screen.getByRole("button", { name: "Re-read" })).toHaveProperty("disabled", true);
    expect(screen.queryByRole("form", { name: "Add a note" })).toBeNull();

    finish(about({ sources: added.sources }));
    expect(await screen.findByText("Minh Tran")).toBeTruthy();
    expect(screen.getByRole("button", { name: "Re-read" })).toHaveProperty("disabled", false);
  });

  it("shows a failed read with what to do about it", async () => {
    vi.mocked(api.readAbout).mockResolvedValue(about({ status: "stale" }));
    vi.mocked(api.readDocuments).mockRejectedValue(
      new CommandError("no AI provider configured", {
        challenge: null,
        retryable: false,
        kind: "noProvider",
        advice: "Add an AI provider in Settings.",
        settingsSection: "AI providers",
      }),
    );
    render(<AboutYouSection />);
    expect((await screen.findByRole("status")).textContent).toMatch(/^Out of date/);
    fireEvent.click(screen.getByRole("button", { name: "Re-read" }));
    const banner = await screen.findByRole("group", { name: "Your documents were not read" });
    expect(within(banner).getByText("no AI provider configured")).toBeTruthy();
    expect(within(banner).getByRole("button", { name: "Open Settings > AI providers" })).toBeTruthy();
  });

  it("removes a document by its name and reads the rest again", async () => {
    vi.mocked(api.removeSource).mockResolvedValue(about({ status: "stale", sources: [about().sources[1]!] }));
    vi.mocked(api.readDocuments).mockResolvedValue(about({ sources: [about().sources[1]!] }));
    render(<AboutYouSection />);
    fireEvent.click(await screen.findByRole("button", { name: "Remove cv.pdf" }));
    expect(api.removeSource).toHaveBeenCalledWith(1);
    await waitFor(() => expect(api.readDocuments).toHaveBeenCalled());
    await waitFor(() => expect(screen.queryByText("cv.pdf")).toBeNull());
  });

  it("adds a picked file; a cancelled picker does nothing", async () => {
    vi.mocked(api.pickDocument).mockResolvedValueOnce(null).mockResolvedValueOnce("/home/me/cv.md");
    vi.mocked(api.addFile).mockResolvedValue(about({ status: "ready" }));
    render(<AboutYouSection />);
    const add = await screen.findByRole("button", { name: "Add file" });
    fireEvent.click(add);
    await waitFor(() => expect(api.pickDocument).toHaveBeenCalledTimes(1));
    expect(api.addFile).not.toHaveBeenCalled();
    fireEvent.click(add);
    await waitFor(() => expect(api.addFile).toHaveBeenCalledWith("/home/me/cv.md"));
    expect(api.readDocuments).not.toHaveBeenCalled();
  });
});
