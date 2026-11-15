import { Button, Caption, Section, Tag } from "@/components/ui/kit";
import { LoadError } from "@/components/ProblemHelp";

import { providersStatus } from "@/lib/integrations/providers";
import type { Provider } from "@/types/integrations";
import { useErrorToast } from "@/hooks/useErrorToast";
import { useProviders, type ProvidersCard } from "@/hooks/integrations/useProviders";
import { ProviderDialog } from "./ProviderDialog";

const HINT =
  "OpenAI- or Anthropic-compatible endpoints; API keys are kept in the OS keyring. The provider tagged default is used for generation.";

/** A provider: a click opens its dialog; one that is not the default can become it. */
function ProviderRow({ provider, card }: { provider: Provider; card: ProvidersCard }) {
  return (
    <li className="flex min-h-11 items-center gap-2 border-b border-border last:border-b-0">
      <button
        type="button"
        id={`provider-${provider.id}`}
        aria-label={`Edit ${provider.name}`}
        onClick={() => card.open(provider)}
        className="flex min-w-0 flex-1 cursor-pointer flex-wrap items-center gap-3 px-3 py-2 text-left hover:bg-track"
      >
        <span className="text-body font-semibold text-ink">{provider.name}</span>
        {provider.isDefault && <Tag tone="ok">default</Tag>}
        <span className="text-small text-muted">{provider.kindLabel}</span>
        <span className="min-w-0 flex-1 truncate text-small text-muted">{provider.model}</span>
      </button>
      {!provider.isDefault && (
        <Button
          id={`default-provider-${provider.id}`}
          size="compact"
          variant="ghost"
          aria-label={`Make ${provider.name} the default`}
          disabled={card.makeDefault.busy}
          onClick={() => void card.makeDefault.run(provider)}
        >
          Make default
        </Button>
      )}
    </li>
  );
}

/** Settings > AI providers: the providers, a dialog to add or edit one, and the default. */
export function ProvidersSection() {
  const card = useProviders();
  const data = card.query.data;
  const status = providersStatus(data?.providers ?? []);
  useErrorToast(card.makeDefault.error, {
    title: "Could not make it the default",
    clear: card.makeDefault.clearError,
  });

  return (
    <Section
      id="settings-providers"
      title="AI providers"
      hint={HINT}
      actions={
        data && (
          <div className="flex items-center gap-2">
            <Tag tone={status.tone}>{status.text}</Tag>
            <Button id="add-provider" size="compact" onClick={() => card.open(undefined)}>
              Add provider
            </Button>
          </div>
        )
      }
    >
      {!data && <LoadError error={card.query.error} title="Could not read the providers" onRetry={card.query.reload} />}
      {data && data.providers.length === 0 && <Caption>No provider yet. Add one to generate CVs.</Caption>}
      {data && data.providers.length > 0 && (
        <ul aria-label="Providers" className="flex flex-col border border-border">
          {data.providers.map((provider) => (
            <ProviderRow key={provider.id} provider={provider} card={card} />
          ))}
        </ul>
      )}
      <ProviderDialog card={card} />
    </Section>
  );
}
