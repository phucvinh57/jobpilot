// Typed calls over the shell's own `shared_*` commands: setup state, appearance, and the one way a
// link leaves the app.

import type { ShellStatus } from "@/types/shell";

import { invoke } from "./bridge";

export function shellStatus(): Promise<ShellStatus> {
  return invoke<ShellStatus>("shared_status");
}

/** The saved appearance, as stored (parse it with `parseAppearance`). */
export function savedAppearance(): Promise<string> {
  return invoke<string>("shared_appearance");
}

export function saveAppearance(appearance: string): Promise<void> {
  return invoke<void>("shared_set_appearance", { appearance });
}

/** Open an http(s) link in the system browser (the webview itself never navigates). */
export function openUrl(url: string): Promise<void> {
  return invoke<void>("shared_open_url", { url });
}

/** `openUrl` for a click: a failure is logged (there is nothing the user can fix). */
export function openLink(url: string): void {
  openUrl(url).catch((e: unknown) => console.warn("opening a link failed", e));
}
