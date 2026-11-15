import { Button, Checkbox, ErrorBanner, Field, Segmented, TextInput } from "@/shared";

import { deleteLabel, keyLabel, type KindDefaults } from "../domain/providers";
import type { ProvidersCard } from "../hooks/useProviders";
import { Notice } from "./Notice";

/** The form of one provider (a new one has `id == 0`). The key box is a password field that is
 *  emptied once the key is saved: a saved key is never shown, only "saved". */
export function ProviderForm({ card }: { card: ProvidersCard }) {
  const { draft, runner, kinds } = card;
  if (!draft) {
    return null;
  }
  const idle = runner.busy === undefined;
  const kindChoices = kinds.map((k: KindDefaults) => ({ value: k.kind, label: k.label }));
  const picked = draft.models.includes(draft.model) ? draft.model : "";

  return (
    <div className="flex flex-col gap-3 pt-2">
      <div className="flex flex-col gap-1">
        <span className="text-small font-medium text-muted">Type</span>
        <div id="provider-kind">
          <Segmented label="Type" choices={kindChoices} value={draft.kind} onChange={card.setKind} disabled={!idle} />
        </div>
      </div>
      <Field label="Name">
        <TextInput
          id="provider-name"
          aria-label="Name"
          placeholder="Name"
          value={draft.name}
          onChange={(e) => card.edit({ name: e.target.value })}
        />
      </Field>
      <Field label="Base URL">
        <TextInput
          id="provider-base-url"
          aria-label="Base URL"
          placeholder="Base URL"
          value={draft.baseUrl}
          onChange={(e) => card.edit({ baseUrl: e.target.value })}
        />
      </Field>
      <Field label={keyLabel(draft)} hint={draft.hasKey ? "A key is saved in your system keyring." : undefined}>
        <TextInput
          id="provider-api-key"
          aria-label={keyLabel(draft)}
          type="password"
          autoComplete="off"
          placeholder="API key (optional for local servers)"
          value={draft.apiKey}
          onChange={(e) => card.edit({ apiKey: e.target.value })}
        />
      </Field>
      <Field label="Model">
        <div className="flex items-center gap-2">
          <div className="min-w-0 flex-1">
            <TextInput
              id="provider-model"
              aria-label="Model"
              placeholder="Model id, e.g. gpt-4o-mini"
              value={draft.model}
              onChange={(e) => card.edit({ model: e.target.value })}
            />
          </div>
          <Button id="load-models" busy={runner.busy === "models"} disabled={!idle} onClick={card.loadModels}>
            Load models
          </Button>
        </div>
        {draft.models.length > 0 && (
          <select
            id="provider-model-picker"
            aria-label="Pick from the loaded models"
            value={picked}
            onChange={(e) => card.edit({ model: e.target.value })}
            className="min-h-10 w-full border border-control-border bg-surface px-3 text-body text-ink focus:border-accent focus:outline-none"
          >
            <option value="" disabled>
              Pick from the loaded models
            </option>
            {draft.models.map((model) => (
              <option key={model} value={model}>
                {model}
              </option>
            ))}
          </select>
        )}
      </Field>
      {draft.kind === "openai" && (
        <Checkbox
          id="json-object-mode"
          label="Endpoint lacks json_schema support: use json_object mode"
          checked={draft.jsonObject}
          onChange={(e) => card.edit({ jsonObject: e.target.checked })}
        />
      )}
      <div className="flex flex-wrap gap-2">
        <Button id="save-provider" busy={runner.busy === "saving"} disabled={!idle} onClick={card.save}>
          Save
        </Button>
        <Button id="test-provider" busy={runner.busy === "testing"} disabled={!idle} onClick={card.test}>
          Test
        </Button>
        {draft.id !== 0 && !draft.isDefault && (
          <Button id="default-provider" disabled={!idle} onClick={card.makeDefault}>
            Make default
          </Button>
        )}
        <Button id="delete-provider" variant="danger" disabled={!idle} onClick={card.remove}>
          {deleteLabel(draft)}
        </Button>
        <Button id="close-provider" variant="ghost" onClick={card.close}>
          Close
        </Button>
      </div>
      {runner.notice && <Notice text={runner.notice} />}
      {runner.error && (
        <ErrorBanner
          id="providers-error"
          error={runner.error}
          title={runner.errorTitle}
          onRetry={runner.retry}
          onDismiss={runner.reset}
        />
      )}
    </div>
  );
}
