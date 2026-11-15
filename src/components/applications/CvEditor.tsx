// The `cv.yaml` editor (CodeMirror 6): YAML with soft wrap, the app's colours, and a way to replace
// the whole text from outside (a CV was read) without telling `onChange` about it.

import { yaml } from "@codemirror/lang-yaml";
import { Compartment, EditorState, Transaction } from "@codemirror/state";
import { EditorView, minimalSetup } from "codemirror";
import { useEffect, useRef } from "react";

import type { EditorDoc } from "@/hooks/applications/useCvTab";
import { cvLook } from "./cvTheme";

interface Props {
  /** The text to show. A new object replaces the document (the edit history starts over). */
  doc: EditorDoc | undefined;
  /** The text can be read but not changed (the AI is working on it). */
  readOnly: boolean;
  /** The user changed the text. */
  onChange: (text: string) => void;
}

function lock(readOnly: boolean) {
  return [EditorState.readOnly.of(readOnly), EditorView.editable.of(!readOnly)];
}

export function CvEditor({ doc, readOnly, onChange }: Props) {
  const host = useRef<HTMLDivElement>(null);
  const view = useRef<EditorView>(null);
  const locked = useRef(new Compartment());
  const changed = useRef(onChange);
  changed.current = onChange;
  const applying = useRef(false);
  const first = useRef({ doc, readOnly });

  useEffect(() => {
    const parent = host.current;
    if (parent === null) {
      return;
    }
    const editor = new EditorView({
      parent,
      state: EditorState.create({
        doc: first.current.doc?.text ?? "",
        extensions: [
          minimalSetup,
          yaml(),
          EditorView.lineWrapping,
          EditorView.contentAttributes.of({ "aria-label": "cv.yaml" }),
          cvLook,
          locked.current.of(lock(first.current.readOnly)),
          EditorView.updateListener.of((update) => {
            if (update.docChanged && !applying.current) {
              changed.current(update.state.doc.toString());
            }
          }),
        ],
      }),
    });
    view.current = editor;
    return () => {
      editor.destroy();
      view.current = null;
    };
  }, []);

  useEffect(() => {
    const editor = view.current;
    if (editor === null || doc === undefined || editor.state.doc.toString() === doc.text) {
      return;
    }
    applying.current = true;
    try {
      editor.dispatch({
        changes: { from: 0, to: editor.state.doc.length, insert: doc.text },
        annotations: Transaction.addToHistory.of(false),
      });
    } finally {
      applying.current = false;
    }
  }, [doc]);

  useEffect(() => {
    view.current?.dispatch({ effects: locked.current.reconfigure(lock(readOnly)) });
  }, [readOnly]);

  return <div ref={host} id="cv-editor" className="h-full min-h-[300px] overflow-hidden border border-border" />;
}
