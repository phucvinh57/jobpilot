//! Telling whether a posting is already known: by canonical URL, and by `slug(company) +
//! slug(role)` (the same job on another board), against the leads (any state) and the
//! applications.

use std::collections::HashSet;

use anyhow::Result;

use crate::store::{Application, Store};

/// Query parameters that only say where a visitor came from.
const TRACKING: &[&str] = &[
    "gh_src", "lever-source", "lever-origin", "source", "src", "ref", "referrer", "fbclid", "gclid", "trk",
    "trackingid", "refid", "mc_cid", "mc_eid", "mkt_tok", "campaign", "cmpid",
];

/// Last path segments of an apply form: the posting is the same page one level up.
const APPLY_SEGMENTS: &[&str] = &["apply", "application"];

/// Words after which a company name is legal-form noise (`Acme Inc.`).
const LEGAL_SUFFIXES: &[&str] =
    &["inc", "llc", "ltd", "limited", "corp", "corporation", "co", "company", "gmbh", "jsc", "plc", "sa", "ag", "pte", "pty", "bv"];

/// The URL in one stable spelling: http(s) only, no `www.`, fragment, tracking parameters, apply
/// suffix or trailing slash, remaining parameters sorted. `None` for anything else (so a source
/// cannot make the app visit a `file:` or `javascript:` link).
pub fn canonical_url(raw: &str) -> Option<String> {
    let mut url = url::Url::parse(raw.trim()).ok()?;
    if !matches!(url.scheme(), "http" | "https") {
        return None;
    }
    let host = url.host_str()?.to_string();
    if let Some(bare) = host.strip_prefix("www.") {
        url.set_host(Some(bare)).ok()?;
    }
    url.set_fragment(None);

    let mut segments: Vec<String> = url.path_segments().map(|s| s.filter(|p| !p.is_empty()).map(str::to_string).collect()).unwrap_or_default();
    if segments.len() >= 2 && segments.last().is_some_and(|l| APPLY_SEGMENTS.contains(&l.to_ascii_lowercase().as_str())) {
        segments.pop();
    }
    url.set_path(&format!("/{}", segments.join("/")));

    let mut query: Vec<(String, String)> = url
        .query_pairs()
        .filter(|(k, _)| {
            let k = k.to_ascii_lowercase();
            !k.starts_with("utm_") && !TRACKING.contains(&k.as_str())
        })
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect();
    query.sort();
    if query.is_empty() {
        url.set_query(None);
    } else {
        url.query_pairs_mut().clear().extend_pairs(query);
    }
    Some(url.to_string())
}

/// `slug(company)-slug(role)`, for catching one job on two boards. Legal suffixes of the company
/// and parenthesised notes of the role (`(Remote)`) do not count. Empty when the company or the
/// role is missing, which never matches anything (see `Store::find_lead`).
pub fn dedup_key(company: &str, role: &str) -> String {
    let mut company_words = words(company);
    while company_words.len() > 1 && company_words.last().is_some_and(|w| LEGAL_SUFFIXES.contains(&w.as_str())) {
        company_words.pop();
    }
    let role_words = words(&strip_brackets(role));
    if company_words.is_empty() || role_words.is_empty() {
        return String::new();
    }
    format!("{}-{}", company_words.join("-"), role_words.join("-"))
}

fn words(s: &str) -> Vec<String> {
    s.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).map(str::to_lowercase).collect()
}

fn strip_brackets(s: &str) -> String {
    let mut out = String::new();
    let mut depth = 0u32;
    for c in s.chars() {
        match c {
            '(' | '[' => depth += 1,
            ')' | ']' => depth = depth.saturating_sub(1),
            _ if depth == 0 => out.push(c),
            _ => {}
        }
    }
    out
}

/// The applications' URLs and keys, loaded once per page of candidates.
#[derive(Default)]
pub struct Known {
    urls: HashSet<String>,
    keys: HashSet<String>,
}

impl Known {
    pub fn from_applications(apps: &[Application]) -> Self {
        let mut known = Self::default();
        for app in apps {
            for url in [app.source.as_deref(), app.apply_url.as_deref()].into_iter().flatten() {
                known.urls.extend(canonical_url(url));
            }
            let key = dedup_key(&app.company, &app.role);
            if !key.is_empty() {
                known.keys.insert(key);
            }
        }
        known
    }

    pub fn load(store: &Store) -> Result<Self> {
        Ok(Self::from_applications(&store.list_applications()?))
    }

    /// Is the posting (canonical `url`, `key`) an application already?
    pub fn has_application(&self, url: &str, key: &str) -> bool {
        self.urls.contains(url) || (!key.is_empty() && self.keys.contains(key))
    }

    /// Is the posting a lead (any state) or an application already?
    pub fn contains(&self, store: &Store, url: &str, key: &str) -> Result<bool> {
        Ok(self.has_application(url, key) || store.lead_exists(url, key)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::{NewApplication, NewLead};

    #[test]
    fn urls_have_one_spelling() {
        let c = |u: &str| canonical_url(u).unwrap();
        assert_eq!(c("https://www.Acme.test/jobs/1/?utm_source=x&gh_src=abc#apply"), "https://acme.test/jobs/1");
        assert_eq!(c("https://jobs.lever.co/globex/abc-1/apply"), "https://jobs.lever.co/globex/abc-1");
        assert_eq!(c("https://jobs.ashbyhq.com/initech/u1/application?utm_campaign=z"), "https://jobs.ashbyhq.com/initech/u1");
        assert_eq!(c("https://careers.acme.test/open?gh_jid=77&b=2&a=1"), "https://careers.acme.test/open?a=1&b=2&gh_jid=77", "the job id stays, parameters are sorted");
        assert_eq!(c("https://news.ycombinator.com/item?id=41000101"), "https://news.ycombinator.com/item?id=41000101");
        assert_eq!(c(" http://acme.test "), "http://acme.test/");
        assert_eq!(c("https://acme.test/apply"), "https://acme.test/apply", "a bare /apply is the page itself");
        assert_eq!(c("https://acme.test/a?utm_x=1"), c("https://acme.test/a/"));
    }

    #[test]
    fn only_http_links_are_candidates() {
        for bad in ["javascript:alert(1)", "file:///etc/passwd", "ftp://x.test/a", "acme.test/jobs", "", "data:text/html,x"] {
            assert_eq!(canonical_url(bad), None, "{bad}");
        }
    }

    #[test]
    fn keys_ignore_legal_forms_case_and_notes() {
        assert_eq!(dedup_key("Acme Inc.", "Senior Backend Engineer (Remote)"), "acme-senior-backend-engineer");
        assert_eq!(dedup_key("ACME", "senior  backend-engineer"), "acme-senior-backend-engineer");
        assert_eq!(dedup_key("Acme Robotics Ltd", "SRE"), "acme-robotics-sre");
        assert_eq!(dedup_key("Co", "Dev"), "co-dev", "a one-word name is never emptied");
        assert_eq!(dedup_key("", "Dev"), "");
        assert_eq!(dedup_key("Acme", "(Remote)"), "");
        assert_eq!(dedup_key("Công ty Việt", "Kỹ sư"), "công-ty-việt-kỹ-sư");
    }

    #[test]
    fn leads_in_any_state_and_applications_are_known() {
        let store = Store::open_in_memory().unwrap();
        store
            .add_lead(&NewLead { url: "https://a.test/1".into(), dedup_key: dedup_key("Acme", "Backend Engineer"), ..Default::default() })
            .unwrap();
        let dismissed = store
            .add_lead(&NewLead { url: "https://a.test/2".into(), dedup_key: "x-y".into(), ..Default::default() })
            .unwrap();
        store.dismiss_lead(dismissed.id, "too junior").unwrap();
        store
            .create_application(&NewApplication {
                slug: "globex-sre".into(),
                dir: "/tmp/globex-sre".into(),
                company: "Globex Corp".into(),
                role: "Site Reliability Engineer".into(),
                source: Some("https://jobs.lever.co/globex/abc?lever-source=x".into()),
                ..Default::default()
            })
            .unwrap();
        let known = Known::load(&store).unwrap();
        let check = |url: &str, key: &str| known.contains(&store, url, key).unwrap();

        assert!(check("https://a.test/1", ""), "same URL as a lead");
        assert!(check("https://b.test/9", "acme-backend-engineer"), "same company and role as a lead, other board");
        assert!(check("https://a.test/2", ""), "a dismissed lead is known too");
        assert!(check("https://jobs.lever.co/globex/abc", ""), "an application's URL, canonicalised");
        assert!(check("https://b.test/10", &dedup_key("Globex", "Site Reliability Engineer")), "same company and role as an application");
        assert!(!check("https://b.test/11", &dedup_key("Initech", "Designer")));
        assert!(!check("https://b.test/12", ""), "an empty key never matches");
    }
}
