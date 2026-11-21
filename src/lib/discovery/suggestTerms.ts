// Settings > Job sources, "What to search for": which preference terms a "Suggest sources" search
// uses, picked value by value, and how many searches that makes.

import type { SuggestTerms } from "@/types/discovery";

/** A list of terms the user picks values from. */
export type TermKind = "roles" | "domains" | "skills" | "places";

/** Searches the shell runs for one search at most (`engine::discover::suggest`). */
export const MAX_SEARCHES = 8;
/** Query forms per term and place ("<term> jobs", "careers", ...). */
const FORMS_PER_TERM = 4;
/** Places combined with each term at most (Remote counts as one). */
const MAX_PLACES = 2;

/** `terms` with `value` added to `kind`, or removed when it is there (a new object). */
export function toggleTerm(terms: SuggestTerms, kind: TermKind, value: string): SuggestTerms {
  const list = terms[kind];
  const next = list.includes(value) ? list.filter((v) => v !== value) : [...list, value];
  return { ...terms, [kind]: next };
}

/** Whether `value` of `kind` is picked. */
export function isPicked(terms: SuggestTerms, kind: TermKind, value: string): boolean {
  return terms[kind].includes(value);
}

/** The picked terms that are searched on their own: roles, domains and skills (places only combine). */
export function pickedCount(terms: SuggestTerms): number {
  return terms.roles.length + terms.domains.length + terms.skills.length;
}

/** "Search" needs at least one role, domain or skill. */
export function canSearch(terms: SuggestTerms): boolean {
  return pickedCount(terms) > 0;
}

/** How many terms are picked and how many searches that makes, under the dialog's lists. */
export function searchNote(terms: SuggestTerms): string {
  const count = pickedCount(terms);
  if (count === 0) {
    return "Pick at least one role, domain or skill.";
  }
  const places = Math.max(1, Math.min(MAX_PLACES, terms.places.length + (terms.remote ? 1 : 0)));
  const total = count * FORMS_PER_TERM * places;
  const picked = count === 1 ? "1 term picked" : `${count} terms picked`;
  if (total > MAX_SEARCHES) {
    return `${picked}: up to ${MAX_SEARCHES} searches, shared between them.`;
  }
  return `${picked}: up to ${total} searches.`;
}
