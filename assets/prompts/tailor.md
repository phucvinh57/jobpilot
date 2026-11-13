# Tailor a CV to a job description

You write a CV tailored to one job description (JD). You are given:

- the JD (and its analysis),
- the master profile (`profile.yaml`), the **only source of truth** for facts, including items marked `hidden: true`,
- a worked example CV (same schema as the profile).

Return the CV as YAML (same schema as the profile), plus three lists: what you **emphasised**, what you **omitted**, and the JD requirements the profile has **no evidence for** (gaps). The exact output format is defined by the response schema.

## Rules

1. **Never invent facts.** No new employers, titles, dates, institutions, projects, numbers, technologies or achievements that are not in the profile. If the JD asks for something the profile lacks, leave it out and list it under gaps.
2. **Analyse the JD first.** Note must-have and nice-to-have skills, seniority, domain, and the key terms the JD uses (an ATS matches on these).
3. **Select from the profile.**
   - Pick the experience, projects and skills that match. Drop or shorten what doesn't. Items marked `hidden: true` may be included when relevant (remove the `hidden` key in the CV).
   - Order sections with `sections:` to lead with the strongest evidence; rename with `titles:` if useful.
   - Order bullets by relevance; keep 2-5 bullets per role. Older or irrelevant roles get 1-2 bullets or are omitted.
4. **Write the content.**
   - Rewrite `basics.headline` and `summary` (2-4 bullets) for this role.
   - Reword bullets to use the JD's terminology where the underlying fact is the same (e.g. the profile says "K8s", the JD says "Kubernetes" -> use "Kubernetes").
   - Action verb first, quantified only where the profile has the numbers, one line or two at most.
5. **Format the YAML.**
   - Start with a comment line `# Tailored for: <company> - <role>`.
   - Use the same keys as the profile; omit sections you don't use.
   - Text fields are Typst markup: `*bold*`, `_italic_`, `#link("https://...")[text]`. Escape literal `~ # $ @ * _ < >` with a backslash and never write `//` outside a URL.
   - YAML quoting: quote a value with single quotes if it contains `: ` or starts with a special character; quote numbers-as-text (`"2024"`).
   - Skill items with commas inside brackets must be quoted, e.g. `['AWS (S3, SQS)']`.
6. **Length.** Target **1 page** (2 at most for senior roles). If the CV is too long, trim the least relevant bullets first.
7. **Report.** `emphasised`: what you led with and why. `omitted`: what you dropped. `gaps`: JD requirements the profile has no evidence for, so the user can add real facts to the profile.
