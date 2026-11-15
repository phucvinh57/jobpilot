import { ExternalLink } from "lucide-react";

import { Banner, Button } from "@/components/ui/kit";

import type { Challenge } from "@/types/trends";
import { openLink } from "@/api/shell";

/** One banner per collection that waits at a login, captcha or block page. The app never solves
 *  it: the user opens the page, deals with it, then continues (or builds the digest without it). */
export function Challenges({
  challenges,
  busy,
  onContinue,
  onWithout,
}: {
  challenges: readonly Challenge[];
  busy: boolean;
  /** Carry on the paused collection of this period. */
  onContinue: (period: string) => void;
  /** Build the digest of this period from what is stored. */
  onWithout: (period: string) => void;
}) {
  return (
    <>
      {challenges.map((wait, i) => (
        <Banner
          key={`${wait.period}-${wait.sourceName}-${i}`}
          id={`trends-waiting-${i}`}
          tone="warn"
          title={`${wait.sourceName} ${wait.what} (${wait.period})`}
          body="Open the page, deal with it yourself, then continue. The other sources are already stored: you can build the digest without this one."
          actions={
            <>
              <Button
                id={`trends-open-page-${i}`}
                size="compact"
                icon={ExternalLink}
                disabled={wait.url === null}
                onClick={() => wait.url && openLink(wait.url)}
              >
                Open page
              </Button>
              <Button
                id={`trends-continue-${i}`}
                size="compact"
                disabled={busy}
                onClick={() => onContinue(wait.period)}
              >
                Continue
              </Button>
              <Button id={`trends-without-${i}`} size="compact" disabled={busy} onClick={() => onWithout(wait.period)}>
                Build digest without it
              </Button>
            </>
          }
        />
      ))}
    </>
  );
}
