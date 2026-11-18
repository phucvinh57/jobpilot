// The rules of the provider form. A key is typed into the form, goes to the shell and the OS
// keyring, and only "a key is saved" comes back.

import type { Tone } from "@/lib/tokens";
import type { ProviderKind, Provider, KindDefaults, ProviderInput } from "@/types/integrations";

/** The provider being edited (`id == 0`: not saved yet). */
export interface Draft {
  id: number;
  kind: ProviderKind;
  name: string;
  baseUrl: string;
  model: string;
  /** Typed key; never shown again after it is saved. */
  apiKey: string;
  isDefault: boolean;
  /** A key is saved in the keyring for this provider (Codex: it is signed in). */
  hasKey: boolean;
  /** Codex: the ChatGPT account it is signed in with. */
  account: string | null;
  /** The kind and address saved with the provider: its saved key goes to this endpoint only
   *  (`null` / empty for a provider not saved yet). */
  savedKind: ProviderKind | null;
  savedBaseUrl: string;
  /** Models loaded from the provider, for the picker. */
  models: string[];
  /** The first click on Delete arms it; the second deletes. */
  confirmDelete: boolean;
}

function defaultsOf(kinds: readonly KindDefaults[], kind: ProviderKind): KindDefaults | undefined {
  return kinds.find((k) => k.kind === kind);
}

/** The kind a new provider starts with. */
export const NEW_KIND: ProviderKind = "openai_compatible";

/** The fields `kind` has in the form (all of them while the kinds are not read yet). */
export function fieldsOf(
  kinds: readonly KindDefaults[],
  kind: ProviderKind,
): Pick<KindDefaults, "needsBaseUrl" | "hasKeyField" | "signIn"> {
  const d = defaultsOf(kinds, kind);
  return { needsBaseUrl: d?.needsBaseUrl ?? true, hasKeyField: d?.hasKeyField ?? true, signIn: d?.signIn ?? false };
}

/** The form for `provider` (`undefined`: a new OpenAI-compatible one with its defaults). */
export function draftOf(provider: Provider | undefined, kinds: readonly KindDefaults[], hasKey = false): Draft {
  if (provider) {
    return {
      id: provider.id,
      kind: provider.kind,
      name: provider.name,
      baseUrl: provider.baseUrl,
      model: provider.model,
      apiKey: "",
      isDefault: provider.isDefault,
      hasKey,
      account: provider.account,
      savedKind: provider.kind,
      savedBaseUrl: provider.baseUrl,
      models: [],
      confirmDelete: false,
    };
  }
  const d = defaultsOf(kinds, NEW_KIND);
  return {
    id: 0,
    kind: NEW_KIND,
    name: d?.name ?? "Local model",
    baseUrl: d?.baseUrl ?? "",
    model: d?.model ?? "",
    apiKey: "",
    isDefault: false,
    hasKey: false,
    account: null,
    savedKind: null,
    savedBaseUrl: "",
    models: [],
    confirmDelete: false,
  };
}

/** Switch kind; replaces name, address and model only when they still hold the old kind's defaults
 *  (or are empty), so what the user typed stays. A kind with a fixed address always takes it. */
export function switchKind(draft: Draft, kind: ProviderKind, kinds: readonly KindDefaults[]): Draft {
  if (draft.kind === kind) {
    return draft;
  }
  const from = defaultsOf(kinds, draft.kind);
  const to = defaultsOf(kinds, kind);
  const pick = (current: string, old: string | undefined, next: string | undefined): string => {
    const text = current.trim();
    return text === "" || text === old ? (next ?? current) : current;
  };
  return {
    ...draft,
    kind,
    name: pick(draft.name, from?.name, to?.name),
    baseUrl: to && !to.needsBaseUrl ? to.baseUrl : pick(draft.baseUrl, from?.baseUrl, to?.baseUrl),
    model: pick(draft.model, from?.model, to?.model),
    models: [],
  };
}

/** The form as the shell takes it. */
export function inputOf(draft: Draft): ProviderInput {
  const key = draft.apiKey.trim();
  return {
    id: draft.id,
    kind: draft.kind,
    name: draft.name.trim(),
    baseUrl: draft.baseUrl.trim(),
    model: draft.model.trim(),
    apiKey: key === "" ? null : key,
  };
}

/** The form still describes the endpoint it was saved for (same kind and address): its saved key
 *  (or sign-in) belongs to it. */
export function sameEndpoint(draft: Pick<Draft, "id" | "kind" | "baseUrl" | "savedKind" | "savedBaseUrl">): boolean {
  return draft.id !== 0 && draft.kind === draft.savedKind && draft.baseUrl.trim() === draft.savedBaseUrl.trim();
}

/** The form as the shell takes it to test it or list its models. A saved key is sent only to the
 *  endpoint it was saved for: once the kind or the address is edited, the call goes without it (as
 *  for a new provider) unless a key is typed. */
export function probeInputOf(draft: Draft): ProviderInput {
  const input = inputOf(draft);
  return input.apiKey === null && !sameEndpoint(draft) ? { ...input, id: 0 } : input;
}

/** What the model list depends on: it is listed again only when this changes. */
export function modelsSignature(input: ProviderInput): string {
  return JSON.stringify([input.id, input.kind, input.baseUrl, input.apiKey]);
}

/** The label of the delete button: "Discard" for a form never saved, then a two-step delete. */
export function deleteLabel(draft: Pick<Draft, "id" | "confirmDelete">): string {
  if (draft.id === 0) {
    return "Discard";
  }
  return draft.confirmDelete ? "Click again to delete" : "Delete";
}

/** A saved key is there and still belongs to the form (a save keeps it). */
export function keyIsSaved(
  draft: Pick<Draft, "id" | "kind" | "baseUrl" | "savedKind" | "savedBaseUrl" | "hasKey">,
): boolean {
  return draft.hasKey && sameEndpoint(draft);
}

/** The label of the key field: a saved key stays unless a new one is typed (or the kind or address
 *  changed: then it is forgotten). */
export function keyLabel(
  draft: Pick<Draft, "id" | "kind" | "baseUrl" | "savedKind" | "savedBaseUrl" | "hasKey">,
): string {
  return keyIsSaved(draft) ? "API key (leave empty to keep the saved one)" : "API key";
}

/** The sign-in line of a Codex provider. */
export function accountLine(account: string | null): string {
  return account ? `Signed in as ${account}` : "Not signed in";
}

/** The tag at the top of the card: which provider generation uses. */
export function providersStatus(providers: readonly Provider[]): { tone: Tone; text: string } {
  const preferred = providers.find((p) => p.isDefault);
  if (preferred) {
    return { tone: "ok", text: `Default: ${preferred.name}` };
  }
  return { tone: "warn", text: providers.length === 0 ? "No provider" : "No default" };
}

/** The hint under the model box: the models are listed on their own while the dialog is open. */
export function modelsHint(loading: boolean, count: number): string {
  if (loading) {
    return "Listing the models of this provider…";
  }
  if (count === 0) {
    return "Type the model id.";
  }
  return `${count} ${count === 1 ? "model" : "models"} available: pick one or type an id.`;
}
