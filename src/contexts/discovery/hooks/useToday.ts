// Today as a screen needs it: the store's state with the derived values the views read (what is
// shown, which lead is open, what "Prepare CV" would act on, the one primary button, the banners).

import { useEffect, useMemo } from "react";

import { useNav } from "@/shared";

import {
  failureRows,
  filterItems,
  needsYouCount,
  openLeadId,
  queuePrimaryFor,
  targetsOf,
  type FailureRow,
  type QueuePrimary,
} from "../domain/queue";
import type { Lead, Source, Waiting } from "../domain/types";
import { addDropped, enter, useTodayStore, type TodayState } from "./todayStore";

export interface TodayView extends TodayState {
  /** The first read has answered. */
  loaded: boolean;
  items: Lead[];
  /** The leads that pass the filters. */
  shown: Lead[];
  /** The lead that is open. */
  open: Lead | undefined;
  /** The leads "Prepare CV" acts on: the ticked ones, else the open one. */
  targets: number[];
  primary: QueuePrimary;
  /** A search is running, from here or from the daily timer: its progress line. */
  searchingLabel: string | undefined;
  waitingSources: Waiting[];
  failureRows: FailureRow[];
  /** What the "NEEDS YOU" header counts. */
  needsYou: number;
  sources: Source[];
  /** The source that found `lead`, by name. */
  sourceOf: (lead: Lead) => Source | undefined;
}

const NO_ITEMS: Lead[] = [];
const NO_SOURCES: Source[] = [];
const NO_WAITING: Waiting[] = [];

/** Read the store, and start following the shell (Today shown: read everything again). */
export function useToday(): TodayView {
  const state = useTodayStore();
  const requested = useNav((s) => s.leadId);
  const dropped = useNav((s) => s.droppedFile);

  useEffect(() => enter(), []);

  // A file dropped on the window: a lead like a pasted link, or the reason it was refused.
  useEffect(() => {
    if (dropped !== undefined) {
      const path = useNav.getState().takeDroppedFile();
      if (path !== undefined) {
        void addDropped(path);
      }
    }
  }, [dropped]);

  const { queue, filter, checked, jobs, failed, prepared, searching } = state;
  const items = queue?.items ?? NO_ITEMS;
  const shown = useMemo(() => filterItems(items, filter), [items, filter]);
  const openId = openLeadId(shown, requested);
  const open = useMemo(() => shown.find((l) => l.id === openId), [shown, openId]);
  const targets = useMemo(() => targetsOf(shown, checked, openId), [shown, checked, openId]);
  const sources = queue?.sources ?? NO_SOURCES;
  const searchingLabel = searching ?? (queue?.searching ? "Searching your sources" : undefined);
  const rows = useMemo(() => failureRows(queue?.failed ?? [], items, jobs, failed), [queue, items, jobs, failed]);
  const primary = queuePrimaryFor({
    total: items.length,
    shown: shown.length,
    targets: targets.length,
    idleTargets: targets.filter((id) => jobs[id] === undefined).length,
    searching: searchingLabel !== undefined,
    sources: sources.filter((s) => s.enabled).length,
  });

  return {
    ...state,
    loaded: queue !== undefined,
    items,
    shown,
    open,
    targets,
    primary,
    searchingLabel,
    waitingSources: queue?.waiting ?? NO_WAITING,
    failureRows: rows,
    needsYou: needsYouCount(queue, rows, prepared.length),
    sources,
    sourceOf: (lead) => sources.find((s) => s.id === lead.sourceId),
  };
}
