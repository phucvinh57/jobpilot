import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { CommandError } from "@/shared";

import * as api from "../api/providersApi";
import type { Provider, Providers } from "../domain/providers";
import { ProvidersSection } from "./ProvidersSection";

vi.mock("../api/providersApi");

const KINDS: Providers["kinds"] = [
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
    id: 1,
    name: "Local",
    kind: "openai",
    kindLabel: "OpenAI-compatible",
    baseUrl: "http://localhost:11434/v1",
    model: "llama3",
    jsonObject: false,
    isDefault: true,
    ...patch,
  };
}

beforeEach(() => {
  vi.mocked(api.loadProviders).mockResolvedValue({
    providers: [
      provider(),
      provider({
        id: 2,
        name: "Claude",
        kind: "anthropic",
        kindLabel: "Anthropic",
        model: "claude-sonnet-4-5",
        isDefault: false,
      }),
    ],
    kinds: KINDS,
  });
  vi.mocked(api.hasKey).mockResolvedValue(true);
});

afterEach(cleanup);

async function openLocal() {
  fireEvent.click(await screen.findByRole("button", { name: /Local/ }));
  return screen.findByRole("button", { name: "Save" });
}

describe("ProvidersSection", () => {
  it("lists the providers with their default, kind and model, and offers Add provider", async () => {
    render(<ProvidersSection />);
    expect(screen.getByRole("heading", { name: "AI providers" })).toBeTruthy();
    expect(await screen.findByText("Default: Local")).toBeTruthy();
    expect(screen.getByText("default")).toBeTruthy();
    expect(screen.getByText("llama3")).toBeTruthy();
    expect(screen.getByText("Anthropic")).toBeTruthy();
    expect(screen.getByRole("button", { name: "Add provider" })).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Save" }), "the form opens on a click").toBeNull();
  });

  it("shows the empty state and a warning tag when there is no provider", async () => {
    vi.mocked(api.loadProviders).mockResolvedValue({ providers: [], kinds: KINDS });
    render(<ProvidersSection />);
    expect(await screen.findByText("No provider yet. Add one to generate CVs.")).toBeTruthy();
    expect(screen.getByText("No provider")).toBeTruthy();
  });

  it("opens a saved provider with an empty key box and says a key is saved", async () => {
    render(<ProvidersSection />);
    await openLocal();
    const key = screen.getByLabelText("API key (leave empty to keep the saved one)") as HTMLInputElement;
    expect(key.type).toBe("password");
    expect(key.autocomplete).toBe("off");
    expect(key.value).toBe("");
    expect(await screen.findByText("A key is saved in your system keyring.")).toBeTruthy();
    expect((screen.getByRole("textbox", { name: "Name" }) as HTMLInputElement).value).toBe("Local");
    expect((screen.getByRole("textbox", { name: "Base URL" }) as HTMLInputElement).value).toBe(
      "http://localhost:11434/v1",
    );
    expect(screen.queryByRole("button", { name: "Make default" }), "it is the default already").toBeNull();
    expect(screen.getByRole("button", { name: "Delete" })).toBeTruthy();
  });

  it("saves a typed key once and empties the box: a key is never shown again", async () => {
    vi.mocked(api.saveProvider).mockResolvedValue(provider());
    render(<ProvidersSection />);
    await openLocal();
    const key = screen.getByLabelText("API key (leave empty to keep the saved one)") as HTMLInputElement;
    fireEvent.change(key, { target: { value: "sk-typed-secret" } });
    fireEvent.click(screen.getByRole("button", { name: "Save" }));

    await waitFor(() => expect(api.saveProvider).toHaveBeenCalled());
    expect(vi.mocked(api.saveProvider).mock.calls[0]?.[0]).toMatchObject({
      id: 1,
      name: "Local",
      apiKey: "sk-typed-secret",
      jsonObject: false,
    });
    expect(await screen.findByText("Saved.")).toBeTruthy();
    expect(key.value).toBe("");
    expect(screen.queryByDisplayValue("sk-typed-secret")).toBeNull();
  });

  it("keeps the saved key when the box is left empty", async () => {
    vi.mocked(api.saveProvider).mockResolvedValue(provider());
    render(<ProvidersSection />);
    await openLocal();
    fireEvent.click(screen.getByRole("button", { name: "Save" }));
    await waitFor(() => expect(api.saveProvider).toHaveBeenCalled());
    expect(vi.mocked(api.saveProvider).mock.calls[0]?.[0]?.apiKey).toBeNull();
  });

  it("adds a provider from the OpenAI-compatible defaults and follows the kind", async () => {
    vi.mocked(api.saveProvider).mockResolvedValue(provider({ id: 3, name: "Anthropic", isDefault: false }));
    render(<ProvidersSection />);
    fireEvent.click(await screen.findByRole("button", { name: "Add provider" }));
    expect((screen.getByRole("textbox", { name: "Name" }) as HTMLInputElement).value).toBe("OpenAI");
    expect(screen.getByRole("button", { name: "Discard" })).toBeTruthy();
    expect(screen.getByLabelText("API key")).toBeTruthy();

    fireEvent.click(screen.getByRole("button", { name: "Type: Anthropic" }));
    expect((screen.getByRole("textbox", { name: "Name" }) as HTMLInputElement).value).toBe("Anthropic");
    expect((screen.getByRole("textbox", { name: "Model" }) as HTMLInputElement).value).toBe("claude-sonnet-4-5");
    expect(
      screen.queryByLabelText("Endpoint lacks json_schema support: use json_object mode"),
      "only OpenAI-compatible endpoints",
    ).toBeNull();
  });

  it("tests the provider and says it works, or why it does not", async () => {
    vi.mocked(api.testProvider).mockResolvedValueOnce(undefined);
    render(<ProvidersSection />);
    await openLocal();
    fireEvent.click(screen.getByRole("button", { name: "Test" }));
    expect(await screen.findByText("✓ Works")).toBeTruthy();

    vi.mocked(api.testProvider).mockRejectedValueOnce(new CommandError("HTTP 401 Unauthorized"));
    fireEvent.click(screen.getByRole("button", { name: "Test" }));
    expect(await screen.findByText("HTTP 401 Unauthorized")).toBeTruthy();
    expect(screen.getByRole("group", { name: "The test failed" })).toBeTruthy();
    expect(screen.queryByText("✓ Works"), "a new run clears the old result").toBeNull();
  });

  it("loads the models for a picker that fills the model box", async () => {
    vi.mocked(api.listModels).mockResolvedValue(["llama3", "mistral"]);
    render(<ProvidersSection />);
    await openLocal();
    fireEvent.click(screen.getByRole("button", { name: "Load models" }));
    expect(await screen.findByText("2 model(s) available; pick one from the list.")).toBeTruthy();
    fireEvent.change(screen.getByRole("combobox", { name: "Pick from the loaded models" }), {
      target: { value: "mistral" },
    });
    expect((screen.getByRole("textbox", { name: "Model" }) as HTMLInputElement).value).toBe("mistral");
  });

  it("makes another provider the default", async () => {
    vi.mocked(api.makeDefault).mockResolvedValue(undefined);
    render(<ProvidersSection />);
    fireEvent.click(await screen.findByRole("button", { name: /Claude/ }));
    fireEvent.click(await screen.findByRole("button", { name: "Make default" }));
    await waitFor(() => expect(api.makeDefault).toHaveBeenCalledWith(2));
    expect(await screen.findByText("This is now the default provider.")).toBeTruthy();
  });

  it("deletes in two steps", async () => {
    vi.mocked(api.removeProvider).mockResolvedValue(undefined);
    render(<ProvidersSection />);
    await openLocal();
    fireEvent.click(screen.getByRole("button", { name: "Delete" }));
    expect(api.removeProvider).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole("button", { name: "Click again to delete" }));
    await waitFor(() => expect(api.removeProvider).toHaveBeenCalledWith(1));
    await waitFor(() => expect(screen.queryByRole("button", { name: "Save" })).toBeNull());
  });

  it("shows why a provider could not be saved", async () => {
    vi.mocked(api.saveProvider).mockRejectedValue(new CommandError("Enter a name."));
    render(<ProvidersSection />);
    await openLocal();
    fireEvent.click(screen.getByRole("button", { name: "Save" }));
    expect(await screen.findByText("Enter a name.")).toBeTruthy();
    expect(screen.getByRole("group", { name: "Could not save" })).toBeTruthy();
  });

  it("shows a failed read with its error", async () => {
    vi.mocked(api.loadProviders).mockRejectedValue(new CommandError("the database is locked"));
    render(<ProvidersSection />);
    expect(await screen.findByText("the database is locked")).toBeTruthy();
  });
});
