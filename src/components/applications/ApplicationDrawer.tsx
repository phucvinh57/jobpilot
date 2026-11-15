// The drawer of an application on the Jobs page: status and round, the ONE primary action (the same
// as the full view's), the steps, the interview round and what the company answered, the timeline,
// and the job description in a dialog. "Open full view" opens the Application screen.

import { FileText, Inbox, Maximize2, Settings } from "lucide-react";
import { useState } from "react";

import {
  Button,
  Caption,
  Dialog,
  Drawer,
  FilterChip,
  SectionLabel,
  Spinner,
  Stepper,
  Tag,
  TextInput,
} from "@/components/ui/kit";
import { LoadError } from "@/components/ProblemHelp";
import { useErrorToast } from "@/hooks/useErrorToast";
import { useNav, type SettingsSection } from "@/stores/nav";
import type { Action } from "@/hooks/useAsync";

import type { ApplicationHeader, MarkKey } from "@/types/applications";
import {
  useApplicationDrawer,
  useApplicationMarks,
  type ApplicationDrawerModel,
  type ApplicationMarks,
} from "@/hooks/applications/useApplicationDrawer";
import { useJobFailureToast } from "@/hooks/applications/useFailureToast";
import { JdTab } from "./JdTab";
import { TimelineTab } from "./TimelineTab";

/** The interview round: a chip per usual round (the current one selected), and a free one. */
function RoundPicker({
  header,
  setRound,
}: {
  header: ApplicationHeader;
  setRound: Action<[round: string | null], ApplicationHeader>;
}) {
  const [other, setOther] = useState("");
  useErrorToast(setRound.error, { title: "The round could not be saved", clear: setRound.clearError });
  const options =
    header.round !== null && !header.roundOptions.includes(header.round)
      ? [...header.roundOptions, header.round]
      : header.roundOptions;
  const saveOther = () => {
    const round = other.trim();
    if (round !== "") {
      void setRound.run(round).then((saved) => saved && setOther(""));
    }
  };
  return (
    <section aria-label="Interview round" className="flex flex-col gap-2">
      <div className="flex items-center justify-between gap-2">
        <SectionLabel>Interview round</SectionLabel>
        {header.round !== null && (
          <Button
            id="round-clear"
            variant="ghost"
            size="compact"
            disabled={setRound.busy}
            onClick={() => void setRound.run(null)}
          >
            Clear
          </Button>
        )}
      </div>
      <div role="group" aria-label="Rounds" className="flex flex-wrap gap-1.5">
        {options.map((option) => (
          <FilterChip
            key={option}
            selected={option === header.round}
            disabled={setRound.busy}
            onClick={() => void setRound.run(option)}
          >
            {option}
          </FilterChip>
        ))}
      </div>
      <form
        className="flex items-center gap-2"
        onSubmit={(e) => {
          e.preventDefault();
          saveOther();
        }}
      >
        <TextInput
          id="round-other"
          aria-label="Other round"
          placeholder="Other…"
          value={other}
          onChange={(e) => setOther(e.target.value)}
          className="h-8 flex-1"
        />
        <Button id="round-save" type="submit" size="compact" disabled={setRound.busy || other.trim() === ""}>
          Save
        </Button>
      </form>
    </section>
  );
}

/** What the company answered, marked by hand. A status never moves back, so a mark asks first:
 *  "Mark as Offer? This can't be undone." with a plain confirm (red for Rejected; never a second
 *  primary button) and Cancel. */
function Marks({ header, mark }: { header: ApplicationHeader; mark: ApplicationMarks["mark"] }) {
  const [asking, setAsking] = useState<MarkKey>();
  const pending = header.marks.find((m) => m.key === asking);
  useErrorToast(mark.error, { title: "The answer could not be saved", clear: mark.clearError });
  return (
    <section aria-label="The company answered" className="flex flex-col gap-2">
      <SectionLabel>The company answered</SectionLabel>
      {pending === undefined ? (
        <div className="flex flex-wrap gap-2">
          {header.marks.map((m) => (
            <Button
              key={m.key}
              id={`mark-${m.key}`}
              size="compact"
              variant={m.key === "rejected" ? "danger" : "secondary"}
              disabled={mark.busy}
              onClick={() => {
                mark.clearError();
                setAsking(m.key);
              }}
            >
              {m.label}
            </Button>
          ))}
        </div>
      ) : (
        <div
          role="group"
          aria-label={`Confirm: ${pending.label}`}
          className="flex flex-col gap-2 border border-border bg-surface-alt p-3"
        >
          <p className="text-body text-ink">Record &ldquo;{pending.label}&rdquo;? This can&apos;t be undone.</p>
          <div className="flex flex-wrap gap-2">
            <Button
              id="mark-confirm"
              size="compact"
              variant={pending.key === "rejected" ? "danger" : "secondary"}
              busy={mark.busy}
              disabled={mark.busy}
              onClick={() => void mark.run(pending.key).then(() => setAsking(undefined))}
            >
              Confirm
            </Button>
            <Button
              id="mark-cancel"
              size="compact"
              variant="ghost"
              disabled={mark.busy}
              onClick={() => setAsking(undefined)}
            >
              Cancel
            </Button>
          </div>
        </div>
      )}
    </section>
  );
}

/** The job description in a dialog; read only once opened. */
function JobDescription({ id }: { id: number }) {
  const [open, setOpen] = useState(false);
  const close = () => setOpen(false);
  return (
    <div className="flex">
      <Button id="drawer-jd" icon={FileText} aria-haspopup="dialog" onClick={() => setOpen(true)}>
        Job description
      </Button>
      <Dialog
        id="drawer-jd-dialog"
        open={open}
        width="lg"
        title="Job description"
        onClose={close}
        footer={
          <Button id="drawer-jd-done" variant="ghost" onClick={close}>
            Close
          </Button>
        }
      >
        {open && <JdTab id={id} />}
      </Dialog>
    </div>
  );
}

function Body({ id, model, header }: { id: number; model: ApplicationDrawerModel; header: ApplicationHeader }) {
  const openSettings = useNav((s) => s.openSettings);
  const marks = useApplicationMarks(id);
  const { head } = model;
  const primary = head.primary;
  const showNextJob = header.showNextJob && primary === null;
  useJobFailureToast(head);
  return (
    <>
      <div className="flex flex-col gap-1.5">
        <div className="flex flex-wrap items-center gap-2 text-body text-muted">
          <Tag tone={header.status.tone}>{header.status.label}</Tag>
          {header.round !== null && <Tag tone="warn">{header.round}</Tag>}
          <span className="min-w-0 truncate">{header.subtitle}</span>
        </div>
        {head.jobLabel !== null && (
          <span className="flex items-center gap-2 text-small text-muted">
            <Spinner className="size-3.5 text-accent" />
            {head.jobLabel}
          </span>
        )}
      </div>
      {(primary !== null || showNextJob) && (
        <div className="flex flex-col gap-1.5">
          <div className="flex flex-wrap items-center gap-2">
            {primary !== null && (
              <Button
                id="primary-action"
                variant="primary"
                busy={primary.loading}
                disabled={!primary.enabled}
                onClick={model.onPrimary}
              >
                {primary.label}
              </Button>
            )}
            {primary?.settings != null && (
              <Button
                id="primary-settings"
                icon={Settings}
                onClick={() => openSettings(primary.settings as SettingsSection)}
              >
                Open Settings &gt; {primary.settings}
              </Button>
            )}
            {showNextJob && (
              <Button id="next-job" icon={Inbox} onClick={model.nextJob}>
                Next job
              </Button>
            )}
          </div>
          {primary?.hint != null && <Caption>{primary.hint}</Caption>}
        </div>
      )}
      <Stepper steps={header.steps} current={header.stepIndex} />
      {header.status.key === "interview" && <RoundPicker header={header} setRound={marks.setRound} />}
      {header.marks.length > 0 && <Marks header={header} mark={marks.mark} />}
      <section aria-label="Timeline" className="flex flex-col gap-2">
        <SectionLabel>Timeline</SectionLabel>
        <TimelineTab id={id} />
      </section>
      <JobDescription id={id} />
    </>
  );
}

/** What the drawer shows for application `id`. Shown with `key={id}`, so nothing the user did in
 *  one application (a failed mark, a half-typed round, a question asked) shows in the next. */
function Content({ id, model }: { id: number; model: ApplicationDrawerModel }) {
  const header = model.header;
  if (header === undefined) {
    return model.head.loadError ? (
      <LoadError error={model.head.loadError} title="The application could not be read" onRetry={model.head.reload} />
    ) : (
      <Caption>Loading...</Caption>
    );
  }
  return header === null ? (
    <Caption>This application is gone.</Caption>
  ) : (
    <Body id={id} model={model} header={header} />
  );
}

function AppDrawer({ id, open }: { id: number; open: boolean }) {
  const model = useApplicationDrawer(id);
  const header = model.header;
  const title = header ? (header.company === "" ? header.role : `${header.company}: ${header.role}`) : "Application";
  return (
    <Drawer
      id="drawer"
      open={open}
      width="lg"
      title={title}
      onClose={model.close}
      actions={
        <Button id="drawer-expand" size="compact" icon={Maximize2} onClick={model.expand}>
          Open full view
        </Button>
      }
    >
      <Content key={id} id={id} model={model} />
    </Drawer>
  );
}

/** The drawer of the open application (`useNav` drawer `{ kind: "app" }`), named "{company}: {role}".
 *  It stays mounted with the last application once closed, so it can slide out and give focus back. */
export function ApplicationDrawer() {
  const id = useNav((s) => (s.drawer?.kind === "app" ? s.drawer.id : undefined));
  const [last, setLast] = useState(id);
  if (id !== undefined && id !== last) {
    setLast(id);
  }
  const shown = id ?? last;
  return shown === undefined ? null : <AppDrawer id={shown} open={id !== undefined} />;
}
