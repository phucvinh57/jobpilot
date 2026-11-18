// Typed calls for AI providers. A key travels one way: from the form
// to the shell, which keeps it in the OS keyring. Nothing here returns one.

import { invoke, notifyChanged } from "./bridge";

import type { Provider, ProviderInput, Providers } from "@/types/integrations";

/** The saved providers and what each kind starts with. */
export function loadProviders(): Promise<Providers> {
  return invoke<Providers>("integrations_providers");
}

/** Whether a key is saved for the provider (never the key). */
export function hasKey(id: number): Promise<boolean> {
  return invoke<boolean>("integrations_provider_has_key", { id });
}

/** A provider changed: the list, and the "No AI provider" banner of the shell, follow. */
function providersChanged(): void {
  notifyChanged("integrations", "health");
}

/** Create (`id == 0`) or update a provider. A typed key is stored in the keyring; none keeps the saved one. */
export async function saveProvider(input: ProviderInput): Promise<Provider> {
  const saved = await invoke<Provider>("integrations_save_provider", { input });
  providersChanged();
  return saved;
}

/** Ask the provider a tiny question. Resolves when it answered as asked; rejects with the reason. */
export function testProvider(input: ProviderInput): Promise<void> {
  return invoke<void>("integrations_test_provider", { input });
}

/** The models the provider offers. */
export function listModels(input: ProviderInput): Promise<string[]> {
  return invoke<string[]>("integrations_list_models", { input });
}

export async function makeDefault(id: number): Promise<void> {
  await invoke<void>("integrations_make_default", { id });
  providersChanged();
}

/** Delete the provider and its key. */
export async function removeProvider(id: number): Promise<void> {
  await invoke<void>("integrations_remove_provider", { id });
  providersChanged();
}

/** Sign the saved Codex provider in with ChatGPT in the system browser; resolves with the account's email. */
export async function codexSignIn(id: number): Promise<string> {
  const account = await invoke<string>("integrations_codex_sign_in", { id });
  providersChanged();
  return account;
}

/** Abort a Codex sign-in that waits for the browser. */
export function codexCancelSignIn(): Promise<void> {
  return invoke<void>("integrations_codex_cancel_sign_in");
}

/** Reuse the Codex CLI's login for the saved Codex provider; resolves with the account's email. */
export async function codexImportCli(id: number): Promise<string> {
  const account = await invoke<string>("integrations_codex_import_cli", { id });
  providersChanged();
  return account;
}

/** Whether a Codex CLI login is there to reuse. */
export function codexCliAvailable(): Promise<boolean> {
  return invoke<boolean>("integrations_codex_cli_available");
}
