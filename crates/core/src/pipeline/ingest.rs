//! JD ingest: a URL, a file or pasted text -> plain readable text.
//!
//! JS-rendered pages come back (almost) empty from a plain HTTP fetch. [`ingest`] reports that as
//! [`NeedsBrowser`]; the caller can render the page with the managed browser (P4) and feed its HTML
//! to [`ingest_html`] (or `JdInput::Html` to the generator).

use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use regex::Regex;

/// Below this many characters a page is treated as "not rendered" (JS app, bot wall).
pub const MIN_PAGE_CHARS: usize = 400;
/// Fewer characters than this in a file or pasted text is an error (nothing to analyse).
const MIN_TEXT_CHARS: usize = 20;
/// Hard cap for a downloaded page.
const MAX_DOWNLOAD_BYTES: usize = 10 * 1024 * 1024;
const FETCH_TIMEOUT: Duration = Duration::from_secs(30);
pub(crate) const USER_AGENT: &str =
    "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36";

/// What the user gave us.
#[derive(Clone, Debug)]
pub enum JdInput {
    /// A job posting link (fetched with plain HTTP).
    Url(String),
    /// A `.md`, `.txt`, `.html` or `.pdf` file.
    File(PathBuf),
    /// Pasted text.
    Text(String),
    /// HTML already fetched or rendered elsewhere (e.g. by the browser); `url` is the page address.
    Html { html: String, url: String },
}

/// The JD as readable text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JdText {
    pub text: String,
    /// The URL or file path the user gave (`None` for pasted text).
    pub source: Option<String>,
    /// The page address when the JD came from the web.
    pub url: Option<String>,
}

/// A plain fetch did not yield a usable page; render it with a browser and use [`ingest_html`].
#[derive(Clone, Debug)]
pub struct NeedsBrowser {
    pub url: String,
    pub reason: String,
}

impl fmt::Display for NeedsBrowser {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} needs a browser to render: {}", self.url, self.reason)
    }
}

impl std::error::Error for NeedsBrowser {}

/// Turn any input into JD text. Downcast the error to [`NeedsBrowser`] to detect JS-rendered pages.
pub async fn ingest(input: JdInput) -> Result<JdText> {
    match input {
        JdInput::Url(url) => ingest_url(url.trim()).await,
        JdInput::File(path) => ingest_file(&path).await,
        JdInput::Text(text) => {
            let text = clean(&text);
            ensure_text(&text, "the pasted text")?;
            Ok(JdText { text, source: None, url: None })
        }
        JdInput::Html { html, url } => ingest_html(&html, &url),
    }
}

/// Extract the readable text from already fetched/rendered HTML. Errors when the page has no text.
pub fn ingest_html(html: &str, url: &str) -> Result<JdText> {
    let text = extract_html(html, Some(url));
    ensure_text(&text, "the page")?;
    let url = (!url.trim().is_empty()).then(|| url.trim().to_string());
    Ok(JdText { text, source: url.clone(), url })
}

/// Input checks that need no network, so callers can refuse bad input before creating a Draft.
pub fn precheck(input: &JdInput) -> Result<()> {
    match input {
        JdInput::Url(url) => parse_http_url(url.trim()).map(|_| ()),
        JdInput::File(path) if !path.is_file() => bail!("{} is not a file", path.display()),
        JdInput::File(_) | JdInput::Html { .. } => Ok(()),
        JdInput::Text(text) => ensure_text(text.trim(), "the pasted text"),
    }
}

fn parse_http_url(url: &str) -> Result<url::Url> {
    let parsed = url::Url::parse(url).with_context(|| format!("`{url}` is not a valid URL"))?;
    if !matches!(parsed.scheme(), "http" | "https") {
        bail!("only http(s) links are supported, got `{url}`");
    }
    Ok(parsed)
}

fn ensure_text(text: &str, what: &str) -> Result<()> {
    if text.chars().count() < MIN_TEXT_CHARS {
        bail!("{what} has no readable text");
    }
    Ok(())
}

// ---------------------------------------------------------------- URL

async fn ingest_url(url: &str) -> Result<JdText> {
    let parsed = parse_http_url(url)?;
    let needs_browser = |reason: String| anyhow::Error::new(NeedsBrowser { url: url.to_string(), reason });

    let client = reqwest::Client::builder()
        .user_agent(USER_AGENT)
        .timeout(FETCH_TIMEOUT)
        .build()
        .context("building the HTTP client")?;
    let resp = client
        .get(parsed.clone())
        .header("Accept", "text/html,application/xhtml+xml,application/pdf;q=0.9,*/*;q=0.5")
        .header("Accept-Language", "en-US,en;q=0.9")
        .send()
        .await
        .with_context(|| format!("fetching {url}"))?;

    let status = resp.status();
    if matches!(status.as_u16(), 401 | 403 | 429 | 503) {
        return Err(needs_browser(format!("the server answered HTTP {status} (bot check or login wall)")));
    }
    if !status.is_success() {
        bail!("fetching {url}: HTTP {status}");
    }
    let content_type = resp
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_lowercase();
    let final_url = resp.url().to_string();
    let bytes = resp.bytes().await.with_context(|| format!("reading {url}"))?;
    if bytes.len() > MAX_DOWNLOAD_BYTES {
        bail!("{url} is larger than {} MB", MAX_DOWNLOAD_BYTES / 1024 / 1024);
    }

    let text = if content_type.contains("application/pdf") || parsed.path().to_lowercase().ends_with(".pdf") {
        let bytes = bytes.to_vec();
        clean(&run_blocking(move || pdf_text(&bytes)).await?)
    } else if content_type.contains("html") || content_type.is_empty() || content_type.contains("xml") {
        let html = String::from_utf8_lossy(&bytes).into_owned();
        let base = final_url.clone();
        run_blocking(move || Ok(extract_html(&html, Some(&base)))).await?
    } else {
        // text/plain, text/markdown, ...
        clean(&String::from_utf8_lossy(&bytes))
    };

    if text.chars().count() < MIN_PAGE_CHARS {
        return Err(needs_browser(format!(
            "only {} characters of text were found (the page is probably rendered by JavaScript)",
            text.chars().count()
        )));
    }
    Ok(JdText { text, source: Some(url.to_string()), url: Some(final_url) })
}

/// Run CPU-bound extraction off the async worker (DOM parsing and PDF decoding can take a while).
async fn run_blocking<T: Send + 'static>(f: impl FnOnce() -> Result<T> + Send + 'static) -> Result<T> {
    tokio::task::spawn_blocking(f).await.context("extraction task failed")?
}

// ---------------------------------------------------------------- files

async fn ingest_file(path: &Path) -> Result<JdText> {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or_default().to_lowercase();
    let bytes = tokio::fs::read(path).await.with_context(|| format!("reading {}", path.display()))?;
    let source = Some(path.to_string_lossy().into_owned());
    let text = match ext.as_str() {
        "pdf" => clean(&run_blocking(move || pdf_text(&bytes)).await?),
        "html" | "htm" | "xhtml" => {
            let html = String::from_utf8_lossy(&bytes).into_owned();
            run_blocking(move || Ok(extract_html(&html, None))).await?
        }
        "md" | "markdown" | "txt" | "text" | "" => clean(&String::from_utf8_lossy(&bytes)),
        other => bail!("unsupported file type `.{other}` (use .md, .txt, .html or .pdf)"),
    };
    ensure_text(&text, &format!("{}", path.display()))?;
    Ok(JdText { text, source, url: None })
}

/// `pdf-extract` panics on some malformed files; turn that into an error.
pub(crate) fn pdf_text(bytes: &[u8]) -> Result<String> {
    let result = std::panic::catch_unwind(|| pdf_extract::extract_text_from_mem(bytes));
    match result {
        Ok(Ok(text)) => Ok(text),
        Ok(Err(e)) => bail!("could not read the PDF: {e}"),
        Err(_) => bail!("could not read the PDF (unsupported or corrupt file)"),
    }
}

// ---------------------------------------------------------------- HTML

/// Readable text of an HTML page: Readability's article as Markdown (with the page title on top),
/// falling back to the whole page rendered as text when Readability finds too little.
pub fn extract_html(html: &str, url: Option<&str>) -> String {
    let article = readable_article(html, url);
    if let Some(text) = &article {
        if text.chars().count() >= MIN_PAGE_CHARS {
            return text.clone();
        }
    }
    let whole = clean(&html2text::from_read(html.as_bytes(), 120).unwrap_or_default());
    // Keep whichever is longer; a short article beats nothing when the page itself is empty.
    match article {
        Some(a) if a.chars().count() > whole.chars().count() => a,
        _ => whole,
    }
}

fn readable_article(html: &str, url: Option<&str>) -> Option<String> {
    let cfg = dom_smoothie::Config { text_mode: dom_smoothie::TextMode::Markdown, ..Default::default() };
    // Readability wants an absolute URL or none at all.
    let url = url.filter(|u| url::Url::parse(u).is_ok());
    let mut readability = dom_smoothie::Readability::new(html, url, Some(cfg)).ok()?;
    let article = readability.parse().ok()?;
    let body = clean(&article.text_content);
    if body.is_empty() {
        return None;
    }
    let title = article.title.trim();
    Some(if title.is_empty() || body.contains(title) { body } else { format!("# {title}\n\n{body}") })
}

/// Normalise line endings and whitespace: trim trailing spaces, at most one blank line in a row.
pub fn clean(text: &str) -> String {
    let text = text.replace("\r\n", "\n").replace('\r', "\n").replace('\u{a0}', " ");
    let mut out = String::with_capacity(text.len());
    let mut blank_run = 0;
    for line in text.lines() {
        let line = line.trim_end();
        if line.is_empty() {
            blank_run += 1;
            if blank_run > 1 {
                continue;
            }
        } else {
            blank_run = 0;
        }
        out.push_str(line);
        out.push('\n');
    }
    // Collapse runs of spaces inside lines (HTML extraction leaves wide gaps) but keep indentation.
    static SPACES: OnceLock<Regex> = OnceLock::new();
    let spaces = SPACES.get_or_init(|| Regex::new(r"(\S) {2,}").expect("static regex"));
    spaces.replace_all(out.trim(), "$1 ").into_owned()
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) const JOB_HTML: &str = r#"<!doctype html>
<html lang="en"><head><title>Senior Rust Engineer - Acme Robotics</title>
<style>.x { color: red }</style><script>var tracking = "do not show this";</script></head>
<body>
<nav><a href="/">Home</a> <a href="/jobs">Jobs</a> <a href="/about">About</a></nav>
<main><article>
<h1>Senior Rust Engineer</h1>
<p>Acme Robotics builds warehouse robots and is hiring a Senior Rust Engineer to join the platform team in Ho Chi Minh City or remotely. You will design and operate the services that coordinate hundreds of robots in real time.</p>
<h2>What you will do</h2>
<ul><li>Design and build backend services in Rust and Go.</li><li>Operate Kubernetes clusters on AWS and improve observability.</li><li>Work with embedded teams on the robot fleet protocol.</li></ul>
<h2>Requirements</h2>
<ul><li>5+ years of professional software engineering experience.</li><li>Strong Rust, PostgreSQL and Linux skills.</li><li>Experience with CI/CD, Docker and Kubernetes.</li></ul>
<p>Nice to have: experience with ROS, MQTT or other robotics middleware. We offer a competitive salary, flexible hours and a learning budget.</p>
<p><a href="https://acme.test/apply/123">Apply now</a></p>
</article></main>
<footer>Copyright Acme Robotics. All rights reserved.</footer>
</body></html>"#;

    #[test]
    fn html_fixture_yields_the_job_text() {
        let jd = ingest_html(JOB_HTML, "https://acme.test/jobs/123").unwrap();
        assert!(jd.text.contains("Senior Rust Engineer"), "{}", jd.text);
        assert!(jd.text.contains("Kubernetes clusters on AWS"));
        assert!(jd.text.contains("5+ years"));
        assert!(!jd.text.contains("do not show this"), "scripts are dropped");
        assert!(!jd.text.contains("color: red"), "styles are dropped");
        assert_eq!(jd.url.as_deref(), Some("https://acme.test/jobs/123"));
        assert!(jd.text.chars().count() >= MIN_PAGE_CHARS);
    }

    #[test]
    fn short_page_falls_back_then_reports_no_text() {
        let spa =
            "<html><head><title>Jobs</title></head><body><div id=\"root\"></div><script>app()</script></body></html>";
        assert!(extract_html(spa, None).chars().count() < MIN_PAGE_CHARS);
        assert!(ingest_html(spa, "https://x.test/").is_err());
    }

    #[test]
    fn clean_collapses_blank_lines_and_spaces() {
        assert_eq!(clean("a  b\r\n\r\n\r\n\r\nc   \n  indented"), "a b\n\nc\n  indented");
    }

    #[tokio::test]
    async fn text_and_files() {
        let jd = ingest(JdInput::Text("  We need a Rust engineer for the platform team.  ".into())).await.unwrap();
        assert_eq!(jd.text, "We need a Rust engineer for the platform team.");
        assert_eq!(jd.source, None);
        assert!(ingest(JdInput::Text("hi".into())).await.is_err());

        let dir = tempfile::tempdir().unwrap();
        let md = dir.path().join("jd.md");
        std::fs::write(&md, "# Backend Engineer\n\nBuild APIs in Go for a fintech company in Singapore.\n").unwrap();
        let jd = ingest(JdInput::File(md.clone())).await.unwrap();
        assert!(jd.text.contains("fintech"));
        assert_eq!(jd.source.as_deref(), Some(md.to_str().unwrap()));

        let html = dir.path().join("jd.html");
        std::fs::write(&html, JOB_HTML).unwrap();
        let jd = ingest(JdInput::File(html)).await.unwrap();
        assert!(jd.text.contains("Senior Rust Engineer"));

        let bad = dir.path().join("jd.docx");
        std::fs::write(&bad, "x").unwrap();
        assert!(ingest(JdInput::File(bad)).await.unwrap_err().to_string().contains("unsupported"));
        assert!(ingest(JdInput::File(dir.path().join("missing.md"))).await.is_err());
    }

    #[tokio::test]
    async fn pdf_files_are_read() {
        let dir = tempfile::tempdir().unwrap();
        let typ = dir.path().join("jd.typ");
        std::fs::write(
            &typ,
            "= Platform Engineer\nWe are hiring a platform engineer who knows Kubernetes and Terraform.\n",
        )
        .unwrap();
        let pdf = dir.path().join("jd.pdf");
        let status = std::process::Command::new(super::super::render::typst_binary())
            .arg("compile")
            .arg(&typ)
            .arg(&pdf)
            .status()
            .expect("typst is needed for tests");
        assert!(status.success());
        let jd = ingest(JdInput::File(pdf.clone())).await.unwrap();
        assert!(jd.text.contains("platform engineer") && jd.text.contains("Kubernetes"), "{}", jd.text);

        std::fs::write(&pdf, "not a pdf").unwrap();
        assert!(ingest(JdInput::File(pdf)).await.unwrap_err().to_string().contains("PDF"));
    }

    #[tokio::test]
    async fn bad_urls_are_rejected_before_any_request() {
        assert!(precheck(&JdInput::Url("nope".into())).is_err());
        assert!(precheck(&JdInput::Url(" https://acme.test/job ".into())).is_ok());
        assert!(precheck(&JdInput::Text("short".into())).is_err());
        assert!(precheck(&JdInput::File("/definitely/missing.md".into())).is_err());
        assert!(ingest(JdInput::Url("not a url".into())).await.is_err());
        let err = ingest(JdInput::Url("ftp://example.com/job".into())).await.unwrap_err().to_string();
        assert!(err.contains("http"), "{err}");
    }

    /// A one-shot HTTP server answering every request with `status`, `content_type` and `body`.
    pub(crate) async fn serve(status: u16, content_type: &'static str, body: String) -> String {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            loop {
                let Ok((mut sock, _)) = listener.accept().await else { return };
                let body = body.clone();
                tokio::spawn(async move {
                    let mut buf = [0u8; 4096];
                    let _ = sock.read(&mut buf).await;
                    let head = format!(
                        "HTTP/1.1 {status} X\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        body.len()
                    );
                    let _ = sock.write_all(head.as_bytes()).await;
                    let _ = sock.write_all(body.as_bytes()).await;
                });
            }
        });
        format!("http://{addr}/job")
    }

    #[tokio::test]
    async fn url_fetch_extracts_text_and_flags_js_pages() {
        let url = serve(200, "text/html; charset=utf-8", JOB_HTML.to_string()).await;
        let jd = ingest(JdInput::Url(url.clone())).await.unwrap();
        assert!(jd.text.contains("Senior Rust Engineer"));
        assert_eq!(jd.source.as_deref(), Some(url.as_str()));

        let spa = "<html><body><div id=root></div><script src=app.js></script></body></html>".to_string();
        let url = serve(200, "text/html", spa).await;
        let err = ingest(JdInput::Url(url)).await.unwrap_err();
        let nb = err.downcast_ref::<NeedsBrowser>().expect("NeedsBrowser");
        assert!(nb.reason.contains("JavaScript"), "{nb}");

        let url = serve(403, "text/html", "blocked".into()).await;
        let err = ingest(JdInput::Url(url)).await.unwrap_err();
        assert!(err.downcast_ref::<NeedsBrowser>().is_some(), "{err}");

        let url = serve(404, "text/html", "nope".into()).await;
        let err = ingest(JdInput::Url(url)).await.unwrap_err();
        assert!(err.downcast_ref::<NeedsBrowser>().is_none() && err.to_string().contains("404"), "{err}");
    }
}
