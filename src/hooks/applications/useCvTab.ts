import { useCallback, useEffect, useMemo, useRef, useState } from "react";

import { toCommandError, type CommandError } from "@/lib/problem";
import { useQuery } from "@/hooks/useAsync";

import { readCv, readCvPage, saveCv } from "@/api/applications";
import { canAsk, editorNote } from "@/lib/applications/cv";
import { Debounce, EDIT_DEBOUNCE, RenderQueue } from "@/lib/applications/renderQueue";
import type { Cv } from "@/types/applications";
import { recheckOnFailure } from "./recheck";
import { usePageUrls } from "./useImages";

/** A text to put into the editor. A new object means "replace the document" (a new CV was read). */
export interface EditorDoc {
  text: string;
}

export interface CvTabInput {
  id: number;
  /** Bumps whenever something rewrote the CV files: read them again. */
  cvEpoch: number;
  /** A generate / revise job is running for this application. */
  busy: boolean;
  /** Start "Ask AI" with the editor text; the failure is shown in the header. */
  revise: (instruction: string, yaml: string) => Promise<void>;
}

export interface CvTabModel {
  cv: Cv | undefined;
  loadError: CommandError | undefined;
  /** Read the CV again (Retry after a failed read). */
  reload: () => void;
  loading: boolean;
  doc: EditorDoc | undefined;
  hasCv: boolean;
  /** The CV does not parse or compile (the edit is kept). */
  problem: CommandError | null;
  /** Fact-guard findings (the edit is kept, the user decides). */
  warnings: string[];
  /** The fact guard ran on the CV shown and found nothing. */
  guardClean: boolean;
  /** The note next to the editor title: locked / rendering / editing. */
  note: string | null;
  rendering: boolean;
  locked: boolean;
  /** Preview pages as image URLs. */
  pages: string[];
  /** A keystroke in the editor. */
  edit: (text: string) => void;
  /** "Ask AI": `true` when it started (the box clears). */
  ask: (instruction: string) => boolean;
}

/**
 * The CV tab: `cv.yaml` in an editor that saves and re-renders 800 ms after the last keystroke (one
 * render at a time), the preview pages, and "Ask AI". An edit that is still waiting is saved when the
 * tab is left.
 */
export function useCvTab({ id, cvEpoch, busy, revise }: CvTabInput): CvTabModel {
  const query = useQuery(() => readCv(id), [id, cvEpoch]);
  const data = query.data;
  const pageUrls = usePageUrls();
  const replacePages = pageUrls.replace;

  const [dirty, setDirty] = useState(false);
  const [rendering, setRendering] = useState(false);
  // What the last render of the CV said. It belongs to the CV that was read when it ended: reading the
  // CV again (the files were rewritten) drops it, and the fact guard's verdict on the files stands.
  const [outcome, setOutcome] = useState<{ cv: Cv; problem: CommandError | null; warnings: string[] } | null>(null);
  const latest = useRef(data);
  latest.current = data;

  const [debounce] = useState(() => new Debounce());
  const [queue] = useState(() => new RenderQueue());
  const text = useRef("");
  const pending = useRef(false);
  const timer = useRef<ReturnType<typeof setTimeout>>(undefined);
  const alive = useRef(true);
  const pageReads = useRef(0);

  const loadPages = useCallback(
    async (count: number) => {
      const read = ++pageReads.current;
      try {
        const blobs = await Promise.all(Array.from({ length: count }, (_, page) => readCvPage(id, page)));
        if (alive.current && read === pageReads.current) {
          replacePages(blobs);
        }
      } catch (e) {
        console.warn("reading the CV previews failed", e);
      }
    },
    [id, replacePages],
  );

  const renderOnce = useCallback(async () => {
    const yaml = text.current;
    pending.current = false;
    setDirty(false);
    try {
      const rendered = await recheckOnFailure(() => saveCv(id, yaml));
      if (alive.current && latest.current !== undefined) {
        setOutcome({ cv: latest.current, problem: null, warnings: rendered.warnings });
      }
      await loadPages(rendered.pageCount);
    } catch (thrown) {
      if (alive.current && latest.current !== undefined) {
        setOutcome({ cv: latest.current, problem: toCommandError(thrown), warnings: [] });
      }
    }
  }, [id, loadPages]);

  const requestRender = useCallback(() => {
    if (!queue.request()) {
      return;
    }
    void (async () => {
      setRendering(true);
      let more = true;
      while (more) {
        await renderOnce();
        more = queue.finish();
      }
      if (alive.current) {
        setRendering(false);
      }
    })();
  }, [queue, renderOnce]);

  // A CV was read: it replaces what the editor shows. A CV without up to date previews (copied in by
  // hand, or edited and never rendered) is rendered once.
  useEffect(() => {
    if (data === undefined) {
      return;
    }
    // An edit that is still waiting to be saved wins over a re-read (another lead's "Prepare CV"
    // ending re-reads every open CV): the save runs, and the editor keeps what was typed.
    if (pending.current) {
      return;
    }
    clearTimeout(timer.current);
    debounce.cancel();
    text.current = data.yaml ?? "";
    pending.current = false;
    setDirty(false);
    void loadPages(data.pageCount);
    if (data.hasCv && data.stale) {
      requestRender();
    }
  }, [data, debounce, loadPages, requestRender]);

  // Leaving the tab (or another application) must not lose an edit that is still waiting.
  useEffect(() => {
    alive.current = true;
    return () => {
      alive.current = false;
      clearTimeout(timer.current);
      if (pending.current) {
        saveCv(id, text.current).catch((e: unknown) => console.warn("saving the CV failed", e));
      }
    };
  }, [id]);

  const edit = useCallback(
    (next: string) => {
      text.current = next;
      pending.current = true;
      setDirty(true);
      const token = debounce.touch();
      clearTimeout(timer.current);
      timer.current = setTimeout(() => {
        if (debounce.isLatest(token)) {
          requestRender();
        }
      }, EDIT_DEBOUNCE);
    },
    [debounce, requestRender],
  );

  const hasCv = data?.hasCv ?? false;
  const last = outcome !== null && outcome.cv === data ? outcome : null;
  const problem = last?.problem ?? null;
  const warnings = last?.warnings ?? [];
  const guardClean = last === null ? (data?.guardClean ?? false) : last.problem === null && last.warnings.length === 0;
  const ask = useCallback(
    (instruction: string): boolean => {
      if (!canAsk({ busy, hasCv, instruction })) {
        return false;
      }
      clearTimeout(timer.current);
      debounce.cancel();
      pending.current = false;
      setDirty(false);
      // The shell writes the editor text to `cv.yaml` before it revises it.
      void revise(instruction.trim(), text.current);
      return true;
    },
    [busy, hasCv, debounce, revise],
  );

  const doc = useMemo(() => (data === undefined ? undefined : { text: data.yaml ?? "" }), [data]);
  return {
    cv: data,
    loadError: query.error,
    reload: query.reload,
    loading: query.loading,
    doc,
    hasCv,
    problem,
    warnings,
    guardClean,
    note: editorNote({ busy, hasCv, rendering, dirty }),
    rendering,
    locked: busy && hasCv,
    pages: pageUrls.urls,
    edit,
    ask,
  };
}
