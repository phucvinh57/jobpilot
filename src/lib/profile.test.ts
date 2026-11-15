import { describe, expect, it } from "vitest";

import { changed, cleanNotes, fade, followNotes, saveShortcut, savePlan } from "./profile";
import type { Profile } from "@/types/profile";

function profile(patch: Partial<Profile> = {}): Profile {
  return {
    jobType: "any",
    fields: [
      {
        key: "work_from",
        label: "Can work from",
        placeholder: "Add a country you may legally work from",
        hint: "Hard limit: jobs that only hire elsewhere are hidden.",
        items: [],
        suggestions: [],
      },
      { key: "locations", label: "Work locations", placeholder: "Add a city or country", items: [], suggestions: [] },
      {
        key: "domains",
        label: "Interested domains",
        placeholder: "Add a domain, e.g. fintech",
        items: [],
        suggestions: [],
      },
      { key: "skills", label: "Skills", placeholder: "Add a skill", items: ["Rust"], suggestions: [] },
    ],
    notes: "",
    canUndo: false,
    limits: { maxListItems: 30, maxItemChars: 80, maxNotesChars: 2000 },
    ...patch,
  };
}

describe("the saved label", () => {
  it("stays, then fades out", () => {
    expect(fade(0)).toBe(1);
    expect(fade(0.5)).toBe(1);
    expect(fade(0.8)).toBeLessThan(1);
    expect(fade(0.8)).toBeGreaterThan(0);
    expect(fade(1)).toBe(0);
    expect(fade(7)).toBe(0);
    expect(fade(-1)).toBe(1);
  });

  it("is not shown for an edit that changed nothing", () => {
    const shown = profile({ jobType: "remote" });
    expect(changed(shown, profile({ jobType: "remote" }))).toBe(false);
    expect(changed(shown, profile({ jobType: "any" }))).toBe(true);
    expect(changed(undefined, shown)).toBe(true);
  });

  it("notices a changed list or note, and ignores what the card shows around them", () => {
    const shown = profile();
    const more = profile();
    const skills = more.fields[3];
    if (skills) {
      skills.items = ["Rust", "Go"];
    }
    expect(changed(shown, more)).toBe(true);
    const country = profile();
    country.fields[0]?.items.push("Vietnam");
    expect(changed(shown, country)).toBe(true);
    expect(changed(shown, profile({ notes: "Senior only" }))).toBe(true);
    // The undo flag and the suggestions are not a change of the preferences.
    expect(changed(shown, profile({ canUndo: true }))).toBe(false);
  });
});

describe("the notes", () => {
  it("are trimmed and cut to the limit in code points", () => {
    expect(cleanNotes("  hi \n", 2000)).toBe("hi");
    expect(cleanNotes("abcdef", 3)).toBe("abc");
    expect(cleanNotes("\u{1F600}\u{1F600}\u{1F600}", 2)).toBe("\u{1F600}\u{1F600}");
  });

  it("read again after a failed read only when saved unchanged", () => {
    expect(savePlan("new", "old", false)).toBe("saveAndRead");
    expect(savePlan("new", "old", true)).toBe("saveAndRead");
    expect(savePlan("same", "same", false)).toBe("nothing");
    expect(savePlan("same", "same", true)).toBe("readAgain");
    expect(savePlan("", "", true)).toBe("nothing");
    expect(savePlan("", "old", false)).toBe("saveAndRead");
  });

  it("follow the stored notes unless the user typed since", () => {
    // Untouched box, the store has other notes (an undo, a reload): show them.
    expect(followNotes("old", "old", "restored", 2000)).toBe("restored");
    // The user typed since the box was synced: leave it.
    expect(followNotes("typing", "old", "restored", 2000)).toBeUndefined();
    // Already showing what is stored.
    expect(followNotes("same", "same", "same", 2000)).toBeUndefined();
    // Trailing spaces are not typing.
    expect(followNotes("old  ", "old", "restored", 2000)).toBe("restored");
  });

  it("name the shortcut after the platform", () => {
    expect(saveShortcut("MacIntel")).toBe("Cmd+Enter");
    expect(saveShortcut("Linux x86_64")).toBe("Ctrl+Enter");
    expect(saveShortcut("")).toBe("Ctrl+Enter");
  });
});
