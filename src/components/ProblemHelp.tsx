// Shared "something is missing or failed" pieces: what to do about it, and the buttons that do
// it. Every screen points the user to the same fixes.

import { ExternalLink, Settings } from "lucide-react";

import { useNav, type SettingsSection } from "@/stores/nav";
import { useShell } from "@/stores/shell";
import { Banner, Button, Caption, FormMessage } from "@/components/ui/kit";
import { useErrorToast } from "@/hooks/useErrorToast";
import { problemActions, type CommandError, type Problem } from "@/lib/problem";

interface ActionsProps {
  problem: Problem;
  /** Run the failed thing again (shown for retryable problems only). */
  onRetry?: () => void;
  /** Open the visible browser of the paused run (`needsHuman`); without it the button opens
   *  Settings > Browser sessions. */
  onOpenBrowser?: () => void;
}

/** The buttons of a problem: Settings shortcut, "Check again", "Open browser", Retry. */
export function ProblemActions({ problem, onRetry, onOpenBrowser }: ActionsProps) {
  const openSettings = useNav((s) => s.openSettings);
  const refresh = useShell((s) => s.refresh);
  return (
    <>
      {problemActions(problem, onRetry !== undefined).map((action) => {
        switch (action) {
          case "settings":
            return (
              <Button
                key={action}
                size="compact"
                icon={Settings}
                onClick={() => openSettings(problem.settingsSection as SettingsSection)}
              >
                Open Settings &gt; {problem.settingsSection}
              </Button>
            );
          case "recheck":
            return (
              <Button key={action} size="compact" onClick={refresh}>
                Check again
              </Button>
            );
          case "openBrowser":
            return (
              <Button
                key={action}
                size="compact"
                icon={ExternalLink}
                onClick={onOpenBrowser ?? (() => openSettings("Browser sessions"))}
              >
                Open browser
              </Button>
            );
          case "retry":
            return (
              <Button key={action} size="compact" onClick={onRetry}>
                Retry
              </Button>
            );
        }
      })}
    </>
  );
}

/** Advice and buttons for a problem; nothing when there is nothing useful to offer. */
export function ProblemHelp(props: ActionsProps) {
  const { problem } = props;
  if (!problem.advice) {
    return null;
  }
  return (
    <div className="flex flex-col gap-1">
      <p className="selectable text-small text-muted">{problem.advice}</p>
      <div className="flex flex-wrap gap-2">
        <ProblemActions {...props} />
      </div>
    </div>
  );
}

/** A failed command as a banner: what happened, what to do, and the buttons that do it. */
export function ErrorBanner({
  error,
  title = "Something went wrong",
  onRetry,
  onOpenBrowser,
  onDismiss,
  id,
}: {
  error: CommandError;
  title?: string;
  onRetry?: () => void;
  onOpenBrowser?: () => void;
  onDismiss?: () => void;
  id?: string;
}) {
  const { problem } = error;
  return (
    <Banner
      id={id}
      tone={problem.kind === "needsHuman" ? "warn" : "danger"}
      title={title}
      body={
        <>
          <p>{error.message}</p>
          {problem.advice && <p>{problem.advice}</p>}
        </>
      }
      actions={
        <>
          <ProblemActions problem={problem} onRetry={onRetry} onOpenBrowser={onOpenBrowser} />
          {onDismiss && (
            <Button size="compact" variant="ghost" onClick={onDismiss}>
              Dismiss
            </Button>
          )}
        </>
      }
    />
  );
}

/** A read that failed: the error goes to a toast, and the place of what could not be read says so
 *  quietly, with a Retry when there is one. Nothing while there is no error. */
export function LoadError({
  error,
  title,
  onRetry,
  id,
}: {
  error: CommandError | undefined;
  title: string;
  onRetry?: () => void;
  id?: string;
}) {
  useErrorToast(error, { title, onRetry });
  if (!error) {
    return null;
  }
  return (
    <div id={id} className="flex flex-wrap items-center gap-2">
      <Caption>{title}.</Caption>
      {onRetry && (
        <Button size="compact" onClick={onRetry}>
          Retry
        </Button>
      )}
    </div>
  );
}

/** Why a form's submit failed, under its fields: the message in red, the advice and buttons of
 *  its problem below. */
export function FormError({
  error,
  title,
  onRetry,
  id,
}: {
  error: CommandError;
  title?: string;
  onRetry?: () => void;
  id?: string;
}) {
  const { problem } = error;
  return (
    <FormMessage id={id}>
      <p>{title ? `${title}: ${error.message}` : error.message}</p>
      {problem.advice && <p className="text-muted">{problem.advice}</p>}
      {problemActions(problem, onRetry !== undefined).length > 0 && (
        <div className="flex flex-wrap gap-2">
          <ProblemActions problem={problem} onRetry={onRetry} />
        </div>
      )}
    </FormMessage>
  );
}
