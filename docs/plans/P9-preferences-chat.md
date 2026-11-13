# P9 Preference chat

Goal: a chat where the AI interviews the user about what job they want, optionally reads their existing CV, and saves structured preferences used by discovery, scoring and trends.

Depends on: P8.

## Model (`core/prefs/model.rs`)

```
Preferences {
  roles[], seniority[], must_skills[], nice_skills[], avoid_skills[],
  domains[], avoid_domains[], locations[], remote: any|remote|hybrid|onsite,
  salary_min, currency, company_size[], company_stage[], visa_needed,
  languages[], exclude_companies[], notes,
  weights { skills, domain, location, salary, seniority }   // sums to 100
}
```

Every field is optional; `Preferences::default()` is valid so P10/P11 work before the chat is done.

## Chat loop (`core/prefs/chat.rs`)

- One turn = `Llm::chat_json(system, history, schema)` returning `{ reply, questions[<=3], patch, complete }`.
  - `patch` is a partial `Preferences`, merged in code. The model never writes the stored document directly.
  - `questions` may carry quick-reply options (rendered as chips).
- System prompt `assets/prompts/preferences.md`: ask what is missing, at most 3 questions per turn, confirm a summary before `complete`.
- Context given to the model: current `Preferences`, a compact view of `profile.yaml` (reuse `tailor::profile_for_prompt`), and the uploaded CV text if any.
- History is stored in `chat_messages`; the conversation can be reopened later to change preferences ("I now also accept hybrid in Singapore").

## CV upload (`core/prefs/cv_upload.rs`)

- Accept `.pdf`, `.md`, `.txt` (reuse the extraction in `pipeline/ingest`), size and length capped.
- Text is stored under `<data_dir>/prefs/uploaded-cv.txt` and used as chat context only.
- Treated as untrusted text: wrapped as data in the prompt.
- "Import into profile" (decision 4): a separate action that proposes a `profile.yaml` diff the user approves. Out of the first PR.

## View (`desktop/prefs.rs`)

- Left: message list + input + "Attach CV" (file dialog and drop).
- Right: live preference summary, each field editable by hand; "Save" is implicit (every merged patch is saved), with an "Undo last change".
- First run: the setup banner points here when no preferences exist.
- Never blocks: each turn goes through `runtime::spawn`; errors map to `health::Problem` (no provider -> Settings, network -> Retry).

## Checks

- Unit: patch merge (lists replace vs. append, weights renormalised), schema validation, upload limits.
- Flow test with `llm::fake`: three scripted turns end with `complete` and a saved row.
- UI: seeded chat history in demo mode, screenshot; chips clickable through agent-desktop.

Risk: Medium for code, High for the prompt file. Size: medium.
