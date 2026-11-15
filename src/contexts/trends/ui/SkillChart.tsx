import { Check, Plus } from "lucide-react";

import { Button, Section, SkillBar } from "@/shared";

import { chartHint } from "../domain/format";
import { addAccessibleName, addTooltip, canAdd } from "../domain/preferences";
import type { Chart } from "../domain/types";
import type { PreferenceAdds } from "../hooks";

/** Simple bars scaled to the most frequent skill: accent for what the preferences name already,
 *  grey for the rest. The count, the share and its change are in the tooltip of the row. */
export function SkillChart({ chart, adds }: { chart: Chart; adds: PreferenceAdds }) {
  return (
    <Section id="trends-chart" title="Skills in demand" hint={chartHint(chart)}>
      <ul aria-label="Skill frequency" className="flex flex-col gap-1">
        {chart.skills.map((skill, i) => {
          // Any skill of the chart, rising or not: adding it makes no claim about the trend.
          const state = adds.stateOf("skills", skill.name, skill.presence);
          const tip = `${skill.name}: ${skill.description}`;
          return (
            <li
              key={skill.name}
              id={`trends-skill-${i}`}
              aria-label={tip}
              title={tip}
              className="flex items-center gap-1"
            >
              <div className="min-w-0 flex-1">
                <SkillBar name={skill.name} count={skill.total} max={chart.max} inProfile={state === "added"} />
              </div>
              <Button
                id={`trends-suggest-skill-${i}`}
                size="compact"
                variant="ghost"
                icon={state === "added" ? Check : Plus}
                busy={state === "adding"}
                disabled={!canAdd(state)}
                aria-label={addAccessibleName(state, skill.name)}
                title={addTooltip(state)}
                onClick={() => void adds.add("skills", skill.name, skill.presence)}
              />
            </li>
          );
        })}
      </ul>
      <div className="flex flex-wrap gap-4">
        <Swatch colorClass="bg-accent" label="In your preferences" />
        <Swatch colorClass="bg-bar-grey" label="Not yet" />
      </div>
    </Section>
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
