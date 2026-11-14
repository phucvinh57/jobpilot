// The drawer of an application on the Jobs page: status and round, the ONE primary action (the same
// as the full view's), the steps, the interview round and what the company answered, the timeline,
// and the job description folded away. "Open full view" opens the Application screen.

import { Inbox, Maximize2, Settings } from "lucide-react";
import { useState } from "react";

import {
  Button,
  Caption,
  Drawer,
  ErrorBanner,
  FilterChip,
  SectionLabel,
  Spinner,
  Stepper,
  Tag,
  TextInput,
  useNav,
  type Action,
  type SettingsSection,
} from "@/shared";

import type { ApplicationHeader } from "../domain/types";
import { useApplicationDrawer, type ApplicationDrawerModel } from "../hooks/useApplicationDrawer";
import { JdTab } from "./JdTab";
import { TimelineTab } from "./TimelineTab";

/** The interview round: a chip per usual round (the current one selected), and a free one. */
function RoundPicker({ header, setRound }: { header: ApplicationHeader; setRound: Action<[round: string | null], ApplicationHeader> }) {
  const [other, setOther] = useState("");
  const options = header.round !== null && !header.roundOptions.includes(header.round) ? [...header.roundOptions, header.round] : header.roundOptions;
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
          <Button id="round-clear" variant="ghost" size="compact" disabled={setRound.busy} onClick={() => void setRound.run(null)}>
            Clear
          </Button>
        )}
      </div>
      <div role="group" aria-label="Rounds" className="flex flex-wrap gap-1.5">
        {options.map((option) => (
          <FilterChip key={option} selected={option === header.round} disabled={setRound.busy} onClick={() => void setRound.run(option)}>
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
        <TextInput id="round-other" aria-label="Other round" placeholder="Other…" value={other} onChange={(e) => setOther(e.target.value)} className="h-8 flex-1" />
        <Button id="round-save" type="submit" size="compact" disabled={setRound.busy || other.trim() === ""}>
          Save
        </Button>
      </form>
      {setRound.error && <ErrorBanner error={setRound.error} title="The round could not be saved" onDismiss={setRound.clearError} />}
    </section>
  );
}

/** What the company answered, marked by hand. */
function Marks({ header, mark }: { header: ApplicationHeader; mark: ApplicationDrawerModel["mark"] }) {
  return (
    <section aria-label="The company answered" className="flex flex-col gap-2">
      <SectionLabel>The company answered</SectionLabel>
      <div className="flex flex-wrap gap-2">
        {header.marks.map((m) => (
          <Button
            key={m.key}
            id={`mark-${m.key}`}
            size="compact"
            variant={m.key === "rejected" ? "danger" : "secondary"}
            disabled={mark.busy}
            onClick={() => void mark.run(m.key)}
          >
            {m.label}
          </Button>
        ))}
      </div>
      {mark.error && <ErrorBanner error={mark.error} title="The answer could not be saved" onDismiss={mark.clearError} />}
    </section>
  );
}

/** The job description, folded away; read only once unfolded. */
function JobDescription({ id }: { id: number }) {
  const [open, setOpen] = useState(false);
  return (
    <details id="drawer-jd" onToggle={(e) => setOpen(e.currentTarget.open)} className="border border-border">
      <summary className="cursor-pointer px-3 py-2 text-body font-semibold text-ink hover:bg-track">Job description</summary>
      {open && (
        <div className="border-t border-border p-3">
          <JdTab id={id} />
        </div>
      )}
    </details>
  );
}

function Body({ id, model, header }: { id: number; model: ApplicationDrawerModel; header: ApplicationHeader }) {
  const openSettings = useNav((s) => s.openSettings);
  const { head } = model;
  const primary = head.primary;
  const showNextJob = header.showNextJob && primary === null;
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
              <Button id="primary-action" variant="primary" busy={primary.loading} disabled={!primary.enabled} onClick={model.onPrimary}>
                {primary.label}
              </Button>
            )}
            {primary?.settings != null && (
              <Button id="primary-settings" icon={Settings} onClick={() => openSettings(primary.settings as SettingsSection)}>
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
      {head.error !== null && <ErrorBanner id="job-error" error={head.error} title="The last job failed" onRetry={head.retry} onDismiss={head.dismiss} />}
      <Stepper steps={header.steps} current={header.stepIndex} />
      {header.status.key === "interview" && <RoundPicker key={`round-${id}`} header={header} setRound={model.setRound} />}
      {header.marks.length > 0 && <Marks header={header} mark={model.mark} />}
      <section aria-label="Timeline" className="flex flex-col gap-2">
        <SectionLabel>Timeline</SectionLabel>
        <TimelineTab id={id} />
      </section>
      <JobDescription key={`jd-${id}`} id={id} />
    </>
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
      {header === undefined &&
        (model.head.loadError ? <ErrorBanner error={model.head.loadError} title="The application could not be read" /> : <Caption>Loading...</Caption>)}
      {header === null && <Caption>This application is gone.</Caption>}
      {header && <Body id={id} model={model} header={header} />}
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
