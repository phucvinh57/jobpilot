import { ChartPie, Check } from "lucide-react";

import { Button, Caption, CiteChip, EmptyState, LinkButton, SectionLabel, Tag } from "@/components/ui/kit";

import { gapSkillName, levelLine, termsLine } from "@/lib/trends/format";
import { addAccessibleName, addLabel, addTooltip, canAdd } from "@/lib/trends/preferences";
import type { Citation, Digest, Gap, Point, Wishes } from "@/types/trends";
import type { PreferenceAdds } from "@/hooks/trends/usePreferenceAdds";
import { openLink } from "@/api/shell";

/** The digest of a report: numbered points, each with the stored items it rests on. (Its title,
 *  when it was made and its actions are in the header of the report view.) */
export function DigestCard({ digest, adds }: { digest: Digest | null; adds: PreferenceAdds }) {
  if (digest === null) {
    return (
      <div id="trends-digest" role="group" aria-label="Digest">
        <EmptyState
          icon={ChartPie}
          title="No digest yet"
          body={'Add sources, then press "New research". You need an AI provider (Settings > AI providers).'}
        />
      </div>
    );
  }
  return (
    <section
      id="trends-digest"
      aria-label={digest.title}
      className="flex flex-col gap-4 border border-border bg-surface p-5"
    >
      <DigestBody digest={digest} adds={adds} />
    </section>
  );
}

function DigestBody({ digest, adds }: { digest: Digest; adds: PreferenceAdds }) {
  const { body } = digest;
  if (body.kind === "plain") {
    // A report this version cannot read back: its text as saved.
    return (
      <div className="selectable flex flex-col gap-1 text-body text-muted">
        {body.lines.map((line, i) => (
          <p key={i}>{line}</p>
        ))}
      </div>
    );
  }
  return (
    <div className="flex flex-col gap-5">
      {body.sections.map((section) => (
        <section
          key={section.key}
          id={`trends-section-${section.key}`}
          aria-label={section.title}
          className="flex flex-col gap-4"
        >
          <SectionLabel>{section.title}</SectionLabel>
          {section.gap && <GapView gap={section.gap} />}
          {section.wishes && <WishesView wishes={section.wishes} />}
          {section.points.length > 0 && (
            <ul aria-label={`${section.title}: points`} className="flex flex-col gap-4">
              {section.points.map((point) => (
                <PointRow key={point.index} point={point} adds={adds} />
              ))}
            </ul>
          )}
        </section>
      ))}
      {body.sections.length === 0 && (
        <p className="text-body text-muted">No point could be tied to a source this time.</p>
      )}
      {body.uncatalogued.length > 0 && (
        <p id="trends-uncatalogued" className="selectable text-small text-muted">
          <span className="font-semibold text-ink">Terms not in the list: </span>
          {termsLine(body.uncatalogued)}
        </p>
      )}
      <Caption>Every point links to the stored items it came from.</Caption>
      {body.dropped > 0 && (
        <Caption>
          {body.dropped} point(s) were left out: no valid source, a quote that is not in the source, or a claim against
          the app's numbers.
        </Caption>
      )}
    </div>
  );
}

/** The gap as the app sorted it: build on, learn next, at risk. */
function GapView({ gap }: { gap: Gap }) {
  return (
    <div id="trends-gap" className="flex flex-col gap-3">
      <Caption>{gap.hint}</Caption>
      {gap.groups.map((group) => (
        <div
          key={group.bucket}
          id={`trends-gap-${group.bucket}`}
          role="group"
          aria-label={group.title}
          className="flex flex-col gap-1.5"
        >
          <p className="text-body">
            <span className="font-semibold text-ink">{group.title}</span>
            <span className="text-small text-muted"> · {group.hint}</span>
          </p>
          {group.skills.length === 0 ? (
            <p className="text-small text-muted">{group.empty}</p>
          ) : (
            <ul className="flex flex-wrap gap-2">
              {group.skills.map((skill) => {
                const level = levelLine(skill);
                return (
                  <li
                    key={skill.skill}
                    aria-label={gapSkillName(skill)}
                    className="flex flex-col gap-0.5 border border-border bg-surface-alt py-0.5 pr-0.5 pl-2"
                  >
                    <div className="flex items-center gap-1.5">
                      <span className="selectable text-small font-medium text-ink">{skill.skill}</span>
                      <span className="text-small text-muted tabular-nums">{skill.share}</span>
                      <Tag tone={skill.direction.tone}>{skill.direction.label}</Tag>
                    </div>
                    {level && <span className="pr-1.5 pb-0.5 text-small text-muted tabular-nums">{level}</span>}
                  </li>
                );
              })}
            </ul>
          )}
        </div>
      ))}
    </div>
  );
}

/** Each wish with what the app measured for it; disagreement is shown, not softened. */
function WishesView({ wishes }: { wishes: Wishes }) {
  return (
    <div className="flex flex-col gap-2">
      <Caption>{wishes.hint}</Caption>
      <ul
        id="trends-wishes"
        aria-label="Your wishes, measured"
        className="flex flex-col divide-y divide-border border border-border"
      >
        {wishes.wishes.map((wish, i) => (
          <li
            key={`${wish.kind}-${wish.wish}`}
            id={`trends-wish-${i}`}
            aria-label={`${wish.wish} (${wish.kind}${wish.avoided ? ", avoided" : ""}): ${wish.signal.label}, ${wish.detail}`}
            className="flex flex-wrap items-center gap-x-2 gap-y-1 px-3 py-2"
          >
            <span className="selectable min-w-0 text-body font-semibold text-ink">{wish.wish}</span>
            <span className="text-small text-muted">
              {wish.avoided ? `${wish.kind} you avoid` : `${wish.kind} you want`}
            </span>
            <span className="ml-auto flex items-center gap-2">
              <span className="text-small text-muted tabular-nums">{wish.detail}</span>
              <Tag tone={wish.signal.tone}>{wish.signal.label}</Tag>
            </span>
          </li>
        ))}
      </ul>
    </div>
  );
}

/** One point: a numbered badge, the topic and text, the quote, the sources it cites and the
 *  "Add to preferences" button. */
function PointRow({ point, adds }: { point: Point; adds: PreferenceAdds }) {
  const { suggest } = point;
  const state = suggest ? adds.stateOf(suggest.field, point.topic, suggest.presence) : undefined;
  return (
    <li className="flex items-start gap-3">
      <span className="flex size-7 shrink-0 items-center justify-center bg-accent-soft text-small font-semibold text-accent-dark">
        {point.number}
      </span>
      <div className="flex min-w-0 flex-1 flex-col gap-1.5">
        {point.topic && <p className="selectable text-body font-semibold text-ink">{point.topic}</p>}
        <p className="selectable text-body text-ink">{point.text}</p>
        {point.quote && <p className="selectable text-small italic text-muted">{`“${point.quote}”`}</p>}
        <div className="flex flex-wrap items-center gap-2">
          {point.citations.map((citation, j) => (
            <Cite key={`${citation.number}-${j}`} id={`trends-cite-${point.index * 16 + j}`} citation={citation} />
          ))}
          {suggest && state && (
            <Button
              id={`trends-suggest-point-${point.index}`}
              size="compact"
              icon={state === "added" ? Check : undefined}
              busy={state === "adding"}
              disabled={!canAdd(state)}
              aria-label={addAccessibleName(state, point.topic)}
              title={addTooltip(state)}
              onClick={() => void adds.add(suggest.field, point.topic, suggest.presence)}
            >
              {addLabel(state)}
            </Button>
          )}
        </div>
      </div>
    </li>
  );
}

/** The numbered chip and the title of a cited item: both open its stored address (only http(s)). */
function Cite({ id, citation }: { id: string; citation: Citation }) {
  const { url } = citation;
  return (
    <div id={id} className="flex max-w-[320px] items-center gap-1.5">
      <CiteChip number={citation.number} onClick={url === null ? undefined : () => openLink(url)} />
      {url === null ? (
        <span className="selectable min-w-0 truncate text-small text-muted">{citation.title}</span>
      ) : (
        <LinkButton aria-label={`Open source: ${citation.title}`} onClick={() => openLink(url)}>
          {citation.title}
        </LinkButton>
      )}
    </div>
  );
}
