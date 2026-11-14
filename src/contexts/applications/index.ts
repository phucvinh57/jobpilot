// Bounded context `applications`. Applications: the CVs / Applied / In progress / Done stages of the
// Jobs page with the application drawer, and the full Application screen (JD, CV, apply, timeline).
//
//   domain/  types mirroring the shell's DTOs + pure helpers (no React, no shell) + tests
//   api/     the context's port: typed functions over `@/shared/bridge` (invoke / events)
//   hooks/   use-cases as React hooks: state, loading, Problem
//   ui/      screens and components; they read hooks only
//
// Other contexts and the app shell import from this file only.

//
// The Jobs page (`src/app`) composes the application stages from these pieces:
//   useBoard()             the stages with their cards (for the stage bar's counts: `appStages`)
//   ApplicationsStage      the table of one stage and the recent activity ({ stage, board }: the
//                          page reads the board once, for the counts and the table)
//   ApplicationDrawer      the drawer of the open application (`useNav` drawer `{ kind: "app" }`);
//                          it holds the primary action while open
//   ApplicationScreen      the full view (screen "application")

export { ApplicationsStage } from "./ui/ApplicationsStage";
export { ApplicationDrawer } from "./ui/ApplicationDrawer";
export { ApplicationScreen } from "./ui/ApplicationScreen";
export { useBoard } from "./hooks/useBoard";
export { appStages } from "./domain/pipeline";
export type { Board, StageKey } from "./domain/types";
