# private-resumes

CVs written as YAML and compiled locally with Typst (`@preview/basic-resume`).

- `profile.yaml` — master profile with every fact. Source of truth.
- `cvs/<slug>.yaml` — one CV per job description, derived from the profile.
- `jds/<slug>.md` — job descriptions.
- `template/cv.typ` — shared renderer. `make cv NAME=<slug>` → `build/<slug>.pdf`; `make profile`; `make all`.

To write a CV for a JD, use the `tailor-cv` skill (`.agents/skills/tailor-cv/SKILL.md`). Never add facts to a CV that are not in `profile.yaml`.

## Commands

```
make profile               # full CV from profile.yaml -> build/profile.pdf
make cv NAME=<slug>        # cvs/<slug>.yaml -> build/<slug>.pdf
make all                   # profile and every CV
make watch [NAME=<slug>]   # rebuild on save
make check [ARGS=--all]    # scripts/check.sh: pre-PR checks for paths changed since main
make hooks                 # run once per clone to enable the pre-push hook
```

## Workflow

- **One worktree per change** (`claude -w <name>`, in `.claude/worktrees/`).
- **Execute an approved plan with subagents.** Split the plan into independent tasks and give each to a `general-purpose` subagent with a self-contained brief (goal, files, steps, checks to run). Run independent tasks in parallel. The main session checks their results, then runs `/pre-pr`.
- **Show the result, don't claim it.** Report the output of `scripts/check.sh`. Never use `--no-verify`.
- **Finish with `/pre-pr` without asking.** Low and Medium PRs are merged by `/pre-pr` once `CI OK` passes. High PRs wait for a human.

## Rules

- Never add a fact to a CV that isn't in `profile.yaml`. New facts go into `profile.yaml` first.
- Don't edit `template/cv.typ` for a single CV.
