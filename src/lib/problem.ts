// Why something failed, in terms the user can act on. The classification is done by the shell
// (`engine::health`); the web app only shows it.

export type ProblemKind = "noProvider" | "noTypst" | "noChrome" | "network" | "needsHuman" | "other";

export interface Problem {
  kind: ProblemKind;
  /** `login` / `captcha` / `blocked`, only with `needsHuman`. */
  challenge: "login" | "captcha" | "blocked" | null;
  /** One line telling the user what to do. */
  advice: string | null;
  /** The Settings section that fixes it. */
  settingsSection: string | null;
  /** Trying the same thing again can succeed. */
  retryable: boolean;
}

export const OTHER: Problem = { kind: "other", challenge: null, advice: null, settingsSection: null, retryable: false };

/** The error of every shell command. */
export class CommandError extends Error {
  readonly problem: Problem;

  constructor(message: string, problem: Problem = OTHER) {
    super(message);
    this.name = "CommandError";
    this.problem = problem;
  }
}

function isProblem(value: unknown): value is Problem {
  return typeof value === "object" && value !== null && typeof (value as { kind?: unknown }).kind === "string";
}

/** Whatever a rejected command threw, as a `CommandError`. */
export function toCommandError(thrown: unknown): CommandError {
  if (thrown instanceof CommandError) {
    return thrown;
  }
  if (typeof thrown === "object" && thrown !== null) {
    const { message, problem } = thrown as { message?: unknown; problem?: unknown };
    if (typeof message === "string") {
      return new CommandError(message, isProblem(problem) ? problem : OTHER);
    }
  }
  return new CommandError(typeof thrown === "string" ? thrown : "Something went wrong.");
}

/** A prerequisite is missing: the setup state should be read again. */
export function needsRecheck(problem: Problem): boolean {
  return problem.kind === "noProvider" || problem.kind === "noTypst" || problem.kind === "noChrome";
}

/** The buttons a problem offers, in order. */
export type ProblemAction = "settings" | "recheck" | "openBrowser" | "retry";

export function problemActions(problem: Problem, canRetry: boolean): ProblemAction[] {
  const actions: ProblemAction[] = [];
  if (problem.settingsSection) {
    actions.push("settings");
  }
  if (problem.kind === "noTypst") {
    actions.push("recheck");
  }
  if (problem.kind === "needsHuman") {
    actions.push("openBrowser");
  }
  if (canRetry && problem.retryable) {
    actions.push("retry");
  }
  return actions;
}
