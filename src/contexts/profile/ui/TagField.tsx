import { Plus, X } from "lucide-react";
import { useState } from "react";

import { Button, FilterChip, TextInput } from "@/shared";

import type { ListField } from "../domain/profile";
import { Row } from "./Row";

/** A list of the preferences as a tag field: chips with a remove button, a small "Add" box, and
 *  suggestion chips. Adding and removing go through the shell, which cleans and compares entries. */
export function TagField({ field, onAdd, onRemove }: { field: ListField; onAdd: (text: string) => void; onRemove: (item: string) => void }) {
  const [typed, setTyped] = useState("");
  const { key, label } = field;

  const add = () => {
    if (typed.trim() === "") {
      return;
    }
    const text = typed;
    setTyped("");
    onAdd(text);
  };

  return (
    <Row label={label}>
      <div className="flex flex-col gap-2">
        {field.items.length > 0 && (
          <ul aria-label={label} className="flex flex-wrap gap-1.5">
            {field.items.map((item, index) => (
              <li
                key={item}
                id={`profile-${key}-${index}`}
                className="inline-flex min-h-7 items-center gap-0.5 bg-accent-soft pl-2.5 pr-0.5 text-small text-accent-dark"
              >
                {item}
                <button
                  type="button"
                  id={`profile-${key}-remove-${index}`}
                  aria-label={`Remove ${item}`}
                  title={`Remove ${item}`}
                  onClick={() => onRemove(item)}
                  className="inline-flex size-6 cursor-pointer items-center justify-center hover:bg-accent/25"
                >
                  <X aria-hidden className="size-3.5" />
                </button>
              </li>
            ))}
          </ul>
        )}
        <form
          className="flex items-center gap-2"
          onSubmit={(e) => {
            e.preventDefault();
            add();
          }}
        >
          <TextInput
            id={`profile-add-${key}`}
            aria-label={`Add to ${label}`}
            className="w-[260px]"
            placeholder={field.placeholder}
            value={typed}
            onChange={(e) => setTyped(e.target.value)}
          />
          <Button id={`profile-${key}-add`} type="submit" aria-label={`Add to ${label}`}>
            Add
          </Button>
        </form>
        {field.suggestions.length > 0 && (
          <div role="group" aria-label={`Suggestions for ${label}`} className="flex flex-wrap items-center gap-1">
            <span className="pr-1 text-small text-muted">Suggestions</span>
            {field.suggestions.map((suggestion, index) => (
              <FilterChip
                key={suggestion}
                id={`profile-${key}-suggest-${index}`}
                selected={false}
                aria-label={`Add ${suggestion} to ${label}`}
                onClick={() => onAdd(suggestion)}
              >
                <Plus aria-hidden className="size-3.5" />
                {suggestion}
              </FilterChip>
            ))}
          </div>
        )}
      </div>
    </Row>
  );
}
