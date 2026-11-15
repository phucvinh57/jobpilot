import { useCallback, useEffect, useRef, useState } from "react";

import { useAction, useQuery } from "@/hooks/useAsync";
import type { CommandError } from "@/lib/problem";

import {
  confirmApply,
  discardApply,
  markApply,
  onApplyProgress,
  openVisible,
  prepareApply,
  readApply,
  saveApplyUrl,
  setDryRun,
  syncApply,
} from "@/api/applications";
import { initialEdits, missingCount, snapshot, toggleRemember, withValue } from "@/lib/applications/apply";
import { LatestSender } from "@/lib/applications/latestSender";
import type { Apply, ApplyProgress, Primary, ReviewEdits } from "@/types/applications";
import { recheckOnFailure } from "./recheck";

export interface ApplySession {
  apply: Apply | undefined;
  loadError: CommandError | undefined;
  /** What the user has typed in the open review (started from the filled form). */
  edits: ReviewEdits;
  /** Required answers still empty, from what is typed right now. */
  missing: number;
  /** The steps of the form run, moving with every step. */
  lines: string[];
  /** Something is running (a form run, or a short action such as closing the browser). */
  busy: boolean;
  setValue: (id: string, value: string) => void;
  toggleRemember: (id: string) => void;
  /** "Review & Submit": open and fill the form. Nothing is sent. */
  prepare: () => void;
  /** The explicit confirmation: Submit, or Continue on a form with more steps. */
  confirm: () => void;
  discard: () => void;
  openVisible: () => void;
  mark: (applied: boolean) => void;
  saveUrl: (url: string) => void;
  setDryRun: (on: boolean) => void;
}

/**
 * The Apply tab of one application. The shell holds the run (open browser, review, handoff); this hook
 * reads it, keeps what the user types, and reports typed answers back (latest wins) so the primary button
 * knows whether required fields are still empty. `onPrimary` gets that primary action.
 */
export function useApplySession(id: number, onPrimary: (primary: Primary | null) => void): ApplySession {
  const query = useQuery(() => readApply(id), [id], ["applications"]);
  const apply = query.data;
  const reload = query.reload;

  // The editors start from a review once; a new review (new generation) starts them again.
  const [typed, setTyped] = useState<{ generation: number; edits: ReviewEdits }>();
  const generation = apply?.generation;
  const edits =
    typed !== undefined && typed.generation === generation ? typed.edits : initialEdits(apply?.review ?? null);

  const onPrimaryRef = useRef(onPrimary);
  onPrimaryRef.current = onPrimary;
  const [sender] = useState(
    () =>
      new LatestSender<ReviewEdits>(async (snap) => {
        const primary = await syncApply(id, snap);
        onPrimaryRef.current(primary);
        // The notice above the rows (required fields empty) is the shell's: read it again.
        reload();
      }),
  );

  const change = useCallback(
    (next: ReviewEdits) => {
      if (apply?.review == null || generation === undefined) {
        return;
      }
      setTyped({ generation, edits: next });
      sender.push(snapshot(apply.review, next));
    },
    [apply, generation, sender],
  );

  const [live, setLive] = useState<ApplyProgress | null>(null);
  useEffect(
    () =>
      onApplyProgress((p) => {
        if (p.appId === id) {
          setLive(p);
        }
      }),
    [id],
  );
  const working = apply?.working ?? null;
  if (working === null && live !== null && apply !== undefined) {
    setLive(null);
  }
  const lines =
    live !== null && working !== null && live.lines.length >= working.lines.length
      ? live.lines
      : (working?.lines ?? []);

  const prepareAction = useAction(() => recheckOnFailure(() => prepareApply(id)));
  const confirmAction = useAction((current: ReviewEdits) => recheckOnFailure(() => confirmApply(id, current)));
  const discardAction = useAction(() => discardApply(id));
  const visibleAction = useAction(() => openVisible(id));
  const markAction = useAction((applied: boolean) => markApply(id, applied));
  const urlAction = useAction((url: string) => saveApplyUrl(id, url));
  const dryAction = useAction(async (on: boolean) => {
    await setDryRun(on);
    reload();
  });

  const review = apply?.review ?? null;
  return {
    apply,
    loadError: query.error,
    edits,
    missing: review === null ? 0 : missingCount(review, edits),
    lines,
    busy:
      apply?.phase === "preparing" ||
      apply?.phase === "submitting" ||
      (apply?.busy ?? null) !== null ||
      prepareAction.busy ||
      confirmAction.busy,
    setValue: (field, value) => change(withValue(edits, field, value)),
    toggleRemember: (field) => change(toggleRemember(edits, field)),
    prepare: () => void prepareAction.run(),
    confirm: () => {
      if (review !== null && apply !== undefined) {
        void confirmAction.run(snapshot(review, edits));
      } else {
        void prepareAction.run();
      }
    },
    discard: () => void discardAction.run(),
    openVisible: () => void visibleAction.run(),
    mark: (applied) => void markAction.run(applied),
    saveUrl: (url) => void urlAction.run(url),
    setDryRun: (on) => void dryAction.run(on),
  };
}
