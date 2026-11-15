import { describe, expect, it } from "vitest";

import { deleteLabel, draftOf, inputOf, keyLabel, modelsHint, providersStatus, switchKind } from "./providers";
import type { KindDefaults, Provider } from "@/types/integrations";

const KINDS: KindDefaults[] = [
  { kind: "openai", label: "OpenAI-compatible", name: "OpenAI", baseUrl: "https://api.openai.com/v1", model: "" },
  {
    kind: "anthropic",
    label: "Anthropic",
    name: "Anthropic",
    baseUrl: "https://api.anthropic.com",
    model: "claude-sonnet-4-5",
  },
];

function provider(patch: Partial<Provider> = {}): Provider {
  return {
    id: 3,
    name: "Local",
    kind: "openai",
    kindLabel: "OpenAI-compatible",
    baseUrl: "http://localhost:11434/v1",
    model: "llama3",
    jsonObject: true,
    isDefault: false,
    ...patch,
  };
}

describe("the provider form", () => {
  it("starts a new provider from the OpenAI-compatible defaults", () => {
    const draft = draftOf(undefined, KINDS);
    expect(draft).toMatchObject({
      id: 0,
      kind: "openai",
      name: "OpenAI",
      baseUrl: "https://api.openai.com/v1",
      model: "",
      apiKey: "",
      hasKey: false,
    });
    expect(draft.models).toEqual([]);
  });

  it("opens a saved provider with an empty key box", () => {
    const draft = draftOf(provider({ isDefault: true }), KINDS, true);
    expect(draft).toMatchObject({
      id: 3,
      name: "Local",
      jsonObject: true,
      isDefault: true,
      hasKey: true,
      apiKey: "",
      confirmDelete: false,
    });
  });

  it("changes the kind and keeps what the user typed", () => {
    // Still the defaults: all three follow the kind.
    const fresh = switchKind(draftOf(undefined, KINDS), "anthropic", KINDS);
    expect(fresh).toMatchObject({
      kind: "anthropic",
      name: "Anthropic",
      baseUrl: "https://api.anthropic.com",
      model: "claude-sonnet-4-5",
    });

    // Typed values stay; empty ones are filled.
    const typed = { ...draftOf(undefined, KINDS), name: "My proxy", baseUrl: "http://proxy.local/v1", model: "  " };
    const switched = switchKind(typed, "anthropic", KINDS);
    expect(switched.name).toBe("My proxy");
    expect(switched.baseUrl).toBe("http://proxy.local/v1");
    expect(switched.model).toBe("claude-sonnet-4-5");
  });

  it("forgets the loaded models when the kind changes, and does nothing for the same kind", () => {
    const draft = { ...draftOf(undefined, KINDS), models: ["gpt-4o"] };
    expect(switchKind(draft, "openai", KINDS)).toBe(draft);
    expect(switchKind(draft, "anthropic", KINDS).models).toEqual([]);
  });

  it("is sent trimmed, with the key only when one was typed", () => {
    const draft = { ...draftOf(provider(), KINDS), name: " Local ", apiKey: "  sk-1  " };
    expect(inputOf(draft)).toEqual({
      id: 3,
      kind: "openai",
      name: "Local",
      baseUrl: "http://localhost:11434/v1",
      model: "llama3",
      jsonObject: true,
      apiKey: "sk-1",
    });
    expect(inputOf({ ...draft, apiKey: "   " }).apiKey).toBeNull();
  });

  it("sends json_object only for OpenAI-compatible endpoints", () => {
    const anthropic = { ...draftOf(provider({ kind: "anthropic" }), KINDS), jsonObject: true };
    expect(inputOf(anthropic).jsonObject).toBe(false);
  });

  it("labels the delete button as a two-step action", () => {
    expect(deleteLabel({ id: 0, confirmDelete: false })).toBe("Discard");
    expect(deleteLabel({ id: 4, confirmDelete: false })).toBe("Delete");
    expect(deleteLabel({ id: 4, confirmDelete: true })).toBe("Click again to delete");
  });

  it("says when a saved key stays", () => {
    expect(keyLabel({ hasKey: false })).toBe("API key");
    expect(keyLabel({ hasKey: true })).toBe("API key (leave empty to keep the saved one)");
  });
});

describe("the card", () => {
  it("tags the provider generation uses", () => {
    expect(providersStatus([provider({ isDefault: true })])).toEqual({ tone: "ok", text: "Default: Local" });
    expect(providersStatus([provider()])).toEqual({ tone: "warn", text: "No default" });
    expect(providersStatus([])).toEqual({ tone: "warn", text: "No provider" });
  });

  it("says whether the models are listed, how many, or to type the id", () => {
    expect(modelsHint(true, 0)).toBe("Listing the models of this provider…");
    expect(modelsHint(false, 0)).toBe("Type the model id.");
    expect(modelsHint(false, 1)).toBe("1 model available: pick one or type an id.");
    expect(modelsHint(false, 12)).toBe("12 models available: pick one or type an id.");
  });
});
