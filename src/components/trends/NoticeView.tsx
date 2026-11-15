import { Card } from "@/components/ui/kit";

import type { InfoNotice } from "@/hooks/trends/runs";

/** The line under the controls: how the last run ended. (A run that did not finish is a toast
 *  with the advice, the Settings shortcut and Retry: see `useTrends`.) */
export function NoticeView({ notice }: { notice: InfoNotice }) {
  return (
    <div id="trends-notice" role="group" aria-label="Result">
      <Card className="gap-1">
        <p className="selectable text-body">{notice.text}</p>
        {notice.details.map((line, i) => (
          <p key={i} className="selectable text-small text-muted">
            {line}
          </p>
        ))}
      </Card>
    </div>
  );
}
