---
name: pre-pr
description: Take finished work to open pull requests — run the checks for what changed, review the diff against docs/REVIEW.md and fix findings, commit, open the PR (or a gh stack of PRs when the changes are related), re-check the final commit locally and merge Low and Medium PRs. Use when the user runs /pre-pr, asks to open/ship a PR, or when work in a worktree is done.
---

# Pre-PR

There is no CI (no GitHub Actions): this skill is the review and the checks, and its local runs of `scripts/check.sh` and `scripts/risk.sh` are the only gate before a merge. Report evidence, not claims.

## 1. Scope

- **Docs-only** (every changed path is under `docs/`, and none is `docs/REVIEW.md`): commit on `main`, `git push origin main`, report the commit and stop. No worktree, no PR, no checks.
- Otherwise never work on `main`. If the change isn't in a worktree under `.claude/worktrees/`, move it into one.
- `git fetch origin main`, read `git status --short` and `git diff origin/main...HEAD --stat`, then run `scripts/risk.sh`: it prints the tier the changed paths require (`docs/REVIEW.md`) and lists the High paths. Use that tier, or a higher one if the change warrants it.
- Choose a shape:
  - **One PR** for one coherent change.
  - **A `gh stack`** when the layers build on each other and each can be reviewed and pass checks alone (e.g. a `core` API → the view that uses it).
  - **Separate worktrees** for unrelated changes.

## 2. Verify

- Run `scripts/check.sh` (Rust: clippy + test; web: tsc, Biome, vitest, build; `assets/`: a Typst render), fix failures and keep its output for the PR body.
- For a UI change, check it visually (CLAUDE.md, "Check the UI visually") and keep screenshots for the PR:
  - Drive each changed state with the dev-tools script on demo data (`JOBPILOT_DEMO=1`, throwaway `JOBPILOT_DATA_DIR`). On a headless Linux box, run it under `xvfb-run -a -s "-screen 0 1440x960x24"`.
  - Wait about 1.5 s after opening a dialog or drawer before `shot()`: one caught mid-animation comes out translucent.
  - Look at every shot before using it (the state it claims, nothing cut off), and drop the ones that don't show it.
  - Save them as `docs/pr-media/<topic>/<n>-<state>.png` (1×, under about 150 KB each) and commit them with the change.

## 3. Review

Review with fresh context against `docs/REVIEW.md`. In a stack, review each layer against its parent.

- Run `code-review` at the tier's level (`low` / `medium` / `high`). For the High tier, also run `security-review`.
- For a change to CV generation (`assets/prompts/`, `pipeline/`), check that no fact can come from outside the user's documents (the profile).
- Fix every Important finding. Fix each other finding, or give a one-line reason not to.
- Repeat steps 2–3 on the new changes only, up to 3 rounds. If findings persist after that, question the approach with the user.
- If the same kind of finding recurs, propose a fix at the source: an `AGENTS.md` rule, a `REVIEW.md` line or a check in `scripts/check.sh`.

## 4. Commit and open

- Branch names are `claude/<topic>`, and stack layers are `claude/<topic>-<layer>`.
- Commit messages are `fix(scope): …` for fixes and a plain imperative sentence for features.
- Never use `--no-verify`, because the pre-push hook re-runs `check.sh`.
- Write each PR body to a scratchpad file following `.github/pull_request_template.md`:
  - **What changed:** the content, not the file list.
  - **Screenshots:** for a UI change, each image from step 2 with a one-line caption, linked by the pushed commit's SHA (`https://github.com/<owner>/<repo>/blob/<sha>/docs/pr-media/<topic>/<name>.png?raw=true`; push first, then write or edit the body). Otherwise "No UI change".
  - **Testing:** the commands actually run and their results.
  - **Risk:** the tier and what could break.
  - **How to revert:** `git revert` is enough unless data was removed. Then say what to restore.
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

## 5. Final check

No CI runs on the PR, so check the exact commit that will merge, on each PR's branch (in a stack, each layer):

1. `git fetch origin main` and make sure the branch contains the latest `origin/main` (merge it in if not, then push).
2. Run `scripts/check.sh` and `scripts/risk.sh --label <tier>` on the pushed head, with a clean `git status`, where `<tier>` is read from the PR itself: `gh pr view <pr> --json labels` must show exactly one `risk: ` label. Keep both outputs for the report (and the PR body if they changed since).
3. If a fix since the PR was opened changed the UI, retake the affected screenshots and update the body.
4. On a failure, fix on the branch, push, and repeat from step 1. Stop and ask after 3 failed rounds, or when a failure is flaky or comes from the environment (missing tool, network).

If a failure isn't caused by the PR (it fails on `main` too), fix it separately and merge `main` in.

## 6. Merge

Each PR carries exactly one label, `risk: low`, `risk: medium` or `risk: high`, matching its tier. Label each stack layer by its own diff (`scripts/risk.sh <parent branch>`). Nothing on GitHub enforces the label: `scripts/risk.sh --label <tier>` passing is the check.

GitHub auto-merge is off for this repo, so this skill merges.

- **Low or Medium:** once step 5 passes on the PR's head commit, run `gh pr merge <pr> --merge --delete-branch`.
- **High:** never merge. A human reads the diff and merges.
- **Stack:** merge only a PR whose base is `main`. Then run `gh stack sync --prune`, and continue with the next layer: run step 5 on it, then merge it if its tier allows.
- If a fix raises the tier to High, run `gh pr edit <pr> --remove-label "risk: <old>" --add-label "risk: high"` and don't merge.

## 7. Report

List each PR URL with its tier, the final `check.sh` and `risk.sh` results, and whether it was merged or waits for a human.
