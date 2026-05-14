# private-resumes

## CV Automation Scripts

### Upload a JSON CV to local RxResume

- Reads a CV JSON file.
- Uses file name (without extension) as the resume name.
- Creates or updates by slug derived from file name.

```bash
bun run cv:upload -- general.json
```

Env:

- Required: `RXRESUME_API_KEY`
- Optional: `RXRESUME_BASE_URL` (fallback to `APP_URL`, then `http://localhost:3000`)

### Generate a tailored CV with OpenAI

- Uses a prompt and a base CV JSON (`general.json` by default).
- Outputs another full Reactive Resume JSON file.

```bash
bun run cv:generate -- --prompt "Tailor this CV for a Senior DevOps role" --base general.json --out devops.json
```

Env:

- Required: `OPENAI_API_KEY`
- Optional: `OPENAI_MODEL` (default `gpt-5.2`)
