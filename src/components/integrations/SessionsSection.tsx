import {
  Banner,
  Button,
  Caption,
  Dialog,
  Field,
  FilterChip,
  Section,
  SectionLabel,
  Tag,
  TextInput,
} from "@/components/ui/kit";
import { FormError, LoadError } from "@/components/ProblemHelp";

import {
  connectionLine,
  connectionTag,
  disconnectedLine,
  NEW_KEY,
  privateTabsLine,
  sessionLine,
  statusLabel,
  statusTone,
  SUGGESTIONS,
} from "@/lib/integrations/sessions";
import type { BrowserStatus, Session } from "@/types/integrations";
import { useErrorToast } from "@/hooks/useErrorToast";
import { useSessions, type SessionsCard } from "@/hooks/integrations/useSessions";

/** Where the extension comes from before it is connected: chrome://extensions, Developer mode,
 *  "Load unpacked" on the folder JobPilot ships, then pairing. */
function InstallSteps({ browser, card }: { browser: BrowserStatus; card: SessionsCard }) {
  const path = browser.extensionPath;
  return (
    <ol aria-label="Install the extension" className="flex list-decimal flex-col gap-1 pl-5 text-small text-ink">
      <li>
        Open <span className="selectable font-semibold">chrome://extensions</span> in Chrome.
      </li>
      <li>Turn on Developer mode (top right).</li>
      <li>
        <span>Press "Load unpacked" and pick this folder:</span>
        {path ? (
          <span className="mt-1 flex flex-wrap items-center gap-2">
            <code
              id="extension-path"
              className="selectable break-all border border-border bg-track px-2 py-1 text-tiny"
            >
              {path}
            </code>
            <Button
              id="copy-extension-path"
              size="compact"
              aria-label="Copy the extension folder"
              onClick={() => card.copyPath(path)}
            >
              Copy
            </Button>
          </span>
        ) : (
          <span className="block text-muted">
            the extension folder of JobPilot (not found here: build it with bun run ext:build).
          </span>
        )}
      </li>
      <li>Open the JobPilot extension's popup: it shows a 6-digit code, and JobPilot asks you to pair here.</li>
    </ol>
  );
}

/** The connection: which browser, whether private tabs work and Forget; or how to install and pair. */
function Connection({ browser, card }: { browser: BrowserStatus; card: SessionsCard }) {
  if (browser.connected) {
    return (
      <div id="browser-connection" className="flex flex-wrap items-center gap-3">
        <div className="flex min-w-[200px] flex-1 flex-col gap-0.5">
          <span id="browser-line" className="selectable text-body font-semibold text-ink">
            {connectionLine(browser)}
          </span>
          <span id="private-tabs-line" className="text-small text-muted">
            {privateTabsLine(browser)}
          </span>
        </div>
        <Button id="forget-browser" onClick={card.openForget} disabled={card.connectionBusy !== undefined}>
          Forget this browser
        </Button>
      </div>
    );
  }
  return (
    <div id="browser-connection" className="flex flex-col gap-2">
      <p id="browser-line" className="text-small text-muted">
        {disconnectedLine(browser)}
      </p>
      {browser.pairingCode !== null && card.pairingCode === undefined && (
        <Banner
          id="pairing-waiting"
          tone="warn"
          title="A browser asks to pair"
          body={`${browser.browser ?? "A browser"} waits for your answer.`}
          actions={
            <Button id="review-pairing" size="compact" onClick={card.reviewPairing}>
              Review
            </Button>
          }
        />
      )}
      {browser.listening && <InstallSteps browser={browser} card={card} />}
    </div>
  );
}

/** "Pair this browser?": the code to compare with the extension's popup; Pair or Decline. */
function PairingDialog({ browser, card }: { browser: BrowserStatus | undefined; card: SessionsCard }) {
  const code = card.pairingCode;
  const working = card.connectionBusy === "pair" || card.connectionBusy === "decline";
  return (
    <Dialog
      id="pairing-dialog"
      open={code !== undefined}
      busy={working}
      width="sm"
      title="Pair this browser?"
      description={`${browser?.browser ?? "A browser"} with the JobPilot extension asks to connect. Check that the code matches the code in the extension's popup.`}
      onClose={card.closePairing}
      footer={
        <>
          <Button
            id="decline-pairing"
            busy={card.connectionBusy === "decline"}
            disabled={working}
            onClick={card.declinePairing}
          >
            Decline
          </Button>
          <Button
            id="approve-pairing"
            variant="primary"
            busy={card.connectionBusy === "pair"}
            disabled={working}
            onClick={card.approvePairing}
          >
            Pair
          </Button>
        </>
      }
    >
      <p
        id="pairing-code"
        aria-label="Pairing code"
        className="selectable text-center font-heading text-h1 font-semibold tracking-[0.3em] text-ink"
      >
        {code}
      </p>
      <Caption>Only pair a browser you opened yourself. JobPilot then opens and fills tabs in it.</Caption>
    </Dialog>
  );
}

/** "Forget this browser?": the extension has to pair again. */
function ForgetDialog({ card }: { card: SessionsCard }) {
  const forgetting = card.connectionBusy === "forget";
  return (
    <Dialog
      id="forget-dialog"
      open={card.forgetOpen}
      busy={forgetting}
      width="sm"
      title="Forget this browser?"
      description="JobPilot stops opening tabs in it until you pair it again. You stay signed in to your sites."
      onClose={card.closeForget}
      footer={
        <>
          <Button id="cancel-forget" variant="ghost" disabled={forgetting} onClick={card.closeForget}>
            Cancel
          </Button>
          <Button
            id="confirm-forget"
            variant="primary"
            busy={forgetting}
            disabled={forgetting}
            onClick={card.forgetBrowser}
          >
            Forget
          </Button>
        </>
      }
    />
  );
}

/** What a row offers: Sign in / Check / Delete, the label of what is running, or the Done button of
 *  a sign-in tab that waits for the user. */
function SessionActions({ session, card, connected }: { session: Session; card: SessionsCard; connected: boolean }) {
  const { id } = session;
  const label = card.busy[id];
  if (label !== undefined) {
    return <span className="text-small text-muted">{label}</span>;
  }
  if (session.loginInProgress) {
    return (
      <div className="flex flex-wrap items-center gap-2">
        <span className="text-small text-muted">Sign in in your browser, then press Done</span>
        <Button
          id={`done-${id}`}
          aria-label={`Done signing in to ${session.name}`}
          onClick={() => card.finishLogin(id)}
        >
          Done
        </Button>
      </div>
    );
  }
  const offline = connected ? undefined : "Connect the JobPilot extension first";
  return (
    <div className="flex flex-wrap gap-2">
      <Button
        id={`sign-in-${id}`}
        aria-label={`Sign in to ${session.name}`}
        disabled={!connected}
        title={offline}
        onClick={() => card.openLogin(id)}
      >
        Sign in
      </Button>
      <Button
        id={`check-${id}`}
        aria-label={`Check ${session.name}`}
        disabled={!connected}
        title={offline}
        onClick={() => card.check(id)}
      >
        Check
      </Button>
      <Button
        id={`delete-${id}`}
        variant="danger"
        aria-label={`Delete ${session.name}`}
        onClick={() => card.remove(id)}
      >
        {card.confirmDelete === id ? "Sure?" : "Delete"}
      </Button>
    </div>
  );
}

function SessionRow({
  session,
  card,
  now,
  connected,
}: {
  session: Session;
  card: SessionsCard;
  now: Date;
  connected: boolean;
}) {
  return (
    <li className="flex flex-wrap items-center gap-3 border-b border-border py-2">
      <div className="flex min-w-[160px] flex-1 flex-col">
        <span className="truncate text-body font-semibold text-ink">{session.name}</span>
        <span className="selectable truncate text-tiny text-muted">{sessionLine(session, now)}</span>
      </div>
      <Tag tone={statusTone(session.status)}>{statusLabel(session.status)}</Tag>
      <SessionActions session={session} card={card} connected={connected} />
    </li>
  );
}

/** The dialog that adds a site: a name, the sign-in address, quick starts. It closes once the
 *  sign-in tab is open; the new row then waits for Done. */
function NewSessionDialog({ card }: { card: SessionsCard }) {
  const creating = card.busy[NEW_KEY] !== undefined;
  return (
    <Dialog
      id="session-dialog"
      open={card.formOpen}
      busy={creating}
      title="Add a site"
      description="A tab opens in your browser on the sign-in address; sign in there, then press Done."
      onClose={card.closeForm}
      onSubmit={card.createAndLogin}
      footer={
        <>
          <Button id="cancel-session" variant="ghost" disabled={creating} onClick={card.closeForm}>
            Cancel
          </Button>
          <Button id="create-session" type="submit" variant="primary" busy={creating} disabled={creating}>
            Open the sign-in tab
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
      <Field label="Sign-in address">
        <TextInput
          id="session-url"
          aria-label="Sign-in address"
          placeholder="Sign-in URL, https://..."
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
        <FormError id="session-create-error" error={card.createError} title="Could not add the site" />
      )}
    </Dialog>
  );
}

/** Settings > Browser: the JobPilot extension in the user's own Chrome, and the sites they are
 *  signed in to there (used to apply and to read sources behind a sign-in). */
export function SessionsSection() {
  const card = useSessions();
  const data = card.query.data;
  const now = new Date();
  const tag = data ? connectionTag(data.browser) : undefined;
  useErrorToast(card.error, { title: card.errorTitle, clear: card.clearError });

  return (
    <Section
      id="settings-browser"
      title="Browser"
      hint="JobPilot opens and fills tabs in your own Chrome through the JobPilot extension. It never reads your other tabs or your cookies."
      actions={tag && <Tag tone={tag.tone}>{tag.text}</Tag>}
    >
      {!data && <LoadError error={card.query.error} title="Could not read the browser" onRetry={card.query.reload} />}
      {data && (
        <>
          <Connection browser={data.browser} card={card} />
          <div role="group" aria-label="Sites you're signed in to" className="flex flex-col gap-1 pt-2">
            <SectionLabel>Sites you're signed in to</SectionLabel>
            <Caption>Applications and sources that need a sign-in use these sites in your browser.</Caption>
            {data.sessions.length > 0 && (
              <ul aria-label="Sites" className="flex flex-col">
                {data.sessions.map((session) => (
                  <SessionRow
                    key={session.id}
                    session={session}
                    card={card}
                    now={now}
                    connected={data.browser.connected}
                  />
                ))}
              </ul>
            )}
            {data.sessions.length === 0 && <Caption>No sites yet</Caption>}
            <div className="pt-1">
              <Button id="new-session" onClick={card.openForm}>
                Add a site
              </Button>
            </div>
          </div>
        </>
      )}
      <NewSessionDialog card={card} />
      <PairingDialog browser={data?.browser} card={card} />
      <ForgetDialog card={card} />
    </Section>
  );
}
