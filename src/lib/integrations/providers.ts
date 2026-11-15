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
  jsonObject: boolean;
  isDefault: boolean;
  /** A key is saved in the keyring for this provider. */
  hasKey: boolean;
  /** Models loaded from the provider, for the picker. */
  models: string[];
  /** The first click on Delete arms it; the second deletes. */
  confirmDelete: boolean;
}

function defaultsOf(kinds: readonly KindDefaults[], kind: ProviderKind): KindDefaults | undefined {
  return kinds.find((k) => k.kind === kind);
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
      jsonObject: provider.jsonObject,
      isDefault: provider.isDefault,
      hasKey,
      models: [],
      confirmDelete: false,
    };
  }
  const d = defaultsOf(kinds, "openai");
  return {
    id: 0,
    kind: "openai",
    name: d?.name ?? "OpenAI",
    baseUrl: d?.baseUrl ?? "",
    model: d?.model ?? "",
    apiKey: "",
    jsonObject: false,
    isDefault: false,
    hasKey: false,
    models: [],
    confirmDelete: false,
  };
}

/** Switch kind; replaces name, address and model only when they still hold the old kind's defaults
 *  (or are empty), so what the user typed stays. */
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
    baseUrl: pick(draft.baseUrl, from?.baseUrl, to?.baseUrl),
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
    // Only OpenAI-compatible endpoints have the setting.
    jsonObject: draft.jsonObject && draft.kind === "openai",
    apiKey: key === "" ? null : key,
  };
}

/** The label of the delete button: "Discard" for a form never saved, then a two-step delete. */
export function deleteLabel(draft: Pick<Draft, "id" | "confirmDelete">): string {
  if (draft.id === 0) {
    return "Discard";
  }
  return draft.confirmDelete ? "Click again to delete" : "Delete";
}

/** The label of the key field: a saved key stays unless a new one is typed. */
export function keyLabel(draft: Pick<Draft, "hasKey">): string {
  return draft.hasKey ? "API key (leave empty to keep the saved one)" : "API key";
}

/** The tag at the top of the card: which provider generation uses. */
export function providersStatus(providers: readonly Provider[]): { tone: Tone; text: string } {
  const preferred = providers.find((p) => p.isDefault);
  if (preferred) {
    return { tone: "ok", text: `Default: ${preferred.name}` };
  }
  return { tone: "warn", text: providers.length === 0 ? "No provider" : "No default" };
}

/** The line after "Load models". */
export function modelsLine(count: number): string {
  return `${count} model(s) available; pick one from the list.`;
}
