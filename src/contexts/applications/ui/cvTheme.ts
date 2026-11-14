// How the `cv.yaml` editor looks: the app's own colours (the `--jp-*` variables, so light and dark
// follow the appearance), and colours for the parts of the YAML that `domain/yamlTokens.ts` finds.

import { RangeSetBuilder, type Extension } from "@codemirror/state";
import { Decoration, EditorView, ViewPlugin, type DecorationSet, type ViewUpdate } from "@codemirror/view";

import { yamlTokens, type TokenKind } from "../domain/yamlTokens";

const KIND_CLASS: Record<TokenKind, string> = {
  key: "cm-jp-key",
  string: "cm-jp-string",
  number: "cm-jp-number",
  atom: "cm-jp-atom",
  comment: "cm-jp-comment",
  meta: "cm-jp-meta",
};

function decorations(text: string): DecorationSet {
  const builder = new RangeSetBuilder<Decoration>();
  for (const token of yamlTokens(text)) {
    if (token.to > token.from) {
      builder.add(token.from, token.to, Decoration.mark({ class: KIND_CLASS[token.kind] }));
    }
  }
  return builder.finish();
}

const highlight = ViewPlugin.fromClass(
  class {
    decorations: DecorationSet;

    constructor(view: EditorView) {
      this.decorations = decorations(view.state.doc.toString());
    }

    update(update: ViewUpdate) {
      if (update.docChanged) {
        this.decorations = decorations(update.state.doc.toString());
      }
    }
  },
  { decorations: (plugin) => plugin.decorations },
);

// The base theme of CodeMirror hard-codes light and dark colours for the selection and the cursor
// (`&light` / `&dark` rules that are more specific than ours), hence `!important` there.
const theme = EditorView.theme({
  "&": {
    height: "100%",
    color: "var(--jp-ink)",
    backgroundColor: "var(--jp-surface)",
    fontSize: "12.5px",
  },
  "&.cm-focused": { outline: "none" },
  ".cm-scroller": {
    fontFamily: 'ui-monospace, SFMono-Regular, Menlo, Consolas, "Liberation Mono", monospace',
    lineHeight: "1.55",
    overflow: "auto",
  },
  ".cm-content": { caretColor: "var(--jp-ink)", padding: "4px 0" },
  ".cm-line": { padding: "0 8px" },
  ".cm-cursor, .cm-dropCursor": { borderLeftColor: "var(--jp-ink) !important" },
  ".cm-selectionBackground, .cm-content ::selection": { backgroundColor: "var(--jp-accent-soft) !important" },
  "&.cm-focused > .cm-scroller > .cm-selectionLayer .cm-selectionBackground": {
    backgroundColor: "var(--jp-accent-soft) !important",
  },
  ".cm-jp-key": { color: "var(--jp-accent-dark)", fontWeight: "600" },
  ".cm-jp-string": { color: "var(--jp-ok-fg)" },
  ".cm-jp-number": { color: "var(--jp-warn-fg)" },
  ".cm-jp-atom": { color: "var(--jp-warn-fg)", fontWeight: "600" },
  ".cm-jp-comment": { color: "var(--jp-subtle)", fontStyle: "italic" },
  ".cm-jp-meta": { color: "var(--jp-muted)" },
});

/** The theme and the YAML colours. */
export const cvLook: Extension = [theme, highlight];
