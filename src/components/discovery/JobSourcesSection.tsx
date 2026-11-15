// Settings > Job sources: where job discovery looks, and in which browser session. What a source
// accepts is checked by the shell when it is added; a source that needs a login is read in a managed
// session the user logged in to under Browser sessions; a source paused at a login, captcha or block
// page is continued from the Jobs page.

import { useState } from "react";

import {
  Banner,
  Button,
  Caption,
  Dialog,
  Field,
  FilterChip,
  Section,
  Switch,
  Tag,
  TextInput,
} from "@/components/ui/kit";
import { FormError, LoadError } from "@/components/ProblemHelp";
import { useErrorToast } from "@/hooks/useErrorToast";
import type { CommandError } from "@/lib/problem";

import { shownSuggestions, suggestionText } from "@/lib/discovery/boards";
import { rowAddress, rowName, sourceTag, statusLine } from "@/lib/discovery/sources";
import type { SessionOption, Source, SourceKind } from "@/types/discovery";
import { useBoardSuggestion } from "@/hooks/discovery/useBoardSuggestion";
import { useJobSources } from "@/hooks/discovery/useJobSources";

function SourceRow({
  source,
  kinds,
  sessions,
  waiting,
  confirming,
  now,
  onEnabled,
  onSession,
  onRemove,
}: {
  source: Source;
  kinds: readonly SourceKind[];
  sessions: readonly SessionOption[];
  waiting: boolean;
  confirming: boolean;
  now: Date;
  onEnabled: (enabled: boolean) => void;
  onSession: (sessionId: number | null) => void;
  onRemove: () => void;
}) {
  const { id } = source;
  const name = rowName(source, kinds);
  const { line, outcome } = statusLine(source, now);
  const tag = sourceTag(outcome, waiting);
  return (
    <li
      id={`source-${id}`}
      aria-label={`${name}: ${line}`}
      className="flex flex-col gap-2 border border-border bg-surface-alt p-3"
    >
      <div className="flex items-center gap-3">
        <Switch
          id={`source-${id}-enabled`}
          ariaLabel={`Search ${name}`}
          checked={source.enabled}
          onChange={onEnabled}
        />
        <div className="flex min-w-0 flex-1 flex-col">
          <span className="truncate font-semibold text-ink">{name}</span>
          <span className="truncate text-small text-muted">{rowAddress(source, kinds)}</span>
        </div>
        <Tag tone={tag.tone}>{tag.text}</Tag>
        <Button id={`source-${id}-remove`} variant="danger" size="compact" onClick={onRemove}>
          {confirming ? "Sure?" : "Remove"}
        </Button>
      </div>
      <Caption className="selectable">{line}</Caption>
      <div role="group" aria-label="Browser session" className="flex flex-wrap items-center gap-1.5">
        <Caption>Browser session:</Caption>
        <FilterChip
          id={`source-${id}-session-none`}
          selected={source.sessionId === null}
          onClick={() => onSession(null)}
        >
          No session
        </FilterChip>
        {sessions.map((session) => (
          <FilterChip
            key={session.id}
            id={`source-${id}-session-${session.id}`}
            selected={source.sessionId === session.id}
            onClick={() => onSession(session.id)}
          >
            {session.name}
          </FilterChip>
        ))}
      </div>
    </li>
  );
}

/** The dialog that adds a source: its kind, an optional name and the board's name or address. A
 *  careers page that lists its jobs on a known board offers that board instead (a plain button). The
 *  reason a source was refused stays in the dialog. Mounted again for every opening, so it starts
 *  empty. */
function AddSourceDialog({
  open,
  kinds,
  adding,
  error,
  onAdd,
  onClose,
}: {
  open: boolean;
  kinds: readonly SourceKind[];
  adding: boolean;
  error: CommandError | undefined;
  onAdd: (kind: string, name: string, url: string) => void;
  onClose: () => void;
}) {
  const [kind, setKind] = useState(kinds[0]?.kind ?? "");
  const [name, setName] = useState("");
  const [url, setUrl] = useState("");
  const hint = kinds.find((k) => k.kind === kind)?.hint ?? "";
  const suggestion = useBoardSuggestion(kind, url);
  return (
    <Dialog
      id="source-dialog"
      open={open}
      title="Add source"
      onClose={onClose}
      onSubmit={() => !adding && onAdd(kind, name, url)}
      footer={
        <>
          <Button id="cancel-source" variant="ghost" disabled={adding} onClick={onClose}>
            Cancel
          </Button>
          <Button id="save-source" type="submit" variant="primary" busy={adding} disabled={adding}>
            Add
          </Button>
        </>
      }
    >
      <div className="flex flex-col gap-1">
        <span className="text-small font-medium text-muted">Kind</span>
        <div role="group" aria-label="Kind" className="flex flex-wrap gap-1.5">
          {kinds.map((k) => (
            <FilterChip
              key={k.kind}
              id={`new-source-kind-${k.kind}`}
              selected={k.kind === kind}
              onClick={() => setKind(k.kind)}
            >
              {k.label}
            </FilterChip>
          ))}
        </div>
      </div>
      <Field label="Name">
        <TextInput
          id="new-source-name"
          placeholder="Name (optional)"
          value={name}
          onChange={(e) => setName(e.target.value)}
        />
      </Field>
      <Field label="Board name or address">
        <TextInput
          id="new-source-url"
          placeholder="Board name or address"
          value={url}
          onChange={(e) => setUrl(e.target.value)}
          onBlur={suggestion.flush}
        />
      </Field>
      <Caption>{hint}</Caption>
      {shownSuggestions(suggestion.hints).map(({ hint: board, bannerId, buttonId }) => (
        <Banner
          key={bannerId}
          id={bannerId}
          tone="accent"
          title={suggestionText(board)}
          body="You can still add the careers page itself."
          actions={
            <Button id={buttonId} size="compact" disabled={adding} onClick={() => onAdd(board.kind, name, board.input)}>
              {`Add ${board.label} source`}
            </Button>
          }
        />
      ))}
      {error && <FormError id="sources-error" error={error} title="Not added" />}
    </Dialog>
  );
}

export function JobSourcesSection() {
  const v = useJobSources();
  const [formOpen, setFormOpen] = useState(false);
  /** How many times the add dialog was opened: each opening starts with an empty form. */
  const [openings, setOpenings] = useState(0);
  /** First click on Remove asks for confirmation; the second one removes. */
  const [confirmRemove, setConfirmRemove] = useState<number>();
  const now = new Date();
  const data = v.data;
  const kinds = data?.kinds ?? [];

  /** Any other change forgets a pending "Sure?". */
  const change = <A extends unknown[]>(fn: (...args: A) => void) => {
    return (...args: A) => {
      setConfirmRemove(undefined);
      fn(...args);
    };
  };

  useErrorToast(v.changeError, { title: "The change was not saved", clear: v.clearChangeError });

  const openForm = () => {
    v.clearAddError();
    setOpenings((n) => n + 1);
    setFormOpen(true);
  };

  const closeForm = () => {
    v.clearAddError();
    setFormOpen(false);
  };

  return (
    <Section
      id="settings-sources"
      title="Job sources"
      hint="Where to look for jobs. Public boards and Hacker News need no login; for a site that wants one, pick a browser session you logged in to."
      actions={
        <Button id="add-source" disabled={data === undefined} onClick={openForm}>
          Add source
        </Button>
      }
    >
      <div className="flex flex-col gap-1">
        <Switch
          id="search-daily"
          label="Search daily while JobPilot is open"
          checked={v.daily}
          onChange={change(v.setDaily)}
        />
        <Caption>Looks again when the last search is a day old. Never while one is running.</Caption>
      </div>
      <ul role="list" aria-label="Sources" className="flex flex-col gap-2">
        {(data?.sources ?? []).map((source) => (
          <SourceRow
            key={source.id}
            source={source}
            kinds={kinds}
            sessions={data?.sessions ?? []}
            waiting={data?.waiting.includes(source.id) ?? false}
            confirming={confirmRemove === source.id}
            now={now}
            onEnabled={change((enabled: boolean) => v.setEnabled(source.id, enabled))}
            onSession={change((sessionId: number | null) => v.setSession(source.id, sessionId))}
            onRemove={() => {
              if (confirmRemove !== source.id) {
                setConfirmRemove(source.id);
              } else {
                setConfirmRemove(undefined);
                v.remove(source.id);
              }
            }}
          />
        ))}
      </ul>
      {v.loaded && data?.sources.length === 0 && (
        <p className="text-body text-muted">No sources yet. Add a company board, a careers page or Hacker News.</p>
      )}
      <LoadError error={v.loadError} title="The job sources could not be read" onRetry={v.reload} />
      <AddSourceDialog
        key={openings}
        open={formOpen}
        kinds={kinds}
        adding={v.adding}
        error={v.addError}
        onAdd={(kind, name, url) => void v.add(kind, name, url).then((added) => added && setFormOpen(false))}
        onClose={closeForm}
      />
    </Section>
  );
}
