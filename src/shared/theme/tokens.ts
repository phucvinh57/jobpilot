// Design tokens: the light and dark palettes, a neutral (zinc-like) shadcn/ui look with one blue
// accent. `applyTheme` writes them as CSS variables (`--jp-*`), and
// `styles.css` maps those onto Tailwind colour names (`bg-surface`, `text-ink`, ...), which are
// the only colours the app has: views never hard-code one.

/** Every colour of the design for one appearance. */
export interface Palette {
  /** Page background. */
  bg: string;
  /** Cards, inputs, popovers. */
  surface: string;
  /** Navigation and table heads: slightly off the surface. */
  surfaceAlt: string;
  border: string;
  /** The edge of controls: a step darker than `border`, so a white control stays visible on a white card. */
  controlBorder: string;
  /** Primary text. */
  ink: string;
  /** Secondary text. */
  muted: string;
  /** Tertiary text (hints, timestamps); still readable. */
  subtle: string;
  accent: string;
  /** Hover/pressed accent, and text on `accentSoft`. */
  accentDark: string;
  accentSoft: string;
  /** Text and icons on a saturated fill (`accent`, `ok`, `warn`, `danger`). */
  onFill: string;
  warn: string;
  /** Text on `warnBg`. */
  warnFg: string;
  warnBg: string;
  ok: string;
  okFg: string;
  okBg: string;
  danger: string;
  dangerFg: string;
  dangerBg: string;
  /** Neutral chip and bar-track fill. */
  track: string;
  /** A bar that is not highlighted (a skill not in the profile yet). */
  barGrey: string;
}

export const LIGHT: Palette = {
  bg: "#FAFAFA",
  surface: "#FFFFFF",
  surfaceAlt: "#F4F4F5",
  border: "#E4E4E7",
  controlBorder: "#D4D4D8",
  ink: "#09090B",
  muted: "#52525B",
  subtle: "#5F5F68",
  accent: "#2563EB",
  accentDark: "#1D4ED8",
  accentSoft: "#EFF4FF",
  onFill: "#FFFFFF",
  warn: "#B45309",
  warnFg: "#92400E",
  warnBg: "#FFF8EB",
  ok: "#15803D",
  okFg: "#166534",
  okBg: "#EFFBF3",
  danger: "#DC2626",
  dangerFg: "#991B1B",
  dangerBg: "#FEF2F2",
  track: "#F4F4F5",
  barGrey: "#A1A1AA",
};

export const DARK: Palette = {
  bg: "#09090B",
  surface: "#111113",
  surfaceAlt: "#0C0C0E",
  border: "#27272A",
  controlBorder: "#3F3F46",
  ink: "#FAFAFA",
  muted: "#A1A1AA",
  subtle: "#8E8E98",
  accent: "#3B82F6",
  accentDark: "#93C5FD",
  accentSoft: "#14213D",
  onFill: "#09090B",
  warn: "#F59E0B",
  warnFg: "#FCD34D",
  warnBg: "#271B07",
  ok: "#22C55E",
  okFg: "#86EFAC",
  okBg: "#0A2615",
  danger: "#EF4444",
  dangerFg: "#FCA5A5",
  dangerBg: "#2A1010",
  track: "#1C1C1F",
  barGrey: "#52525B",
};

export type Mode = "light" | "dark";

export function paletteFor(mode: Mode): Palette {
  return mode === "dark" ? DARK : LIGHT;
}

/** `surfaceAlt` -> `--jp-surface-alt`. */
export function cssVar(key: keyof Palette): string {
  return `--jp-${key.replace(/[A-Z]/g, (c) => `-${c.toLowerCase()}`)}`;
}

/** What a status or a banner means. Each tone is a foreground / background / dot triple. */
export type Tone = "neutral" | "accent" | "warn" | "ok" | "danger";

export interface ToneColors {
  /** Text and icons. */
  fg: keyof Palette;
  /** Tag and banner fill. */
  bg: keyof Palette;
  /** Strong colour for dots, bars and borders. */
  dot: keyof Palette;
}

export const TONES: Record<Tone, ToneColors> = {
  neutral: { fg: "muted", bg: "track", dot: "barGrey" },
  accent: { fg: "accentDark", bg: "accentSoft", dot: "accent" },
  warn: { fg: "warnFg", bg: "warnBg", dot: "warn" },
  ok: { fg: "okFg", bg: "okBg", dot: "ok" },
  danger: { fg: "dangerFg", bg: "dangerBg", dot: "danger" },
};

/** Tailwind classes of a tone (full names, so Tailwind sees them). */
export const TONE_CLASS: Record<Tone, { fg: string; bg: string; dot: string; border: string }> = {
  neutral: { fg: "text-muted", bg: "bg-track", dot: "bg-bar-grey", border: "border-bar-grey/35" },
  accent: { fg: "text-accent-dark", bg: "bg-accent-soft", dot: "bg-accent", border: "border-accent/35" },
  warn: { fg: "text-warn-fg", bg: "bg-warn-bg", dot: "bg-warn", border: "border-warn/35" },
  ok: { fg: "text-ok-fg", bg: "bg-ok-bg", dot: "bg-ok", border: "border-ok/35" },
  danger: { fg: "text-danger-fg", bg: "bg-danger-bg", dot: "bg-danger", border: "border-danger/35" },
};
