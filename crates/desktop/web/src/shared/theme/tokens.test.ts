import { describe, expect, it } from "vitest";

import { cssVar, DARK, LIGHT, paletteFor, TONES, type Palette } from "./tokens";

/** WCAG relative luminance of `#RRGGBB`. */
function luminance(hex: string): number {
  const channel = (i: number) => {
    const v = parseInt(hex.slice(i, i + 2), 16) / 255;
    return v <= 0.03928 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4;
  };
  return 0.2126 * channel(1) + 0.7152 * channel(3) + 0.0722 * channel(5);
}

function contrast(a: string, b: string): number {
  const [la, lb] = [luminance(a), luminance(b)];
  return (Math.max(la, lb) + 0.05) / (Math.min(la, lb) + 0.05);
}

describe("tokens", () => {
  it("follows the mode", () => {
    expect(paletteFor("light")).toBe(LIGHT);
    expect(paletteFor("dark")).toBe(DARK);
    expect(LIGHT.bg).not.toBe(DARK.bg);
  });

  it("keeps the design hexes", () => {
    expect(LIGHT.bg).toBe("#F6F6F3");
    expect(LIGHT.accent).toBe("#2A4FD6");
    expect(DARK.bg).toBe("#121317");
  });

  it("names css variables in kebab case", () => {
    expect(cssVar("bg")).toBe("--jp-bg");
    expect(cssVar("surfaceAlt")).toBe("--jp-surface-alt");
    expect(cssVar("accentSoft")).toBe("--jp-accent-soft");
  });

  it("maps tones to palette pairs", () => {
    expect(TONES.neutral.bg).toBe("track");
    expect(TONES.accent).toEqual({ fg: "accentDark", bg: "accentSoft", dot: "accent" });
    expect(TONES.warn.fg).toBe("warnFg");
    expect(TONES.ok.dot).toBe("ok");
    expect(TONES.danger.bg).toBe("dangerBg");
  });

  // Every text colour reads on the surface it is drawn on, in both appearances (WCAG AA).
  it.each([
    ["light", LIGHT],
    ["dark", DARK],
  ] as const)("text contrast is at least 4.5 (%s)", (_name, p: Palette) => {
    const pairs: [string, string, string][] = [
      ["ink on bg", p.ink, p.bg],
      ["ink on surface", p.ink, p.surface],
      ["muted on bg", p.muted, p.bg],
      ["muted on surface", p.muted, p.surface],
      ["subtle on surface", p.subtle, p.surface],
      ["subtle on surfaceAlt", p.subtle, p.surfaceAlt],
      ["onFill on accent", p.onFill, p.accent],
      ["accentDark on accentSoft", p.accentDark, p.accentSoft],
      ["warnFg on warnBg", p.warnFg, p.warnBg],
      ["okFg on okBg", p.okFg, p.okBg],
      ["dangerFg on dangerBg", p.dangerFg, p.dangerBg],
      ["muted on track", p.muted, p.track],
    ];
    for (const [name, fg, bg] of pairs) {
      expect(contrast(fg, bg), name).toBeGreaterThanOrEqual(4.5);
    }
  });
});
