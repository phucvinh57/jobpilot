// The Leads stage of the Jobs page as its views need it: the store's state with the derived values
// they read (what is shown, which lead the drawer shows, what "Prepare CV" would act on, the one
// primary button and where it sits, the banners). `useLeadsLifecycle` is what the page calls once.

import { useEffect, useMemo } from "react";

import { useNav } from "@/stores/nav";

import {
  failureRows,
  filterDismissed,
  filterItems,
  needsYouCount,
  openLeadId,
  primaryPlace,
  queuePrimaryFor,
  targetsOf,
  type FailureRow,
  type PrimaryPlace,
  type QueuePrimary,
} from "@/lib/discovery/queue";
import type { DismissedJob, Lead, Source, Waiting } from "@/types/discovery";
import { addDropped, enter, requestedLead, useTodayStore, type TodayState } from "@/stores/today";

export interface TodayView extends TodayState {
  /** The first read has answered. */
  loaded: boolean;
  items: Lead[];
  /** The leads that pass the filters. */
  shown: Lead[];
  /** The lead the drawer shows (`undefined`: the lead drawer is closed). */
  open: Lead | undefined;
  /** The leads the bulk bar's "Prepare N CVs" acts on: the ticked ones, else the open one. */
  targets: number[];
  /** What the primary button of the list (bulk bar, Search now) does. */
  primary: QueuePrimary;
  /** What the drawer's "Prepare CV" does: it acts on the open lead only, ticked or not. */
  openPrimary: QueuePrimary;
  /** Where the one primary button sits (`otherDrawerOpen`: an application's drawer is open). */
  placeFor: (otherDrawerOpen: boolean) => PrimaryPlace;
  /** An application's drawer is open on the Jobs page. */
  appDrawerOpen: boolean;
  /** A search is running, from here or from the daily timer: its progress line. */
  searchingLabel: string | undefined;
  waitingSources: Waiting[];
  failureRows: FailureRow[];
  /** What the "NEEDS YOU" header counts. */
  needsYou: number;
  sources: Source[];
  /** The source that found `lead`, by name. */
  sourceOf: (lead: Lead) => Source | undefined;
  /** How many jobs are dismissed (the "Dismissed" chip). */
  dismissedCount: number;
  /** The dismissed jobs as read (`[]` until "Dismissed" is picked). */
  dismissedItems: DismissedJob[];
  /** The dismissed jobs that pass the search. */
  dismissedShown: DismissedJob[];
}

const NO_ITEMS: Lead[] = [];
const NO_SOURCES: Source[] = [];
const NO_WAITING: Waiting[] = [];
const NO_DISMISSED: DismissedJob[] = [];

/** Read the store and the drawer. Reads only: `useLeadsLifecycle` starts the reading. */
export function useToday(): TodayView {
  const state = useTodayStore();
  const drawer = useNav((s) => s.drawer);
  const requested = requestedLead(drawer);

  const { queue, filter, checked, jobs, failed, prepared, searching } = state;
  const items = queue?.items ?? NO_ITEMS;
  const shown = useMemo(() => filterItems(items, filter), [items, filter]);
  const openId = openLeadId(shown, requested);
  const open = useMemo(() => shown.find((l) => l.id === openId), [shown, openId]);
  const targets = useMemo(() => targetsOf(shown, checked, openId), [shown, checked, openId]);
  const sources = queue?.sources ?? NO_SOURCES;
  const searchingLabel = searching ?? (queue?.searching ? "Searching your sources" : undefined);
  const rows = useMemo(() => failureRows(queue?.failed ?? [], items, jobs, failed), [queue, items, jobs, failed]);
  const facts = {
    total: items.length,
    shown: shown.length,
    searching: searchingLabel !== undefined,
    sources: sources.filter((s) => s.enabled).length,
  };
  const primary = queuePrimaryFor({
    ...facts,
    targets: targets.length,
    idleTargets: targets.filter((id) => jobs[id] === undefined).length,
  });
  const openPrimary = queuePrimaryFor({
    ...facts,
    targets: open === undefined ? 0 : 1,
    idleTargets: open === undefined || jobs[open.id] !== undefined ? 0 : 1,
  });
  const appDrawerOpen = drawer?.kind === "app";
  const dismissedItems = state.dismissed ?? NO_DISMISSED;
  const dismissedShown = useMemo(() => filterDismissed(dismissedItems, filter.search), [dismissedItems, filter.search]);

  return {
    ...state,
    loaded: queue !== undefined,
    items,
    shown,
    open,
    targets,
    primary,
    openPrimary,
    placeFor: (otherDrawerOpen) =>
      primaryPlace({
        drawerOpen: open !== undefined,
        otherDrawerOpen: otherDrawerOpen || appDrawerOpen,
        // The Dismissed view has no ticks: the bulk bar is not there.
        checked: filter.show === "dismissed" ? 0 : checked.length,
        empty: items.length === 0,
      }),
    appDrawerOpen,
    searchingLabel,
    waitingSources: queue?.waiting ?? NO_WAITING,
    failureRows: rows,
    needsYou: needsYouCount(queue, rows, prepared.length),
    sources,
    sourceOf: (lead) => sources.find((s) => s.id === lead.sourceId),
    dismissedCount: queue?.dismissed ?? 0,
    dismissedItems,
    dismissedShown,
  };
}

/** What the Jobs page needs once, whatever stage it shows: read the queue and the profile's skills
 *  again (and follow the shell from then on), and turn a file dropped on the window into a Lead. */
export function useLeadsLifecycle(): void {
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
}

/** The Leads stage in numbers, for the stage bar. */
export interface LeadsSummary {
  /** New leads in the list (before the filters). */
  newLeads: number;
  /** What the "NEEDS YOU" header counts (the sidebar badge shows the same). */
  needsYou: number;
  /** The first read has answered. */
  loaded: boolean;
}

/** The Leads stage in numbers. Reads only. */
export function useLeadsSummary(): LeadsSummary {
  const t = useToday();
  return { newLeads: t.items.length, needsYou: t.needsYou, loaded: t.loaded };
}
