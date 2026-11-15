import { Button, Checkbox, Combobox, Dialog, Field, FormMessage, Segmented, TextInput } from "@/components/ui/kit";
import { FormError } from "@/components/ProblemHelp";

import { deleteLabel, keyLabel, modelsHint } from "@/lib/integrations/providers";
import type { KindDefaults } from "@/types/integrations";
import type { ProvidersCard } from "@/hooks/integrations/useProviders";

/** The dialog that adds a provider (`id == 0`) or edits one. The key box is a password field: a
 *  saved key is never shown, only "saved". The model box suggests the provider's models, listed
 *  on their own from the address and key. */
export function ProviderDialog({ card }: { card: ProvidersCard }) {
  const { draft, runner, kinds } = card;
  const idle = runner.busy === undefined;
  const kindChoices = kinds.map((k: KindDefaults) => ({ value: k.kind, label: k.label }));

  return (
    <Dialog
      id="provider-dialog"
      open={draft !== undefined}
      title={draft && draft.id !== 0 ? `Edit ${draft.name || "provider"}` : "Add provider"}
      onClose={card.close}
      onSubmit={() => idle && card.save()}
      footer={
        draft && (
          <>
            {draft.id !== 0 && (
              <Button id="delete-provider" variant="danger" disabled={!idle} onClick={card.remove} className="mr-auto">
                {deleteLabel(draft)}
              </Button>
            )}
            <Button id="test-provider" busy={runner.busy === "testing"} disabled={!idle} onClick={card.test}>
              Test
            </Button>
            <Button id="close-provider" variant="ghost" onClick={card.close}>
              Cancel
            </Button>
            <Button id="save-provider" type="submit" variant="primary" busy={runner.busy === "saving"} disabled={!idle}>
              Save
            </Button>
          </>
        )
      }
    >
      {draft && (
        <>
          <div className="flex flex-col gap-1.5">
            <span className="text-small font-medium text-muted">Type</span>
            <div id="provider-kind">
              <Segmented
                label="Type"
                choices={kindChoices}
                value={draft.kind}
                onChange={card.setKind}
                disabled={!idle}
              />
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
          <Field label="Model" hint={modelsHint(card.modelsLoading, draft.models.length)}>
            <Combobox
              id="provider-model"
              label="Model"
              placeholder="Model id, e.g. gpt-4o-mini"
              value={draft.model}
              options={draft.models}
              loading={card.modelsLoading}
              onChange={(model) => card.edit({ model })}
            />
          </Field>
          {draft.kind === "openai" && (
            <Checkbox
              id="json-object-mode"
              label="Endpoint lacks json_schema support: use json_object mode"
              checked={draft.jsonObject}
              onChange={(e) => card.edit({ jsonObject: e.target.checked })}
            />
          )}
          {runner.notice && <FormMessage tone="ok">{runner.notice}</FormMessage>}
          {runner.error && (
            <FormError id="providers-error" error={runner.error} title={runner.errorTitle} onRetry={runner.retry} />
          )}
        </>
      )}
    </Dialog>
  );
}
