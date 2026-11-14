// Bounded context `discovery`. Leads (the Leads stage of the Jobs page) and job Sources.
//
//   domain/  types mirroring the shell's DTOs + pure helpers (no React, no shell) + tests
//   api/     the context's port: typed functions over `@/shared/bridge` (invoke / events)
//   hooks/   use-cases as React hooks: state, loading, Problem
//   ui/      screens and components; they read hooks only
//
// Other contexts and the app shell import from this file only.
//
// The Jobs page (`src/app`) composes the Leads stage from these pieces; each reads the context's
// store itself and takes no data props:
//   useLeadsLifecycle()   call once while the page is shown (any stage): reads the queue again,
//                         follows the shell from then on, and turns a dropped file into a Lead
//   useLeadsSummary()     { newLeads, needsYou, loaded } for the stage bar's count
//   LeadsHeaderLine       the date and the last search (or the search running), for the subtitle
//   SearchNowButton       `#queue-search`; primary only when the list is empty and no drawer is
//                         open ({ otherDrawerOpen }: an application's drawer holds the primary)
//   LinkBox               the link box with "Add job"
//   SourcesLine           the job sources' status line and the note of the last search
//   NeedsYou              the "Needs you" banners (nothing when nothing waits)
//   LeadsStage            the list (filters, bulk bar, table) or the empty state ({ otherDrawerOpen })
//   LeadDrawer            the drawer of the open lead (`useNav` drawer `{ kind: "lead" }`); it holds
//                         the primary "Prepare CV" while open
// The open lead is the nav store's drawer: `useNav.getState().openLead(id | "best")`.

export { LeadsHeaderLine, SearchNowButton, SourcesLine } from "./ui/JobsHeader";
export { LinkBox } from "./ui/LinkBox";
export { NeedsYou } from "./ui/NeedsYou";
export { LeadsStage } from "./ui/LeadsStage";
export { LeadDrawer } from "./ui/LeadDrawer";
export { useLeadsLifecycle, useLeadsSummary, type LeadsSummary } from "./hooks/useToday";
export { JobSourcesSection } from "./ui/JobSourcesSection";
