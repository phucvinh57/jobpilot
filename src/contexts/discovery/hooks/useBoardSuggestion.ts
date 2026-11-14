// Settings > Job sources, the add form: the job boards behind the careers page being entered. The
// page is checked once the address rests (or the field is left), in the background: typing never
// waits, an answer for an older address is dropped, and a failure only means no suggestion.

import { useCallback, useEffect, useRef, useState } from "react";

import * as api from "../api/discoveryApi";
import { SUGGEST_DELAY_MS, suggestTarget } from "../domain/boards";
import type { BoardHint } from "../domain/types";

export interface BoardSuggestion {
  /** The boards of the address as it stands now (empty while it is checked, or when there are none). */
  hints: BoardHint[];
  /** Check the address now (the field was left). */
  flush: () => void;
}

export function useBoardSuggestion(kind: string, url: string): BoardSuggestion {
  const target = suggestTarget(kind, url);
  /** Each address's answer, so a late answer for another address never hides the right one. */
  const [answers, setAnswers] = useState<ReadonlyMap<string, BoardHint[]>>(new Map());
  /** Addresses checked already: each one is read once. */
  const asked = useRef(new Set<string>());

  const check = useCallback((address: string | undefined) => {
    if (address === undefined || asked.current.has(address)) {
      return;
    }
    asked.current.add(address);
    const keep = (hints: BoardHint[]) => setAnswers((known) => new Map(known).set(address, hints));
    void Promise.resolve(address).then(api.suggestBoards).then(keep, () => keep([]));
  }, []);

  useEffect(() => {
    if (target === undefined) {
      return;
    }
    const timer = setTimeout(() => check(target), SUGGEST_DELAY_MS);
    return () => clearTimeout(timer);
  }, [target, check]);

  return {
    hints: (target === undefined ? undefined : answers.get(target)) ?? [],
    flush: () => check(target),
  };
}
