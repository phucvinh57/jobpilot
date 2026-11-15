// An application stage of the Jobs page (CVs, Applied, In progress, Done): a table of the
// applications in it, each row opening its drawer, and the latest events across all of them. No
// primary button here: the action is in the drawer.

import clsx from "clsx";
import { useState } from "react";

import {
  Button,
  Caption,
  Card,
  DataTable,
  ErrorBanner,
  Heading,
  Tag,
  TONE_CLASS,
  useNav,
  type DataColumn,
  type Query,
} from "@/shared";

import {
  ACTIVITY_PREVIEW,
  activityLabel,
  cardLabel,
  emptyHint,
  shownActivity,
  stageCards,
  stageLabel,
  stageOf,
} from "../domain/pipeline";
import type { ActivityItem, Board, BoardCard, StageKey } from "../domain/types";

const COLUMNS: readonly DataColumn<BoardCard>[] = [
  {
    key: "role",
    header: "Role",
    className: "w-full max-w-0",
    cell: (card) => (
      <div className="flex min-w-0 flex-col">
        <span className="truncate font-semibold text-ink">{card.title}</span>
        {card.subtitle !== "" && <span className="truncate text-small text-muted">{card.subtitle}</span>}
      </div>
    ),
  },
  {
    key: "status",
    header: "Status",
    // The shell's words for what happens next (how it ended, for Done).
    cell: (card) => <Tag tone={card.next.tone}>{card.next.label}</Tag>,
  },
  {
    key: "age",
    header: "Age",
    className: "text-right",
    cell: (card) => <span className="text-small text-subtle tabular-nums">{card.age}</span>,
  },
];

/** What an empty stage says; the CVs stage leads to the Leads. */
function Empty({ stage }: { stage: StageKey }) {
  const showStage = useNav((s) => s.showStage);
  return (
    <div className="flex flex-col items-center gap-1.5">
      <span className="font-semibold text-ink">Nothing here yet</span>
      <span className="text-small">{emptyHint(stage)}</span>
      {stage === "cv" && (
        <Button id="pick-lead" size="compact" className="mt-1" onClick={() => showStage("leads")}>
          Pick a lead
        </Button>
      )}
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
        <span className={clsx("size-2 shrink-0", TONE_CLASS[item.tone].dot)} />
        <span className="min-w-0 flex-1 truncate text-body text-ink">{item.text}</span>
        <span className="shrink-0 text-small text-subtle">{item.when}</span>
      </button>
    </li>
  );
}

function Activity({ items, onOpen }: { items: readonly ActivityItem[]; onOpen: (appId: number) => void }) {
  const [all, setAll] = useState(false);
  const more = items.length > ACTIVITY_PREVIEW;
  return (
    <Card>
      <div className="flex items-center justify-between gap-2">
        <Heading>Recent activity</Heading>
        {more && (
          <Button
            id="activity-toggle"
            variant="ghost"
            size="compact"
            aria-expanded={all}
            onClick={() => setAll((on) => !on)}
          >
            {all ? "Show less" : `Show all ${items.length}`}
          </Button>
        )}
      </div>
      {items.length === 0 ? (
        <Caption>Nothing has happened yet.</Caption>
      ) : (
        <ul aria-label="Recent activity" className="flex flex-col">
          {shownActivity(items, all).map((item) => (
            <ActivityButton key={item.id} item={item} onOpen={() => onOpen(item.appId)} />
          ))}
        </ul>
      )}
    </Card>
  );
}

/** The table of `stage`. `board` is the Jobs page's one read of the board (`useBoard`), shared with
 *  the stage bar's counts. */
export function ApplicationsStage({ stage, board }: { stage: StageKey; board: Query<Board> }) {
  const drawer = useNav((s) => s.drawer);
  const openAppDrawer = useNav((s) => s.openAppDrawer);
  const data = board.data;
  const label = stageLabel(stage);

  if (data === undefined) {
    return board.error ? (
      <ErrorBanner
        id="board-error"
        error={board.error}
        title="The applications could not be read"
        onRetry={board.reload}
      />
    ) : (
      <Caption>Loading the applications...</Caption>
    );
  }
  return (
    <>
      {board.error && (
        <ErrorBanner
          id="board-error"
          error={board.error}
          title="The applications could not be read"
          onRetry={board.reload}
        />
      )}
      <DataTable
        id={`table-${stage}`}
        label={label}
        columns={COLUMNS}
        rows={stageCards(data, stage)}
        rowKey={(card) => String(card.id)}
        rowLabel={cardLabel}
        onOpen={(card) => openAppDrawer(card.id)}
        openKey={drawer?.kind === "app" ? String(drawer.id) : null}
        empty={<Empty stage={stage} />}
      />
      <Activity items={data.activity} onOpen={(appId) => openAppDrawer(appId, stageOf(data, appId))} />
    </>
  );
}
