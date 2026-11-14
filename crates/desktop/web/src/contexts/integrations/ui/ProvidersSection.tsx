import clsx from "clsx";

import { Button, Caption, ErrorBanner, Section, Tag } from "@/shared";

import { providersStatus, type Provider } from "../domain/providers";
import { useProviders } from "../hooks/useProviders";
import { ProviderForm } from "./ProviderForm";

const HINT =
  "OpenAI- or Anthropic-compatible endpoints; API keys are kept in the OS keyring. The provider tagged default is used for generation.";

function ProviderRow({ provider, editing, onOpen }: { provider: Provider; editing: boolean; onOpen: () => void }) {
  return (
    <button
      type="button"
      id={`provider-${provider.id}`}
      aria-pressed={editing}
      onClick={onOpen}
      className={clsx(
        "flex min-h-11 w-full cursor-pointer flex-wrap items-center gap-3 rounded-button px-3 py-2 text-left hover:bg-track",
        editing && "bg-accent-soft hover:bg-accent-soft",
      )}
    >
      <span className="text-body font-semibold text-ink">{provider.name}</span>
      {provider.isDefault && <Tag tone="ok">default</Tag>}
      <span className="text-small text-muted">{provider.kindLabel}</span>
      <span className="min-w-0 flex-1 truncate text-small text-muted">{provider.model}</span>
    </button>
  );
}

/** Settings > AI providers: add and edit providers, test them, choose the default. */
export function ProvidersSection() {
  const card = useProviders();
  const data = card.query.data;
  const status = providersStatus(data?.providers ?? []);

  return (
    <Section id="settings-providers" title="AI providers" hint={HINT} actions={data && <Tag tone={status.tone}>{status.text}</Tag>}>
      {card.query.error && !data && <ErrorBanner error={card.query.error} title="Could not read the providers" onRetry={card.query.reload} />}
      {data && data.providers.length === 0 && <Caption>No provider yet. Add one to generate CVs.</Caption>}
      {data && data.providers.length > 0 && (
        <div className="flex flex-col gap-1">
          {data.providers.map((provider) => (
            <ProviderRow key={provider.id} provider={provider} editing={card.draft?.id === provider.id} onOpen={() => card.open(provider)} />
          ))}
        </div>
      )}
      {data && !card.draft && (
        <div>
          <Button id="add-provider" onClick={() => card.open(undefined)}>
            Add provider
          </Button>
        </div>
      )}
      <ProviderForm card={card} />
    </Section>
  );
}
