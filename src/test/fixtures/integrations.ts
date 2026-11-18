// Realistic sample data for the Settings > AI providers and Browser tests (not shipped: only the tests import it).

import type { BrowserSection, BrowserStatus, KindDefaults, Provider, Session } from "@/types/integrations";

/** The five provider kinds as the shell describes them, in the picker's order. */
export const PROVIDER_KINDS: KindDefaults[] = [
  {
    kind: "openai_compatible",
    label: "OpenAI-compatible",
    name: "Local model",
    baseUrl: "",
    model: "",
    needsBaseUrl: true,
    hasKeyField: true,
    signIn: false,
  },
  {
    kind: "anthropic_compatible",
    label: "Anthropic-compatible",
    name: "Anthropic-compatible",
    baseUrl: "",
    model: "",
    needsBaseUrl: true,
    hasKeyField: true,
    signIn: false,
  },
  {
    kind: "openai",
    label: "OpenAI",
    name: "OpenAI",
    baseUrl: "https://api.openai.com/v1",
    model: "gpt-4o-mini",
    needsBaseUrl: false,
    hasKeyField: true,
    signIn: false,
  },
  {
    kind: "anthropic",
    label: "Anthropic",
    name: "Anthropic",
    baseUrl: "https://api.anthropic.com",
    model: "claude-sonnet-4-5",
    needsBaseUrl: false,
    hasKeyField: true,
    signIn: false,
  },
  {
    kind: "codex",
    label: "Codex (ChatGPT)",
    name: "Codex",
    baseUrl: "https://chatgpt.com/backend-api/codex",
    model: "gpt-5-codex",
    needsBaseUrl: false,
    hasKeyField: false,
    signIn: true,
  },
];

/** A saved local OpenAI-compatible provider (Ollama), the default. */
export function aiProvider(patch: Partial<Provider> = {}): Provider {
  return {
    id: 1,
    name: "Local",
    kind: "openai_compatible",
    kindLabel: "OpenAI-compatible",
    baseUrl: "http://localhost:11434/v1",
    model: "llama3",
    isDefault: true,
    account: null,
    ...patch,
  };
}

/** A paired Chrome with the extension connected, allowed in Incognito. */
export const CONNECTED: BrowserStatus = {
  listening: true,
  port: 47615,
  connected: true,
  browser: "Chrome 153",
  extensionVersion: "0.1.0",
  incognitoAllowed: true,
  pairingCode: null,
  pairingWaitingForExtension: false,
  paired: true,
  extensionPath: "/opt/jobpilot/extension",
};

/** No extension has connected yet. */
export const DISCONNECTED: BrowserStatus = {
  ...CONNECTED,
  connected: false,
  browser: null,
  extensionVersion: null,
  incognitoAllowed: false,
  paired: false,
};

/** An extension waits for the user to approve its code. */
export const PAIRING: BrowserStatus = {
  ...DISCONNECTED,
  browser: "Chrome 153",
  extensionVersion: "0.1.0",
  pairingCode: "482913",
};

/** The user approved the code in JobPilot; the extension's popup has not said Pair yet. */
export const PAIRING_APPROVED: BrowserStatus = {
  ...PAIRING,
  pairingWaitingForExtension: true,
};

export function site(patch: Partial<Session> = {}): Session {
  return {
    id: 1,
    name: "LinkedIn",
    host: "www.linkedin.com",
    status: "ok",
    verifiedAt: new Date(Date.now() - 5 * 60_000).toISOString(),
    loginInProgress: false,
    ...patch,
  };
}

/** The section with two sites: one signed in, one signed out and never checked. */
export function browserSection(patch: Partial<BrowserSection> = {}): BrowserSection {
  return {
    browser: CONNECTED,
    sessions: [site(), site({ id: 2, name: "Indeed", host: "secure.indeed.com", status: "expired", verifiedAt: null })],
    ...patch,
  };
}
