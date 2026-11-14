// Design tokens: the light and dark palettes. Colours come from the approved design (LIGHT) and
// a derived dark set (DARK). `applyTheme` writes them as CSS variables (`--jp-*`), and
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
  bg: "#F6F6F3",
  surface: "#FFFFFF",
  surfaceAlt: "#FBFBF9",
  border: "#E4E4DF",
  controlBorder: "#D6D6D0",
  ink: "#17181C",
  muted: "#5D6068",
  subtle: "#64676F",
  accent: "#2A4FD6",
  accentDark: "#1C3AA8",
  accentSoft: "#E8EDFC",
  onFill: "#FFFFFF",
  warn: "#B4540A",
  warnFg: "#8A4108",
  warnBg: "#FDF3E7",
  ok: "#1F7A4D",
  okFg: "#17603C",
  okBg: "#E6F4EC",
  danger: "#B42318",
  dangerFg: "#912018",
  dangerBg: "#FDECEA",
  track: "#EFEFEB",
  barGrey: "#B9BAB4",
};

export const DARK: Palette = {
  bg: "#121317",
  surface: "#1A1B20",
  surfaceAlt: "#16171B",
  border: "#2A2C33",
  controlBorder: "#3A3D45",
  ink: "#ECECEA",
  muted: "#A3A6AE",
  subtle: "#9396A0",
  accent: "#6F8BFF",
  accentDark: "#9AAEFF",
  accentSoft: "#1E2647",
  onFill: "#0E1220",
  warn: "#F0A050",
  warnFg: "#F6C08A",
  warnBg: "#3A2614",
  ok: "#4CC38A",
  okFg: "#8FDDB5",
  okBg: "#14301F",
  danger: "#F4796E",
  dangerFg: "#F9A39B",
  dangerBg: "#3A1816",
  track: "#262830",
  barGrey: "#5A5D66",
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
