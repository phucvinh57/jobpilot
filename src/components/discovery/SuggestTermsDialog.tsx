// Settings > Job sources, "Suggest sources" > "What to search for": the user picks, value by value,
// which preference terms the search uses (roles, domains, skills, places and Remote). The last pick
// comes preselected; "Search" starts the search and the caller closes the dialog.

import { LoadError } from "@/components/ProblemHelp";
import { Button, Caption, Checkbox, Dialog, FilterChip, SectionLabel } from "@/components/ui/kit";

import type { CommandError } from "@/lib/problem";
import { canSearch, isPicked, searchNote, type TermKind } from "@/lib/discovery/suggestTerms";
import type { SuggestOptions, SuggestTerms } from "@/types/discovery";

const SECTIONS: { kind: TermKind; title: string; empty: string }[] = [
  { kind: "roles", title: "Roles", empty: "No roles in your preferences yet: add them in Profile > Preferences." },
  {
    kind: "domains",
    title: "Domains",
    empty: "No domains in your preferences yet: add them in Profile > Preferences.",
  },
  { kind: "skills", title: "Skills", empty: "No skills in your preferences yet: add them in Profile > Preferences." },
  { kind: "places", title: "Places", empty: "No places in your preferences yet: add them in Profile > Preferences." },
];

export function SuggestTermsDialog({
  open,
  options,
  terms,
  loadError,
  onRetry,
  onToggle,
  onToggleRemote,
  onClose,
  onSearch,
}: {
  open: boolean;
  /** What can be picked; `undefined` while it is read. */
  options: SuggestOptions | undefined;
  /** The current pick. */
  terms: SuggestTerms | undefined;
  loadError: CommandError | undefined;
  onRetry: () => void;
  onToggle: (kind: TermKind, value: string) => void;
  onToggleRemote: () => void;
  onClose: () => void;
  onSearch: () => void;
}) {
  const ready = options !== undefined && terms !== undefined && canSearch(terms);
  return (
    <Dialog
      id="suggest-terms"
      open={open}
      title="What to search for"
      description="JobPilot searches the web for job boards and careers pages for each term you pick."
      onClose={onClose}
      onSubmit={() => ready && onSearch()}
      footer={
        <>
          <Button id="suggest-terms-cancel" variant="ghost" onClick={onClose}>
            Cancel
          </Button>
          <Button id="suggest-terms-search" type="submit" variant="primary" disabled={!ready}>
            Search
          </Button>
        </>
      }
    >
      {options === undefined || terms === undefined ? (
        loadError ? (
          <LoadError
            id="suggest-terms-load-error"
            error={loadError}
            title="Your preferences could not be read"
            onRetry={onRetry}
          />
        ) : (
          <Caption>Reading your preferences…</Caption>
        )
      ) : (
        <>
          {SECTIONS.map(({ kind, title, empty }) => {
            const values = options.available[kind];
            return (
              <div key={kind} role="group" aria-label={title} className="flex flex-col gap-1.5">
                <SectionLabel>{title}</SectionLabel>
                {values.length === 0 ? (
                  <Caption>{empty}</Caption>
                ) : (
                  <div className="flex flex-wrap gap-1.5">
                    {values.map((value, index) => (
                      <FilterChip
                        key={value}
                        id={`suggest-term-${kind}-${index}`}
                        selected={isPicked(terms, kind, value)}
                        onClick={() => onToggle(kind, value)}
                      >
                        {value}
                      </FilterChip>
                    ))}
                  </div>
                )}
                {kind === "places" && options.available.remote && (
                  <Checkbox id="suggest-term-remote" label="Remote" checked={terms.remote} onChange={onToggleRemote} />
                )}
              </div>
            );
          })}
          <Caption>{searchNote(terms)}</Caption>
        </>
      )}
    </Dialog>
  );
}
