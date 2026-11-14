// The JD tab: where the posting came from, the company brief (or the key requirements when there is
// none), and the posting text. The text is untrusted: it is shown as plain text, never as markup.

import { Caption, Card, ErrorBanner, Heading, LinkButton, NOT_FOUND, openUrl, SectionLabel, Tag, CompanyBlock, InterviewBlock, type Insight } from "@/shared";

import { useJd } from "../hooks/useTabs";

/** A row of tags with a label in front: skills. `must` = required (accent), else neutral. */
function TagRow({ label, items, must }: { label: string; items: readonly string[]; must: boolean }) {
  return (
    <div className="flex flex-wrap items-center gap-1.5">
      <span className="w-[84px] shrink-0 text-small text-muted">{label}</span>
      {items.map((item) => (
        <Tag key={item} tone={must ? "accent" : "neutral"}>
          {item}
        </Tag>
      ))}
    </div>
  );
}

/** The company brief: the company and the interview process (the blocks Today shows too), and the
 *  skills the posting asks for, with "not found" for what the texts did not say. */
function Brief({ insight }: { insight: Insight }) {
  const none = insight.requiredSkills.length === 0 && insight.preferredSkills.length === 0;
  return (
    <Card id="jd-brief">
      <Heading>Company brief</Heading>
      <CompanyBlock insight={insight} />
      <div className="flex flex-col gap-1.5">
        <SectionLabel>Skills</SectionLabel>
        {none && <Caption>{NOT_FOUND}</Caption>}
        {insight.requiredSkills.length > 0 && <TagRow label="Required" items={insight.requiredSkills} must />}
        {insight.preferredSkills.length > 0 && <TagRow label="Preferred" items={insight.preferredSkills} must={false} />}
      </div>
      <InterviewBlock insight={insight} />
    </Card>
  );
}

export function JdTab({ id }: { id: number }) {
  const { data, error } = useJd(id);

  if (data === undefined) {
    return error ? <ErrorBanner error={error} title="The job description could not be read" /> : null;
  }
  const hasRequirements = data.mustHave.length > 0 || data.niceToHave.length > 0;
  return (
    <div id="jd-tab" className="flex flex-col gap-3">
      {data.source !== null &&
        (data.sourceIsLink ? (
          <LinkButton id="jd-source" onClick={() => void openUrl(data.source ?? "").catch(() => undefined)}>
            {data.source}
          </LinkButton>
        ) : (
          <p className="truncate text-body text-muted">{data.source}</p>
        ))}
      {/* The brief lists the skills too, so the plain requirements are for jobs without one. */}
      {data.insight !== null && <Brief insight={data.insight} />}
      {data.insight === null && hasRequirements && (
        <Card id="jd-requirements">
          <Heading>Key requirements</Heading>
          {data.mustHave.length > 0 && <TagRow label="Must have" items={data.mustHave} must />}
          {data.niceToHave.length > 0 && <TagRow label="Nice to have" items={data.niceToHave} must={false} />}
        </Card>
      )}
      <Card id="jd-card">
        <Heading>Job description</Heading>
        {data.text === null ? (
          <p className="text-body text-muted">No JD text yet.</p>
        ) : (
          <p id="jd-text" className="selectable max-h-[480px] overflow-y-auto whitespace-pre-wrap break-words text-body text-ink">
            {data.text}
          </p>
        )}
      </Card>
    </div>
  );
}
