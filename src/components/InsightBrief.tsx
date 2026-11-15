// The company brief, drawn the same way wherever it is shown: "The company" (domain, what it
// does, the facts, the pages it comes from) and "Interview process" (the stages in order, then
// notes). The lead drawer shows it under a Lead, the Application screen's JD tab under the posting.

import { openUrl } from "@/api/shell";
import { Caption, LinkButton, SectionLabel, Tag } from "@/components/ui/kit";

/** The stored brief of a job (`InsightDto` in `shared/insight.rs`). */
export interface Insight {
  domain: string;
  businessSummary: string;
  /** The company facts on one line; `null` when none was found. */
  facts: string | null;
  requiredSkills: string[];
  preferredSkills: string[];
  /** The interview stages in order; empty when the process was not found. */
  interviewSteps: string[];
  interviewNotes: string | null;
  /** http(s) pages the brief comes from. */
  sources: string[];
}

/** What a brief section says where the texts did not. */
export const NOT_FOUND = "Not found in the posting or the company pages.";

/** "The company": domain, what it does, the facts, and the pages the brief comes from. */
export function CompanyBlock({ insight }: { insight: Insight }) {
  const known = insight.domain !== "" || insight.businessSummary !== "" || insight.facts !== null;
  return (
    <div className="flex flex-col gap-2">
      <SectionLabel>The company</SectionLabel>
      {insight.domain && (
        <div className="flex">
          <Tag tone="accent">{insight.domain}</Tag>
        </div>
      )}
      {insight.businessSummary && <p className="selectable text-body text-ink">{insight.businessSummary}</p>}
      {insight.facts && <Caption className="selectable">{insight.facts}</Caption>}
      {!known && <Caption>{NOT_FOUND}</Caption>}
      {insight.sources.length > 0 && (
        <div className="flex flex-col gap-0.5">
          {insight.sources.map((url) => (
            <LinkButton key={url} aria-label={`Source: ${url}`} onClick={() => void openUrl(url).catch(() => {})}>
              {url}
            </LinkButton>
          ))}
        </div>
      )}
    </div>
  );
}

/** "Interview process": the stages in order, then any notes. */
export function InterviewBlock({ insight }: { insight: Insight }) {
  const steps = insight.interviewSteps;
  return (
    <div className="flex flex-col gap-2">
      <SectionLabel>Interview process</SectionLabel>
      {steps.length > 0 && (
        <ol aria-label="Interview stages" className="flex flex-col gap-1">
          {steps.map((step, i) => (
            <li key={`${i}-${step}`} aria-label={`Step ${i + 1}: ${step}`} className="flex items-start gap-2 text-body">
              <span className="w-[18px] shrink-0 text-subtle">{i + 1}.</span>
              <span className="selectable min-w-0 flex-1 text-ink">{step}</span>
            </li>
          ))}
        </ol>
      )}
      {insight.interviewNotes && <Caption className="selectable">{insight.interviewNotes}</Caption>}
      {steps.length === 0 && !insight.interviewNotes && <Caption>{NOT_FOUND}</Caption>}
    </div>
  );
}
