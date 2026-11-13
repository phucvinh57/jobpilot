# Review guide

For human reviewers and the `/pre-pr` skill. When a bug slips through, add a line under **Misses**.

## Failures

- **A CV states a fact that is not in `profile.yaml`:** an employer, title, date, number, technology or achievement. Important.
- **`profile.yaml` changed without a source from the user.** Add or change a fact only when the user gave it.
- **Contact details or links changed** (email, phone, location, GitHub, LinkedIn, website).
- **Typst markup is unescaped or renders wrong:** a literal `~ # $ @ * _ < >` without a backslash, `//` outside a URL, a broken link, text cut off or overflowing.
- **A CV is over 2 pages.** The target is 1.
- **A JD must-have is missing:** the profile supports it but the CV omits it, or the CV doesn't use the JD's terminology where the fact is the same.
- **Docs out of date** with the change (README, AGENTS.md, skills).

## Risk tiers

Set by the paths touched. The highest wins. Each PR gets exactly one label: `risk: low`, `risk: medium` or `risk: high`. Low and Medium PRs are merged by `/pre-pr` once `CI OK` passes. High PRs wait for a human. The required `Risk label` check (`.github/workflows/risk.yml`) enforces the label, the High paths, and no auto-merge on High. Change the list there along with this one.

- **High** (a human reads the diff): `profile.yaml`, `assets/cv.typ`, `assets/prompts/`, `scripts/`, `.githooks/`, `.github/`, `.claude/`, `.agents/`, `AGENTS.md`, `CLAUDE.md`, `docs/REVIEW.md`.
- **Low:** `README.md`, other docs.
- **No PR:** a change that touches only `docs/` (except this file) is pushed directly to `main`, and CI ignores `docs/**`.
- **Medium:** everything else (`crates/`, `Cargo.*`).

## Ignore

`target/`, `dist/` and generated PDFs.

## Reporting

- Start with the number of findings, or "No issues found."
- Cite `path:line` for each. Never guess file names.
- Label each Important, Nit, or Pre-existing.
- For a bug fix, name the root cause. Name the class of bug so every instance gets fixed.

## Rounds

At most 3 per PR; after that, question the approach. After round 1, review only new commits. Don't re-raise anything already fixed or answered.

## Misses

One line per bug that shipped: PR, what broke, which check should have caught it.
