// The Pipeline: every application as a card on a board (CV review, Ready to apply, Submitted,
// Interview, Closed), and the latest events across all of them. No primary button here: the board
// is for looking around, the action is on the application.

import clsx from "clsx";
import { LayoutDashboard } from "lucide-react";

import { Button, Caption, Card, EmptyState, ErrorBanner, Heading, Page, Tag, TONE_CLASS, useNav } from "@/shared";

import { activityLabel, cardLabel, columnLabel, pipelineSummary } from "../domain/pipeline";
import type { ActivityItem, BoardCard, BoardColumn } from "../domain/types";
import { usePipeline } from "../hooks/usePipeline";

/** The narrowest a column gets before the board scrolls sideways: 5 columns of 200 px and 4 gaps of 14 px. */
const BOARD_MIN_WIDTH = "min-w-[1056px]";

/** A card: what the job is, what happens next and how long ago it changed. (The kit's card has no age.) */
function ApplicationCard({ card, onOpen }: { card: BoardCard; onOpen: () => void }) {
  return (
    <button
      type="button"
      id={`card-${card.id}`}
      aria-label={cardLabel(card)}
      onClick={onOpen}
      className="flex cursor-pointer flex-col gap-1.5 border border-border bg-surface p-3 text-left transition-colors hover:border-accent/50"
    >
      <span className="text-body font-semibold text-ink">{card.title}</span>
      {card.subtitle !== "" && <span className="text-small text-muted">{card.subtitle}</span>}
      <span className="flex items-center justify-between gap-2 pt-1">
        <Tag tone={card.next.tone}>{card.next.label}</Tag>
        <span className="shrink-0 text-small text-subtle">{card.age}</span>
      </span>
    </button>
  );
}

function Column({ column, onOpen }: { column: BoardColumn; onOpen: (id: number) => void }) {
  const count = column.cards.length;
  return (
    <div
      id={`column-${column.key}`}
      role="group"
      aria-label={columnLabel(column.label, count)}
      className="flex min-w-[200px] flex-1 flex-col gap-2.5 bg-track p-3"
    >
      <div className="flex justify-between px-1 py-0.5 text-body font-semibold text-ink">
        <span>{column.label}</span>
        <span className="font-medium text-subtle">{count}</span>
      </div>
      {column.cards.map((card) => (
        <ApplicationCard key={card.id} card={card} onOpen={() => onOpen(card.id)} />
      ))}
      {count === 0 && <p className="px-1 text-small text-subtle">Nothing here</p>}
    </div>
  );
}

/** One line of the activity feed that opens its application (the kit's row is not a button). */
function ActivityButton({ item, onOpen }: { item: ActivityItem; onOpen: () => void }) {
  return (
    <li>
      <button
        type="button"
        id={`activity-${item.id}`}
        aria-label={activityLabel(item)}
        onClick={onOpen}
        className="flex w-full cursor-pointer items-center gap-3 px-1 py-1.5 text-left hover:bg-track"
      >
        <span className={clsx("size-2 shrink-0 rounded-full", TONE_CLASS[item.tone].dot)} />
        <span className="min-w-0 flex-1 truncate text-body text-ink">{item.text}</span>
        <span className="shrink-0 text-small text-subtle">{item.when}</span>
      </button>
    </li>
  );
}

export function PipelineScreen() {
  const board = usePipeline();
  const openApplication = useNav((s) => s.openApplication);
  const show = useNav((s) => s.show);
  const data = board.data;

  return (
    <Page label="Pipeline" title="Pipeline" subtitle={data && <Caption>{pipelineSummary(data.count)}</Caption>}>
      {board.error && data === undefined && <ErrorBanner id="pipeline-error" error={board.error} title="The pipeline could not be read" />}
      {data !== undefined &&
        (data.count === 0 ? (
          <EmptyState
            icon={LayoutDashboard}
            title="No applications yet"
            body="Prepare a CV for a job on Today and it shows up here."
            action={
              <Button id="pipeline-to-today" onClick={() => show("today")}>
                Go to Today
              </Button>
            }
          />
        ) : (
          <>
            {/* The board scrolls sideways when the window is too narrow for five columns. */}
            <div id="board" className="overflow-x-auto pb-2">
              <div className={clsx("flex w-full items-stretch gap-3.5", BOARD_MIN_WIDTH)}>
                {data.columns.map((column) => (
                  <Column key={column.key} column={column} onOpen={openApplication} />
                ))}
              </div>
            </div>
            <Card>
              <Heading>Recent activity</Heading>
              {data.activity.length === 0 ? (
                <Caption>Nothing has happened yet.</Caption>
              ) : (
                <ul aria-label="Recent activity" className="flex flex-col">
                  {data.activity.map((item) => (
                    <ActivityButton key={item.id} item={item} onOpen={() => openApplication(item.appId)} />
                  ))}
                </ul>
              )}
            </Card>
          </>
        ))}
    </Page>
  );
}
