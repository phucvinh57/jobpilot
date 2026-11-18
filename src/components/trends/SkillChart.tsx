import { Check, Flag, Plus } from "lucide-react";
import { Fragment, useState } from "react";

import { Banner, Button, Caption, Section, Segmented, SkillBar, Tag } from "@/components/ui/kit";

import { chartHint, levelLine, pickScope, skillRowName } from "@/lib/trends/format";
import { addAccessibleName, addTooltip, canAdd } from "@/lib/trends/preferences";
import type { Chart, ChartScope, ChartSkill, ScopeKey } from "@/types/trends";
import type { PreferenceAdds } from "@/hooks/trends/usePreferenceAdds";

/** The share of postings naming each skill, per scope (Local / Global, switched with
 *  `#trends-scope` when the report has a place). Bars are scaled to the most named skill: accent
 *  for skills in the profile (in older reports: the preferences), grey otherwise. Each row carries
 *  its direction and a flag for the user's wishes, and, when the report has them, its level in the
 *  period's own months with the range that level has at this many postings (the bar is the share
 *  over every month). Under the chart: what the counts saw and did not see. */
export function SkillChart({ chart, adds }: { chart: Chart; adds: PreferenceAdds }) {
  const [picked, setPicked] = useState<ScopeKey | undefined>(undefined);
  const scope = pickScope(chart, picked);
  if (scope === undefined) {
    return null;
  }
  const switcher =
    chart.scopes.length > 1 ? (
      <div id="trends-scope">
        <Segmented
          label="Market scope"
          choices={chart.scopes.map((s) => ({ value: s.key, label: s.label }))}
          value={scope.key}
          onChange={setPicked}
        />
      </div>
    ) : undefined;
  return (
    <Section id="trends-chart" title="Skills in demand" hint={chartHint(chart, scope)} actions={switcher}>
      {scope.note && (
        <p id="trends-scope-note" className="text-small font-medium text-warn-fg">
          {scope.note}
        </p>
      )}
      {scope.skills.length === 0 ? (
        <Caption>No posting of this scope names a listed skill yet.</Caption>
      ) : (
        <ul aria-label={`Skill share, ${scope.label}`} className="flex flex-col gap-1">
          {scope.skills.map((skill, i) => (
            <SkillRow key={skill.name} index={i} skill={skill} scope={scope} marks={chart.marks} adds={adds} />
          ))}
        </ul>
      )}
      {chart.marks && <Months scope={scope} />}
      <div className="flex flex-wrap gap-4">
        <Swatch colorClass="bg-accent" label={chart.marks ? "In your profile" : "In your preferences"} />
        <Swatch colorClass="bg-bar-grey" label="Not yet" />
        {chart.marks && (
          <span className="flex items-center gap-1.5">
            <Flag aria-hidden className="size-3 text-accent" />
            <span className="text-small text-muted">In your wishes</span>
          </span>
        )}
      </div>
      <Coverage chart={chart} />
      {chart.pipeline && (
        <p id="trends-pipeline" className="text-small text-muted">
          {chart.pipeline}
        </p>
      )}
    </Section>
  );
}

function SkillRow({
  index,
  skill,
  scope,
  marks,
  adds,
}: {
  index: number;
  skill: ChartSkill;
  scope: ChartScope;
  marks: boolean;
  adds: PreferenceAdds;
}) {
  // Any skill of the chart, rising or not: adding it makes no claim about the trend.
  const state = adds.stateOf("skills", skill.name, skill.presence);
  const tip = skillRowName(skill);
  const level = levelLine(skill);
  return (
    <li id={`trends-skill-${index}`} aria-label={tip} title={tip} className="flex flex-col">
      <div className="flex items-center gap-1">
        <div className="min-w-0 flex-1">
          <SkillBar
            name={skill.name}
            count={skill.percent}
            max={scope.maxPercent}
            inProfile={marks ? skill.inProfile : state === "added"}
          />
        </div>
        {marks && (
          <>
            <span className="flex w-3.5 shrink-0 justify-center">
              {skill.wished && <Flag aria-hidden className="size-3 text-accent" />}
            </span>
            <span className="flex w-14 shrink-0 justify-end">
              {skill.direction && <Tag tone={skill.direction.tone}>{skill.direction.label}</Tag>}
            </span>
          </>
        )}
        <Button
          id={`trends-suggest-skill-${index}`}
          size="compact"
          variant="ghost"
          icon={state === "added" ? Check : Plus}
          busy={state === "adding"}
          disabled={!canAdd(state)}
          aria-label={addAccessibleName(state, skill.name)}
          title={addTooltip(state)}
          onClick={() => void adds.add("skills", skill.name, skill.presence)}
        />
      </div>
      {level && <p className="-mt-1 truncate text-small text-muted tabular-nums">{level}</p>}
    </li>
  );
}

/** What the counts saw and did not see. A complete report says it in a plain line; when something
 *  was not seen (postings that could not be read, items a source left unread, a month without
 *  postings) it is a notice. Nothing for a report saved before the tests. */
function Coverage({ chart }: { chart: Chart }) {
  if (chart.coverage.length === 0) {
    return null;
  }
  if (chart.partial) {
    return (
      <Banner
        id="trends-coverage"
        tone="warn"
        title="The counts did not see everything"
        body={
          <ul className="flex flex-col gap-0.5">
            {chart.coverage.map((line) => (
              <li key={line}>
                <Sentence text={line} />
              </li>
            ))}
          </ul>
        }
      />
    );
  }
  return (
    <p id="trends-coverage" className="selectable text-small text-muted">
      {chart.coverage.map((line, i) => (
        <Fragment key={line}>
          {i > 0 && " "}
          <Sentence text={line} />
        </Fragment>
      ))}
    </p>
  );
}

/** A sentence of the coverage with each month (`2026-06`) kept whole when the line wraps. */
function Sentence({ text }: { text: string }) {
  return (
    <>
      {text.split(/(\d{4}-\d{2})/).map((part, i) =>
        i % 2 === 1 ? (
          <span key={i} className="whitespace-nowrap">
            {part}
          </span>
        ) : (
          part
        ),
      )}
    </>
  );
}

/** Postings read per month: what every share rests on. */
function Months({ scope }: { scope: ChartScope }) {
  if (scope.months.length === 0) {
    return null;
  }
  return (
    <div className="flex flex-wrap items-baseline gap-x-3 gap-y-1">
      <span className="text-small text-muted">Postings per month:</span>
      <ul id="trends-months" aria-label="Postings per month" className="flex flex-wrap gap-x-3 gap-y-1">
        {scope.months.map((m) => (
          <li key={m.month} aria-label={`${m.month}: ${m.documents} postings`} className="text-small tabular-nums">
            <span className="text-muted">{m.label}</span> <span className="font-semibold text-ink">{m.documents}</span>
          </li>
        ))}
      </ul>
    </div>
  );
}

function Swatch({ colorClass, label }: { colorClass: string; label: string }) {
  return (
    <span className="flex items-center gap-1.5">
      <span aria-hidden className={`size-2.5 ${colorClass}`} />
      <span className="text-small text-muted">{label}</span>
    </span>
  );
}
