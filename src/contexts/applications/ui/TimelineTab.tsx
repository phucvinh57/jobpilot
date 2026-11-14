// The Timeline tab: the application's events, newest first, and the emails that wait for a review.
// Email subjects and senders are untrusted text: shown as text only.

import { ActivityRow, Banner, Button, Caption, Card, ErrorBanner, Heading } from "@/shared";

import { useTimeline } from "../hooks/useTabs";

export function TimelineTab({ id }: { id: number }) {
  const { query, resolve } = useTimeline(id);
  const data = query.data;

  if (data === undefined) {
    return query.error ? <ErrorBanner error={query.error} title="The timeline could not be read" /> : null;
  }
  return (
    <div id="timeline" className="flex flex-col gap-3">
      {resolve.error && <ErrorBanner error={resolve.error} title="That email could not be handled" onDismiss={resolve.clearError} />}
      {data.reviews.map((review) => (
        <div key={review.id} id={`review-${review.id}`}>
          <Banner
            tone="warn"
            title={review.title}
            body={review.body}
            actions={
              <>
                <Button id={`accept-${review.id}`} size="compact" disabled={resolve.busy} onClick={() => void resolve.run(review.id, true)}>
                  Accept
                </Button>
                <Button id={`dismiss-${review.id}`} size="compact" variant="ghost" disabled={resolve.busy} onClick={() => void resolve.run(review.id, false)}>
                  Dismiss
                </Button>
              </>
            }
          />
        </div>
      ))}
      <Card>
        <Heading>History</Heading>
        {data.events.length === 0 ? (
          <Caption>Nothing has happened yet.</Caption>
        ) : (
          <div className="flex flex-col">
            {data.events.map((event) => (
              // One list per row, so a row that cut its text can carry the whole text as its tooltip.
              <ul key={event.id} id={`event-${event.id}`} title={event.full ?? undefined}>
                <ActivityRow tone={event.tone} text={event.text} when={event.when} />
              </ul>
            ))}
          </div>
        )}
      </Card>
    </div>
  );
}
