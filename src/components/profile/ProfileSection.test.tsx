import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { CommandError } from "@/lib/problem";

import * as api from "@/api/profile";
import type { Profile } from "@/types/profile";
import { ProfileSection } from "./ProfileSection";

vi.mock("@/api/profile");

function profile(patch: Partial<Profile> = {}): Profile {
  return {
    profilePath: "/home/me/.local/share/jobpilot/profile.yaml",
    jobType: "hybrid",
    fields: [
      {
        key: "locations",
        label: "Work locations",
        placeholder: "Add a city or country",
        items: ["Hanoi"],
        suggestions: ["Remote"],
      },
      {
        key: "domains",
        label: "Interested domains",
        placeholder: "Add a domain, e.g. fintech",
        items: [],
        suggestions: [],
      },
      {
        key: "skills",
        label: "Skills",
        placeholder: "Add a skill",
        items: ["Rust", "Go"],
        suggestions: ["Kubernetes"],
      },
    ],
    notes: "Senior roles only",
    cv: { name: "cv.pdf", chars: 1234, truncated: false },
    canUndo: true,
    limits: { maxListItems: 30, maxItemChars: 80, maxNotesChars: 2000 },
    ...patch,
  };
}

beforeEach(() => {
  vi.mocked(api.readProfile).mockResolvedValue(profile());
});

afterEach(cleanup);

describe("ProfileSection", () => {
  it("shows the CV, the job type, every tag field, the notes and the profile file", async () => {
    render(<ProfileSection />);
    expect(screen.getByRole("heading", { name: "Profile & preferences" })).toBeTruthy();
    expect(await screen.findByText("cv.pdf")).toBeTruthy();
    expect(screen.getByText("1234 characters read")).toBeTruthy();
    expect(screen.getByRole("button", { name: "Replace" })).toBeTruthy();

    expect(screen.getByRole("button", { name: "Job type: Hybrid" }).getAttribute("aria-pressed")).toBe("true");
    expect(screen.getByRole("button", { name: "Job type: Remote" }).getAttribute("aria-pressed")).toBe("false");

    expect(screen.getByText("Hanoi")).toBeTruthy();
    expect(screen.getByText("Rust")).toBeTruthy();
    expect(screen.getByRole("button", { name: "Remove Rust" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "Add Kubernetes to Skills" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "Add Remote to Work locations" })).toBeTruthy();
    expect(screen.getByRole("textbox", { name: "Add to Interested domains" })).toBeTruthy();

    const notes = screen.getByRole("textbox", { name: "Anything else" });
    await waitFor(() => expect((notes as HTMLTextAreaElement).value).toBe("Senior roles only"));
    expect(screen.getByText("/home/me/.local/share/jobpilot/profile.yaml")).toBeTruthy();
    expect(screen.getByRole("button", { name: "Undo" })).toHaveProperty("disabled", false);
  });

  it("has no primary action: every change is saved at once", async () => {
    render(<ProfileSection />);
    await screen.findByText("cv.pdf");
    // Save is plain, and off while the notes are as saved.
    expect(screen.getByRole("button", { name: "Save" })).toHaveProperty("disabled", true);
    for (const button of screen.getAllByRole("button")) {
      expect(button.className).not.toContain("bg-accent text-on-fill");
    }
  });

  it("adds a suggestion with one click and shows the answer", async () => {
    const next = profile();
    next.fields[2]?.items.push("Kubernetes");
    next.fields[2]!.suggestions = [];
    vi.mocked(api.addItem).mockResolvedValue({ profile: next, message: null });
    render(<ProfileSection />);

    fireEvent.click(await screen.findByRole("button", { name: "Add Kubernetes to Skills" }));
    expect(api.addItem).toHaveBeenCalledWith("skills", "Kubernetes");
    expect(await screen.findByText("Kubernetes")).toBeTruthy();
    expect(await screen.findByRole("status", { name: "Saved" })).toBeTruthy();
  });

  it("adds what is typed and clears the box", async () => {
    vi.mocked(api.addItem).mockResolvedValue({ profile: profile(), message: null });
    render(<ProfileSection />);
    const box = (await screen.findByRole("textbox", { name: "Add to Skills" })) as HTMLInputElement;
    fireEvent.change(box, { target: { value: "  Zig " } });
    fireEvent.submit(box.closest("form") as HTMLFormElement);
    expect(api.addItem).toHaveBeenCalledWith("skills", "  Zig ");
    expect(box.value).toBe("");
  });

  it("says why a topic was not added", async () => {
    vi.mocked(api.addItem).mockResolvedValue({
      profile: profile(),
      message: '"php" is in your avoid list, so it was not added.',
    });
    render(<ProfileSection />);
    const box = (await screen.findByRole("textbox", { name: "Add to Skills" })) as HTMLInputElement;
    fireEvent.change(box, { target: { value: "php" } });
    fireEvent.submit(box.closest("form") as HTMLFormElement);
    expect(await screen.findByText(/is in your avoid list/)).toBeTruthy();
    expect(screen.getByRole("group", { name: "Not saved" })).toBeTruthy();
  });

  it("removes an entry by its name", async () => {
    const next = profile();
    next.fields[2]!.items = ["Go"];
    vi.mocked(api.removeItem).mockResolvedValue(next);
    render(<ProfileSection />);
    fireEvent.click(await screen.findByRole("button", { name: "Remove Rust" }));
    expect(api.removeItem).toHaveBeenCalledWith("skills", "Rust");
    await waitFor(() => expect(screen.queryByText("Rust")).toBeNull());
  });

  it("saves the notes and shows the line when the model could not read them", async () => {
    vi.mocked(api.saveNotes).mockResolvedValue(profile({ notes: "Senior, remote" }));
    vi.mocked(api.readNotes).mockResolvedValue({
      profile: profile({ notes: "Senior, remote" }),
      line: "Your notes are saved; I could not read everything from them.",
    });
    render(<ProfileSection />);
    const notes = (await screen.findByRole("textbox", { name: "Anything else" })) as HTMLTextAreaElement;
    await waitFor(() => expect(notes.value).toBe("Senior roles only"));

    fireEvent.change(notes, { target: { value: "Senior, remote" } });
    const save = screen.getByRole("button", { name: "Save" });
    expect(save).toHaveProperty("disabled", false);
    fireEvent.click(save);

    expect(api.saveNotes).toHaveBeenCalledWith("Senior, remote");
    await waitFor(() => expect(api.readNotes).toHaveBeenCalledWith("Senior, remote"));
    expect(await screen.findByRole("status", { name: "Notes saved, not read" })).toBeTruthy();
    // Saved unchanged after a failed read: Save is on again, to read them again.
    await waitFor(() => expect(screen.getByRole("button", { name: "Save" })).toHaveProperty("disabled", false));
  });

  it("shows that the notes are being read", async () => {
    vi.mocked(api.saveNotes).mockResolvedValue(profile({ notes: "x" }));
    vi.mocked(api.readNotes).mockReturnValue(new Promise(() => {}));
    render(<ProfileSection />);
    const notes = (await screen.findByRole("textbox", { name: "Anything else" })) as HTMLTextAreaElement;
    await waitFor(() => expect(notes.value).toBe("Senior roles only"));
    fireEvent.change(notes, { target: { value: "x" } });
    fireEvent.click(screen.getByRole("button", { name: "Save" }));
    expect(await screen.findByRole("status", { name: "Reading your notes" })).toBeTruthy();
  });

  it("undoes the last change", async () => {
    vi.mocked(api.undo).mockResolvedValue(profile({ jobType: "any", canUndo: false }));
    render(<ProfileSection />);
    fireEvent.click(await screen.findByRole("button", { name: "Undo" }));
    expect(api.undo).toHaveBeenCalled();
    await waitFor(() =>
      expect(screen.getByRole("button", { name: "Job type: Any" }).getAttribute("aria-pressed")).toBe("true"),
    );
    expect(await screen.findByRole("status", { name: "Undone" })).toBeTruthy();
  });

  it("offers Upload CV when none is attached", async () => {
    vi.mocked(api.readProfile).mockResolvedValue(profile({ cv: null }));
    render(<ProfileSection />);
    expect(await screen.findByRole("button", { name: "Upload CV" })).toBeTruthy();
    expect(screen.getByText("A .pdf, .md or .txt file.")).toBeTruthy();
  });

  it("imports the chosen CV", async () => {
    vi.mocked(api.readProfile).mockResolvedValue(profile({ cv: null }));
    vi.mocked(api.pickCv).mockResolvedValue("/tmp/my-cv.pdf");
    vi.mocked(api.importCv).mockResolvedValue(profile({ cv: { name: "my-cv.pdf", chars: 99, truncated: true } }));
    render(<ProfileSection />);
    fireEvent.click(await screen.findByRole("button", { name: "Upload CV" }));
    expect(await screen.findByText("my-cv.pdf")).toBeTruthy();
    expect(api.importCv).toHaveBeenCalledWith("/tmp/my-cv.pdf");
    expect(screen.getByText("99 characters read, cut to the limit")).toBeTruthy();
  });

  it("shows a failed read with its error", async () => {
    vi.mocked(api.readProfile).mockRejectedValue(new CommandError("the database is locked"));
    render(<ProfileSection />);
    expect(await screen.findByText("the database is locked")).toBeTruthy();
    expect(screen.getByRole("group", { name: "Could not read your preferences" })).toBeTruthy();
  });

  it("shows a failed change in the card", async () => {
    vi.mocked(api.setJobType).mockRejectedValue(new CommandError("the preferences could not be saved"));
    render(<ProfileSection />);
    fireEvent.click(await screen.findByRole("button", { name: "Job type: Remote" }));
    expect(await screen.findByText("the preferences could not be saved")).toBeTruthy();
  });
});
