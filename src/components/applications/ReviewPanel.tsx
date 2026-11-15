// The review of a filled form: every field with the answer the run gave it, editable here. Nothing is
// sent until the user presses the primary button (Submit, or Continue on a form with more steps).
// Field labels, values and warnings come from a web page: shown as text only.

import clsx from "clsx";

import { Banner, Button, Checkbox, FilterChip, Section, Tag, TextArea, TextInput } from "@/components/ui/kit";
import { ErrorBanner, ProblemHelp } from "@/components/ProblemHelp";

import {
  canRemember,
  checkboxValue,
  chosenOptions,
  isChecked,
  isChosen,
  isHighlighted,
  readOnlyText,
  rowLabel,
  stepTag,
  toggleChoice,
  fieldValue,
} from "@/lib/applications/apply";
import type { ApplyReview, ReviewRow } from "@/types/applications";
import type { ApplySession } from "@/hooks/applications/useApplySession";
import { Screenshots } from "./Screenshots";

/** The editor of one row, by how its field is answered. */
function RowEditor({ row, value, onChange }: { row: ReviewRow; value: string; onChange: (value: string) => void }) {
  switch (row.input) {
    case "readonly":
      return (
        <p className={clsx("selectable text-body", row.shown === "" ? "text-muted" : "text-ink")}>
          {readOnlyText(row)}
        </p>
      );
    case "checkbox":
      return (
        <Checkbox
          id={`cb-${row.id}`}
          label="Yes"
          checked={isChecked(value)}
          onChange={(e) => onChange(checkboxValue(e.target.checked))}
        />
      );
    case "chips":
    case "multiChips": {
      const multi = row.input === "multiChips";
      const chosen = chosenOptions(multi, value);
      return (
        <div role="group" aria-label={row.label} className="flex flex-wrap gap-2">
          {row.options.map((option, i) => (
            // A choice is a toggle (tinted when chosen), never a second primary button.
            <FilterChip
              key={option}
              id={`chip-${row.id}-${i}`}
              selected={isChosen(chosen, option)}
              onClick={() => onChange(toggleChoice(multi, value, option))}
            >
              {option}
            </FilterChip>
          ))}
        </div>
      );
    }
    case "area":
      return (
        <TextArea
          aria-label={row.label}
          rows={3}
          placeholder="Your answer"
          value={value}
          onChange={(e) => onChange(e.target.value)}
        />
      );
    case "line":
      return (
        <TextInput
          aria-label={row.label}
          placeholder={row.placeholder}
          value={value}
          onChange={(e) => onChange(e.target.value)}
        />
      );
  }
}

function Row({ row, session }: { row: ReviewRow; session: ApplySession }) {
  const value = fieldValue(session.edits, row);
  const highlighted = row.editable && isHighlighted(row, value);
  const step = stepTag(row);
  return (
    <div
      id={`row-${row.id}`}
      className={clsx(
        "flex flex-col gap-1.5 border p-3",
        highlighted ? "border-warn bg-warn-bg" : "border-border bg-surface",
      )}
    >
      <div className="flex items-center gap-2">
        <span className="min-w-0 flex-1 text-body font-semibold text-ink">{rowLabel(row)}</span>
        {step !== null && <Tag tone="neutral">{step}</Tag>}
        <Tag tone={row.tag.tone}>{row.tag.label}</Tag>
      </div>
      <RowEditor row={row} value={value} onChange={(next) => session.setValue(row.id, next)} />
      {row.warning !== null && <p className="selectable text-small text-warn-fg">{row.warning}</p>}
      {canRemember(row, value) && (
        <Checkbox
          id={`remember-${row.id}`}
          label="Remember this answer in profile.yaml"
          checked={session.edits.remember.includes(row.id)}
          onChange={() => session.toggleRemember(row.id)}
        />
      )}
    </div>
  );
}

export function ReviewPanel({
  id,
  review,
  session,
  generation,
}: {
  id: number;
  review: ApplyReview;
  session: ApplySession;
  generation: number;
}) {
  const apply = session.apply;
  const error = apply?.error ?? null;
  return (
    <div className="flex flex-col gap-3">
      <Section
        title={review.title}
        hint={review.subtitle}
        actions={
          <Button id="discard-review" disabled={session.busy} onClick={session.discard}>
            {review.closeLabel}
          </Button>
        }
      />
      {review.notice !== null && (
        <Banner id="review-notice" tone={review.notice.tone} title={review.notice.title} body={review.notice.body} />
      )}
      {error !== null && <ErrorBanner id="apply-error" error={error} title="The application step failed" />}
      {review.notes.map((note, i) => (
        <div key={`${i}-${note.text}`} className="flex flex-col gap-1">
          <p className="selectable text-body text-warn-fg">{note.text}</p>
          <ProblemHelp problem={note.problem} />
        </div>
      ))}
      <div className="flex flex-col gap-2">
        {review.rows.map((row) => (
          <Row key={row.id} row={row} session={session} />
        ))}
      </div>
      {review.attached !== null && (
        <div className="flex items-center gap-2">
          <Tag tone="neutral">attached</Tag>
          <span className="text-body text-ink">{review.attached}</span>
        </div>
      )}
      {review.screenshots > 0 && (
        <Screenshots id={id} kind="filled" count={review.screenshots} version={generation} title="Filled form" />
      )}
    </div>
  );
}
