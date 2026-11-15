// Links leave the app through the shell only (system browser, http(s) only).

import { openUrl } from "@/stores/shell";

/** Open `url` in the system browser; a failure is logged (there is nothing the user can fix). */
export function open(url: string): void {
  openUrl(url).catch((e: unknown) => console.warn("opening a link failed", e));
}
