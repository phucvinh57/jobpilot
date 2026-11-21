// Settings > Job sources, "Suggested sources" as the screen needs it: pick what to search for in a
// dialog (the preference terms, the last pick preselected), run the search (minutes, with its
// progress line), continue or stop a paused one, and add or dismiss what it found. The list and
// the paused flag come with the job sources (`JobSources`); every change reads them again through the
// `sources` notification the api layer sends. Failures are toasts.

import { useCallback, useRef, useState } from "react";

import { useAction, useQuery } from "@/hooks/useAsync";
import { useErrorToast } from "@/hooks/useErrorToast";
import { useGraphProgress } from "@/hooks/useGraphProgress";

import * as api from "@/api/discovery";
import { addedText, suggestionName, suggestNote } from "@/lib/discovery/suggestions";
import { type TermKind, toggleTerm } from "@/lib/discovery/suggestTerms";
import type { CommandError } from "@/lib/problem";
import { toast } from "@/stores/toast";
import type { JobSources, SourceSuggestion, SuggestOptions, SuggestRun, SuggestTerms } from "@/types/discovery";

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
  /** The "What to search for" dialog is open. */
  dialogOpen: boolean;
  /** What the dialog offers; `undefined` while it is read (or the dialog is closed). */
  options: SuggestOptions | undefined;
  /** Reading the options failed. */
  optionsError: CommandError | undefined;
  /** The dialog's current pick (seeded from the last pick); `undefined` until the options are read. */
  terms: SuggestTerms | undefined;
  /** "Suggest sources": open the dialog (the options are read again each time). */
  suggest: () => void;
  reloadOptions: () => void;
  toggle: (kind: TermKind, value: string) => void;
  toggleRemote: () => void;
  closeDialog: () => void;
  /** "Search": close the dialog and search with the pick. */
  start: () => void;
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
  const [dialogOpen, setDialogOpen] = useState(false);
  /** Bumped on every open, so the options are read again (new preferences show). */
  const [opened, setOpened] = useState(0);
  /** The user's changes to the pick since the dialog opened; `undefined` = the last pick as read. */
  const [edited, setEdited] = useState<SuggestTerms>();
  /** The terms of the last search, for the Retry of its error toast. */
  const lastTerms = useRef<SuggestTerms | undefined>(undefined);

  const optionsQuery = useQuery<SuggestOptions | undefined>(
    () => (dialogOpen ? api.readSuggestOptions() : Promise.resolve(undefined)),
    [dialogOpen, opened],
  );
  const options = dialogOpen ? optionsQuery.data : undefined;
  const optionsError = dialogOpen ? optionsQuery.error : undefined;
  const terms = options === undefined ? undefined : (edited ?? options.picked);

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
  const resume = useCallback(() => void runSearch(() => runResume()), [runSearch, runResume]);
  const searchWith = useCallback(
    (picked: SuggestTerms) => {
      lastTerms.current = picked;
      void runSearch(() => runSuggest(picked));
    },
    [runSearch, runSuggest],
  );
  const retrySearch = useCallback(() => {
    if (lastTerms.current !== undefined) {
      searchWith(lastTerms.current);
    }
  }, [searchWith]);

  const suggest = useCallback(() => {
    setEdited(undefined);
    setOpened((n) => n + 1);
    setDialogOpen(true);
  }, []);
  const closeDialog = useCallback(() => setDialogOpen(false), []);
  const toggle = useCallback(
    (kind: TermKind, value: string) => {
      if (terms !== undefined) {
        setEdited(toggleTerm(terms, kind, value));
      }
    },
    [terms],
  );
  const toggleRemote = useCallback(() => {
    if (terms !== undefined) {
      setEdited({ ...terms, remote: !terms.remote });
    }
  }, [terms]);
  const start = useCallback(() => {
    if (terms === undefined) {
      return;
    }
    setDialogOpen(false);
    searchWith(terms);
  }, [terms, searchWith]);

  useErrorToast(search.error, {
    title: "The search for sources failed",
    onRetry: retrySearch,
    clear: search.clearError,
  });
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
    dialogOpen,
    options,
    optionsError,
    terms,
    suggest,
    reloadOptions: optionsQuery.reload,
    toggle,
    toggleRemote,
    closeDialog,
    start,
    resume,
    cancel: () => void runStop(),
    openBrowser: () => void runOpen(),
    add,
    addAll,
    dismiss: (id) => void runDismiss(id),
  };
}
