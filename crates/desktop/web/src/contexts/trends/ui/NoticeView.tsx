import { Card, ErrorBanner } from "@/shared";

import type { Job } from "../domain/jobs";
import type { Notice } from "../hooks";

/** The line under the controls: how the last run ended, or why it did not finish (with the
 *  advice, the Settings shortcut and Retry that carries on where it stopped). */
export function NoticeView({ notice, onRetry }: { notice: Notice; onRetry: (job: Job) => void }) {
  return (
    <div id="trends-notice" role="group" aria-label="Result">
      {notice.kind === "info" ? (
        <Card className="gap-1">
          <p className="selectable text-body">{notice.text}</p>
          {notice.details.map((line, i) => (
            <p key={i} className="selectable text-small text-muted">
              {line}
            </p>
          ))}
        </Card>
      ) : (
        <ErrorBanner error={notice.error} title="The run did not finish" onRetry={() => onRetry(notice.retry)} />
      )}
    </div>
  );
}
