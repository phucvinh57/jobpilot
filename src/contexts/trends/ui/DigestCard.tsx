import { ChartPie, Check } from "lucide-react";

import { Button, Caption, CiteChip, EmptyState, Heading, LinkButton, SectionLabel } from "@/shared";

import { digestHint } from "../domain/format";
import { addAccessibleName, addLabel, addTooltip, canAdd } from "../domain/preferences";
import type { Citation, Digest, Point } from "../domain/types";
import type { PreferenceAdds } from "../hooks";
import { open } from "./links";

/** The latest digest: numbered points, each with the stored items it rests on. */
export function DigestCard({ digest, adds }: { digest: Digest | null; adds: PreferenceAdds }) {
  if (digest === null) {
    return (
      <div id="trends-digest" role="group" aria-label="Digest">
        <EmptyState
          icon={ChartPie}
          title="No digest yet"
          body={'Add sources, then press "Collect & build digest". You need an AI provider (Settings > AI providers).'}
        />
      </div>
    );
  }
  return (
    <section id="trends-digest" aria-label={digest.title} className="flex flex-col gap-4 border border-border bg-surface p-5">
      <div className="flex flex-col gap-0.5">
        <Heading>{digest.title}</Heading>
        <Caption>{digestHint(digest.when, digest.itemsRead)}</Caption>
      </div>
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
        <section key={section.title} aria-label={section.title} className="flex flex-col gap-4">
          <SectionLabel>{section.title}</SectionLabel>
          <ul className="flex flex-col gap-4">
            {section.points.map((point) => (
              <PointRow key={point.index} point={point} adds={adds} />
            ))}
          </ul>
        </section>
      ))}
      {body.sections.length === 0 && <p className="text-body text-muted">No point could be tied to a source this time.</p>}
      <Caption>Every point links to the stored items it came from.</Caption>
      {body.dropped > 0 && (
        <Caption>{body.dropped} point(s) were left out: no valid source, or a quote that is not in the source.</Caption>
      )}
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
      <CiteChip number={citation.number} onClick={url === null ? undefined : () => open(url)} />
      {url === null ? (
        <span className="selectable min-w-0 truncate text-small text-muted">{citation.title}</span>
      ) : (
        <LinkButton aria-label={`Open source: ${citation.title}`} onClick={() => open(url)}>
          {citation.title}
        </LinkButton>
      )}
    </div>
  );
}
