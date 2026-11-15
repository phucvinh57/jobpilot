import { describe, expect, it } from "vitest";

import {
  addAccessibleName,
  addButton,
  addLabel,
  addTooltip,
  canAdd,
  itemKey,
  topicKey,
  type AddButton,
} from "./preferences";

describe("the add button", () => {
  it("follows where the topic stands", () => {
    expect(addButton("absent", false)).toBe("add");
    // A save is running: no second click.
    expect(addButton("absent", true)).toBe("adding");
    expect(addButton("added", false)).toBe("added");
    // Saved: it reads Added even before the pending flag clears.
    expect(addButton("added", true)).toBe("added");
    expect(addButton("avoided", false)).toBe("avoided");
  });

  it("is enabled only when there is something to add", () => {
    const others: AddButton[] = ["adding", "added", "avoided"];
    for (const state of others) {
      expect(canAdd(state), state).toBe(false);
    }
    expect(canAdd("add")).toBe(true);
  });

  it("says what it does", () => {
    expect(addLabel("added")).toBe("Added");
    expect(addLabel("avoided")).toBe("Add to preferences");
    expect(addTooltip("avoided")).toBe("In your avoid list");
    expect(addTooltip("added")).toBe("Added to your preferences");
    expect(addAccessibleName("avoided", "Go")).toContain("avoid");
    expect(addAccessibleName("add", "Go")).toBe("Add Go to preferences");
    expect(addAccessibleName("added", "Go")).toBe("Go is in your preferences");
  });
});

describe("topic keys", () => {
  it("compare topics the way the preferences do", () => {
    expect(itemKey("  ÉCOLE ")).toBe("école");
    expect(topicKey("skills", " Rust ")).toBe(topicKey("skills", "rust"));
    expect(topicKey("skills", "Rust")).not.toBe(topicKey("domains", "Rust"));
  });

  it("cut an entry to 80 characters", () => {
    expect(itemKey("a".repeat(100))).toBe("a".repeat(80));
    // Characters, not UTF-16 units.
    expect(Array.from(itemKey("\u{1f600}".repeat(90)))).toHaveLength(80);
  });
});
