// Settings > Job sources, "Suggested sources": JobPilot searches the web in the user's Chrome for job
// boards and pages that fit the roles, domains or skills they pick in a dialog, checks each one, and
// lists those with jobs here to add or dismiss. Every button here is plain: the section's main
// action stays where it is (the dialog's "Search" is its own screen's primary).

import { Banner, Button, Caption, SectionLabel } from "@/components/ui/kit";

import { useSourceSuggestions } from "@/hooks/discovery/useSourceSuggestions";
import { suggestionLine, suggestionName } from "@/lib/discovery/suggestions";
import type { JobSources, SourceSuggestion } from "@/types/discovery";
import { SuggestTermsDialog } from "./SuggestTermsDialog";

function SuggestionRow({
  suggestion,
  disabled,
  onAdd,
  onDismiss,
}: {
  suggestion: SourceSuggestion;
  disabled: boolean;
  onAdd: () => void;
  onDismiss: () => void;
}) {
  const { id } = suggestion;
  const name = suggestionName(suggestion);
  const line = suggestionLine(suggestion);
  return (
    <li
      id={`suggestion-${id}`}
      aria-label={`${name}: ${line}`}
      className="flex items-center gap-3 border border-border bg-surface-alt p-3"
    >
      <div className="flex min-w-0 flex-1 flex-col">
        <span className="truncate font-semibold text-ink">{name}</span>
        <span className="truncate text-small text-muted">{line}</span>
      </div>
      <Button id={`suggestion-add-${id}`} size="compact" aria-label={`Add ${name}`} disabled={disabled} onClick={onAdd}>
        Add
      </Button>
      <Button
        id={`suggestion-dismiss-${id}`}
        variant="ghost"
        size="compact"
        aria-label={`Dismiss ${name}`}
        disabled={disabled}
        onClick={onDismiss}
      >
        Dismiss
      </Button>
    </li>
  );
}

export function SuggestedSources({ data }: { data: JobSources | undefined }) {
  const v = useSourceSuggestions(data);
  const { suggestions, running, waiting } = v;
  return (
    <section
      id="suggested-sources"
      aria-label="Suggested sources"
      className="flex flex-col gap-2 border-t border-border pt-3"
    >
      <div className="flex flex-wrap items-center justify-between gap-2">
        <div className="flex min-w-0 flex-1 flex-col gap-0.5">
          <SectionLabel>Suggested sources</SectionLabel>
          <Caption>
            JobPilot searches the web in your Chrome for job boards and pages that fit the roles, domains or skills you
            pick, and checks each one.
          </Caption>
        </div>
        <div className="flex shrink-0 items-center gap-2">
          {suggestions.length >= 2 && (
            <Button id="suggestion-add-all" disabled={v.changing || running} onClick={v.addAll}>
              Add all
            </Button>
          )}
          <Button
            id="suggest-sources"
            busy={running}
            disabled={data === undefined || running || waiting}
            onClick={v.suggest}
          >
            {running ? "Searching…" : "Suggest sources"}
          </Button>
        </div>
      </div>
      {running && (
        <p id="suggest-progress" role="status" className="text-small text-muted">
          {v.note ?? "Starting the search…"}
        </p>
      )}
      {waiting && (
        <Banner
          id="suggest-waiting"
          tone="warn"
          title="The search is paused: the search page asks you to confirm in Chrome."
          body="Open the browser, deal with the page there, then continue."
          actions={
            <>
              <Button id="suggest-open-browser" size="compact" onClick={v.openBrowser}>
                Open browser
              </Button>
              <Button id="suggest-resume" size="compact" onClick={v.resume}>
                Continue
              </Button>
              <Button id="suggest-cancel" variant="ghost" size="compact" onClick={v.cancel}>
                Stop
              </Button>
            </>
          }
        />
      )}
      {suggestions.length > 0 && (
        <ul role="list" aria-label="Suggestions" className="flex flex-col gap-2">
          {suggestions.map((suggestion) => (
            <SuggestionRow
              key={suggestion.id}
              suggestion={suggestion}
              disabled={v.changing}
              onAdd={() => v.add(suggestion)}
              onDismiss={() => v.dismiss(suggestion.id)}
            />
          ))}
        </ul>
      )}
      <SuggestTermsDialog
        open={v.dialogOpen}
        options={v.options}
        terms={v.terms}
        loadError={v.optionsError}
        onRetry={v.reloadOptions}
        onToggle={v.toggle}
        onToggleRemote={v.toggleRemote}
        onClose={v.closeDialog}
        onSearch={v.start}
      />
      {v.foundNothing && !running && !waiting && suggestions.length === 0 && (
        <p className="text-body text-muted">No new sources found. Try again later or add one yourself.</p>
      )}
    </section>
  );
}
