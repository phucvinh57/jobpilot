# private-resumes

CVs written as YAML and compiled locally with Typst (`@preview/basic-resume`).

- `profile.yaml` — master profile with every fact. Source of truth.
- `cvs/<slug>.yaml` — one CV per job description, derived from the profile.
- `jds/<slug>.md` — job descriptions.
- `template/cv.typ` — shared renderer. `make cv NAME=<slug>` → `build/<slug>.pdf`; `make profile`; `make all`.

To write a CV for a JD, use the `tailor-cv` skill (`.agents/skills/tailor-cv/SKILL.md`). Never add facts to a CV that are not in `profile.yaml`.
