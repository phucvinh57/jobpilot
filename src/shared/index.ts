// The shared kernel of the web app: what every context uses and none of them owns.
// (The bridge to the shell is imported separately, from `@/shared/bridge`, by `api/` layers only.)

export * from "./ui/kit";
export * from "./ui/rules";
export { TONE_CLASS, type Tone } from "./theme/tokens";
export { APPEARANCES, type Appearance } from "./theme/theme";
export { CommandError, toCommandError, needsRecheck, type Problem, type ProblemKind } from "./problem/problem";
export { ErrorBanner, ProblemActions, ProblemHelp } from "./problem/ProblemHelp";
export { useAction, useQuery, type Action, type Query } from "./lib/async";
export {
  sectionId,
  SETTINGS_SECTIONS,
  STAGES,
  useNav,
  type DrawerTarget,
  type Screen,
  type SettingsSection,
  type Stage,
} from "./nav/nav";
export { openUrl, useGraphProgress, useShell, type GraphProgress } from "./shell/shell";
export { CompanyBlock, InterviewBlock, NOT_FOUND, type Insight } from "./insight/InsightBrief";
