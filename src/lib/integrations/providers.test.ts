import { describe, expect, it } from "vitest";

import {
  accountLine,
  deleteLabel,
  fieldsOf,
  draftOf,
  inputOf,
  keyIsSaved,
  keyLabel,
  modelsHint,
  modelsSignature,
  probeInputOf,
  providersStatus,
  switchKind,
} from "./providers";
import { aiProvider, PROVIDER_KINDS } from "@/test/fixtures/integrations";
import type { Provider } from "@/types/integrations";

const KINDS = PROVIDER_KINDS;

function provider(patch: Partial<Provider> = {}): Provider {
  return aiProvider({ id: 3, isDefault: false, ...patch });
}

describe("the provider form", () => {
  it("starts a new provider from the OpenAI-compatible defaults", () => {
    const draft = draftOf(undefined, KINDS);
    expect(draft).toMatchObject({
      id: 0,
      kind: "openai_compatible",
      name: "Local model",
      baseUrl: "",
      model: "",
      apiKey: "",
      hasKey: false,
      account: null,
    });
    expect(draft.models).toEqual([]);
  });

  it("opens a saved provider with an empty key box", () => {
    const draft = draftOf(provider({ isDefault: true }), KINDS, true);
    expect(draft).toMatchObject({
      id: 3,
      name: "Local",
      isDefault: true,
      hasKey: true,
      apiKey: "",
      confirmDelete: false,
    });
    expect(draftOf(provider({ kind: "codex", account: "me@example.com" }), KINDS).account).toBe("me@example.com");
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
    // Back to a compatible kind: the fixed address is not kept, the user enters theirs.
    expect(switchKind(fresh, "anthropic_compatible", KINDS)).toMatchObject({
      name: "Anthropic-compatible",
      baseUrl: "",
      model: "",
    });

    // Typed values stay; empty ones are filled.
    const typed = { ...draftOf(undefined, KINDS), name: "My proxy", baseUrl: "http://proxy.local/v1", model: "  " };
    const switched = switchKind(typed, "anthropic_compatible", KINDS);
    expect(switched.name).toBe("My proxy");
    expect(switched.baseUrl).toBe("http://proxy.local/v1");
    expect(switched.model).toBe("");
  });

  it("gives an official kind its own address, whatever was typed", () => {
    const typed = { ...draftOf(undefined, KINDS), baseUrl: "http://proxy.local/v1" };
    expect(switchKind(typed, "openai", KINDS).baseUrl).toBe("https://api.openai.com/v1");
    expect(switchKind(typed, "codex", KINDS)).toMatchObject({
      name: "Codex",
      baseUrl: "https://chatgpt.com/backend-api/codex",
      model: "gpt-5-codex",
    });
  });

  it("forgets the loaded models when the kind changes, and does nothing for the same kind", () => {
    const draft = { ...draftOf(undefined, KINDS), models: ["gpt-4o"] };
    expect(switchKind(draft, "openai_compatible", KINDS)).toBe(draft);
    expect(switchKind(draft, "anthropic", KINDS).models).toEqual([]);
  });

  it("knows which fields each kind has", () => {
    expect(fieldsOf(KINDS, "openai_compatible")).toEqual({ needsBaseUrl: true, hasKeyField: true, signIn: false });
    expect(fieldsOf(KINDS, "openai")).toEqual({ needsBaseUrl: false, hasKeyField: true, signIn: false });
    expect(fieldsOf(KINDS, "codex")).toEqual({ needsBaseUrl: false, hasKeyField: false, signIn: true });
    expect(fieldsOf([], "codex"), "kinds not read yet: every field").toEqual({
      needsBaseUrl: true,
      hasKeyField: true,
      signIn: false,
    });
  });

  it("says which ChatGPT account Codex is signed in with", () => {
    expect(accountLine("me@example.com")).toBe("Signed in as me@example.com");
    expect(accountLine(null)).toBe("Not signed in");
  });

  it("is sent trimmed, with the key only when one was typed", () => {
    const draft = { ...draftOf(provider(), KINDS), name: " Local ", apiKey: "  sk-1  " };
    expect(inputOf(draft)).toEqual({
      id: 3,
      kind: "openai_compatible",
      name: "Local",
      baseUrl: "http://localhost:11434/v1",
      model: "llama3",
      apiKey: "sk-1",
    });
    expect(inputOf({ ...draft, apiKey: "   " }).apiKey).toBeNull();
    expect(inputOf(draft), "the JSON mode is the engine's, never the form's").not.toHaveProperty("jsonObject");
  });

  it("labels the delete button as a two-step action", () => {
    expect(deleteLabel({ id: 0, confirmDelete: false })).toBe("Discard");
    expect(deleteLabel({ id: 4, confirmDelete: false })).toBe("Delete");
    expect(deleteLabel({ id: 4, confirmDelete: true })).toBe("Click again to delete");
  });

  it("says when a saved key stays: only while the kind and address are the saved ones", () => {
    const saved = draftOf(provider(), KINDS, true);
    expect(keyIsSaved(saved)).toBe(true);
    expect(keyLabel(saved)).toBe("API key (leave empty to keep the saved one)");
    expect(keyLabel({ ...saved, hasKey: false })).toBe("API key");
    const otherKind = switchKind(saved, "anthropic_compatible", KINDS);
    expect(keyIsSaved(otherKind), "another kind").toBe(false);
    expect(keyLabel(otherKind)).toBe("API key");
    expect(keyIsSaved({ ...saved, baseUrl: "http://elsewhere/v1" }), "another address").toBe(false);
    expect(keyIsSaved({ ...draftOf(undefined, KINDS), hasKey: true }), "never saved").toBe(false);
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

  it("sends the saved key only to the endpoint it was saved for when testing or listing the models", () => {
    const saved = draftOf(
      provider({ id: 4, name: "Proxy", baseUrl: "https://api.openai.com/v1", model: "gpt-4o", isDefault: true }),
      KINDS,
      true,
    );
    expect(probeInputOf(saved).id, "same kind and address: the saved key").toBe(4);
    const moved = { ...saved, baseUrl: "https://api.openai.co/v1" };
    expect(probeInputOf(moved).id, "edited address: no saved key").toBe(0);
    expect(probeInputOf({ ...moved, apiKey: "sk-typed" })).toMatchObject({ id: 4, apiKey: "sk-typed" });
    expect(probeInputOf({ ...saved, baseUrl: " https://api.openai.com/v1 " }).id, "spaces do not count").toBe(4);
    expect(probeInputOf({ ...saved, kind: "anthropic_compatible" }).id, "edited kind: no saved key").toBe(0);
    expect(probeInputOf(draftOf(undefined, KINDS)).id, "a new provider has none").toBe(0);
  });

  it("lists the models again only when the id, kind, address or key change", () => {
    const a = inputOf(draftOf(undefined, KINDS));
    expect(modelsSignature({ ...a, name: "Other", model: "x" })).toBe(modelsSignature(a));
    expect(modelsSignature({ ...a, baseUrl: "http://localhost:1/v1" })).not.toBe(modelsSignature(a));
    expect(modelsSignature({ ...a, apiKey: "sk" })).not.toBe(modelsSignature(a));
  });
});
