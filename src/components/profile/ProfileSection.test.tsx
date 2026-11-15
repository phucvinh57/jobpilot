import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { CommandError } from "@/lib/problem";

import * as api from "@/api/profile";
import type { Profile } from "@/types/profile";
import { ProfileSection } from "./ProfileSection";

vi.mock("@/api/profile");

function profile(patch: Partial<Profile> = {}): Profile {
  return {
    jobType: "hybrid",
    fields: [
      {
        key: "work_from",
        label: "Can work from",
        placeholder: "Add a country you may legally work from",
        hint: "Hard limit: jobs that only hire elsewhere are hidden.",
        items: ["Vietnam"],
        suggestions: [],
      },
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
  it("shows the job type, every tag field and the notes", async () => {
    render(<ProfileSection />);
    expect(screen.getByRole("heading", { name: "Preferences" })).toBeTruthy();
    expect(await screen.findByRole("button", { name: "Job type: Hybrid" })).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Upload CV" })).toBeNull();

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
    expect(screen.getByRole("button", { name: "Undo" })).toHaveProperty("disabled", false);
  });

  it("shows Can work from first, with its hint", async () => {
    render(<ProfileSection />);
    const group = await screen.findByRole("group", { name: "Can work from" });
    expect(group.textContent).toContain("Hard limit: jobs that only hire elsewhere are hidden.");
    expect(screen.getByText("Vietnam").id).toBe("profile-work_from-0");
    expect(document.getElementById("profile-add-work_from")).toBeTruthy();
    const labels = screen.getAllByRole("group").map((g) => g.getAttribute("aria-label"));
    expect(labels.indexOf("Can work from")).toBeLessThan(labels.indexOf("Work locations"));
    // Only the field with a hint shows one.
    expect(screen.getByRole("group", { name: "Work locations" }).textContent).not.toContain("Hard limit");
  });

  it("adds a country to work from", async () => {
    vi.mocked(api.addItem).mockResolvedValue({ profile: profile(), message: null });
    render(<ProfileSection />);
    const box = (await screen.findByRole("textbox", { name: "Add to Can work from" })) as HTMLInputElement;
    fireEvent.change(box, { target: { value: "Thailand" } });
    fireEvent.submit(box.closest("form") as HTMLFormElement);
    expect(api.addItem).toHaveBeenCalledWith("work_from", "Thailand");
  });

  it("has no primary action: every change is saved at once", async () => {
    render(<ProfileSection />);
    await screen.findByRole("button", { name: "Job type: Hybrid" });
    // Save is plain, and off while the notes are as saved.
    expect(screen.getByRole("button", { name: "Save" })).toHaveProperty("disabled", true);
    for (const button of screen.getAllByRole("button")) {
      expect(button.className).not.toContain("bg-accent text-on-fill");
    }
  });

  it("adds a suggestion with one click and shows the answer", async () => {
    const next = profile();
    next.fields[3]?.items.push("Kubernetes");
    next.fields[3]!.suggestions = [];
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
    next.fields[3]!.items = ["Go"];
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
