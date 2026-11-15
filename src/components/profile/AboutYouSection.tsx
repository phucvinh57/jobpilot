import { FileText, PenLine, RefreshCw, StickyNote, Upload, X } from "lucide-react";

import { FormError, LoadError } from "@/components/ProblemHelp";
import {
  Button,
  Caption,
  Chip,
  Dialog,
  EmptyState,
  Field,
  Section,
  SectionLabel,
  Spinner,
  TextArea,
  TextInput,
} from "@/components/ui/kit";
import { useAbout, type AboutCard } from "@/hooks/profile/useAbout";
import { useErrorToast } from "@/hooks/useErrorToast";
import { educationLine, roleLine, sourceLine, statusLine, truncatedNote } from "@/lib/profile/about";
import type { About, Source, Summary } from "@/types/profile";
import { Row } from "./Row";

const TITLE = "About you";
const INTRO =
  "Give JobPilot your CV, a LinkedIn export or a few notes about your work. Every CV it writes uses only facts from these documents.";

const TONE_TEXT = { muted: "text-muted", warn: "text-warn", ok: "text-ok" } as const;

function SourceRow({ source, about, card }: { source: Source; about: About; card: AboutCard }) {
  const Icon = source.kind === "file" ? FileText : StickyNote;
  return (
    <li
      id={`about-source-${source.id}`}
      className="flex items-center gap-3 border-b border-border py-2 last:border-b-0"
    >
      <Icon aria-hidden className="size-4 shrink-0 text-muted" />
      <div className="flex min-w-0 flex-1 flex-col">
        <span className="selectable truncate text-body font-medium text-ink">{source.name}</span>
        <span className="text-small text-muted">
          {sourceLine(source)}
          {source.truncated && <span className="text-warn"> · {truncatedNote(about.limits.maxChars)}</span>}
        </span>
      </div>
      <Button
        id={`about-remove-${source.id}`}
        variant="ghost"
        size="compact"
        icon={X}
        aria-label={`Remove ${source.name}`}
        title={`Remove ${source.name}`}
        busy={card.removing.has(source.id)}
        onClick={() => card.remove(source.id)}
      >
        Remove
      </Button>
    </li>
  );
}

/** The "Add a note" dialog: a title, the note, Save. A failed save stays in the dialog. */
function NoteDialog({ card }: { card: AboutCard }) {
  const blocked = card.noteBody.trim() === "" ? null : card.noteBlocked;
  return (
    <Dialog
      id="about-text-dialog"
      open={card.noteOpen}
      title="Add a note"
      description="Facts only: they may appear on a CV."
      onClose={card.closeNote}
      onSubmit={card.saveNote}
      footer={
        <>
          <Button id="about-text-cancel" variant="ghost" disabled={card.savingNote} onClick={card.closeNote}>
            Cancel
          </Button>
          <Button
            id="about-text-save"
            type="submit"
            variant="primary"
            busy={card.savingNote}
            disabled={card.noteBlocked !== null}
            title={card.noteBlocked ?? undefined}
          >
            Save
          </Button>
        </>
      }
    >
      <Field label="Title">
        <TextInput
          id="about-text-title"
          placeholder="e.g. Side projects"
          value={card.noteTitle}
          onChange={(e) => card.setNoteTitle(e.target.value)}
        />
      </Field>
      <Field label="Text">
        <TextArea
          id="about-text-body"
          className="min-h-28"
          placeholder="What you did, where and when, with which tools."
          value={card.noteBody}
          onChange={(e) => card.setNoteBody(e.target.value)}
        />
      </Field>
      {blocked && <p className="text-small text-warn">{blocked}</p>}
      {card.noteError && <FormError id="about-text-error" error={card.noteError} title="Not added" />}
    </Dialog>
  );
}

/** A labelled list of plain lines (untrusted text: shown as text). */
function Lines({ label, lines }: { label: string; lines: string[] }) {
  if (lines.length === 0) {
    return null;
  }
  return (
    <Row label={label}>
      <ul aria-label={label} className="flex flex-col gap-1">
        {lines.map((line, index) => (
          // The same line may come twice (two stints at one company): the key carries the position.
          <li key={`${index}:${line}`} className="selectable text-body text-ink">
            {line}
          </li>
        ))}
      </ul>
    </Row>
  );
}

/** What was understood from the documents, read-only. */
function SummaryView({ summary }: { summary: Summary }) {
  const who = summary.name !== "" || summary.headline !== "";
  return (
    <div
      id="about-summary"
      role="group"
      aria-label="What JobPilot understood"
      className="flex flex-col gap-4 border-t border-border pt-4"
    >
      <SectionLabel>What JobPilot understood</SectionLabel>
      {who && (
        <div className="flex flex-col">
          {summary.name !== "" && <p className="selectable text-body font-semibold text-ink">{summary.name}</p>}
          {summary.headline !== "" && <Caption className="selectable">{summary.headline}</Caption>}
        </div>
      )}
      <Lines label="Roles" lines={summary.roles.map(roleLine)} />
      <Lines label="Education" lines={summary.education.map(educationLine)} />
      {summary.skills.length > 0 && (
        <Row label="Skills">
          <ul aria-label="Skills" className="flex flex-wrap gap-1.5">
            {summary.skills.map((skill) => (
              <li key={skill}>
                <Chip kind="match">{skill}</Chip>
              </li>
            ))}
          </ul>
        </Row>
      )}
      <Lines label="Languages" lines={summary.languages.length > 0 ? [summary.languages.join(", ")] : []} />
      <Lines label="Projects" lines={summary.projects.length > 0 ? [summary.projects.join(", ")] : []} />
      <Lines
        label="Certifications"
        lines={summary.certifications.length > 0 ? [summary.certifications.join(", ")] : []}
      />
    </div>
  );
}

function LeftOut({ items }: { items: string[] }) {
  if (items.length === 0) {
    return null;
  }
  return (
    <Row label="Left out — not found in your documents" hint="Add a document that states it, then read again.">
      <ul id="about-left-out" aria-label="Left out" className="flex flex-col gap-1">
        {items.map((item) => (
          <li key={item} className="selectable text-small text-muted">
            {item}
          </li>
        ))}
      </ul>
    </Row>
  );
}

/** Settings > About you: the documents the profile is read from (files and notes), where the
 *  reading stands, and a read-only summary of what was understood. No primary button: adding a
 *  document reads them again on its own, and Re-read is a plain button. */
export function AboutYouSection() {
  const card = useAbout();
  const { about } = card;
  useErrorToast(card.failure?.error, { title: card.failure?.title, clear: card.clearFailure });

  if (!about) {
    return (
      <Section id="about-you" title={TITLE} hint={INTRO}>
        {card.loadError ? (
          <LoadError
            id="about-error"
            error={card.loadError}
            title="Could not read your documents"
            onRetry={card.retry}
          />
        ) : (
          <Caption>Loading…</Caption>
        )}
      </Section>
    );
  }

  const line = statusLine(about, card.reading, new Date());
  const hasSources = about.sources.length > 0;

  return (
    <Section id="about-you" title={TITLE} hint={INTRO}>
      <div className="flex flex-col gap-4">
        {hasSources ? (
          <ul id="about-sources" aria-label="Your documents" className="flex flex-col">
            {about.sources.map((source) => (
              <SourceRow key={source.id} source={source} about={about} card={card} />
            ))}
          </ul>
        ) : (
          <EmptyState
            icon={FileText}
            title="No documents yet"
            body="Add your CV as a .pdf, .md or .txt file, or write a few lines about your work."
          />
        )}

        <div className="flex flex-wrap items-center gap-2">
          <Button id="about-add-file" icon={Upload} busy={card.adding} onClick={card.addFile}>
            Add file
          </Button>
          <Button id="about-add-text" icon={PenLine} aria-haspopup="dialog" onClick={card.openNote}>
            Add text
          </Button>
        </div>
        <NoteDialog card={card} />

        {hasSources && (
          <div className="flex flex-wrap items-center gap-3">
            <p
              id="about-status"
              role="status"
              aria-live="polite"
              className={`flex min-w-0 flex-1 items-center gap-2 text-small ${TONE_TEXT[line.tone]}`}
            >
              {card.reading && <Spinner />}
              {line.text}
            </p>
            {(line.reread || card.reading) && (
              <Button id="about-reread" size="compact" icon={RefreshCw} disabled={card.reading} onClick={card.reread}>
                Re-read
              </Button>
            )}
          </div>
        )}

        {about.summary && <SummaryView summary={about.summary} />}
        <LeftOut items={about.leftOut} />
      </div>
    </Section>
  );
}
