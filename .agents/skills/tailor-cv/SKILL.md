---
name: tailor-cv
description: Write a CV tailored to a job description (JD) from the master profile (profile.yaml), then compile it to PDF with Typst. Use when the user gives a JD (text, file or URL) and wants a CV for it, or asks to update/rework an existing tailored CV in cvs/.
---

# Tailor a CV to a job description

Repo layout (paths relative to the repo root):

- `profile.yaml` — master profile, the **only source of truth** for facts. Read all of it, including comments and items with `hidden: true`.
- `jds/<slug>.md` — job descriptions.
- `cvs/<slug>.yaml` — tailored CVs (same schema as `profile.yaml`). `cvs/icreationslab-system-devops.yaml` is a worked example.
- `template/cv.typ` — renders any of the YAML files. Do not edit it for a single CV.
- `make cv NAME=<slug>` → `build/<slug>.pdf`.

## Steps

1. **Get the JD.** If given text or a URL, save the JD as `jds/<slug>.md` (top line: company, role, source URL if any). `<slug>` is kebab-case `company-role`, e.g. `acme-backend-engineer`. If the JD file already exists, reuse it.
2. **Analyse the JD.** List must-have and nice-to-have skills, seniority, domain, and the key terms the JD uses (an ATS matches on these).
3. **Select from `profile.yaml`.**
   - Pick the experience, projects and skills that match. Drop or shorten what doesn't; items marked `hidden: true` may be included when relevant (remove the `hidden` key in the CV).
   - Order sections with `sections:` to lead with the strongest evidence; rename with `titles:` if useful.
   - Order bullets by relevance; keep 2–5 bullets per role. Older/irrelevant roles get 1–2 or are omitted.
4. **Write the content.**
   - Rewrite `basics.headline` and `summary` (2–4 bullets) for this role.
   - Reword bullets to use the JD's terminology where the underlying fact is the same (e.g. profile says "K8s", JD says "Kubernetes" → use "Kubernetes").
   - **Never invent facts**: no new employers, titles, dates, numbers, technologies or achievements that are not in `profile.yaml`. If the JD asks for something the profile lacks, leave it out and tell the user (step 7).
   - Action verb first, quantified where the profile has numbers, one line or two max.
5. **Write `cvs/<slug>.yaml`.** Start with a comment line `# Tailored for: <company> — <role>`. Same keys as `profile.yaml`; omit sections you don't use.
   - Text is Typst markup: `*bold*`, `_italic_`, `#link("https://...")[text]`. Escape literal `~ # $ @ * _ < >` with a backslash and never write `//` outside a URL.
   - YAML: quote a value with single quotes if it contains `: ` or starts with a special character; quote numbers-as-text (`"2024"`).
   - Skill items with commas inside brackets must be quoted, e.g. `['AWS (S3, SQS)']`.
6. **Compile and check.** Run `make cv NAME=<slug>`. Fix any errors (usually unescaped markup or bad YAML) and rerun. Target **1 page** (2 max for senior roles): if it overflows, trim the least relevant bullets. Check the page count with `pdfinfo build/<slug>.pdf | grep Pages`.
7. **Report** to the user, briefly: output path, page count, what was emphasised, what was omitted, and JD requirements the profile has no evidence for (so the user can add real facts to `profile.yaml`).

## Updating the master profile

When the user shares new facts (new job, project, achievement), add them to `profile.yaml`, not just to one CV, so future CVs can use them.
