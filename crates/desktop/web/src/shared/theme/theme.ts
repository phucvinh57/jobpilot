// Light / dark: the saved choice (`system` follows the desktop), applied as CSS variables.

import { cssVar, paletteFor, type Mode, type Palette } from "./tokens";

export type Appearance = "system" | "light" | "dark";

export const APPEARANCES: { value: Appearance; label: string }[] = [
  { value: "system", label: "System" },
  { value: "light", label: "Light" },
  { value: "dark", label: "Dark" },
];

/** Remembers the last choice so the first frame has the right colours before the shell answers. */
const CACHE_KEY = "jobpilot.appearance";

export function parseAppearance(value: unknown): Appearance {
  return value === "light" || value === "dark" ? value : "system";
}

/** The mode a choice settles on. */
export function resolveMode(appearance: Appearance, systemDark: boolean): Mode {
  if (appearance === "system") {
    return systemDark ? "dark" : "light";
  }
  return appearance;
}

function systemQuery(): MediaQueryList | undefined {
  return typeof window.matchMedia === "function" ? window.matchMedia("(prefers-color-scheme: dark)") : undefined;
}

/** Write the palette of `appearance` onto the document. */
export function applyTheme(appearance: Appearance): Mode {
  const mode = resolveMode(appearance, systemQuery()?.matches ?? false);
  const palette = paletteFor(mode);
  const root = document.documentElement;
  for (const key of Object.keys(palette) as (keyof Palette)[]) {
    root.style.setProperty(cssVar(key), palette[key]);
  }
  root.dataset.theme = mode;
  root.style.colorScheme = mode;
  try {
    localStorage.setItem(CACHE_KEY, appearance);
  } catch {
    // Storage can be unavailable: the choice is still saved by the shell.
  }
  return mode;
}

/** The choice remembered from the last run (`system` when there is none). */
export function cachedAppearance(): Appearance {
  try {
    return parseAppearance(localStorage.getItem(CACHE_KEY));
  } catch {
    return "system";
  }
}

/** Call `onChange` when the desktop switches between light and dark. Returns the unsubscribe. */
export function watchSystem(onChange: () => void): () => void {
  const query = systemQuery();
  if (!query) {
    return () => {};
  }
  query.addEventListener("change", onChange);
  return () => query.removeEventListener("change", onChange);
}
