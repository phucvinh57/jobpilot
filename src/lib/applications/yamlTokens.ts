// Which parts of a YAML text the editor colours. The parser is the one of `@codemirror/lang-yaml`;
// the colours themselves come from the `--jp-*` theme (see `ui/cvTheme.ts`), so they follow
// the appearance and never hard-code a colour.

import { yamlLanguage } from "@codemirror/lang-yaml";

export type TokenKind = "key" | "string" | "number" | "atom" | "comment" | "meta";

export interface Token {
  from: number;
  to: number;
  kind: TokenKind;
}

const NUMBER = /^[-+]?(0x[0-9a-f]+|0o[0-7]+|(\d[\d_]*)(\.\d*)?([eE][-+]?\d+)?|\.\d+)$/i;
const ATOM = /^(true|false|null|~)$/i;

/** The kind of a plain scalar: a number or a keyword stands out, other text is left alone. */
export function scalarKind(text: string): TokenKind | null {
  const value = text.trim();
  if (NUMBER.test(value)) {
    return "number";
  }
  if (ATOM.test(value)) {
    return "atom";
  }
  return null;
}

/** The coloured ranges of `text`, in document order. Keys, quoted strings, numbers and keywords,
 *  comments, and anchors / aliases / tags. Plain text values keep the text colour. */
export function yamlTokens(text: string): Token[] {
  const tokens: Token[] = [];
  yamlLanguage.parser.parse(text).iterate({
    enter(node) {
      switch (node.name) {
        case "Key":
          tokens.push({ from: node.from, to: node.to, kind: "key" });
          return false;
        case "QuotedLiteral":
          tokens.push({ from: node.from, to: node.to, kind: "string" });
          return false;
        case "Literal": {
          const kind = scalarKind(text.slice(node.from, node.to));
          if (kind !== null) {
            tokens.push({ from: node.from, to: node.to, kind });
          }
          return false;
        }
        case "Comment":
          tokens.push({ from: node.from, to: node.to, kind: "comment" });
          return false;
        case "Anchor":
        case "Alias":
        case "Tag":
        case "DirectiveName":
        case "DirectiveEnd":
        case "DocEnd":
          tokens.push({ from: node.from, to: node.to, kind: "meta" });
          return false;
        default:
          return undefined;
      }
    },
  });
  return tokens;
}
