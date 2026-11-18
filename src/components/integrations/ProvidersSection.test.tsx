import { cleanup, fireEvent, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { CommandError } from "@/lib/problem";
import { renderWithToasts } from "@/test/render";

import * as api from "@/api/providers";
import { aiProvider, PROVIDER_KINDS } from "@/test/fixtures/integrations";
import { ProvidersSection } from "./ProvidersSection";

vi.mock("@/api/providers");

const KINDS = PROVIDER_KINDS;
const provider = aiProvider;

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
  vi.mocked(api.listModels).mockResolvedValue([]);
  vi.mocked(api.codexCliAvailable).mockResolvedValue(false);
});

afterEach(cleanup);

const render = () => renderWithToasts(<ProvidersSection />);

async function openLocal() {
  fireEvent.click(await screen.findByRole("button", { name: "Edit Local" }));
  return screen.findByRole("dialog", { name: "Edit Local" });
}

describe("ProvidersSection", () => {
  it("lists the providers with their default, kind and model, and offers Add provider", async () => {
    render();
    expect(screen.getByRole("heading", { name: "AI providers" })).toBeTruthy();
    expect(await screen.findByText("Default: Local")).toBeTruthy();
    expect(screen.getByText("default")).toBeTruthy();
    expect(screen.getByText("llama3")).toBeTruthy();
    expect(screen.getByText("Anthropic")).toBeTruthy();
    expect(screen.getByRole("button", { name: "Add provider" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "Make Claude the default" })).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Make Local the default" }), "it is the default already").toBeNull();
    expect(screen.queryByRole("dialog"), "the dialog opens on a click").toBeNull();
  });

  it("shows the empty state and a warning tag when there is no provider", async () => {
    vi.mocked(api.loadProviders).mockResolvedValue({ providers: [], kinds: KINDS });
    render();
    expect(await screen.findByText("No provider yet. Add one to generate CVs.")).toBeTruthy();
    expect(screen.getByText("No provider")).toBeTruthy();
  });

  it("opens a saved provider in a dialog with an empty key box and says a key is saved", async () => {
    render();
    const dialog = await openLocal();
    const key = within(dialog).getByLabelText("API key (leave empty to keep the saved one)") as HTMLInputElement;
    expect(key.type).toBe("password");
    expect(key.autocomplete).toBe("off");
    expect(key.value).toBe("");
    expect(await within(dialog).findByText("A key is saved in your system keyring.")).toBeTruthy();
    expect((within(dialog).getByRole("textbox", { name: "Name" }) as HTMLInputElement).value).toBe("Local");
    expect((within(dialog).getByRole("textbox", { name: "Base URL" }) as HTMLInputElement).value).toBe(
      "http://localhost:11434/v1",
    );
    expect(within(dialog).getByRole("button", { name: "Delete" })).toBeTruthy();
    expect(within(dialog).queryByRole("button", { name: "Load models" }), "the models load on their own").toBeNull();
  });

  it("saves a typed key once, closes the dialog and says so: a key is never shown again", async () => {
    vi.mocked(api.saveProvider).mockResolvedValue(provider());
    render();
    const dialog = await openLocal();
    const key = within(dialog).getByLabelText("API key (leave empty to keep the saved one)");
    fireEvent.change(key, { target: { value: "sk-typed-secret" } });
    fireEvent.click(within(dialog).getByRole("button", { name: "Save" }));

    await waitFor(() => expect(api.saveProvider).toHaveBeenCalled());
    expect(vi.mocked(api.saveProvider).mock.calls[0]?.[0]).toMatchObject({
      id: 1,
      name: "Local",
      apiKey: "sk-typed-secret",
    });
    expect(vi.mocked(api.saveProvider).mock.calls[0]?.[0]).not.toHaveProperty("jsonObject");
    expect(await screen.findByRole("status", { name: "Provider saved" })).toBeTruthy();
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    expect(screen.queryByDisplayValue("sk-typed-secret")).toBeNull();
  });

  it("keeps the saved key when the box is left empty", async () => {
    vi.mocked(api.saveProvider).mockResolvedValue(provider());
    render();
    const dialog = await openLocal();
    fireEvent.click(within(dialog).getByRole("button", { name: "Save" }));
    await waitFor(() => expect(api.saveProvider).toHaveBeenCalled());
    expect(vi.mocked(api.saveProvider).mock.calls[0]?.[0]?.apiKey).toBeNull();
  });

  it("adds a provider from the OpenAI-compatible defaults and follows the kind", async () => {
    render();
    fireEvent.click(await screen.findByRole("button", { name: "Add provider" }));
    const dialog = await screen.findByRole("dialog", { name: "Add provider" });
    expect((within(dialog).getByRole("textbox", { name: "Name" }) as HTMLInputElement).value).toBe("Local model");
    expect(within(dialog).queryByRole("button", { name: "Delete" }), "nothing to delete yet").toBeNull();
    expect(within(dialog).getByRole("button", { name: "Cancel" })).toBeTruthy();
    expect(within(dialog).getByLabelText("API key")).toBeTruthy();
    expect(
      within(dialog).getByRole("textbox", { name: "Base URL" }),
      "a compatible kind asks for its address",
    ).toBeTruthy();
    expect(within(dialog).queryByRole("checkbox"), "the JSON mode is not the user's").toBeNull();

    fireEvent.click(within(dialog).getByRole("button", { name: "Type: Anthropic" }));
    expect((within(dialog).getByRole("textbox", { name: "Name" }) as HTMLInputElement).value).toBe("Anthropic");
    expect((within(dialog).getByRole("combobox", { name: "Model" }) as HTMLInputElement).value).toBe(
      "claude-sonnet-4-5",
    );
  });

  it("offers the five kinds", async () => {
    render();
    fireEvent.click(await screen.findByRole("button", { name: "Add provider" }));
    const dialog = await screen.findByRole("dialog", { name: "Add provider" });
    const picker = within(dialog).getByRole("group", { name: "Type" });
    expect(
      within(picker)
        .getAllByRole("button")
        .map((b) => b.textContent),
    ).toEqual(["OpenAI-compatible", "Anthropic-compatible", "OpenAI", "Anthropic", "Codex (ChatGPT)"]);
  });

  it("shows the fixed address of an official kind instead of a Base URL box, and asks for its key", async () => {
    render();
    fireEvent.click(await screen.findByRole("button", { name: "Add provider" }));
    const dialog = await screen.findByRole("dialog", { name: "Add provider" });
    fireEvent.click(within(dialog).getByRole("button", { name: "Type: OpenAI" }));
    expect(within(dialog).queryByRole("textbox", { name: "Base URL" })).toBeNull();
    expect(within(dialog).getByText("Address: https://api.openai.com/v1")).toBeTruthy();
    const key = within(dialog).getByLabelText("API key") as HTMLInputElement;
    expect(key.placeholder).toBe("API key");

    fireEvent.click(within(dialog).getByRole("button", { name: "Type: Anthropic-compatible" }));
    expect(within(dialog).getByRole("textbox", { name: "Base URL" })).toBeTruthy();
  });

  it("signs Codex in with ChatGPT instead of a key or an address", async () => {
    vi.mocked(api.codexCliAvailable).mockResolvedValue(true);
    render();
    fireEvent.click(await screen.findByRole("button", { name: "Add provider" }));
    const dialog = await screen.findByRole("dialog", { name: "Add provider" });
    fireEvent.click(within(dialog).getByRole("button", { name: "Type: Codex (ChatGPT)" }));
    expect(within(dialog).queryByLabelText("API key")).toBeNull();
    expect(within(dialog).queryByRole("textbox", { name: "Base URL" })).toBeNull();
    expect(within(dialog).getByText("Not signed in")).toBeTruthy();
    expect(within(dialog).getByRole("button", { name: "Sign in with ChatGPT" })).toBeTruthy();
    expect(await within(dialog).findByRole("button", { name: "Use my Codex CLI login" })).toBeTruthy();
    expect(
      within(dialog).getByText(
        "Unofficial: uses your ChatGPT plan through the Codex backend. It may stop working without notice.",
      ),
    ).toBeTruthy();
    await waitFor(() => expect(api.listModels).toHaveBeenCalled());
    expect(vi.mocked(api.listModels).mock.calls.at(-1)?.[0], "the fixed list of Codex models").toMatchObject({
      kind: "codex",
      apiKey: null,
    });
    const primaries = within(dialog)
      .getAllByRole("button")
      .filter((b) => b.id === "save-provider");
    expect(primaries).toHaveLength(1);
  });

  it("saves a new Codex provider first, then signs it in and says with which account", async () => {
    vi.mocked(api.saveProvider).mockResolvedValue(
      provider({ id: 9, name: "Codex", kind: "codex", kindLabel: "Codex (ChatGPT)", isDefault: false }),
    );
    let finish: (account: string) => void = () => undefined;
    vi.mocked(api.codexSignIn).mockReturnValue(
      new Promise<string>((resolve) => {
        finish = resolve;
      }),
    );
    vi.mocked(api.codexCancelSignIn).mockResolvedValue(undefined);
    render();
    fireEvent.click(await screen.findByRole("button", { name: "Add provider" }));
    const dialog = await screen.findByRole("dialog", { name: "Add provider" });
    fireEvent.click(within(dialog).getByRole("button", { name: "Type: Codex (ChatGPT)" }));
    fireEvent.click(within(dialog).getByRole("button", { name: "Sign in with ChatGPT" }));

    await waitFor(() => expect(api.codexSignIn).toHaveBeenCalledWith(9));
    expect(vi.mocked(api.saveProvider).mock.calls[0]?.[0]).toMatchObject({ id: 0, kind: "codex" });
    fireEvent.click(await within(dialog).findByRole("button", { name: "Cancel sign-in" }));
    expect(api.codexCancelSignIn).toHaveBeenCalled();

    finish("me@example.com");
    expect(await within(dialog).findByText("Signed in as me@example.com")).toBeTruthy();
    expect(screen.getByRole("dialog"), "the dialog stays open on the saved provider").toBeTruthy();
    expect(within(dialog).queryByRole("button", { name: "Cancel sign-in" })).toBeNull();
  });

  it("saves an edited provider before signing in, and shows in the form why the sign-in failed", async () => {
    const codex = provider({ id: 5, name: "Codex", kind: "codex", kindLabel: "Codex (ChatGPT)" });
    vi.mocked(api.saveProvider).mockResolvedValue(codex);
    vi.mocked(api.codexSignIn).mockRejectedValue(new CommandError("the sign-in was cancelled"));
    vi.mocked(api.loadProviders).mockResolvedValue({ providers: [codex], kinds: KINDS });
    render();
    fireEvent.click(await screen.findByRole("button", { name: "Edit Codex" }));
    const dialog = await screen.findByRole("dialog", { name: "Edit Codex" });
    fireEvent.change(within(dialog).getByRole("textbox", { name: "Name" }), { target: { value: "My Codex" } });
    fireEvent.click(within(dialog).getByRole("button", { name: "Sign in with ChatGPT" }));
    expect(await within(dialog).findByText("Could not sign in: the sign-in was cancelled")).toBeTruthy();
    expect(vi.mocked(api.saveProvider).mock.calls[0]?.[0], "the form is saved first").toMatchObject({
      id: 5,
      kind: "codex",
      name: "My Codex",
    });
    expect(api.codexSignIn).toHaveBeenCalledWith(5);
  });

  it("no longer says a key is saved once the kind changes, and tests without it", async () => {
    vi.mocked(api.testProvider).mockResolvedValue(undefined);
    render();
    const dialog = await openLocal();
    expect(await within(dialog).findByText("A key is saved in your system keyring.")).toBeTruthy();
    fireEvent.click(within(dialog).getByRole("button", { name: "Type: Anthropic-compatible" }));
    expect(within(dialog).queryByText("A key is saved in your system keyring.")).toBeNull();
    expect(within(dialog).getByLabelText("API key")).toBeTruthy();
    fireEvent.click(within(dialog).getByRole("button", { name: "Test" }));
    await waitFor(() => expect(api.testProvider).toHaveBeenCalled());
    expect(vi.mocked(api.testProvider).mock.calls[0]?.[0], "another kind: not with the saved key").toMatchObject({
      id: 0,
      kind: "anthropic_compatible",
      apiKey: null,
    });
  });

  it("tests the provider and says in the form that it works, or why it does not", async () => {
    vi.mocked(api.testProvider).mockResolvedValueOnce(undefined);
    render();
    const dialog = await openLocal();
    fireEvent.click(within(dialog).getByRole("button", { name: "Test" }));
    expect(await within(dialog).findByText("✓ Works")).toBeTruthy();

    vi.mocked(api.testProvider).mockRejectedValueOnce(new CommandError("HTTP 401 Unauthorized"));
    fireEvent.click(within(dialog).getByRole("button", { name: "Test" }));
    expect(await within(dialog).findByText("The test failed: HTTP 401 Unauthorized")).toBeTruthy();
    expect(within(dialog).getByRole("alert")).toBeTruthy();
    expect(within(dialog).queryByText("✓ Works"), "a new run clears the old result").toBeNull();
  });

  it("lists the models on its own and suggests them in the model box", async () => {
    vi.mocked(api.listModels).mockResolvedValue(["llama3", "mistral"]);
    render();
    const dialog = await openLocal();
    expect(await within(dialog).findByText("2 models available: pick one or type an id.")).toBeTruthy();
    expect(vi.mocked(api.listModels).mock.calls[0]?.[0]).toMatchObject({ id: 1, baseUrl: "http://localhost:11434/v1" });

    const model = within(dialog).getByRole("combobox", { name: "Model" }) as HTMLInputElement;
    fireEvent.click(model);
    fireEvent.click(await screen.findByRole("option", { name: "mistral" }));
    expect(model.value).toBe("mistral");
  });

  it("lists the models again once the edited address is left, without the saved key", async () => {
    vi.mocked(api.listModels).mockResolvedValue(["llama3"]);
    render();
    const dialog = await openLocal();
    await waitFor(() => expect(api.listModels).toHaveBeenCalledTimes(1));
    const address = within(dialog).getByRole("textbox", { name: "Base URL" });
    fireEvent.change(address, { target: { value: "http://localhost:8080/v1" } });
    await new Promise((resolve) => setTimeout(resolve, 50));
    expect(api.listModels, "nothing goes to a half-typed address").toHaveBeenCalledTimes(1);

    fireEvent.blur(address);
    await waitFor(() => expect(api.listModels).toHaveBeenCalledTimes(2));
    expect(vi.mocked(api.listModels).mock.calls[1]?.[0], "a new address: not with the saved key").toMatchObject({
      id: 0,
      baseUrl: "http://localhost:8080/v1",
      apiKey: null,
    });
    fireEvent.blur(address);
    await new Promise((resolve) => setTimeout(resolve, 50));
    expect(api.listModels, "unchanged: not listed again").toHaveBeenCalledTimes(2);
  });

  it("makes another provider the default from its row", async () => {
    vi.mocked(api.makeDefault).mockResolvedValue(undefined);
    render();
    fireEvent.click(await screen.findByRole("button", { name: "Make Claude the default" }));
    await waitFor(() => expect(api.makeDefault).toHaveBeenCalledWith(2));
    expect(await screen.findByRole("status", { name: "Default provider" })).toBeTruthy();
  });

  it("reports a failed Make default in a toast", async () => {
    vi.mocked(api.makeDefault).mockRejectedValue(new CommandError("the database is locked"));
    render();
    fireEvent.click(await screen.findByRole("button", { name: "Make Claude the default" }));
    const failed = await screen.findByRole("alert", { name: "Could not make it the default" });
    expect(within(failed).getByText("the database is locked")).toBeTruthy();
  });

  it("deletes in two steps and closes the dialog", async () => {
    vi.mocked(api.removeProvider).mockResolvedValue(undefined);
    render();
    const dialog = await openLocal();
    fireEvent.click(within(dialog).getByRole("button", { name: "Delete" }));
    expect(api.removeProvider).not.toHaveBeenCalled();
    fireEvent.click(within(dialog).getByRole("button", { name: "Click again to delete" }));
    await waitFor(() => expect(api.removeProvider).toHaveBeenCalledWith(1));
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    expect(await screen.findByRole("status", { name: "Provider deleted" })).toBeTruthy();
  });

  it("shows in the form why a provider could not be saved, and keeps it open", async () => {
    vi.mocked(api.saveProvider).mockRejectedValue(new CommandError("Enter a name."));
    render();
    const dialog = await openLocal();
    fireEvent.click(within(dialog).getByRole("button", { name: "Save" }));
    expect(await within(dialog).findByText("Could not save: Enter a name.")).toBeTruthy();
    expect(screen.getByRole("dialog")).toBeTruthy();
    expect(screen.queryByRole("alert", { name: "Could not save" }), "a form error is no toast").toBeNull();
  });

  it("reports a failed read in a toast and offers Retry", async () => {
    vi.mocked(api.loadProviders).mockRejectedValue(new CommandError("the database is locked"));
    render();
    const failed = await screen.findByRole("alert", { name: "Could not read the providers" });
    expect(within(failed).getByText("the database is locked")).toBeTruthy();
    expect(screen.getByText("Could not read the providers.")).toBeTruthy();
    expect(screen.getByRole("button", { name: "Retry" })).toBeTruthy();
  });
});
