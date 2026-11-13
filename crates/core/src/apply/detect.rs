//! Deterministic reading of what the page shows: is a CAPTCHA in the way, did the submission go
//! through. Pure functions over [`PageSignals`] (collected by `js::signals`), so they are unit tested
//! without a browser.

use std::sync::OnceLock;

use regex::Regex;
use serde::Deserialize;

use super::ats::Adapter;

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct PageSignals {
    pub url: String,
    pub title: String,
    /// Visible text (truncated).
    pub text: String,
    /// Visible fillable fields inside the form root; 0 when the form is gone.
    pub field_count: usize,
    pub frames: Vec<Frame>,
    /// Visible validation messages.
    pub errors: Vec<String>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct Frame {
    pub src: String,
    pub w: f64,
    pub h: f64,
    pub visible: bool,
    /// The small floating reCAPTCHA badge (invisible reCAPTCHA, not a challenge).
    pub badge: bool,
}

fn regex(cell: &'static OnceLock<Regex>, pattern: &str) -> &'static Regex {
    cell.get_or_init(|| Regex::new(pattern).expect("static regex"))
}

/// Why a CAPTCHA blocks automation, or `None` when the page is clear. Invisible reCAPTCHA (the badge
/// most Greenhouse forms carry) is not a blocker; a visible challenge widget or a "verify you are
/// human" wall is.
pub fn captcha_blocking(sig: &PageSignals) -> Option<String> {
    static VENDOR: OnceLock<Regex> = OnceLock::new();
    static TEXT: OnceLock<Regex> = OnceLock::new();
    let vendor =
        regex(&VENDOR, r"(?i)recaptcha|hcaptcha|turnstile|challenges\.cloudflare\.com|arkoselabs|funcaptcha|geetest");
    let text = regex(
        &TEXT,
        r"(?i)verify (that )?you(?:'| a)?re? (a )?human|verify you are human|i'?m not a robot|i am not a robot|are you a robot|complete the security check|checking if the site connection is secure|unusual traffic from your",
    );
    for f in &sig.frames {
        let invisible = f.src.contains("size=invisible");
        if vendor.is_match(&f.src) && f.visible && !f.badge && !invisible && f.w >= 100.0 && f.h >= 50.0 {
            return Some("a CAPTCHA is shown on the page".into());
        }
    }
    if text.is_match(&sig.text) {
        return Some("the page asks to verify that you are human".into());
    }
    None
}

/// Text that says an application went through.
pub fn success_text(text: &str) -> bool {
    static RE: OnceLock<Regex> = OnceLock::new();
    regex(
        &RE,
        r"(?i)thank(s| you) for (applying|your (application|interest|submission)|submitting)|application (has been |was |is )?(successfully )?(submitted|received|sent|complete)|(we('ve| have)|we) received your application|your application (has been |was )?(successfully )?(submitted|received|sent)|successfully (submitted|applied)|application sent|you('ve| have) applied",
    )
    .is_match(text)
}

/// `url` without its `#fragment`.
fn base(url: &str) -> &str {
    url.split('#').next().unwrap_or(url)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Verdict {
    /// The submission went through (success text / confirmation URL with the form gone).
    Submitted,
    /// A CAPTCHA appeared.
    Captcha(String),
    /// Still on the form with these validation messages.
    Rejected(Vec<String>),
    /// Cannot tell: the page changed but says nothing clear, or nothing happened (yet).
    Unclear { changed: bool },
}

/// Judge the page after the submit click. `before_url` is the form URL at click time.
pub fn judge(adapter: &Adapter, before_url: &str, sig: &PageSignals) -> Verdict {
    if let Some(reason) = captcha_blocking(sig) {
        return Verdict::Captcha(reason);
    }
    let changed = base(&sig.url) != base(before_url);
    let form_gone = sig.field_count == 0;
    let url_hit = changed && adapter.success_url.iter().any(|p| sig.url.contains(p));
    if url_hit || (success_text(&sig.text) && (changed || form_gone)) {
        return Verdict::Submitted;
    }
    if !sig.errors.is_empty() && !form_gone {
        return Verdict::Rejected(sig.errors.clone());
    }
    Verdict::Unclear { changed: changed || form_gone }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::apply::ats::{adapter, Ats};

    fn frame(src: &str, w: f64, h: f64, visible: bool, badge: bool) -> Frame {
        Frame { src: src.into(), w, h, visible, badge }
    }

    #[test]
    fn visible_challenges_block_but_the_invisible_badge_does_not() {
        let clear = PageSignals { text: "First name\nLast name\nSubmit application".into(), ..Default::default() };
        assert_eq!(captcha_blocking(&clear), None);

        let badge = PageSignals {
            frames: vec![frame("https://www.google.com/recaptcha/enterprise/anchor?size=invisible", 256.0, 60.0, true, true)],
            ..Default::default()
        };
        assert_eq!(captcha_blocking(&badge), None, "invisible reCAPTCHA is normal on Greenhouse forms");

        let v2 = PageSignals {
            frames: vec![frame("https://www.google.com/recaptcha/api2/anchor?size=normal", 304.0, 78.0, true, false)],
            ..Default::default()
        };
        assert!(captcha_blocking(&v2).is_some());

        let challenge = PageSignals {
            frames: vec![frame("https://www.google.com/recaptcha/api2/bframe", 400.0, 580.0, true, false)],
            ..Default::default()
        };
        assert!(captcha_blocking(&challenge).is_some());

        let hidden = PageSignals {
            frames: vec![frame("https://newassets.hcaptcha.com/captcha/v1/static/hcaptcha.html", 300.0, 80.0, false, false)],
            ..Default::default()
        };
        assert_eq!(captcha_blocking(&hidden), None, "a hidden frame is not in the way");

        let tiny = PageSignals { frames: vec![frame("https://hcaptcha.com/x", 1.0, 1.0, true, false)], ..Default::default() };
        assert_eq!(captcha_blocking(&tiny), None);

        let unrelated = PageSignals { frames: vec![frame("https://www.youtube.com/embed/x", 600.0, 400.0, true, false)], ..Default::default() };
        assert_eq!(captcha_blocking(&unrelated), None);
    }

    #[test]
    fn human_verification_walls_are_detected_by_text() {
        for text in [
            "Please verify you are human to continue",
            "Verify that you are human",
            "Checking if the site connection is secure",
            "Complete the security check to access example.com",
            "I'm not a robot",
        ] {
            assert!(captcha_blocking(&PageSignals { text: text.into(), ..Default::default() }).is_some(), "{text}");
        }
        assert!(captcha_blocking(&PageSignals { text: "We are a human-first company".into(), ..Default::default() }).is_none());
    }

    #[test]
    fn success_text_patterns() {
        for text in [
            "Thank you for applying!",
            "Thanks for applying to Acme",
            "Your application has been submitted.",
            "We have received your application",
            "Application received",
            "Application sent",
            "Thank you for your application",
        ] {
            assert!(success_text(text), "{text}");
        }
        for text in ["Apply for this job", "Submit application", "Please submit your application by Friday", "Thank you for your patience while the page loads"] {
            assert!(!success_text(text), "{text}");
        }
    }

    #[test]
    fn judging_the_page_after_submit() {
        let a = adapter(Ats::Greenhouse);
        let form = "https://boards.greenhouse.io/acme/jobs/1";
        let sig = |url: &str, text: &str, fields: usize, errors: &[&str]| PageSignals {
            url: url.into(),
            text: text.into(),
            field_count: fields,
            errors: errors.iter().map(|s| s.to_string()).collect(),
            ..Default::default()
        };

        // Confirmation text and the form is gone.
        assert_eq!(judge(&a, form, &sig(form, "Thank you for applying!", 0, &[])), Verdict::Submitted);
        // Confirmation URL.
        assert_eq!(judge(&a, form, &sig("https://boards.greenhouse.io/acme/jobs/1/confirmation", "", 0, &[])), Verdict::Submitted);
        // The thank-you phrase on the still-filled form (e.g. a header) is not enough.
        assert_eq!(
            judge(&a, form, &sig(form, "Thank you for your interest in Acme", 9, &[])),
            Verdict::Unclear { changed: false }
        );
        // Validation errors on the form.
        assert_eq!(
            judge(&a, form, &sig(form, "Email: missing or invalid", 9, &["Email: missing or invalid"])),
            Verdict::Rejected(vec!["Email: missing or invalid".into()])
        );
        // A new page without a clear message is for the LLM to read.
        assert_eq!(judge(&a, form, &sig("https://acme.com/done", "All set", 0, &[])), Verdict::Unclear { changed: true });
        // A fragment change is not navigation.
        assert_eq!(judge(&a, form, &sig(&format!("{form}#app"), "", 9, &[])), Verdict::Unclear { changed: false });
        // CAPTCHA wins over everything else.
        let mut blocked = sig(form, "Thank you for applying", 0, &[]);
        blocked.text.push_str("\nverify you are human");
        assert!(matches!(judge(&a, form, &blocked), Verdict::Captcha(_)));
    }
}
