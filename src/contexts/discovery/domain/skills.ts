// The skills a posting asks for, split by whether the profile has them. A false "not in your
// profile" costs a glance; a false match would be a claim, so names match only when they are the
// same once normalised.

/** The skills of a posting, split by whether the profile has them. */
export interface SkillSplit {
  /** Asked for and in the profile. */
  matched: string[];
  /** Asked for and not in the profile. */
  gaps: string[];
}

/** `name` without a closing `( ... )` group at its end (nested groups included), unless that would
 *  leave nothing. */
function withoutTrailingParenthetical(name: string): string {
  if (!name.endsWith(")")) {
    return name;
  }
  const inner = name.slice(0, -1);
  let depth = 1;
  for (let at = inner.length - 1; at >= 0; at -= 1) {
    const c = inner[at];
    if (c === ")") {
      depth += 1;
    } else if (c === "(") {
      depth -= 1;
      if (depth === 0) {
        const head = inner.slice(0, at).trimEnd();
        return head === "" ? name : head;
      }
    }
  }
  return name;
}

/** A skill name for comparing: lower case, spaces collapsed, a trailing parenthetical dropped
 *  (`Kubernetes (EKS)` -> `kubernetes`). `/` and `&` stay: `CI/CD` is one skill, not `CI` and `CD`. */
export function normalizeSkill(skill: string): string {
  const name = skill.toLowerCase().split(/\s+/).filter(Boolean).join(" ");
  return withoutTrailingParenthetical(name);
}

/** The same skill: equal names once normalised. Nothing looser: `Java` is not `Java Spring` or
 *  `JavaScript`, `Cloud` is not `Google Cloud`, `C` is not `C++`, `CI/CD` is not `CI`. */
export function sameSkill(a: string, b: string): boolean {
  const [x, y] = [normalizeSkill(a), normalizeSkill(b)];
  return x !== "" && x === y;
}

/** The posting's skills (required first, then preferred, each once) split by the profile's. */
export function splitSkills(wanted: { required: readonly string[]; preferred: readonly string[] }, profile: readonly string[]): SkillSplit {
  const seen = new Set<string>();
  const split: SkillSplit = { matched: [], gaps: [] };
  for (const skill of [...wanted.required, ...wanted.preferred]) {
    const key = normalizeSkill(skill);
    if (key === "" || seen.has(key)) {
      continue;
    }
    seen.add(key);
    const name = skill.trim();
    if (profile.some((have) => sameSkill(name, have))) {
      split.matched.push(name);
    } else {
      split.gaps.push(name);
    }
  }
  return split;
}

/** `Rust, Go` or `none`. */
export function listOrNone(items: readonly string[]): string {
  return items.length === 0 ? "none" : items.join(", ");
}
