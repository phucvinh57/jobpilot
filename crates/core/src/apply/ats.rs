//! ATS detection and the small per-ATS adapters: which URL shows the form, where the buttons are and
//! what a finished application looks like. Adapters only carry selectors and patterns; they never
//! contain logic that depends on what the page says.

use url::Url;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Ats {
    Greenhouse,
    Lever,
    Ashby,
    LinkedIn,
    Workday,
    Generic,
}

impl Ats {
    pub fn label(self) -> &'static str {
        match self {
            Ats::Greenhouse => "Greenhouse",
            Ats::Lever => "Lever",
            Ats::Ashby => "Ashby",
            Ats::LinkedIn => "LinkedIn Easy Apply",
            Ats::Workday => "Workday",
            Ats::Generic => "Generic form",
        }
    }

    /// LinkedIn and Workday only show the form to a logged-in user.
    pub fn needs_session(self) -> bool {
        matches!(self, Ats::LinkedIn | Ats::Workday)
    }

    /// Workday is best effort: the app opens a visible browser instead of driving it.
    pub fn handoff_only(self) -> bool {
        self == Ats::Workday
    }

    /// Domain a matching browser session must have a cookie for (`None` = no login needed).
    pub fn session_domain(self) -> Option<&'static str> {
        match self {
            Ats::LinkedIn => Some("linkedin.com"),
            Ats::Workday => Some("myworkdayjobs.com"),
            _ => None,
        }
    }
}

/// Which ATS serves `url` (host / path based, never fetches anything).
pub fn detect_ats(url: &str) -> Ats {
    let Ok(parsed) = Url::parse(url.trim()) else { return Ats::Generic };
    let host = parsed.host_str().unwrap_or("").to_ascii_lowercase();
    let path = parsed.path().to_ascii_lowercase();
    let on = |domain: &str| host == domain || host.ends_with(&format!(".{domain}"));

    if on("greenhouse.io") || parsed.query_pairs().any(|(k, _)| k == "gh_jid") {
        return Ats::Greenhouse;
    }
    if on("lever.co") {
        return Ats::Lever;
    }
    if on("ashbyhq.com") {
        return Ats::Ashby;
    }
    if on("linkedin.com") && path.starts_with("/jobs") {
        return Ats::LinkedIn;
    }
    if on("myworkdayjobs.com") || on("myworkdaysite.com") {
        return Ats::Workday;
    }
    Ats::Generic
}

/// Selectors and patterns for one ATS. Text patterns are case-insensitive regexes matched against the
/// whole visible label of a button; `selectors` are tried first.
#[derive(Clone, Copy, Debug)]
pub struct Adapter {
    /// Scope for field extraction and button search (first visible match; else the whole document).
    pub root: &'static [&'static str],
    /// Button/link that opens the form on a job page.
    pub apply_button: &'static [&'static str],
    pub apply_text: &'static str,
    pub next: &'static [&'static str],
    pub next_text: &'static str,
    pub submit: &'static [&'static str],
    pub submit_text: &'static str,
    /// Substrings of the URL after a finished application.
    pub success_url: &'static [&'static str],
}

const GENERIC_APPLY: &str = r"apply( now| for this job| for this position)?|apply to this job|start (your )?application";
const GENERIC_NEXT: &str = r"next( step)?|continue|save (&|and) continue|review( (your )?application)?";
const GENERIC_SUBMIT: &str =
    r"submit( (your )?application)?|send( (your )?application)?|apply( now)?|finish|complete application";

pub fn adapter(ats: Ats) -> Adapter {
    match ats {
        Ats::Greenhouse => Adapter {
            root: &["#application_form", "#application-form", "form#application", "main form"],
            apply_button: &["#apply_button", "a[href='#app']", "a.btn[href*='#app']"],
            apply_text: GENERIC_APPLY,
            next: &[],
            next_text: GENERIC_NEXT,
            submit: &["#submit_app", "button[type=submit]", "input[type=submit]"],
            submit_text: GENERIC_SUBMIT,
            success_url: &["/confirmation", "confirmed=true", "/thanks"],
        },
        Ats::Lever => Adapter {
            root: &["form#application-form", "form.application-form", "form[action*='apply']", "main form"],
            apply_button: &["a.postings-btn[href*='/apply']", "a[href$='/apply']"],
            apply_text: GENERIC_APPLY,
            next: &[],
            next_text: GENERIC_NEXT,
            submit: &["#btn-submit", "button.postings-btn[type=submit]", "button[type=submit]"],
            submit_text: GENERIC_SUBMIT,
            success_url: &["/thanks", "/confirmation"],
        },
        Ats::Ashby => Adapter {
            root: &["form", "main form"],
            apply_button: &["a[href$='/application']"],
            apply_text: GENERIC_APPLY,
            next: &[],
            next_text: GENERIC_NEXT,
            submit: &["button[type=submit]"],
            submit_text: GENERIC_SUBMIT,
            success_url: &["/thanks", "/confirmation", "submitted"],
        },
        Ats::LinkedIn => Adapter {
            root: &["div.jobs-easy-apply-modal", "[role=dialog][aria-labelledby*='easy-apply']", "[role=dialog]"],
            apply_button: &["button.jobs-apply-button", "button[aria-label*='Easy Apply']", ".jobs-apply-button--top-card button"],
            apply_text: r"easy apply",
            next: &["button[aria-label='Continue to next step']", "button[data-easy-apply-next-button]"],
            next_text: GENERIC_NEXT,
            submit: &["button[aria-label='Submit application']", "button[data-live-test-easy-apply-submit-button]"],
            submit_text: r"submit application",
            success_url: &[],
        },
        // Handoff only; the values are never used to drive the page.
        Ats::Workday => Adapter {
            root: &[],
            apply_button: &["a[data-automation-id='adventureButton']"],
            apply_text: GENERIC_APPLY,
            next: &["button[data-automation-id='bottom-navigation-next-button']"],
            next_text: GENERIC_NEXT,
            submit: &["button[data-automation-id='bottom-navigation-next-button']"],
            submit_text: GENERIC_SUBMIT,
            success_url: &[],
        },
        Ats::Generic => Adapter {
            root: &["form"],
            apply_button: &[],
            apply_text: GENERIC_APPLY,
            next: &[],
            next_text: GENERIC_NEXT,
            submit: &["button[type=submit]", "input[type=submit]"],
            submit_text: GENERIC_SUBMIT,
            success_url: &["/thanks", "/thank-you", "/confirmation", "/success"],
        },
    }
}

/// The URL that shows the application form for a job-page URL (Lever `/apply`, Ashby `/application`).
/// Everything else is opened as given.
pub fn form_url(ats: Ats, url: &str) -> String {
    let Ok(mut parsed) = Url::parse(url.trim()) else { return url.trim().to_string() };
    let path = parsed.path().trim_end_matches('/').to_string();
    match ats {
        Ats::Lever if !path.ends_with("/apply") && path.matches('/').count() >= 2 => {
            parsed.set_path(&format!("{path}/apply"));
            parsed.to_string()
        }
        Ats::Ashby if !path.ends_with("/application") && path.matches('/').count() >= 2 => {
            parsed.set_path(&format!("{path}/application"));
            parsed.to_string()
        }
        _ => parsed.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_the_ats_from_host_and_path() {
        let cases = [
            ("https://boards.greenhouse.io/acme/jobs/123", Ats::Greenhouse),
            ("https://job-boards.greenhouse.io/acme/jobs/123", Ats::Greenhouse),
            ("https://acme.com/careers?gh_jid=456", Ats::Greenhouse),
            ("https://jobs.lever.co/acme/abc-123", Ats::Lever),
            ("https://jobs.ashbyhq.com/acme/uuid", Ats::Ashby),
            ("https://www.linkedin.com/jobs/view/12345/", Ats::LinkedIn),
            ("https://acme.wd5.myworkdayjobs.com/en-US/External/job/X", Ats::Workday),
            ("https://acme.com/careers/backend", Ats::Generic),
            ("https://www.linkedin.com/in/someone", Ats::Generic),
            ("https://notgreenhouse.io/jobs", Ats::Generic),
            ("not a url", Ats::Generic),
        ];
        for (url, want) in cases {
            assert_eq!(detect_ats(url), want, "{url}");
        }
    }

    #[test]
    fn login_and_handoff_rules() {
        assert!(Ats::LinkedIn.needs_session() && Ats::Workday.needs_session());
        assert!(!Ats::Greenhouse.needs_session() && !Ats::Generic.needs_session());
        assert!(Ats::Workday.handoff_only() && !Ats::LinkedIn.handoff_only());
        assert_eq!(Ats::LinkedIn.session_domain(), Some("linkedin.com"));
    }

    #[test]
    fn form_urls() {
        assert_eq!(form_url(Ats::Lever, "https://jobs.lever.co/acme/abc-123"), "https://jobs.lever.co/acme/abc-123/apply");
        assert_eq!(form_url(Ats::Lever, "https://jobs.lever.co/acme/abc-123/apply"), "https://jobs.lever.co/acme/abc-123/apply");
        assert_eq!(form_url(Ats::Lever, "https://jobs.lever.co/acme"), "https://jobs.lever.co/acme");
        assert_eq!(
            form_url(Ats::Ashby, "https://jobs.ashbyhq.com/acme/uuid?utm=x"),
            "https://jobs.ashbyhq.com/acme/uuid/application?utm=x"
        );
        assert_eq!(form_url(Ats::Greenhouse, "https://boards.greenhouse.io/acme/jobs/1"), "https://boards.greenhouse.io/acme/jobs/1");
    }

    #[test]
    fn adapters_have_a_submit_pattern() {
        for ats in [Ats::Greenhouse, Ats::Lever, Ats::Ashby, Ats::LinkedIn, Ats::Workday, Ats::Generic] {
            let a = adapter(ats);
            assert!(!a.submit_text.is_empty());
            regex::Regex::new(&format!("(?i)^(?:{})$", a.submit_text)).unwrap();
            regex::Regex::new(&format!("(?i)^(?:{})$", a.next_text)).unwrap();
            regex::Regex::new(&format!("(?i)^(?:{})$", a.apply_text)).unwrap();
        }
    }
}
