// The open lead's brief: why it fits (one bar per criterion with the reason under it) and the skills
// the posting asks for, matched against the profile. The company and the interview process are
// drawn by the shared `CompanyBlock` / `InterviewBlock`. Nothing here reads the shell: the data is
// the lead's stored score and brief, and the profile's skill names Today read once.

import { Caption, Chip, FitBar, SectionLabel, Tag } from "@/shared";

import { listOrNone, splitSkills } from "../domain/skills";
import { capitalize } from "../domain/text";
import type { Criterion } from "../domain/types";

/** "Why it fits": one bar per criterion with the reason under it. Nothing when the lead was not scored. */
export function Breakdown({ criteria }: { criteria: readonly Criterion[] | null }) {
  if (criteria === null) {
    return null;
  }
  return (
    <div id="queue-breakdown" role="group" aria-label="Score breakdown" className="flex min-w-[220px] flex-1 flex-col gap-2.5">
      <SectionLabel>Why it fits</SectionLabel>
      {criteria.map((c) => {
        const label = `${capitalize(c.criterion)} ${c.weight}%`;
        const reason = c.known ? c.reason : "The posting or your preferences say nothing about it.";
        return (
          <div key={c.criterion} className="flex flex-col gap-0.5">
            {c.known ? (
              <FitBar label={label} value={c.score} />
            ) : (
              <div className="flex items-center gap-3">
                <span className="w-[120px] shrink-0 truncate text-small text-muted">{label}</span>
                <span className="text-small text-subtle">Not judged</span>
              </div>
            )}
            {reason.trim() !== "" && <p className="selectable text-small text-muted">{reason}</p>}
          </div>
        );
      })}
      <Caption>Weighted by your preferences.</Caption>
    </div>
  );
}

/** The skills the posting asks for: chips for those the profile has, dashed chips for the others.
 *  Without a readable profile they are plain tags. Nothing when the brief lists no skills. */
export function SkillsBlock({
  skills,
  profileSkills,
}: {
  skills: { required: readonly string[]; preferred: readonly string[] } | undefined;
  profileSkills: readonly string[];
}) {
  if (!skills || (skills.required.length === 0 && skills.preferred.length === 0)) {
    return null;
  }
  const judged = profileSkills.length > 0;
  const split = splitSkills(skills, profileSkills);
  const summary = judged
    ? `Skills the posting asks for. In your profile: ${listOrNone(split.matched)}. Not in your profile: ${listOrNone(split.gaps)}.`
    : `Skills the posting asks for: ${listOrNone(split.gaps)}.`;
  return (
    <div id="queue-skills" role="group" aria-label={summary} className="flex min-w-[220px] flex-1 flex-col gap-2.5">
      <SectionLabel>Skills</SectionLabel>
      <div className="flex flex-wrap gap-1.5">
        {judged ? (
          <>
            {split.matched.map((s) => (
              <Chip key={`match-${s}`} kind="match">
                {s}
              </Chip>
            ))}
            {split.gaps.map((s) => (
              <Chip key={`gap-${s}`} kind="gap">
                {s}
              </Chip>
            ))}
          </>
        ) : (
          split.gaps.map((s) => (
            <Tag key={s} tone="neutral">
              {s}
            </Tag>
          ))
        )}
      </div>
      <Caption>{judged ? "Blue: in your profile. Dashed: asked for, not in your profile." : "What the posting asks for."}</Caption>
    </div>
  );
}
