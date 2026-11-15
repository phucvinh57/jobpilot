import { Button, Caption, Dialog, Field, FilterChip, Section, Tag, TextInput } from "@/components/ui/kit";
import { FormError, LoadError } from "@/components/ProblemHelp";

import {
  browserLine,
  browserTag,
  downloadLabel,
  NEW_KEY,
  sessionLine,
  statusTone,
  SUGGESTIONS,
} from "@/lib/integrations/sessions";
import type { Session } from "@/types/integrations";
import { useErrorToast } from "@/hooks/useErrorToast";
import { useSessions, type SessionsCard } from "@/hooks/integrations/useSessions";

/** What a row offers: Check / Open / Sign out / Delete, or the label of what is running, or the
 *  Done button of a login window that waits for the user. */
function SessionActions({ session, card }: { session: Session; card: SessionsCard }) {
  const { id } = session;
  const label = card.busy[id];
  if (label !== undefined) {
    return <span className="text-small text-muted">{label}</span>;
  }
  if (session.loginInProgress) {
    return (
      <div className="flex flex-wrap items-center gap-2">
        <span className="text-small text-muted">Log in in the browser window, then press Done</span>
        <Button
          id={`done-${id}`}
          aria-label={`Done logging in to ${session.name}`}
          onClick={() => card.finishLogin(id)}
        >
          Done
        </Button>
      </div>
    );
  }
  const armed = (action: "signOut" | "delete") => card.confirm?.id === id && card.confirm.action === action;
  return (
    <div className="flex flex-wrap gap-2">
      <Button id={`check-${id}`} aria-label={`Check ${session.name}`} onClick={() => card.check(id)}>
        Check
      </Button>
      <Button id={`open-${id}`} aria-label={`Open ${session.name}`} onClick={() => card.openLogin(id)}>
        Open
      </Button>
      <Button
        id={`sign-out-${id}`}
        aria-label={`Sign out of ${session.name}`}
        onClick={() => card.destructive(id, "signOut")}
      >
        {armed("signOut") ? "Sure?" : "Sign out"}
      </Button>
      <Button
        id={`delete-${id}`}
        variant="danger"
        aria-label={`Delete ${session.name}`}
        onClick={() => card.destructive(id, "delete")}
      >
        {armed("delete") ? "Sure?" : "Delete"}
      </Button>
    </div>
  );
}

function SessionRow({ session, card, now }: { session: Session; card: SessionsCard; now: Date }) {
  return (
    <li className="flex flex-wrap items-center gap-3 border-b border-border py-2">
      <div className="flex min-w-[160px] flex-1 flex-col">
        <span className="truncate text-body font-semibold text-ink">{session.name}</span>
        <span className="selectable truncate text-tiny text-muted">{sessionLine(session, now)}</span>
      </div>
      <Tag tone={statusTone(session.status)}>{session.status}</Tag>
      <SessionActions session={session} card={card} />
    </li>
  );
}

/** The dialog that adds a session: a name, the login address, quick starts. It closes once the
 *  login browser is open; the new row then waits for Done. */
function NewSessionDialog({ card }: { card: SessionsCard }) {
  const creating = card.busy[NEW_KEY] !== undefined;
  return (
    <Dialog
      id="session-dialog"
      open={card.formOpen}
      title="New session"
      description="A browser opens on the login address; log in there, then press Done."
      onClose={card.closeForm}
      onSubmit={card.createAndLogin}
      footer={
        <>
          <Button id="cancel-session" variant="ghost" disabled={creating} onClick={card.closeForm}>
            Cancel
          </Button>
          <Button id="create-session" type="submit" variant="primary" busy={creating} disabled={creating}>
            Open browser to log in
          </Button>
        </>
      }
    >
      <Field label="Name">
        <TextInput
          id="session-name"
          aria-label="Name"
          placeholder="Name, e.g. LinkedIn"
          value={card.name}
          onChange={(e) => card.setName(e.target.value)}
        />
      </Field>
      <Field label="Login address">
        <TextInput
          id="session-url"
          aria-label="Login address"
          placeholder="Login URL, https://..."
          value={card.url}
          onChange={(e) => card.setUrl(e.target.value)}
        />
      </Field>
      <div role="group" aria-label="Suggestions" className="flex flex-wrap items-center gap-2">
        <span className="text-tiny text-muted">Suggestions:</span>
        {SUGGESTIONS.map((suggestion, index) => (
          <FilterChip
            key={suggestion.name}
            id={`suggestion-${index}`}
            selected={card.name === suggestion.name && card.url === suggestion.url}
            onClick={() => card.fill(suggestion.name, suggestion.url)}
          >
            {suggestion.name}
          </FilterChip>
        ))}
      </div>
      {card.createError && (
        <FormError id="session-create-error" error={card.createError} title="Could not create the session" />
      )}
    </Dialog>
  );
}

/** Settings > Browser sessions: app-owned browser profiles used to apply. */
export function SessionsSection() {
  const card = useSessions();
  const data = card.query.data;
  const now = new Date();
  const chrome = data?.chrome ?? null;
  const tag = data ? browserTag(chrome, card.download) : undefined;
  useErrorToast(card.error, { title: card.errorTitle, clear: card.clearError });

  return (
    <Section
      id="settings-sessions"
      title="Browser sessions"
      hint="Logins kept in a managed browser profile, separate from your own browser."
      actions={tag && <Tag tone={tag.tone}>{tag.text}</Tag>}
    >
      {!data && <LoadError error={card.query.error} title="Could not read the sessions" onRetry={card.query.reload} />}
      {data && (
        <>
          <div className="flex flex-wrap items-center gap-3">
            <p id="browser-line" className="selectable min-w-[200px] flex-1 truncate text-small text-muted">
              {browserLine(chrome, card.download)}
            </p>
            <Button
              id="download-browser"
              busy={card.download.active}
              disabled={card.download.active}
              onClick={card.downloadBrowser}
            >
              {downloadLabel(chrome)}
            </Button>
          </div>
          {data.sessions.length > 0 && (
            <ul aria-label="Sessions" className="flex flex-col">
              {data.sessions.map((session) => (
                <SessionRow key={session.id} session={session} card={card} now={now} />
              ))}
            </ul>
          )}
          {data.sessions.length === 0 && <Caption>No sessions yet</Caption>}
          <div>
            <Button id="new-session" onClick={card.openForm}>
              New session
            </Button>
          </div>
        </>
      )}
      <NewSessionDialog card={card} />
    </Section>
  );
}
