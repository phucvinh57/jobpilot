import { Button, Caption, ErrorBanner, Field, FilterChip, Section, Tag, TextInput } from "@/shared";

import { browserLine, browserTag, downloadLabel, NEW_KEY, sessionLine, statusTone, SUGGESTIONS, type Session } from "../domain/sessions";
import { useSessions, type SessionsCard } from "../hooks/useSessions";

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
        <Button id={`done-${id}`} aria-label={`Done logging in to ${session.name}`} onClick={() => card.finishLogin(id)}>
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
      <Button id={`sign-out-${id}`} aria-label={`Sign out of ${session.name}`} onClick={() => card.destructive(id, "signOut")}>
        {armed("signOut") ? "Sure?" : "Sign out"}
      </Button>
      <Button id={`delete-${id}`} variant="danger" aria-label={`Delete ${session.name}`} onClick={() => card.destructive(id, "delete")}>
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

/** The form that adds a session: a name, the login address, quick starts. */
function NewSession({ card }: { card: SessionsCard }) {
  const creating = card.busy[NEW_KEY];
  return (
    <div className="flex flex-col gap-3 border border-border bg-surface-alt p-3">
      <Field label="Name">
        <TextInput id="session-name" aria-label="Name" placeholder="Name, e.g. LinkedIn" value={card.name} onChange={(e) => card.setName(e.target.value)} />
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
          <FilterChip key={suggestion.name} id={`suggestion-${index}`} selected={false} onClick={() => card.fill(suggestion.name, suggestion.url)}>
            {suggestion.name}
          </FilterChip>
        ))}
      </div>
      <div className="flex flex-wrap items-center gap-2">
        <Button id="create-session" busy={creating !== undefined} onClick={card.createAndLogin}>
          Open browser to log in
        </Button>
        <Button id="cancel-session" variant="ghost" disabled={creating !== undefined} onClick={card.closeForm}>
          Cancel
        </Button>
        {creating !== undefined && <span className="text-small text-muted">{creating}</span>}
      </div>
    </div>
  );
}

/** Settings > Browser sessions: app-owned browser profiles used to apply. */
export function SessionsSection() {
  const card = useSessions();
  const data = card.query.data;
  const now = new Date();
  const chrome = data?.chrome ?? null;
  const tag = data ? browserTag(chrome, card.download) : undefined;

  return (
    <Section
      id="settings-sessions"
      title="Browser sessions"
      hint="Logins kept in a managed browser profile, separate from your own browser."
      actions={tag && <Tag tone={tag.tone}>{tag.text}</Tag>}
    >
      {card.query.error && !data && <ErrorBanner error={card.query.error} title="Could not read the sessions" onRetry={card.query.reload} />}
      {data && (
        <>
          <div className="flex flex-wrap items-center gap-3">
            <p id="browser-line" className="selectable min-w-[200px] flex-1 truncate text-small text-muted">
              {browserLine(chrome, card.download)}
            </p>
            <Button id="download-browser" busy={card.download.active} disabled={card.download.active} onClick={card.downloadBrowser}>
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
          {data.sessions.length === 0 && !card.formOpen && <Caption>No sessions yet</Caption>}
          {card.formOpen ? (
            <NewSession card={card} />
          ) : (
            <div>
              <Button id="new-session" onClick={card.openForm}>
                New session
              </Button>
            </div>
          )}
        </>
      )}
      {card.error && <ErrorBanner id="sessions-error" error={card.error} title="Something went wrong" onDismiss={card.clearError} />}
    </Section>
  );
}
