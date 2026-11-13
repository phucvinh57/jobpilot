# P11 Job insight

Goal: for each job, a short brief: business domain, company information, skill list, interview process when it can be found.

Depends on: P8 (the `insight_json` columns). Independent of P9/P10.

## Model (`pipeline/insight.rs`)

```
Insight {
  domain,                 // e.g. "B2B payments"
  business_summary,       // what the company sells, to whom
  company { size, stage, funding, hq, founded },   // each "" when unknown
  skills { required[], preferred[] },              // reuses Analysis must/nice-to-have
  interview { stages[], notes, found: bool },
  sources[]               // URL per claim group
}
```

Rule: unknown stays empty and the UI says "not found". Nothing is guessed.

## Inputs

1. The JD text (already stored).
2. Company pages chosen by code, not by the model: the site root, `/about`, `/careers` on the company's own domain (taken from the posting URL, or the ATS board's company link). Plain fetch, browser fallback, capped size.
3. Interview process: the JD and the careers page only in the first version. Review sites (Glassdoor and the like) are login-walled and anti-bot; they are out of the first version; the search API from decision 6 can add public write-ups later.

One `complete_json` call with a fixed schema; every text is passed as data.

## Graph

`insight` is a subgraph, reused as one node by `source_run` (P10) and callable directly for user-added JDs: `company_urls` (derived in code) -> `fetch_pages` (the pages in parallel, each optional, short timeout) -> `extract` (one LLM call) -> end. No interrupt: a page that needs a login or shows a captcha is skipped and the fields stay empty.

## Use

- `insight(core, jd_text, url) -> Insight`, called by discovery for every lead that passes the hard filters and by the existing pipeline after `analyze` for user-added JDs.
- Stored in `leads.insight_json` and copied to `applications.insight_json` on promotion.
- `detail/jd.rs` shows it above the JD text: domain, business, company facts, skills, interview stages, source links.

## Checks

- Unit: schema, company-URL derivation (ATS URLs, career subdomains), empty fields when pages are missing.
- Flow test with `llm::fake` and local fixture pages.
- Screenshot of the JD tab with demo insight.

Risk: Medium. Size: small to medium.
