import { describe, expect, it } from "vitest";

import { scalarKind, yamlTokens } from "./yamlTokens";

function pieces(text: string) {
  return yamlTokens(text).map((t) => [t.kind, text.slice(t.from, t.to)]);
}

describe("scalar kinds", () => {
  it("tells numbers and keywords from text", () => {
    expect(scalarKind("42")).toBe("number");
    expect(scalarKind("-3.5")).toBe("number");
    expect(scalarKind("1e6")).toBe("number");
    expect(scalarKind("true")).toBe("atom");
    expect(scalarKind("Null")).toBe("atom");
    expect(scalarKind("~")).toBe("atom");
    expect(scalarKind("Rust")).toBeNull();
    expect(scalarKind("2020-2024")).toBeNull();
  });
});

describe("the YAML tokens", () => {
  it("finds keys, quoted strings, numbers, keywords and comments", () => {
    const text = 'name: "Ada"\nyears: 12 # a comment\nremote: true\nrole: Engineer\n';
    expect(pieces(text)).toEqual([
      ["key", "name"],
      ["string", '"Ada"'],
      ["key", "years"],
      ["number", "12"],
      ["comment", "# a comment"],
      ["key", "remote"],
      ["atom", "true"],
      ["key", "role"],
    ]);
  });

  it("finds the keys of nested mappings and lists", () => {
    const text = "jobs:\n  - company: Acme\n    tech: [Rust, 'Go']\n";
    expect(pieces(text)).toEqual([
      ["key", "jobs"],
      ["key", "company"],
      ["key", "tech"],
      ["string", "'Go'"],
    ]);
  });

  it("colours the anchor, alias and tag marks", () => {
    const kinds = yamlTokens("base: &b x\nuse: *b\nn: !!str 5\n").map((t) => t.kind);
    expect(kinds).toContain("meta");
  });

  it("is empty for empty text, in order for the rest", () => {
    expect(yamlTokens("")).toEqual([]);
    const tokens = yamlTokens("a: 1\nb: 2\n");
    const starts = tokens.map((t) => t.from);
    expect(starts).toEqual([...starts].sort((a, b) => a - b));
  });
});
