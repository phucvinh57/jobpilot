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
};

/** Where the digest looks: the sources (enable, remove) and the buttons that add one (a feed or a
 *  page in a dialog that asks for its address). */
export function SourcesSection({ sources, kinds }: { sources: readonly Source[]; kinds: readonly SourceKindInfo[] }) {
  const editing = useSourceEditing();
  const open = kinds.find((k) => k.kind === editing.adding);
  useErrorToast(editing.listError, { title: editing.listErrorTitle, clear: editing.clearListError });
  return (
    <Section
      id="trends-sources"
      title="Sources"
      hint="Where the digest looks. Feeds and pages are read-only; a site that wants a login or captcha pauses the run and waits for you."
    >
      {sources.length === 0 && <Caption>No sources yet: add Hacker News, the YC companies, a feed or a page.</Caption>}
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
            aria-haspopup={kind.needsUrl ? "dialog" : undefined}
            disabled={!kind.needsUrl && (kind.added || editing.busy)}
            onClick={() => void editing.choose(kind)}
          >
            {kind.label}
          </Button>
        ))}
      </div>
      <Dialog
        id="trends-source-dialog"
        open={open !== undefined}
        title={open ? `Add ${open.label}` : "Add a source"}
        onClose={editing.cancel}
        onSubmit={() => open && !editing.busy && void editing.submit(open)}
        footer={
          <>
            <Button id="trends-add-cancel" variant="ghost" onClick={editing.cancel}>
              Cancel
            </Button>
            <Button id="trends-add-source" type="submit" variant="primary" busy={editing.busy}>
              Add
            </Button>
          </>
        }
      >
        <Field label="Name (optional)">
          <TextInput
            id="trends-source-name"
            placeholder="Name (optional)"
            value={editing.name}
            onChange={(e) => editing.setName(e.target.value)}
          />
        </Field>
        <Field label="Address">
          <TextInput
            id="trends-source-url"
            placeholder="https://example.com/feed.xml"
            inputMode="url"
            value={editing.url}
            onChange={(e) => editing.setUrl(e.target.value)}
          />
        </Field>
        {editing.formError && <FormMessage id="trends-source-error">{editing.formError}</FormMessage>}
      </Dialog>
    </Section>
  );
}
