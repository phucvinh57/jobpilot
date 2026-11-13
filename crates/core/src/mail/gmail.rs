//! Minimal read-only Gmail REST client plus message parsing (headers, text body).

use std::fmt;
use std::time::Duration;

use anyhow::{anyhow, Result};
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use chrono::{DateTime, Utc};
use regex::Regex;
use serde::Deserialize;
use serde_json::Value;

pub const API_BASE: &str = "https://gmail.googleapis.com/gmail/v1";
/// Plain-text body characters kept for classification.
pub const MAX_BODY_CHARS: usize = 4000;
/// Messages listed by the first (full) sync.
pub const FIRST_SYNC_CAP: usize = 500;
/// First sync window.
pub const FIRST_SYNC_QUERY: &str = "newer_than:60d -in:sent -in:drafts";

/// Typed API failures the sync reacts to (find them with `anyhow::Error::downcast_ref`).
#[derive(Debug)]
pub enum ApiError {
    /// HTTP 404: a deleted message or an expired `startHistoryId`.
    NotFound(String),
    /// HTTP 401: the access token was rejected.
    Unauthorized(String),
}

impl fmt::Display for ApiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ApiError::NotFound(m) | ApiError::Unauthorized(m) => f.write_str(m),
        }
    }
}

impl std::error::Error for ApiError {}

pub fn is_not_found(e: &anyhow::Error) -> bool {
    matches!(e.downcast_ref::<ApiError>(), Some(ApiError::NotFound(_)))
}

pub fn is_unauthorized(e: &anyhow::Error) -> bool {
    matches!(e.downcast_ref::<ApiError>(), Some(ApiError::Unauthorized(_)))
}

// ---------------------------------------------------------------- wire types

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Profile {
    pub email_address: String,
    #[serde(default)]
    pub history_id: String,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct MessageList {
    #[serde(default)]
    messages: Vec<MessageRef>,
    next_page_token: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct MessageRef {
    id: String,
    #[serde(default)]
    label_ids: Vec<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct HistoryPage {
    #[serde(default)]
    history: Vec<HistoryRecord>,
    next_page_token: Option<String>,
    history_id: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct HistoryRecord {
    #[serde(default)]
    messages_added: Vec<MessageAdded>,
}

#[derive(Debug, Default, Deserialize)]
struct MessageAdded {
    message: MessageRef,
}

/// Raw `users.messages.get` result (`format=full`).
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RawMessage {
    #[serde(default)]
    id: String,
    #[serde(default)]
    snippet: String,
    /// Milliseconds since the epoch, as a string.
    internal_date: Option<String>,
    #[serde(default)]
    payload: Part,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Part {
    #[serde(default)]
    mime_type: String,
    #[serde(default)]
    filename: String,
    #[serde(default)]
    headers: Vec<Header>,
    #[serde(default)]
    body: PartBody,
    #[serde(default)]
    parts: Vec<Part>,
}

#[derive(Debug, Default, Deserialize)]
struct Header {
    name: String,
    value: String,
}

#[derive(Debug, Default, Deserialize)]
struct PartBody {
    /// base64url
    data: Option<String>,
}

/// New message ids from `users.history.list`, plus the mailbox's current history id.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoryDelta {
    pub message_ids: Vec<String>,
    pub history_id: Option<String>,
}

/// A message reduced to what the tracker needs.
#[derive(Debug, Clone)]
pub struct MailMessage {
    pub id: String,
    /// Decoded `From` header (`Name <addr>`).
    pub from: String,
    pub subject: String,
    pub date: DateTime<Utc>,
    pub snippet: String,
    /// Plain text (HTML converted), at most [`MAX_BODY_CHARS`] characters.
    pub body: String,
}

// ---------------------------------------------------------------- client

pub struct GmailClient {
    http: reqwest::Client,
    base: String,
    token: String,
}

impl GmailClient {
    pub fn new(access_token: &str) -> Self {
        Self::with_base(API_BASE, access_token)
    }

    pub fn with_base(base: &str, access_token: &str) -> Self {
        let http = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(20))
            .timeout(Duration::from_secs(60))
            .user_agent(concat!("jobpilot/", env!("CARGO_PKG_VERSION")))
            .build()
            .unwrap_or_default();
        Self { http, base: base.trim_end_matches('/').to_string(), token: access_token.to_string() }
    }

    async fn get(&self, path: &str, query: &[(&str, &str)]) -> Result<Value> {
        let resp = self
            .http
            .get(format!("{}/{}", self.base, path))
            .bearer_auth(&self.token)
            .query(query)
            .send()
            .await
            .map_err(|e| anyhow!("cannot reach Gmail: {}", e.without_url()))?;
        let status = resp.status();
        let body = resp.text().await.map_err(|e| anyhow!("reading Gmail's answer failed: {}", e.without_url()))?;
        if status.is_success() {
            return serde_json::from_str(&body).map_err(|_| anyhow!("Gmail did not return JSON (HTTP {status})"));
        }
        let detail = serde_json::from_str::<Value>(&body)
            .ok()
            .and_then(|v| v.pointer("/error/message").and_then(Value::as_str).map(str::to_string))
            .unwrap_or_default();
        let msg = if detail.is_empty() {
            format!("Gmail answered HTTP {}", status.as_u16())
        } else {
            format!("Gmail answered HTTP {}: {detail}", status.as_u16())
        };
        Err(match status.as_u16() {
            404 => ApiError::NotFound(msg).into(),
            401 => ApiError::Unauthorized(msg).into(),
            _ => anyhow!(msg),
        })
    }

    pub async fn profile(&self) -> Result<Profile> {
        Ok(serde_json::from_value(self.get("users/me/profile", &[]).await?)?)
    }

    /// Newest-first message ids matching `q`, at most `cap`.
    pub async fn list_messages(&self, q: &str, cap: usize) -> Result<Vec<String>> {
        let mut ids = Vec::new();
        let mut page: Option<String> = None;
        loop {
            let mut query = vec![("q", q), ("maxResults", "100")];
            if let Some(p) = page.as_deref() {
                query.push(("pageToken", p));
            }
            let list: MessageList = serde_json::from_value(self.get("users/me/messages", &query).await?)?;
            ids.extend(list.messages.into_iter().map(|m| m.id));
            match list.next_page_token {
                Some(p) if ids.len() < cap => page = Some(p),
                _ => break,
            }
        }
        ids.truncate(cap);
        Ok(ids)
    }

    /// Messages added since `start_history_id`. A [`ApiError::NotFound`] means the id expired.
    pub async fn history_added(&self, start_history_id: &str) -> Result<HistoryDelta> {
        let mut pages = Vec::new();
        let mut page: Option<String> = None;
        loop {
            let mut query = vec![
                ("startHistoryId", start_history_id),
                ("historyTypes", "messageAdded"),
                ("maxResults", "500"),
            ];
            if let Some(p) = page.as_deref() {
                query.push(("pageToken", p));
            }
            let h: HistoryPage = serde_json::from_value(self.get("users/me/history", &query).await?)?;
            let next = h.next_page_token.clone();
            pages.push(h);
            match next {
                Some(p) => page = Some(p),
                None => break,
            }
        }
        Ok(collect_history(pages))
    }

    pub async fn get_message(&self, id: &str) -> Result<MailMessage> {
        let v = self.get(&format!("users/me/messages/{id}"), &[("format", "full")]).await?;
        parse_message(serde_json::from_value(v)?)
    }
}

/// Merge history pages into the new message ids (deduplicated, oldest first), skipping mail
/// the user sent or that landed in drafts/spam/trash.
pub(crate) fn collect_history(pages: Vec<HistoryPage>) -> HistoryDelta {
    const SKIP: [&str; 4] = ["SENT", "DRAFT", "SPAM", "TRASH"];
    let mut ids: Vec<String> = Vec::new();
    let mut history_id = None;
    for page in pages {
        if page.history_id.is_some() {
            history_id = page.history_id;
        }
        for record in page.history {
            for added in record.messages_added {
                let m = added.message;
                if m.label_ids.iter().any(|l| SKIP.contains(&l.as_str())) || ids.contains(&m.id) {
                    continue;
                }
                ids.push(m.id);
            }
        }
    }
    HistoryDelta { message_ids: ids, history_id }
}

// ---------------------------------------------------------------- message parsing

pub(crate) fn parse_message(raw: RawMessage) -> Result<MailMessage> {
    anyhow::ensure!(!raw.id.is_empty(), "Gmail returned a message without an id");
    let header = |name: &str| -> String {
        raw.payload
            .headers
            .iter()
            .find(|h| h.name.eq_ignore_ascii_case(name))
            .map(|h| decode_mime_words(h.value.trim()))
            .unwrap_or_default()
    };
    let date = raw
        .internal_date
        .as_deref()
        .and_then(|ms| ms.parse::<i64>().ok())
        .and_then(DateTime::from_timestamp_millis)
        .or_else(|| DateTime::parse_from_rfc2822(header("Date").trim()).ok().map(|d| d.with_timezone(&Utc)))
        .unwrap_or_else(Utc::now);
    Ok(MailMessage {
        from: header("From"),
        subject: header("Subject"),
        date,
        snippet: html_unescape(&raw.snippet),
        body: truncate_chars(&normalize_text(&extract_body(&raw.payload)), MAX_BODY_CHARS),
        id: raw.id,
    })
}

/// Gmail snippets arrive HTML-escaped (`&#39;`, `&amp;`).
fn html_unescape(s: &str) -> String {
    s.replace("&#39;", "'").replace("&quot;", "\"").replace("&lt;", "<").replace("&gt;", ">").replace("&amp;", "&")
}

fn decode_base64url(data: &str) -> Option<Vec<u8>> {
    URL_SAFE_NO_PAD.decode(data.trim().trim_end_matches('=')).ok()
}

fn find_part<'a>(part: &'a Part, mime: &str) -> Option<&'a Part> {
    if part.filename.is_empty() && part.mime_type.eq_ignore_ascii_case(mime) && part.body.data.is_some() {
        return Some(part);
    }
    part.parts.iter().find_map(|p| find_part(p, mime))
}

/// The `text/plain` body; else the `text/html` body converted to text; else empty.
fn extract_body(payload: &Part) -> String {
    if let Some(bytes) = find_part(payload, "text/plain").and_then(|p| decode_base64url(p.body.data.as_deref()?)) {
        return String::from_utf8_lossy(&bytes).into_owned();
    }
    if let Some(bytes) = find_part(payload, "text/html").and_then(|p| decode_base64url(p.body.data.as_deref()?)) {
        return html2text::from_read(&bytes[..], 100).unwrap_or_default();
    }
    String::new()
}

/// Trim lines and collapse runs of blank lines.
fn normalize_text(s: &str) -> String {
    let mut out = String::new();
    let mut blank = 0;
    for line in s.replace('\r', "").lines() {
        let line = line.trim();
        if line.is_empty() {
            blank += 1;
            continue;
        }
        if !out.is_empty() {
            out.push('\n');
            if blank > 0 {
                out.push('\n');
            }
        }
        blank = 0;
        out.push_str(line);
    }
    out
}

pub(crate) fn truncate_chars(s: &str, max: usize) -> String {
    match s.char_indices().nth(max) {
        Some((i, _)) => s[..i].to_string(),
        None => s.to_string(),
    }
}

/// Decode RFC 2047 encoded words (`=?UTF-8?B?...?=`, `=?utf-8?Q?...?=`) in a header value.
pub fn decode_mime_words(value: &str) -> String {
    if !value.contains("=?") {
        return value.to_string();
    }
    let gap = Regex::new(r"(\?=)\s+(=\?)").expect("valid regex");
    let word = Regex::new(r"=\?([^?\s]+)\?([bBqQ])\?([^?\s]*)\?=").expect("valid regex");
    let joined = gap.replace_all(value, "$1$2");
    word.replace_all(&joined, |c: &regex::Captures| {
        let charset = c[1].to_ascii_lowercase();
        let bytes = if c[2].eq_ignore_ascii_case("b") {
            base64::engine::general_purpose::STANDARD_NO_PAD.decode(c[3].trim_end_matches('=')).map_err(|_| ())
        } else {
            decode_q(&c[3])
        };
        match bytes {
            Ok(b) if matches!(charset.as_str(), "iso-8859-1" | "latin1" | "windows-1252" | "us-ascii") => {
                b.iter().map(|&x| x as char).collect::<String>()
            }
            Ok(b) => String::from_utf8_lossy(&b).into_owned(),
            Err(_) => c[0].to_string(),
        }
    })
    .into_owned()
}

fn decode_q(s: &str) -> std::result::Result<Vec<u8>, ()> {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'_' => out.push(b' '),
            b'=' if i + 2 < b.len() => {
                let hex = std::str::from_utf8(&b[i + 1..i + 3]).map_err(|_| ())?;
                out.push(u8::from_str_radix(hex, 16).map_err(|_| ())?);
                i += 2;
            }
            c => out.push(c),
        }
        i += 1;
    }
    Ok(out)
}

/// Split a `From` header into `(display name, address)`.
pub fn parse_from(from: &str) -> (String, String) {
    let from = from.trim();
    match (from.rfind('<'), from.rfind('>')) {
        (Some(l), Some(r)) if l < r => {
            let name = from[..l].trim().trim_matches('"').trim().to_string();
            (name, from[l + 1..r].trim().to_string())
        }
        _ => (String::new(), from.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn b64(s: &str) -> String {
        URL_SAFE_NO_PAD.encode(s)
    }

    fn fixture_message() -> RawMessage {
        let json = serde_json::json!({
            "id": "18c0ffee",
            "threadId": "t1",
            "snippet": "Hi Vinh, Thank you for applying &#39;Backend&#39; &amp; more",
            "internalDate": "1731500000000",
            "payload": {
                "mimeType": "multipart/alternative",
                "headers": [
                    {"name": "FROM", "value": "=?UTF-8?B?QWNtZSBUYWxlbnQ=?= <no-reply@acme.com>"},
                    {"name": "Subject", "value": "=?UTF-8?Q?Your_application_to_Acme_=E2=80=93_update?="},
                    {"name": "Date", "value": "Wed, 13 Nov 2024 12:00:00 +0000"}
                ],
                "body": {"size": 0},
                "parts": [
                    {"mimeType": "text/plain", "filename": "", "headers": [],
                     "body": {"size": 40, "data": b64("Hello Vinh,\r\n\r\n\r\n\r\nUnfortunately we will not move forward.\r\n")}},
                    {"mimeType": "text/html", "filename": "", "headers": [],
                     "body": {"size": 40, "data": b64("<p>HTML <b>version</b></p>")}},
                    {"mimeType": "application/pdf", "filename": "terms.pdf", "headers": [],
                     "body": {"attachmentId": "A1", "size": 9000}}
                ]
            }
        });
        serde_json::from_value(json).unwrap()
    }

    #[test]
    fn parses_headers_and_plain_body() {
        let m = parse_message(fixture_message()).unwrap();
        assert_eq!(m.id, "18c0ffee");
        assert_eq!(m.from, "Acme Talent <no-reply@acme.com>");
        assert_eq!(m.subject, "Your application to Acme \u{2013} update");
        assert_eq!(m.snippet, "Hi Vinh, Thank you for applying 'Backend' & more");
        assert_eq!(m.date.timestamp_millis(), 1731500000000);
        assert_eq!(m.body, "Hello Vinh,\n\nUnfortunately we will not move forward.");
    }

    #[test]
    fn falls_back_to_html_and_date_header() {
        let json = serde_json::json!({
            "id": "m2", "snippet": "",
            "payload": {
                "mimeType": "text/html",
                "headers": [{"name": "Date", "value": "Wed, 13 Nov 2024 12:00:00 +0000"}],
                "body": {"data": b64("<html><body><h1>Interview</h1><p>Are you free <b>Friday</b>?</p></body></html>")}
            }
        });
        let m = parse_message(serde_json::from_value(json).unwrap()).unwrap();
        assert!(m.body.contains("Interview"), "{}", m.body);
        assert!(m.body.contains("Friday"), "{}", m.body);
        assert!(!m.body.contains('<'), "{}", m.body);
        assert_eq!(m.date.timestamp(), 1731499200);
        assert_eq!(m.from, "");
    }

    #[test]
    fn decodes_padded_base64_and_truncates() {
        let long = "x".repeat(MAX_BODY_CHARS + 500);
        let padded = base64::engine::general_purpose::URL_SAFE.encode(&long);
        let json = serde_json::json!({
            "id": "m3",
            "payload": {"mimeType": "text/plain", "body": {"data": padded}}
        });
        let m = parse_message(serde_json::from_value(json).unwrap()).unwrap();
        assert_eq!(m.body.chars().count(), MAX_BODY_CHARS);
        assert_eq!(truncate_chars("h\u{e9}llo", 2), "h\u{e9}");
    }

    #[test]
    fn decodes_mime_words() {
        assert_eq!(decode_mime_words("plain"), "plain");
        assert_eq!(decode_mime_words("=?utf-8?q?Caf=C3=A9_Jobs?="), "Caf\u{e9} Jobs");
        assert_eq!(decode_mime_words("=?UTF-8?B?VGjDtG5nIGLDoW8g?= =?UTF-8?B?dHV54buDbiBk4bulbmc=?="), "Th\u{f4}ng b\u{e1}o tuy\u{1ec3}n d\u{1ee5}ng");
        assert_eq!(decode_mime_words("=?ISO-8859-1?Q?Andr=E9?= Smith"), "Andr\u{e9} Smith");
    }

    #[test]
    fn splits_from_headers() {
        assert_eq!(parse_from("Acme Talent <no-reply@acme.com>"), ("Acme Talent".into(), "no-reply@acme.com".into()));
        assert_eq!(parse_from("\"Doe, Jane\" <jane@x.io>"), ("Doe, Jane".into(), "jane@x.io".into()));
        assert_eq!(parse_from("jobs@acme.com"), (String::new(), "jobs@acme.com".into()));
    }

    #[test]
    fn parses_profile_and_lists() {
        let p: Profile = serde_json::from_str(
            r#"{"emailAddress":"me@gmail.com","messagesTotal":10,"threadsTotal":5,"historyId":"4242"}"#,
        )
        .unwrap();
        assert_eq!(p, Profile { email_address: "me@gmail.com".into(), history_id: "4242".into() });

        let list: MessageList = serde_json::from_str(
            r#"{"messages":[{"id":"a","threadId":"x"},{"id":"b","threadId":"y"}],"nextPageToken":"tok","resultSizeEstimate":2}"#,
        )
        .unwrap();
        assert_eq!(list.messages.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(), ["a", "b"]);
        assert_eq!(list.next_page_token.as_deref(), Some("tok"));

        // An empty mailbox has no `messages` key at all.
        let empty: MessageList = serde_json::from_str(r#"{"resultSizeEstimate":0}"#).unwrap();
        assert!(empty.messages.is_empty());
    }

    #[test]
    fn history_pages_collect_new_inbox_messages() {
        let page1: HistoryPage = serde_json::from_str(
            r#"{"history":[
                {"id":"10","messages":[{"id":"m1"}],"messagesAdded":[{"message":{"id":"m1","labelIds":["INBOX","UNREAD"]}}]},
                {"id":"11","messagesAdded":[{"message":{"id":"m2","labelIds":["SENT"]}}]},
                {"id":"12","messagesAdded":[{"message":{"id":"m3","labelIds":["INBOX"]}}]}
            ],"nextPageToken":"p2","historyId":"900"}"#,
        )
        .unwrap();
        let page2: HistoryPage = serde_json::from_str(
            r#"{"history":[{"id":"13","messagesAdded":[{"message":{"id":"m1","labelIds":["INBOX"]}},{"message":{"id":"m4"}}]}],"historyId":"950"}"#,
        )
        .unwrap();
        let delta = collect_history(vec![page1, page2]);
        assert_eq!(delta.message_ids, ["m1", "m3", "m4"]);
        assert_eq!(delta.history_id.as_deref(), Some("950"));

        // No changes: Gmail returns just the history id.
        let none: HistoryPage = serde_json::from_str(r#"{"historyId":"960"}"#).unwrap();
        assert_eq!(collect_history(vec![none]), HistoryDelta { message_ids: vec![], history_id: Some("960".into()) });
    }
}
