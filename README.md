# private-resumes

My CVs as YAML, compiled locally to PDF with [Typst](https://typst.app) using the
[`basic-resume`](https://typst.app/universe/package/basic-resume) package.

```
profile.yaml          master profile: everything I've done (source of truth)
cvs/<slug>.yaml       one tailored CV per job description
jds/<slug>.md         job descriptions
template/cv.typ       shared renderer
build/<slug>.pdf      output (gitignored)
```

## Setup

```bash
cargo install --locked typst-cli
```

The package is downloaded automatically on first compile.

## Build

```bash
make profile                 # full CV from profile.yaml -> build/profile.pdf
make cv NAME=<slug>          # cvs/<slug>.yaml -> build/<slug>.pdf
make all                     # everything
make watch NAME=<slug>       # rebuild on save (omit NAME for profile)
```

Optional per-file `style:` block: `font`, `font-size` (e.g. `"10pt"`), `accent-color` (hex), `paper`.

## Tailoring a CV to a JD

The `tailor-cv` skill (`.agents/skills/tailor-cv/SKILL.md`) reads the JD and
`profile.yaml`, writes `cvs/<slug>.yaml`, and compiles it. It never adds facts
that aren't in the profile.

- **Claude Code**: `/tailor-cv <paste JD or path>` (via `.claude/skills/` symlink)
- **OpenAI Codex**: `$tailor-cv <paste JD or path>` (reads `.agents/skills/`)

New achievements go into `profile.yaml` first, so every future CV can use them.
