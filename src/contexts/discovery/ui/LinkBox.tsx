// The link box with "Add job", and under it what is happening to what was put in: a spinner while
// the posting is read, or why it failed with what to do about it.

import { Plus, X } from "lucide-react";
import { useState } from "react";

import { Button, ProblemHelp, Spinner, TextInput } from "@/shared";

import { waitingLabel } from "../domain/text";
import type { AddFailure } from "../domain/addQueue";
import type { LeadInput } from "../domain/types";
import { addInput, clearAddFeedback, submitLink } from "../hooks/todayStore";
import { useToday } from "../hooks/useToday";

interface Props {
  /** The posting being read. */
  adding: LeadInput | undefined;
  /** How many wait behind it. */
  waiting: number;
  failures: readonly AddFailure[];
  note: string | undefined;
}

/** The link box (`#queue-link`) with "Add job" (`#queue-add-job`), from what the store holds. */
export function LinkBox() {
  const t = useToday();
  return <LinkBoxView adding={t.adding} waiting={t.waiting.length} failures={t.addFailures} note={t.linkNote} />;
}

function LinkBoxView({ adding, waiting, failures, note }: Props) {
  const [raw, setRaw] = useState("");
  const blank = raw.trim() === "";
  const behind = waitingLabel(waiting);

  // "Add job" or Enter: the box is emptied once what it holds is taken.
  const submit = () => {
    if (blank) {
      return;
    }
    void submitLink(raw).then((taken) => {
      if (taken) {
        setRaw("");
      }
    });
  };

  return (
    <div className="flex flex-col gap-2">
      <div className="flex items-center gap-2">
        <div className="relative min-w-0 flex-1">
          <Plus aria-hidden className="pointer-events-none absolute top-1/2 left-3 size-4 -translate-y-1/2 text-muted" />
          <TextInput
            id="queue-link"
            aria-label="Job link or posting text"
            placeholder="Paste a job link and press Enter, or drop a file"
            className="pr-9 pl-9"
            value={raw}
            onChange={(e) => {
              setRaw(e.target.value);
              // Typing again clears the "could not add" messages and the remark. Emptying the box
              // (after Enter, or its clear button) does not: it would hide what Enter just said.
              if (e.target.value !== "") {
                clearAddFeedback();
              }
            }}
            onKeyDown={(e) => {
              if (e.key === "Enter") {
                e.preventDefault();
                submit();
              }
            }}
          />
          {!blank && (
            <button
              type="button"
              aria-label="Clear"
              onClick={() => setRaw("")}
              className="absolute top-1/2 right-2 flex size-6 -translate-y-1/2 cursor-pointer items-center justify-center text-muted hover:bg-track hover:text-ink"
            >
              <X aria-hidden className="size-4" />
            </button>
          )}
        </div>
        {/* A plain button: the primary one is "Prepare CV" or "Search now". */}
        <Button id="queue-add-job" disabled={blank} onClick={submit}>
          Add job
        </Button>
      </div>
      {adding !== undefined && (
        <div
          id="add-lead-progress"
          role="status"
          aria-label={behind ? `Reading the posting, ${behind}` : "Reading the posting"}
          className="flex items-center gap-2 border border-border bg-surface px-3 py-2 text-body text-muted"
        >
          <Spinner className="text-accent" />
          <span>Reading the posting&hellip;</span>
          {behind && <span>&middot; {behind}</span>}
        </div>
      )}
      {note && (
        <p id="add-lead-note" className="text-body text-muted">
          {note}
        </p>
      )}
      {failures.map((failure, n) => {
        const { input, problem } = failure;
        const retry = input ? () => void addInput(input) : undefined;
        // A plain error has no advice and no Retry: offer to run it again.
        const plain = input !== undefined && !problem?.advice;
        return (
          <div
            key={`${n}-${failure.message}`}
            id={`add-lead-failure-${n}`}
            role="group"
            aria-label={`The posting could not be added: ${failure.message}`}
            className="flex flex-col gap-1"
          >
            <p className="selectable text-body text-danger-fg">{failure.message}</p>
            {problem && <ProblemHelp problem={problem} onRetry={retry} />}
            {plain && (
              <div className="flex">
                <Button id={`add-lead-again-${n}`} size="compact" onClick={retry}>
                  Try again
                </Button>
              </div>
            )}
          </div>
        );
      })}
    </div>
  );
}
