import { Button, Caption, Combobox, Dialog, Field, FormMessage, Segmented, TextInput } from "@/components/ui/kit";
import { FormError } from "@/components/ProblemHelp";

import { accountLine, deleteLabel, fieldsOf, keyIsSaved, keyLabel, modelsHint } from "@/lib/integrations/providers";
import type { KindDefaults } from "@/types/integrations";
import type { ProvidersCard } from "@/hooks/integrations/useProviders";

const CODEX_HINT = "Unofficial: uses your ChatGPT plan through the Codex backend. It may stop working without notice.";

/** The dialog that adds a provider (`id == 0`) or edits one. The key box is a password field: a
 *  saved key is never shown, only "saved". The model box suggests the provider's models, listed
 *  on opening and again after the address or key is edited (on leaving the box). Which fields a
 *  kind has comes from the shell: a base URL for the compatible kinds, a key for all but Codex,
 *  which signs in with ChatGPT instead. */
export function ProviderDialog({ card }: { card: ProvidersCard }) {
  const { draft, runner, kinds } = card;
  const idle = runner.busy === undefined;
  const kindChoices = kinds.map((k: KindDefaults) => ({ value: k.kind, label: k.label }));
  const fields = draft && fieldsOf(kinds, draft.kind);
  const fixedAddress = draft && kinds.find((k) => k.kind === draft.kind)?.baseUrl;

  return (
    <Dialog
      id="provider-dialog"
      open={draft !== undefined}
      busy={runner.busy === "saving"}
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
      {draft && fields && (
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
          {fields.needsBaseUrl ? (
            <Field label="Base URL">
              <TextInput
                id="provider-base-url"
                aria-label="Base URL"
                placeholder="Base URL, e.g. http://localhost:11434/v1"
                value={draft.baseUrl}
                onChange={(e) => card.edit({ baseUrl: e.target.value })}
                onBlur={card.listModels}
              />
            </Field>
          ) : (
            fixedAddress && <Caption>Address: {fixedAddress}</Caption>
          )}
          {fields.hasKeyField && (
            <Field
              label={keyLabel(draft)}
              hint={keyIsSaved(draft) ? "A key is saved in your system keyring." : undefined}
            >
              <TextInput
                id="provider-api-key"
                aria-label={keyLabel(draft)}
                type="password"
                autoComplete="off"
                placeholder={fields.needsBaseUrl ? "API key (optional for local servers)" : "API key"}
                value={draft.apiKey}
                onChange={(e) => card.edit({ apiKey: e.target.value })}
                onBlur={card.listModels}
              />
            </Field>
          )}
          {fields.signIn && (
            <div id="codex-account" className="flex flex-col gap-2">
              <span className="text-small font-medium text-muted">ChatGPT account</span>
              <span className="text-body text-ink">{accountLine(draft.account)}</span>
              <div className="flex flex-wrap items-center gap-2">
                <Button id="codex-sign-in" busy={runner.busy === "signing-in"} disabled={!idle} onClick={card.signIn}>
                  Sign in with ChatGPT
                </Button>
                {runner.busy === "signing-in" && (
                  <Button id="codex-cancel-sign-in" variant="ghost" onClick={card.cancelSignIn}>
                    Cancel sign-in
                  </Button>
                )}
                {card.cliAvailable && (
                  <Button
                    id="codex-import-cli"
                    busy={runner.busy === "importing"}
                    disabled={!idle}
                    onClick={card.importCli}
                  >
                    Use my Codex CLI login
                  </Button>
                )}
              </div>
              <Caption>{CODEX_HINT}</Caption>
            </div>
          )}
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
          {runner.notice && <FormMessage tone="ok">{runner.notice}</FormMessage>}
          {runner.error && <FormError id="providers-error" error={runner.error} title={runner.errorTitle} />}
        </>
      )}
    </Dialog>
  );
}
