// Settings > Job sources, "Suggested sources" as the screen needs it: run the search (minutes, with
// its progress line), continue or stop a paused one, and add or dismiss what it found. The list and
// the paused flag come with the job sources (`JobSources`); every change reads them again through the
// `sources` notification the api layer sends. Failures are toasts.

import { useCallback, useState } from "react";

import { useAction } from "@/hooks/useAsync";
import { useErrorToast } from "@/hooks/useErrorToast";
import { useGraphProgress } from "@/hooks/useGraphProgress";

import * as api from "@/api/discovery";
import { addedText, suggestionName, suggestNote } from "@/lib/discovery/suggestions";
import { toast } from "@/stores/toast";
import type { JobSources, SourceSuggestion, SuggestRun } from "@/types/discovery";

export interface SourceSuggestionsView {
  /** The suggestions waiting for the user, most matching postings first. */
  suggestions: SourceSuggestion[];
  /** The search is paused at a page the user must deal with in Chrome (and is not running now). */
  waiting: boolean;
  /** A search (or its "Continue") is running. */
  running: boolean;
  /** How far the running search is ("Searching 3 of 8"); `undefined` before its first step. */
  note: string | undefined;
  /** The last search ended and found nothing new. */
  foundNothing: boolean;
  /** An Add, Add all or Dismiss is running (the row buttons wait for it). */
  changing: boolean;
  suggest: () => void;
  resume: () => void;
  cancel: () => void;
  openBrowser: () => void;
  add: (suggestion: SourceSuggestion) => void;
  addAll: () => void;
  dismiss: (id: number) => void;
}

export function useSourceSuggestions(data: JobSources | undefined): SourceSuggestionsView {
  const [note, setNote] = useState<string>();
  const [foundNothing, setFoundNothing] = useState(false);

  const search = useAction(api.suggestSources);
  const resumeSearch = useAction(api.resumeSuggest);
  const stop = useAction(api.cancelSuggest);
  const open = useAction(api.openSuggestBrowser);
  const addOne = useAction(api.addSuggestion);
  const addEvery = useAction(api.addAllSuggestions);
  const dismissOne = useAction(api.dismissSuggestion);
  // Also a search started before this screen was shown (the shell says one is running).
  const running = search.busy || resumeSearch.busy || (data?.suggestRunning ?? false);

  useGraphProgress((p) => {
    const text = suggestNote(p);
    if (text !== undefined) {
      setNote(text);
    }
  });

  /** Start the search or its "Continue", and remember whether it ended with nothing. */
  const runSearch = useCallback(async (work: () => Promise<SuggestRun | undefined>) => {
    setNote(undefined);
    setFoundNothing(false);
    const ran = await work();
    setNote(undefined);
    setFoundNothing(ran !== undefined && !ran.waiting && ran.suggestions.length === 0);
  }, []);

  const { run: runSuggest } = search;
  const { run: runResume } = resumeSearch;
  const suggest = useCallback(() => void runSearch(() => runSuggest()), [runSearch, runSuggest]);
  const resume = useCallback(() => void runSearch(() => runResume()), [runSearch, runResume]);

  useErrorToast(search.error, { title: "The search for sources failed", onRetry: suggest, clear: search.clearError });
  useErrorToast(resumeSearch.error, {
    title: "The search for sources failed",
    onRetry: resume,
    clear: resumeSearch.clearError,
  });
  useErrorToast(stop.error, { title: "The search was not stopped", clear: stop.clearError });
  useErrorToast(open.error, { title: "The browser could not be opened", clear: open.clearError });
  useErrorToast(addOne.error, { title: "Not added", clear: addOne.clearError });
  useErrorToast(addEvery.error, { title: "Not added", clear: addEvery.clearError });
  useErrorToast(dismissOne.error, { title: "Not dismissed", clear: dismissOne.clearError });

  const { run: runAdd } = addOne;
  const add = useCallback(
    (suggestion: SourceSuggestion) => {
      void runAdd(suggestion.id).then((added) => {
        if (added !== undefined) {
          toast.ok("Source added", suggestionName(suggestion));
        }
      });
    },
    [runAdd],
  );

  const { run: runAddAll } = addEvery;
  const addAll = useCallback(() => {
    void runAddAll().then((out) => {
      if (out === undefined) {
        return;
      }
      if (out.added > 0) {
        toast.ok(addedText(out.added));
      }
      if (out.error !== null) {
        toast.error(out.error, {
          title: out.failed === 1 ? "1 source was not added" : `${out.failed} sources were not added`,
        });
      }
    });
  }, [runAddAll]);

  const { run: runStop } = stop;
  const { run: runOpen } = open;
  const { run: runDismiss } = dismissOne;

  return {
    suggestions: data?.suggestions ?? [],
    waiting: (data?.suggestWaiting ?? false) && !running,
    running,
    note,
    foundNothing,
    changing: addOne.busy || addEvery.busy || dismissOne.busy,
    suggest,
    resume,
    cancel: () => void runStop(),
    openBrowser: () => void runOpen(),
    add,
    addAll,
    dismiss: (id) => void runDismiss(id),
  };
}
