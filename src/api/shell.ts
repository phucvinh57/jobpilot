// Typed calls over the shell's own `shared_*` commands: setup state, appearance, and the one way a
// link leaves the app.

import type { Appearance } from "@/lib/theme";
import type { Activity, ShellStatus } from "@/types/shell";

import { invoke } from "./bridge";

export function shellStatus(): Promise<ShellStatus> {
  return invoke<ShellStatus>("shared_status");
}

/** The background jobs: graph runs that are running or waiting for the user. */
export function activity(): Promise<Activity[]> {
  return invoke<Activity[]>("shared_activity");
}

/** The saved appearance, as stored (parse it with `parseAppearance`). */
export function savedAppearance(): Promise<string> {
  return invoke<string>("shared_appearance");
}

export function saveAppearance(appearance: Appearance): Promise<void> {
  return invoke<void>("shared_set_appearance", { appearance });
}

/** Open an http(s) link in the system browser (the webview itself never navigates). */
function openUrl(url: string): Promise<void> {
  return invoke<void>("shared_open_url", { url });
}

/** `openUrl` for a click: a failure is logged (there is nothing the user can fix). */
export function openLink(url: string): void {
  openUrl(url).catch((e: unknown) => console.warn("opening a link failed", e));
}
