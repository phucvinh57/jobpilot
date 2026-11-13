---
name: pre-pr
description: Take finished work to open pull requests — run the checks for what changed, review the diff against docs/REVIEW.md and fix findings, commit, open the PR (or a gh stack of PRs when the changes are related), watch CI and merge Low and Medium PRs. Use when the user runs /pre-pr, asks to open/ship a PR, or when work in a worktree is done.
---

# Pre-PR

CI has no AI reviewer, so this skill is the review. Report evidence, not claims.

## 1. Scope

- Never work on `main`. If the change isn't in a worktree under `.claude/worktrees/`, move it into one.
- Read `git status --short` and `git diff main...HEAD --stat`, then pick the risk tier from `docs/REVIEW.md`.
- Choose a shape:
  - **One PR** for one coherent change.
  - **A `gh stack`** when the layers build on each other and each can be reviewed and pass checks alone (e.g. a `core` API → the view that uses it).
  - **Separate worktrees** for unrelated changes.

## 2. Verify

- Run `scripts/check.sh` (cargo build/test/clippy for Rust changes, a Typst render for `profile.yaml`/`assets/`), fix failures and keep its output for the PR body.
- For a UI change, check it visually (AGENTS.md, "Check the UI visually").

## 3. Review

Review with fresh context against `docs/REVIEW.md`. In a stack, review each layer against its parent.

- Run `code-review` at the tier's level (`low` / `medium` / `high`). For the High tier, also run `security-review`.
- For a change to CV generation (`assets/prompts/`, `pipeline/`), check that no fact can come from outside `profile.yaml`.
- Fix every Important finding. Fix each other finding, or give a one-line reason not to.
- Repeat steps 2–3 on the new changes only, up to 3 rounds. If findings persist after that, question the approach with the user.
- If the same kind of finding recurs, propose a fix at the source: an `AGENTS.md` rule, a `REVIEW.md` line or a check in `scripts/check.sh`.

## 4. Commit and open

- Branch names are `claude/<topic>`, and stack layers are `claude/<topic>-<layer>`.
- Commit messages are `fix(scope): …` for fixes and a plain imperative sentence for features.
- Never use `--no-verify`, because the pre-push hook re-runs `check.sh`.
- Write each PR body to a scratchpad file following `.github/pull_request_template.md`:
  - **What changed:** the content, not the file list.
  - **Testing:** the commands actually run and their results.
  - **Risk:** the tier and what could break.
  - **How to revert:** `git revert` is enough unless `profile.yaml` facts were removed. Then say what to restore.
  - Any findings left open, with their reasons.
  - End with `🤖 Generated with [Claude Code](https://claude.com/claude-code)`.

Single PR:

```sh
git switch -c claude/<topic>
git add -A && git commit -m "<message>"
git push -u origin HEAD
gh pr create --base main --title "<title>" --body-file <body.md> --label "risk: <tier>"
# existing PR: gh pr edit --body-file <body.md> --add-label "risk: <tier>"
```

Stack (bottom-up from `main`):

```sh
gh stack init claude/<topic>-<layer1>
git add <files> && git commit -m "<message>"
gh stack add -Am "<message>" claude/<topic>-<layer2>
gh stack submit --auto --open
gh pr edit <branch> --title "<title>" --body-file <body.md> --add-label "risk: <tier>"
```

To fix a layer: `gh stack checkout <branch>`, commit, `gh stack rebase --upstack`, then `gh stack push`.

## 5. Watch CI

For each PR, run `gh pr checks <pr> --watch --fail-fast` in the background. On failure:

1. Read the log with `gh run view <id> --log-failed | tail -80`.
2. Reproduce with `check.sh`, fix on the PR's branch and push. In a stack, also run `rebase --upstack` and `push`.
3. Repeat until the checks pass. Skipped checks count as passing.

If a failure isn't caused by the PR (it fails on `main` too, or it comes from the CI setup), fix it separately and merge `main` in. Stop and ask after 3 failed rounds on one check, or when a failure is flaky or comes from infrastructure.

## 6. Merge

Each PR carries exactly one label, `risk: low`, `risk: medium` or `risk: high`, matching its tier. Label each stack layer by its own diff. The required `Risk label` check fails when the label is missing or the PR touches a High path without `risk: high`.

GitHub auto-merge is off for this repo, so this skill merges.

- **Low or Medium:** once `CI OK` passes, run `gh pr merge <pr> --merge --delete-branch`.
- **High:** never merge. A human reads the diff and merges.
- **Stack:** merge only a PR whose base is `main`. Then run `gh stack sync --prune`, and continue with the next layer: watch its CI, then merge it if its tier allows.
- If a fix raises the tier to High, run `gh pr edit <pr> --remove-label "risk: <old>" --add-label "risk: high"` and don't merge.

## 7. Report

List each PR URL with its tier, final check results, and whether it was merged or waits for a human.
