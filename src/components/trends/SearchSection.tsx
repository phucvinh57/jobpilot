import { Eye, EyeOff } from "lucide-react";

import { Button, Field, FormMessage, Section, TextInput } from "@/components/ui/kit";

import type { SearchKey } from "@/types/trends";
import { useSearchKeys } from "@/hooks/trends/useSearchKeys";

/** The search API keys that switch report search on. Write-only: the screen knows whether a key
 *  is saved, never the key; a typed one goes to the OS keyring and the field is cleared. */
export function SearchSection({ keys }: { keys: readonly SearchKey[] }) {
  const editing = useSearchKeys();
  const on = keys.some((k) => k.saved);
  return (
    <Section
      id="trends-search"
      title="Report search"
      hint="With a Brave Search or Tavily API key, the digest also reads annual and quarterly reports found for your domains and roles. The key is kept in the OS keyring, never in a file."
    >
      <p className="text-body text-muted">{on ? "Report search is on." : "Report search is off: no key saved."}</p>
      {keys.map((key, i) => {
        const shown = editing.isShown(key.provider);
        return (
          <div key={key.provider} className="flex flex-wrap items-end gap-2">
            <div className="min-w-[260px] flex-1">
              <Field label={`${key.label} · ${key.saved ? "key saved" : "no key"}`}>
                <TextInput
                  type={shown ? "text" : "password"}
                  placeholder={`${key.label} API key`}
                  value={editing.valueOf(key.provider)}
                  onChange={(e) => editing.type(key.provider, e.target.value)}
                />
              </Field>
            </div>
            <Button
              variant="ghost"
              icon={shown ? EyeOff : Eye}
              aria-label={`${shown ? "Hide" : "Show"} the ${key.label} API key`}
              onClick={() => editing.toggleShown(key.provider)}
            />
            <Button
              id={`trends-key-save-${i}`}
              aria-label={`Save the ${key.label} API key`}
              disabled={editing.busy}
              onClick={() => void editing.save(key)}
            >
              {key.saved ? "Replace key" : "Save key"}
            </Button>
            {key.saved && (
              <Button
                id={`trends-key-remove-${i}`}
                variant="danger"
                aria-label={`Remove the ${key.label} API key`}
                disabled={editing.busy}
                onClick={() => void editing.clear(key)}
              >
                Remove key
              </Button>
            )}
          </div>
        );
      })}
      {/* The key rows are a form that is always there: its error stays under it. */}
      {editing.error && <FormMessage id="trends-key-error">{editing.error}</FormMessage>}
    </Section>
  );
}
