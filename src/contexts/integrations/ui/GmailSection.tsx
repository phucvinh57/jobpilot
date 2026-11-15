import { Button, Caption, ErrorBanner, Field, HealthDot, Section, Tag, TextInput } from "@/shared";

import { disconnectLabel, lastSyncLine, pollLine, secretLabel, syncResult, type Gmail } from "../domain/gmail";
import { useGmail, type GmailCard } from "../hooks/useGmail";
import { Notice } from "./Notice";

const HINT = "Read-only access to spot confirmations, interviews and rejections for your applications.";

/** Not connected: the OAuth client of the user's own Google Cloud project, then Connect. */
function SignedOut({ gmail, card }: { gmail: Gmail; card: GmailCard }) {
  const { runner } = card;
  return (
    <div className="flex flex-col gap-3 pt-2">
      <Caption>
        Create an OAuth client of type &quot;Desktop app&quot; in the Google Cloud Console, enable the Gmail API, then
        paste its client ID and secret. The secret is kept in the OS keyring.
      </Caption>
      <Field label="Client ID">
        <TextInput
          id="gmail-client-id"
          aria-label="Client ID"
          placeholder="Client ID (ends in .apps.googleusercontent.com)"
          value={card.clientId}
          onChange={(e) => card.setClientId(e.target.value)}
        />
      </Field>
      <Field label={secretLabel(gmail.hasClientSecret)}>
        <TextInput
          id="gmail-client-secret"
          aria-label={secretLabel(gmail.hasClientSecret)}
          type="password"
          autoComplete="off"
          placeholder={gmail.hasClientSecret ? "Client secret (leave empty to keep the saved one)" : "Client secret"}
          value={card.clientSecret}
          onChange={(e) => card.setClientSecret(e.target.value)}
        />
      </Field>
      <div className="flex items-center gap-2">
        <Button id="gmail-connect" busy={card.connecting} disabled={runner.busy !== undefined} onClick={card.connect}>
          Connect Gmail
        </Button>
        {card.connecting && (
          <>
            <span className="text-small text-muted">waiting for browser&hellip;</span>
            <Button id="gmail-cancel" variant="ghost" onClick={card.cancelConnect}>
              Cancel
            </Button>
          </>
        )}
      </div>
    </div>
  );
}

/** Connected: the account, the last sync, Sync now and a two-step Disconnect. */
function SignedIn({ gmail, account, card }: { gmail: Gmail; account: string; card: GmailCard }) {
  const { runner } = card;
  const idle = runner.busy === undefined;
  const result = syncResult(gmail.lastReport);
  return (
    <div className="flex flex-col gap-3 pt-2">
      <div className="flex items-center gap-2">
        <HealthDot state="ok" />
        <p className="selectable text-body font-semibold text-ink">Connected as {account}</p>
      </div>
      <div className="flex flex-col gap-1 text-small">
        <p className="text-muted">{lastSyncLine(gmail.lastSync)}</p>
        {result.text !== "" && (
          <p className={result.ok ? "selectable text-muted" : "selectable text-danger"}>{result.text}</p>
        )}
        <p className="text-muted">{pollLine(gmail.pollMinutes)}</p>
      </div>
      <div className="flex gap-2">
        <Button id="gmail-sync" busy={runner.busy === "syncing"} disabled={!idle} onClick={card.syncNow}>
          Sync now
        </Button>
        <Button
          id="gmail-disconnect"
          variant="danger"
          busy={runner.busy === "disconnecting"}
          disabled={!idle}
          onClick={card.disconnect}
        >
          {disconnectLabel(card.confirmDisconnect)}
        </Button>
      </div>
    </div>
  );
}

/** Settings > Gmail: read-only access used to track replies. */
export function GmailSection() {
  const card = useGmail();
  const gmail = card.query.data;
  const { runner } = card;
  const account = gmail?.account ?? null;
  const connected = account !== null;

  return (
    <Section
      id="settings-gmail"
      title="Gmail"
      hint={HINT}
      actions={gmail && <Tag tone={connected ? "ok" : "neutral"}>{connected ? "Connected" : "Not connected"}</Tag>}
    >
      {card.query.error && !gmail && (
        <ErrorBanner error={card.query.error} title="Could not read the Gmail settings" onRetry={card.query.reload} />
      )}
      {gmail &&
        (account !== null ? (
          <SignedIn gmail={gmail} account={account} card={card} />
        ) : (
          <SignedOut gmail={gmail} card={card} />
        ))}
      {runner.notice && <Notice text={runner.notice} />}
      {runner.error && (
        <ErrorBanner
          id="gmail-error"
          error={runner.error}
          title={runner.errorTitle}
          onRetry={runner.retry}
          onDismiss={runner.reset}
        />
      )}
    </Section>
  );
}
