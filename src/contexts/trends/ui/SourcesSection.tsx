import { Plus } from "lucide-react";

import { Button, Caption, Field, FilterChip, Section, Switch, Tag, TextInput } from "@/shared";

import type { Source, SourceKind, SourceKindInfo } from "../domain/types";
import { useSourceEditing } from "../hooks";

const ADD_ID: Record<SourceKind, string> = {
  hackernews: "trends-add-hn",
  yc: "trends-add-yc",
  rss: "trends-add-rss",
  page: "trends-add-page",
};

/** Where the digest looks: the sources (enable, remove) and the buttons that add one. */
export function SourcesSection({ sources, kinds }: { sources: readonly Source[]; kinds: readonly SourceKindInfo[] }) {
  const editing = useSourceEditing();
  const open = kinds.find((k) => k.kind === editing.adding);
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
        {kinds.map((kind) =>
          kind.needsUrl ? (
            <FilterChip
              key={kind.kind}
              id={ADD_ID[kind.kind]}
              selected={editing.adding === kind.kind}
              onClick={() => void editing.choose(kind)}
            >
              <Plus aria-hidden className="size-4" />
              {kind.label}
            </FilterChip>
          ) : (
            <Button
              key={kind.kind}
              id={ADD_ID[kind.kind]}
              size="compact"
              icon={Plus}
              disabled={kind.added || editing.busy}
              onClick={() => void editing.choose(kind)}
            >
              {kind.label}
            </Button>
          ),
        )}
      </div>
      {open && (
        <form
          className="flex flex-col gap-3"
          onSubmit={(e) => {
            e.preventDefault();
            void editing.submit(open);
          }}
        >
          <Field label="Name (optional)">
            <TextInput placeholder="Name (optional)" value={editing.name} onChange={(e) => editing.setName(e.target.value)} />
          </Field>
          <Field label="Address">
            <TextInput
              placeholder="https://example.com/feed.xml"
              inputMode="url"
              value={editing.url}
              onChange={(e) => editing.setUrl(e.target.value)}
            />
          </Field>
          <div className="flex flex-wrap gap-2">
            <Button id="trends-add-source" type="submit" busy={editing.busy}>
              Add {open.label}
            </Button>
            <Button id="trends-add-cancel" variant="ghost" onClick={editing.cancel}>
              Cancel
            </Button>
          </div>
        </form>
      )}
      {editing.error && (
        <p role="alert" className="selectable min-w-0 text-body text-danger">
          {editing.error}
        </p>
      )}
    </Section>
  );
}
