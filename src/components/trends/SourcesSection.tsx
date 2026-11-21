import { Plus } from "lucide-react";

import { Button, Caption, Dialog, Field, FormMessage, Section, Switch, Tag, TextInput } from "@/components/ui/kit";

import type { Source, SourceKind, SourceKindInfo } from "@/types/trends";
import { useErrorToast } from "@/hooks/useErrorToast";
import { useSourceEditing } from "@/hooks/trends/useSourceEditing";

const ADD_ID: Record<SourceKind, string> = {
  hackernews: "trends-add-hn",
  yc: "trends-add-yc",
  rss: "trends-add-rss",
  page: "trends-add-page",
  person: "trends-add-person",
};

/** Where the digest looks: the sources (enable, remove) and the buttons that add one (a feed or a
 *  page in a dialog that asks for its address, a person to follow in one that asks for their name). */
export function SourcesSection({ sources, kinds }: { sources: readonly Source[]; kinds: readonly SourceKindInfo[] }) {
  const editing = useSourceEditing();
  const open = kinds.find((k) => k.kind === editing.adding);
  const person = open?.needsName === true;
  useErrorToast(editing.listError, { title: editing.listErrorTitle, clear: editing.clearListError });
  return (
    <Section
      id="trends-sources"
      title="Sources"
      hint="Where the digest looks. Feeds and pages are read-only; a site that wants a login or captcha pauses the run and waits for you. Follow a person to read what they say in interviews and talks: the report quotes them and links each conversation."
    >
      {sources.length === 0 && (
        <Caption>No sources yet: add Hacker News, the YC companies, a feed, a page or a person to follow.</Caption>
      )}
      <ul className="flex flex-col">
        {sources.map((source) => (
          <li key={source.id} className="flex items-center gap-3 border-b border-border py-2">
            <Switch
              id={`trend-source-enabled-${source.id}`}
              checked={source.enabled}
              ariaLabel={`Collect from ${source.name}`}
              disabled={editing.busy}
              onChange={(enabled) => void editing.setEnabled(source.id, enabled)}
            />
            <div className="flex min-w-0 flex-1 flex-col gap-0.5">
              <div className="flex flex-wrap items-center gap-x-2">
                <span className="selectable min-w-0 truncate text-body font-semibold text-ink">{source.name}</span>
                <Tag tone={source.tag.tone}>{source.tag.label}</Tag>
              </div>
              <Caption>{source.description}</Caption>
              <Caption>{source.statusText}</Caption>
            </div>
            <Button
              id={`trends-remove-source-${source.id}`}
              variant="danger"
              size="compact"
              aria-label={`Remove ${source.name}`}
              disabled={editing.busy}
              onClick={() => void editing.remove(source.id)}
            >
              Remove
            </Button>
          </li>
        ))}
      </ul>
      <div className="flex flex-wrap gap-2">
        {kinds.map((kind) => (
          <Button
            key={kind.kind}
            id={ADD_ID[kind.kind]}
            size="compact"
            icon={Plus}
            aria-haspopup={kind.needsUrl || kind.needsName ? "dialog" : undefined}
            disabled={!kind.needsUrl && !kind.needsName && (kind.added || editing.busy)}
            onClick={() => void editing.choose(kind)}
          >
            {kind.label}
          </Button>
        ))}
      </div>
      <Dialog
        id="trends-source-dialog"
        open={open !== undefined}
        busy={editing.busy}
        title={open ? (open.needsName ? "Follow a person" : `Add ${open.label}`) : "Add a source"}
        onClose={editing.cancel}
        onSubmit={() => open && !editing.busy && void editing.submit(open)}
        footer={
          <>
            <Button id="trends-add-cancel" variant="ghost" onClick={editing.cancel}>
              Cancel
            </Button>
            <Button id="trends-add-source" type="submit" variant="primary" busy={editing.busy}>
              {open?.needsName ? "Follow" : "Add"}
            </Button>
          </>
        }
      >
        <Field label={person ? "Name" : "Name (optional)"}>
          <TextInput
            id="trends-source-name"
            placeholder={person ? "Jensen Huang" : "Name (optional)"}
            value={editing.name}
            onChange={(e) => editing.setName(e.target.value)}
          />
        </Field>
        <Field label={person ? "Feed or page of their interviews (optional)" : "Address"}>
          <TextInput
            id="trends-source-url"
            placeholder={person ? "https://example.com/podcast.xml" : "https://example.com/feed.xml"}
            inputMode="url"
            value={editing.url}
            onChange={(e) => editing.setUrl(e.target.value)}
          />
        </Field>
        {person && (
          <Caption>
            Each run looks for Hacker News stories that name this person and reads the interview, talk or article they
            link. A podcast feed or a transcript page of their own is read too. The report quotes what they say and
            links the conversation; it is their opinion, never counted in the market numbers.
          </Caption>
        )}
        {editing.formError && <FormMessage id="trends-source-error">{editing.formError}</FormMessage>}
      </Dialog>
    </Section>
  );
}
