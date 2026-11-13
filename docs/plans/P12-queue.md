# P12 Queue and apply entry

Goal: the user works through found jobs in order of match, gets a prepared CV for each, reviews it and applies with one button.

Depends on: P10 (leads), P11 (insight). Reuses the existing pipeline and apply flow unchanged.

## Queue view (`desktop/queue.rs`)

- List of leads in state `new`, sorted by score (default), with company, role, location, score and the top reasons.
- Selecting a lead shows the insight brief and the score breakdown.
- Actions per lead: **Prepare CV**, **Dismiss** (with an optional reason that is fed back into preferences as an exclusion), **Open posting**.
- Header: "Search now", last run time, per-source status, and the "needs you" banner from P10 with "Open browser".
- Filters: source, minimum score, location.

## Lead -> application (`core/discover/promote.rs`)

- `promote(lead_id)`: creates an application from the stored JD (`JdInput::Text` + source URL), copies the insight, marks the lead `promoted`, then runs the existing `pipeline::generate`.
- CVs are written only on demand (decision 2): nothing is promoted automatically. "Prepare CV" on several selected leads runs them one at a time.
- From here on the job is a normal application: `Draft -> CvReady -> ...` in the sidebar, with the existing primary action chain.

## Apply (already built; verified here)

Decision 3 keeps this flow:

1. **Approve CV**: the user reads the PDF/YAML, revises by chat or by hand.
2. **Review & Submit**: the managed browser opens the form, the app fills it, the user checks every answer.
3. **Submit**: the app submits and saves proof; when a form cannot be driven or a challenge appears, it hands over to the user in the visible browser ("finish manually").

Only addition: a "Next in queue" button after Submit/Dismiss to keep the user moving.

The one-primary-button rule holds: in the queue the primary is "Prepare CV" (or "Search now" when the queue is empty).

## Checks

- Unit: promote is idempotent, no promotion without a user action, sorting and filters.
- `primary_for` tests extended for the queue states.
- End-to-end on a throwaway data dir: fake source -> lead -> Prepare CV -> approve -> dry-run apply on the local fixture form.

Risk: Medium. Size: medium.
