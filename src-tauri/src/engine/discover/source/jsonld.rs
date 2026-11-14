//! schema.org `JobPosting` items in a page's `<script type="application/ld+json">` blocks. Many
//! careers pages and hiring systems publish them for search engines: a listing page as an
//! `ItemList` of postings, a posting page as one `JobPosting` with its full description.
//!
//! The blocks are page text, so everything here is untrusted data: a posting's `url` is only an
//! address the caller still checks (the same-site rule for a listing), and the description is HTML
//! that only ever becomes text ([`html_to_text`]).

use std::sync::OnceLock;

use chrono::{DateTime, NaiveDate, NaiveDateTime, Utc};
use regex::Regex;
use serde_json::Value;
use url::Url;

use super::{html_to_text, one_line, unescape_markup};

/// Largest script block read: a bigger one is skipped (a site's whole catalogue, or not data).
const MAX_BLOCK_BYTES: usize = 64 * 1024;
/// Most postings taken from one page.
const MAX_POSTINGS: usize = 500;
/// How deep the JSON is walked looking for postings (`@graph` in an `ItemList` in an array ...).
const MAX_DEPTH: usize = 6;
/// Longest single-line field kept (title, company, location ...).
const FIELD_CHARS: usize = 200;

/// One `JobPosting`, as the page describes it. Every text field is plain text on one line, `""`
/// when the page does not say.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct JsonLdPosting {
    /// The posting's address resolved against the page (http or https); `None` when it has none.
    pub url: Option<Url>,
    pub title: String,
    pub company: String,
    /// Each place as "locality, region, country", places joined by "; ".
    pub location: String,
    /// `jobLocationType` is `TELECOMMUTE`.
    pub remote: bool,
    /// The description as the page gives it (HTML; never shown as HTML).
    pub description_html: String,
    pub date_posted: String,
    pub valid_through: String,
    pub employment_type: String,
    /// "USD 100000-150000 per year".
    pub salary: String,
}

impl JsonLdPosting {
    /// The description as text.
    pub(crate) fn description_text(&self) -> String {
        let html = &self.description_html;
        // Some sites escape the markup inside the JSON string (`&lt;p&gt;`).
        if html.contains("&lt;") && !html.contains('<') {
            html_to_text(&unescape_markup(html))
        } else {
            html_to_text(html)
        }
    }

    /// Did the posting close before `now`? A date without a time closes at the end of that day; a
    /// missing or unreadable date never expires.
    pub(crate) fn expired(&self, now: DateTime<Utc>) -> bool {
        let raw = self.valid_through.trim();
        if raw.is_empty() {
            return false;
        }
        if let Some(at) = parse_datetime(raw) {
            return at < now;
        }
        parse_date(raw).is_some_and(|day| day < now.date_naive())
    }
}

/// The text of a posting page built from its `JobPosting`: a short header (only the fields the
/// page gives), a blank line, then the description as text.
pub(crate) fn posting_text(posting: &JsonLdPosting) -> String {
    let remote = if posting.remote { "yes" } else { "" };
    let header: Vec<String> = [
        ("Title", posting.title.as_str()),
        ("Company", &posting.company),
        ("Location", &posting.location),
        ("Remote", remote),
        ("Employment type", &posting.employment_type),
        ("Salary", &posting.salary),
        ("Posted", &posting.date_posted),
        ("Valid through", &posting.valid_through),
    ]
    .into_iter()
    .filter(|(_, v)| !v.is_empty())
    .map(|(k, v)| format!("{k}: {v}"))
    .collect();
    let body = posting.description_text();
    match (header.is_empty(), body.is_empty()) {
        (true, _) => body,
        (false, true) => header.join("\n"),
        (false, false) => format!("{}\n\n{body}", header.join("\n")),
    }
}

/// Every `JobPosting` in the `application/ld+json` blocks of `html`, in page order, at most
/// [`MAX_POSTINGS`]. A block may be one object, an array, an `@graph`, or an `ItemList` whose
/// `itemListElement` entries hold the postings (in `item`, or as the entry itself). Blocks that do
/// not parse (common) or are larger than [`MAX_BLOCK_BYTES`] are skipped.
pub(crate) fn job_postings(html: &str, base: &Url) -> Vec<JsonLdPosting> {
    let mut found = Vec::new();
    for block in ld_blocks(html) {
        if found.len() >= MAX_POSTINGS {
            break;
        }
        if block.len() > MAX_BLOCK_BYTES {
            continue;
        }
        let Ok(value) = serde_json::from_str::<Value>(strip_wrappers(block)) else { continue };
        let mut items = Vec::new();
        collect(&value, 0, &mut items);
        found.extend(items.into_iter().map(|v| posting(v, base)));
    }
    found.truncate(MAX_POSTINGS);
    found
}

/// The bodies of the `<script type="application/ld+json">` elements (the type in any case).
fn ld_blocks(html: &str) -> impl Iterator<Item = &str> {
    static SCRIPT: OnceLock<Regex> = OnceLock::new();
    static LD_TYPE: OnceLock<Regex> = OnceLock::new();
    let script = SCRIPT.get_or_init(|| Regex::new(r"(?is)<script\b([^>]*)>(.*?)</script\s*>").expect("static regex"));
    let ld_type = LD_TYPE.get_or_init(|| {
        Regex::new(r#"(?i)\btype\s*=\s*(?:"\s*application/ld\+json\s*"|'\s*application/ld\+json\s*'|application/ld\+json\b)"#).expect("static regex")
    });
    script.captures_iter(html).filter_map(move |c| {
        let attrs = c.get(1)?.as_str();
        ld_type.is_match(attrs).then(|| c.get(2).map(|m| m.as_str()).unwrap_or_default())
    })
}

/// A block without the HTML comment or CDATA markers some pages wrap their JSON in.
fn strip_wrappers(block: &str) -> &str {
    let mut s = block.trim();
    for (open, close) in [("<!--", "-->"), ("//<![CDATA[", "//]]>"), ("<![CDATA[", "]]>")] {
        if let Some(inner) = s.strip_prefix(open) {
            s = inner.strip_suffix(close).unwrap_or(inner).trim();
        }
    }
    s
}

/// The `JobPosting` objects inside `value`.
fn collect<'a>(value: &'a Value, depth: usize, out: &mut Vec<&'a Value>) {
    if depth > MAX_DEPTH || out.len() >= MAX_POSTINGS {
        return;
    }
    match value {
        Value::Array(items) => items.iter().for_each(|v| collect(v, depth + 1, out)),
        Value::Object(map) => {
            if has_type(value, "JobPosting") {
                out.push(value);
                return;
            }
            if let Some(graph) = map.get("@graph") {
                collect(graph, depth + 1, out);
            }
            if let Some(elements) = map.get("itemListElement") {
                let elements = match elements {
                    Value::Array(items) => items.iter().collect(),
                    one => vec![one],
                };
                for element in elements {
                    collect(element.get("item").unwrap_or(element), depth + 1, out);
                }
            }
        }
        _ => {}
    }
}

/// Is `@type` `kind`, or an array that holds it (with or without a schema.org prefix)?
fn has_type(value: &Value, kind: &str) -> bool {
    let is = |v: &Value| v.as_str().is_some_and(|t| t.rsplit(['/', ':']).next() == Some(kind));
    match value.get("@type") {
        Some(Value::Array(types)) => types.iter().any(is),
        Some(t) => is(t),
        None => false,
    }
}

fn posting(v: &Value, base: &Url) -> JsonLdPosting {
    let url = str_of(v.get("url"))
        .and_then(|u| base.join(u.trim()).ok())
        .filter(|u| matches!(u.scheme(), "http" | "https") && u.host_str().is_some())
        .map(|mut u| {
            u.set_fragment(None);
            u
        });
    let company = match v.get("hiringOrganization") {
        Some(Value::Object(org)) => str_of(org.get("name")).unwrap_or_default().to_string(),
        other => str_of(other).unwrap_or_default().to_string(),
    };
    let location = {
        let mut places = Vec::new();
        for place in list(v.get("jobLocation")) {
            let line = address_line(place.get("address").unwrap_or(place));
            if !line.is_empty() && !places.contains(&line) {
                places.push(line);
            }
        }
        // A remote job often names where applicants may live instead.
        if places.is_empty() {
            for area in list(v.get("applicantLocationRequirements")) {
                let name = name_of(area);
                if !name.is_empty() && !places.contains(&name) {
                    places.push(name);
                }
            }
        }
        line(&places.join("; "))
    };
    let remote = list(v.get("jobLocationType")).iter().any(|t| t.as_str().is_some_and(|t| t.trim().eq_ignore_ascii_case("TELECOMMUTE")));
    let employment_type = list(v.get("employmentType"))
        .iter()
        .filter_map(|t| t.as_str())
        .map(humanize)
        .filter(|t| !t.is_empty())
        .collect::<Vec<_>>()
        .join(", ");
    JsonLdPosting {
        url,
        title: line(str_of(v.get("title")).or_else(|| str_of(v.get("name"))).unwrap_or_default()),
        company: line(&company),
        location,
        remote,
        description_html: str_of(v.get("description")).unwrap_or_default().to_string(),
        date_posted: line(str_of(v.get("datePosted")).unwrap_or_default()),
        valid_through: line(str_of(v.get("validThrough")).unwrap_or_default()),
        employment_type: line(&employment_type),
        salary: salary(v.get("baseSalary")),
    }
}

/// A string value (`None` for anything else).
fn str_of(v: Option<&Value>) -> Option<&str> {
    v.and_then(Value::as_str)
}

/// A value that may be one item or an array of them.
fn list(v: Option<&Value>) -> Vec<&Value> {
    match v {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::Array(items)) => items.iter().collect(),
        Some(one) => vec![one],
    }
}

/// A plain-text, one-line form of a page value (entities decoded, tags dropped).
fn line(text: &str) -> String {
    if text.trim().is_empty() {
        return String::new();
    }
    let plain = if text.contains('<') || text.contains('&') { html_to_text(text) } else { text.to_string() };
    one_line(&plain, FIELD_CHARS)
}

/// A name: the string itself or its `name`.
fn name_of(v: &Value) -> String {
    match v {
        Value::String(s) => line(s),
        Value::Object(map) => line(str_of(map.get("name")).unwrap_or_default()),
        _ => String::new(),
    }
}

/// "locality, region, country" of a `PostalAddress` (or an address given as a string).
fn address_line(address: &Value) -> String {
    if let Value::String(s) = address {
        return line(s);
    }
    let mut parts: Vec<String> = Vec::new();
    for key in ["addressLocality", "addressRegion", "addressCountry"] {
        let part = address.get(key).map(name_of).unwrap_or_default();
        if !part.is_empty() && !parts.contains(&part) {
            parts.push(part);
        }
    }
    parts.join(", ")
}

/// `FULL_TIME` -> "Full time".
fn humanize(raw: &str) -> String {
    let words = raw.trim().replace(['_', '-'], " ").to_lowercase();
    let mut chars = words.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

/// `baseSalary` (a `MonetaryAmount`) as one line: "USD 100000-150000 per year".
fn salary(v: Option<&Value>) -> String {
    let Some(amount) = v else { return String::new() };
    let number = |v: Option<&Value>| -> Option<String> {
        match v? {
            Value::Number(n) => match n.as_f64() {
                Some(f) if f.fract() == 0.0 && f.abs() < 1e15 => Some(format!("{}", f as i64)),
                _ => Some(n.to_string()),
            },
            Value::String(s) if !s.trim().is_empty() => Some(line(s)),
            _ => None,
        }
    };
    let currency = line(str_of(amount.get("currency")).unwrap_or_default());
    let (figure, unit) = match amount.get("value") {
        Some(q @ Value::Object(_)) => {
            let figure = match (number(q.get("minValue")), number(q.get("maxValue")), number(q.get("value"))) {
                (Some(min), Some(max), _) if min != max => format!("{min}-{max}"),
                (Some(one), _, _) | (None, Some(one), _) | (None, None, Some(one)) => one,
                _ => String::new(),
            };
            (figure, line(str_of(q.get("unitText")).unwrap_or_default()))
        }
        other => (number(other).unwrap_or_default(), String::new()),
    };
    if figure.is_empty() {
        return String::new();
    }
    let mut out = if currency.is_empty() { figure } else { format!("{currency} {figure}") };
    if !unit.is_empty() {
        out.push_str(&format!(" per {}", unit.to_lowercase()));
    }
    one_line(&out, FIELD_CHARS)
}

/// A date and time with an offset (RFC 3339, or without seconds or offset as UTC).
pub(crate) fn parse_datetime(text: &str) -> Option<DateTime<Utc>> {
    let text = text.trim();
    if let Ok(d) = DateTime::parse_from_rfc3339(text) {
        return Some(d.with_timezone(&Utc));
    }
    for format in ["%Y-%m-%dT%H:%M%:z", "%Y-%m-%dT%H:%M:%S%z"] {
        if let Ok(d) = DateTime::parse_from_str(text, format) {
            return Some(d.with_timezone(&Utc));
        }
    }
    for format in ["%Y-%m-%dT%H:%M:%S%.f", "%Y-%m-%dT%H:%M", "%Y-%m-%d %H:%M:%S"] {
        if let Ok(d) = NaiveDateTime::parse_from_str(text, format) {
            return Some(d.and_utc());
        }
    }
    None
}

/// The day a date (or the start of a date-time) names: `2026-01-31`, `2026-01-31T...`.
pub(crate) fn parse_date(text: &str) -> Option<NaiveDate> {
    let day = text.trim().get(..10)?;
    NaiveDate::parse_from_str(day, "%Y-%m-%d").ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base() -> Url {
        Url::parse("https://acme.test/careers").unwrap()
    }

    fn script(json: &str) -> String {
        format!(r#"<html><head><script type="application/ld+json">{json}</script></head><body></body></html>"#)
    }

    fn titles(html: &str) -> Vec<String> {
        job_postings(html, &base()).into_iter().map(|p| p.title).collect()
    }

    #[test]
    fn postings_come_from_an_object_an_array_a_graph_and_an_item_list() {
        let one = r#"{"@context":"https://schema.org","@type":"JobPosting","title":"One","url":"/jobs/1"}"#;
        assert_eq!(titles(&script(one)), ["One"]);
        let array = r#"[{"@type":"JobPosting","title":"A"},{"@type":"Organization","name":"Acme"},{"@type":"JobPosting","title":"B"}]"#;
        assert_eq!(titles(&script(array)), ["A", "B"]);
        let graph = r#"{"@context":"https://schema.org","@graph":[{"@type":"WebPage"},{"@type":["JobPosting","Thing"],"title":"G"}]}"#;
        assert_eq!(titles(&script(graph)), ["G"], "@type as an array");
        let list = r#"{"@type":"ItemList","itemListElement":[
            {"@type":"ListItem","position":1,"item":{"@type":"JobPosting","title":"L1"}},
            {"@type":"JobPosting","title":"L2"},
            {"@type":"ListItem","position":3,"url":"/x"}]}"#;
        assert_eq!(titles(&script(list)), ["L1", "L2"]);
        let typed = r#"{"@type":"http://schema.org/JobPosting","title":"Prefixed"}"#;
        assert_eq!(titles(&script(typed)), ["Prefixed"]);
        // Several blocks, in page order; the type attribute in any case and quoting.
        let html = format!("{}<script TYPE='Application/LD+JSON'>{}</script><script>{one}</script>", script(one), r#"{"@type":"JobPosting","title":"Two"}"#);
        assert_eq!(titles(&html), ["One", "Two"]);
    }

    #[test]
    fn bad_and_oversized_blocks_are_skipped() {
        let good = r#"{"@type":"JobPosting","title":"Good"}"#;
        let broken = r#"{"@type":"JobPosting","title":"Broken",}"#;
        let huge = format!(r#"{{"@type":"JobPosting","title":"Huge","description":"{}"}}"#, "x".repeat(MAX_BLOCK_BYTES));
        let html = format!("{}{}{}", script(broken), script(&huge), script(good));
        assert_eq!(titles(&html), ["Good"]);
        assert_eq!(titles(&script("<!-- {\"@type\":\"JobPosting\",\"title\":\"Wrapped\"} -->")), ["Wrapped"]);
        assert!(titles("<p>no data</p>").is_empty());
        assert!(titles(&script(r#"{"@type":"Organization","name":"Acme"}"#)).is_empty());
        let many = format!("[{}]", vec![good; MAX_POSTINGS + 20].join(","));
        assert_eq!(titles(&script(&many)).len(), MAX_POSTINGS);
    }

    #[test]
    fn fields_are_read_as_plain_text() {
        let json = r#"{"@type":"JobPosting","title":"Data &amp; ML Engineer","url":"/careers/data-ml#apply",
            "hiringOrganization":{"@type":"Organization","name":"Acme"},
            "jobLocation":[{"@type":"Place","address":{"addressLocality":"Hanoi","addressRegion":"Hanoi","addressCountry":{"@type":"Country","name":"VN"}}},
                           {"@type":"Place","address":{"addressLocality":"Berlin","addressCountry":"DE"}}],
            "jobLocationType":"TELECOMMUTE","employmentType":["FULL_TIME","CONTRACTOR"],
            "baseSalary":{"@type":"MonetaryAmount","currency":"USD","value":{"@type":"QuantitativeValue","minValue":100000,"maxValue":150000,"unitText":"YEAR"}},
            "datePosted":"2026-10-01","validThrough":"2026-12-31T23:59",
            "description":"<p>Build <b>pipelines</b>.</p><ul><li>Rust</li><li>SQL</li></ul>"}"#;
        let p = job_postings(&script(json), &base()).remove(0);
        assert_eq!(p.url.as_ref().map(Url::as_str), Some("https://acme.test/careers/data-ml"));
        assert_eq!(p.title, "Data & ML Engineer");
        assert_eq!(p.company, "Acme");
        assert_eq!(p.location, "Hanoi, VN; Berlin, DE");
        assert!(p.remote);
        assert_eq!(p.employment_type, "Full time, Contractor");
        assert_eq!(p.salary, "USD 100000-150000 per year");
        let text = posting_text(&p);
        assert!(text.starts_with("Title: Data & ML Engineer\nCompany: Acme\nLocation: Hanoi, VN; Berlin, DE\nRemote: yes\nEmployment type: Full time, Contractor\nSalary: USD 100000-150000 per year\nPosted: 2026-10-01\nValid through: 2026-12-31T23:59\n\n"), "{text}");
        assert!(text.contains("Build pipelines") && text.contains("Rust") && !text.contains("<b>"), "{text}");
    }

    #[test]
    fn other_shapes_of_the_fields() {
        let json = r#"{"@type":"JobPosting","title":"T","hiringOrganization":"Acme Ltd","url":"javascript:alert(1)",
            "jobLocation":{"address":"Remote, Europe"},"employmentType":"PART_TIME",
            "baseSalary":{"currency":"EUR","value":{"value":"40","unitText":"HOUR"}},
            "description":"&lt;p&gt;Escaped &lt;b&gt;markup&lt;/b&gt;&lt;/p&gt;"}"#;
        let p = job_postings(&script(json), &base()).remove(0);
        assert_eq!(p.url, None, "only http(s) addresses");
        assert_eq!((p.company.as_str(), p.location.as_str(), p.employment_type.as_str()), ("Acme Ltd", "Remote, Europe", "Part time"));
        assert_eq!(p.salary, "EUR 40 per hour");
        assert!(!p.remote);
        assert_eq!(p.description_text(), "Escaped markup");
        let only = r#"{"@type":"JobPosting","jobLocationType":["TELECOMMUTE"],"applicantLocationRequirements":{"@type":"Country","name":"USA"},"baseSalary":{"currency":"USD","value":90000}}"#;
        let p = job_postings(&script(only), &base()).remove(0);
        assert!(p.remote);
        assert_eq!((p.location.as_str(), p.salary.as_str()), ("USA", "USD 90000"));
        assert_eq!(posting_text(&p), "Location: USA\nRemote: yes\nSalary: USD 90000");
    }

    #[test]
    fn a_posting_expires_after_its_valid_through() {
        let now = DateTime::parse_from_rfc3339("2026-11-14T12:00:00Z").unwrap().with_timezone(&Utc);
        let at = |v: &str| JsonLdPosting { valid_through: v.into(), ..Default::default() };
        assert!(at("2020-01-31").expired(now));
        assert!(at("2026-11-13T23:59:59Z").expired(now));
        assert!(!at("2026-11-14").expired(now), "a date runs to the end of that day");
        assert!(!at("2026-11-14T13:00:00+00:00").expired(now));
        assert!(!at("2099-01-01").expired(now));
        assert!(!at("").expired(now) && !at("soon").expired(now), "no readable date never expires");
    }

    #[test]
    fn the_fixtures_read() {
        let list = job_postings(include_str!("../fixtures/career_page_jsonld.html"), &base());
        assert_eq!(list.len(), 4);
        let one = job_postings(include_str!("../fixtures/posting_jsonld.html"), &Url::parse("https://acme.test/careers/senior-rust-engineer").unwrap());
        assert_eq!(one.len(), 1);
        assert!(one[0].description_text().chars().count() > 200);
    }
}
