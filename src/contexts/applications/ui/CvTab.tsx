// The CV tab: what the posting asks for (checked against the profile), the fact check, `cv.yaml` in
// an editor, the CV on paper, and "Ask AI". The CV and the posting are untrusted text: shown as text.

import { Check, ExternalLink, Minus } from "lucide-react";
import { useState } from "react";

import { Banner, Button, Caption, Card, ErrorBanner, Heading, Section, Tag, TextInput, TONE_CLASS } from "@/shared";

import { ASK_PLACEHOLDER, emptyEditorText, previewPlaceholder, requirementLabel, showTraced } from "../domain/cv";
import type { Requirement } from "../domain/types";
import { useCvTab, type CvTabInput } from "../hooks/useCvTab";
import { CvEditor } from "./CvEditor";

/** One line of the "What they ask for" checklist. */
function RequirementRow({ requirement }: { requirement: Requirement }) {
  const tone = requirement.covered ? TONE_CLASS.ok : TONE_CLASS.warn;
  return (
    <li className="flex items-start gap-2.5">
      <span role="img" aria-label={requirementLabel(requirement.covered)} className={`shrink-0 pt-0.5 ${tone.fg}`}>
        {requirement.covered ? <Check aria-hidden className="size-4" /> : <Minus aria-hidden className="size-4" />}
      </span>
      <span className="flex min-w-0 flex-1 flex-col gap-0.5">
        <span className="flex flex-wrap items-center gap-2 text-body text-ink">
          {requirement.text}
          {requirement.nice && <Tag tone="neutral">Nice to have</Tag>}
        </span>
        {requirement.note !== null && <Caption>{requirement.note}</Caption>}
      </span>
    </li>
  );
}

export function CvTab(input: CvTabInput) {
  const cv = useCvTab(input);
  const [instruction, setInstruction] = useState("");
  const data = cv.cv;

  if (data === undefined) {
    return cv.loadError ? <ErrorBanner error={cv.loadError} title="The CV could not be read" /> : null;
  }
  const traced = showTraced({
    guardClean: cv.guardClean,
    hasCv: cv.hasCv,
    hasProblem: cv.problem !== null,
    warnings: cv.warnings.length,
    busy: input.busy,
  });

  return (
    <div id="cv-tab" className="flex min-h-[520px] flex-col gap-3">
      <div className="grid min-h-0 flex-1 grid-cols-1 gap-4 lg:grid-cols-[minmax(0,1fr)_minmax(0,1fr)]">
        {/* Left: what the posting asks for, the fact check, then the YAML. */}
        <div id="cv-left" className="flex min-w-0 flex-col gap-3">
          {data.requirements.length > 0 && (
            <Section title="What they ask for" hint="From the job description, checked against your profile." id="cv-gaps">
              <ul className="flex flex-col gap-2.5">
                {data.requirements.map((requirement) => (
                  <RequirementRow key={requirement.text} requirement={requirement} />
                ))}
              </ul>
              {data.hasGaps && (
                <div className="flex">
                  <Button id="open-profile" variant="ghost" size="compact" icon={ExternalLink} onClick={cv.openProfile}>
                    Open profile.yaml
                  </Button>
                </div>
              )}
              {cv.openError && <p className="text-small text-danger-fg">{cv.openError.message}</p>}
            </Section>
          )}
          {traced && (
            <Banner
              id="cv-traced"
              tone="ok"
              title="Every fact traced to your profile"
              body="Employers, titles, dates and technologies all match profile.yaml."
            />
          )}
          {cv.problem && <ErrorBanner id="cv-problem" error={cv.problem} title="The CV does not compile" />}
          {cv.warnings.length > 0 && (
            <Banner
              id="cv-warnings"
              tone="warn"
              title="Not backed by profile.yaml (kept, check before approving):"
              body={
                <ul className="flex max-h-[120px] flex-col gap-1 overflow-y-auto font-mono">
                  {cv.warnings.map((warning) => (
                    <li key={warning}>{warning}</li>
                  ))}
                </ul>
              }
            />
          )}
          <Card className="min-h-[340px] flex-1">
            <div className="flex items-center justify-between gap-2">
              <p className="text-body font-semibold text-ink">cv.yaml</p>
              {cv.note !== null && <Caption>{cv.note}</Caption>}
            </div>
            <div className="min-h-0 flex-1">
              {cv.hasCv ? (
                <CvEditor doc={cv.doc} readOnly={cv.locked} onChange={cv.edit} />
              ) : (
                <p className="p-2 text-body text-muted">{emptyEditorText(input.busy)}</p>
              )}
            </div>
          </Card>
        </div>

        {/* Right: the CV on paper. The pages come from Typst as opaque white sheets in both appearances. */}
        <div className="flex min-w-0 flex-col gap-2 rounded-card bg-track p-4">
          <Heading>Preview</Heading>
          <div id="cv-preview" className="flex max-h-[900px] flex-1 flex-col items-center gap-3 overflow-y-auto">
            {cv.pages.map((url, i) => (
              <img key={url} src={url} alt={`CV page ${i + 1} of ${cv.pages.length}`} className="w-full max-w-[720px] border border-border shadow-sm" />
            ))}
            {cv.pages.length === 0 && (
              <div className="flex min-h-[240px] w-full max-w-[720px] items-start rounded-banner border border-dashed border-border p-6 font-serif text-body text-muted">
                {previewPlaceholder(cv.rendering)}
              </div>
            )}
          </div>
        </div>
      </div>

      <TextInput
        id="ask-ai"
        aria-label="Ask AI"
        placeholder={ASK_PLACEHOLDER}
        value={instruction}
        disabled={input.busy || !cv.hasCv}
        onChange={(e) => setInstruction(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === "Enter" && cv.ask(instruction)) {
            setInstruction("");
          }
        }}
      />
    </div>
  );
}
