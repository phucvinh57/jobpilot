//! The transcript of a video, read from the video's own page in a tab of the user's Chrome.
//!
//! A video site hands its captions to its own player only: a plain HTTP client gets an empty
//! answer unless it presents itself as the site's app, which JobPilot never does. So the page is
//! opened like any other rendered page: a **private**, guarded, background tab, not disguised
//! (`navigator.webdriver` stays true). The script opens the page's transcript panel the way a
//! reader would (the description's "Show transcript" button) and returns the lines it shows. The
//! tab is closed whatever happened.
//!
//! Nothing here gets past a wall: a consent, login, captcha or block page instead of the video is
//! a [`Walled`] error, and the caller leaves the video for a later run. A video without a
//! transcript is `Ok(None)`, and only when the video's page was really shown: a page that did not
//! load, a player that shows a message instead of the video, or a tab that went away is an error,
//! so the video is read again later instead of being taken for one without a transcript.

use std::time::Duration;

use anyhow::{Context, Result};
use serde::Deserialize;

use super::bridge::Bridge;
use super::challenge;
use super::fetch::{timed_out, CloseOnDrop, LOAD_TIMEOUT, RENDER_TIMEOUT};
use super::guard::{allows, Policy};
use super::tab::{OpenOptions, Profile, Tab};
use crate::graph::nodes::ChallengeKind;
use crate::net::http::NotPublic;

/// How long the page may take to show its transcript button once loaded. A page still without
/// one has no transcript.
const BUTTON_TIMEOUT: Duration = Duration::from_secs(12);
/// How long the panel may take to show its lines once the button was pressed.
const LINES_TIMEOUT: Duration = Duration::from_secs(15);
const POLL: Duration = Duration::from_millis(500);
/// Steps that may fail in one read (the page navigating under the script).
const MAX_FAILED_STEPS: usize = 3;

/// A consent, login, captcha or block page was shown instead of the video.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Walled(pub ChallengeKind);

impl std::fmt::Display for Walled {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("the video site showed a consent, login or robot check instead of the video")
    }
}

impl std::error::Error for Walled {}

/// One step on the page, safe to repeat: the transcript's lines when the panel shows them, else
/// press what opens it (the description's expander, then its transcript button; each once). The
/// video is muted and paused at every step: nothing is played to the user.
///
/// The selectors: the transcript section of the description is found by its element, not by its
/// label, so the page's language does not matter. A line is a segment of the panel (the current
/// markup, then the older one); when neither matches, the panel's own text is taken and the
/// timestamps are dropped in Rust.
const STEP_JS: &str = r#"(() => {
  const state = (window.__jobpilotTranscript ||= { expanded: false, pressed: false });
  document.querySelectorAll('video').forEach(v => { v.muted = true; if (!v.paused) v.pause(); });
  const texts = sel => Array.from(document.querySelectorAll(sel)).map(e => (e.innerText || e.textContent || '').trim()).filter(Boolean);
  let lines = texts('transcript-segment-view-model span[role="text"]');
  if (!lines.length) lines = texts('ytd-transcript-segment-renderer .segment-text');
  if (!lines.length && state.pressed) {
    const panel = Array.from(document.querySelectorAll('ytd-engagement-panel-section-list-renderer'))
      .find(e => (e.getAttribute('target-id') || '').includes('transcript') && e.getAttribute('visibility') !== 'ENGAGEMENT_PANEL_VISIBILITY_HIDDEN');
    const body = panel && panel.querySelector('#content, #body');
    const rows = body ? (body.innerText || '').split('\n').map(l => l.trim()).filter(Boolean) : [];
    // Rows of an unknown markup still pair a timestamp with what was said; a message does not.
    if (rows.filter(l => /^\d{1,2}(:\d{2}){1,2}$/.test(l)).length >= 2) lines = rows;
  }
  if (lines.length) return JSON.stringify({ lines });
  if (!state.pressed) {
    if (!state.expanded) {
      const expand = document.querySelector('#description-inline-expander #expand, ytd-text-inline-expander #expand');
      if (expand) { expand.click(); state.expanded = true; }
    }
    const button = document.querySelector('ytd-video-description-transcript-section-renderer button');
    if (button) { button.click(); state.pressed = true; }
  }
  const shown = e => { const r = e.getBoundingClientRect(); return r.width > 0 && r.height > 0; };
  return JSON.stringify({
    pressed: state.pressed,
    described: !!document.querySelector('#description-inline-expander, ytd-text-inline-expander'),
    refused: Array.from(document.querySelectorAll('yt-playability-error-supported-renderers, ytd-enforcement-message-view-model, .ytp-error')).some(shown),
  });
})()"#;

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Step {
    lines: Vec<String>,
    pressed: bool,
    /// The page drew the video's description: it is the video's page, loaded.
    described: bool,
    /// The player shows a message instead of the video (unavailable, a sign-in or robot check).
    refused: bool,
}

/// The transcript of the video at `url` (its watch page), or `None` when the page offers none.
///
/// `Err` holds a [`BridgeError`](super::BridgeError) when the browser cannot be used (the
/// extension is not connected, or not allowed in Incognito) and a [`Walled`] when the site showed
/// a wall. The error text never contains `url`.
///
/// The read takes at most `limit` (and never more than a render may).
pub async fn video_transcript(bridge: &Bridge, url: &str, policy: Policy, limit: Duration) -> Result<Option<String>> {
    let parsed = url::Url::parse(url).context("the video address is not valid")?;
    if !allows(&policy, &parsed).await {
        return Err(NotPublic.into());
    }
    let opened = bridge.open("about:blank", OpenOptions { profile: Profile::Private, focus: false, guard: Some(policy) }).await;
    let tab = CloseOnDrop(opened.context("opening a browser tab")?);
    let result = tokio::time::timeout(limit.min(RENDER_TIMEOUT), read(&tab.0, &parsed)).await.unwrap_or_else(|_| Err(timed_out()));
    tab.0.close().await;
    result
}

async fn read(tab: &Tab, url: &url::Url) -> Result<Option<String>> {
    // Chrome's text for a failed navigation names the page: it is not passed on.
    if tab.goto(url.as_str()).await.is_err() {
        anyhow::bail!("the video's page could not be opened");
    }
    tab.wait_loaded(LOAD_TIMEOUT).await;
    if let Some(kind) = challenge::detect_at(tab, Some(url.as_str())).await {
        return Err(Walled(kind).into());
    }
    // A consent or robot check lives on another host than the video.
    let landed = tab.url().await.ok().and_then(|u| url::Url::parse(&u).ok());
    if landed.is_some_and(|l| l.host_str() != url.host_str()) {
        return Err(Walled(ChallengeKind::Blocked).into());
    }

    let started = tokio::time::Instant::now();
    let mut pressed_at = None;
    let mut last = 0;
    let mut failed = 0;
    loop {
        let step: Step = match tab.eval_as::<String>(STEP_JS).await {
            Ok(json) => serde_json::from_str(&json).unwrap_or_default(),
            // The page may navigate under the script once or twice; a tab that is gone, or a
            // browser that stopped answering, is not "no transcript".
            Err(e) => {
                failed += 1;
                if failed > MAX_FAILED_STEPS || !tab.is_open() {
                    return Err(e.context("reading the video's page"));
                }
                Step::default()
            }
        };
        // The panel fills in one go; a count that held for one poll is the whole transcript.
        if !step.lines.is_empty() && step.lines.len() == last {
            return Ok(text_of(&step.lines));
        }
        last = step.lines.len();
        if step.refused && !step.pressed {
            anyhow::bail!("the video site shows a message instead of the video");
        }
        if step.pressed && pressed_at.is_none() {
            pressed_at = Some(tokio::time::Instant::now());
        }
        let waited_out = match pressed_at {
            Some(at) => at.elapsed() >= LINES_TIMEOUT,
            None => started.elapsed() >= BUTTON_TIMEOUT,
        };
        if waited_out && step.lines.is_empty() {
            // "No transcript" needs the video's page in front of us: its description drawn, and
            // no transcript section in it. Anything else is a page that did not load.
            anyhow::ensure!(step.described, "the video's page did not finish loading");
            return Ok(None);
        }
        tokio::time::sleep(POLL).await;
    }
}

/// The spoken text: the lines joined, without timestamps (`0:00`, `1:02:03`) and chapter marks.
fn text_of(lines: &[String]) -> Option<String> {
    let is_time = |l: &str| !l.is_empty() && l.split(':').count() >= 2 && l.split(':').all(|p| !p.is_empty() && p.len() <= 2 && p.bytes().all(|b| b.is_ascii_digit()));
    let spoken: Vec<&str> = lines.iter().flat_map(|l| l.lines()).map(str::trim).filter(|l| !l.is_empty() && !is_time(l)).collect();
    let text = spoken.join(" ").split_whitespace().collect::<Vec<_>>().join(" ");
    (!text.is_empty()).then_some(text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::browser::BridgeError;

    #[test]
    fn the_text_is_the_spoken_lines_without_timestamps() {
        let lines: Vec<String> = ["0:00", "I think every company", "0:04", " will run an AI factory ", "1:02:03", "by 2030: that is the plan", "Chapter 2\n12:30\nand then some"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert_eq!(text_of(&lines).unwrap(), "I think every company will run an AI factory by 2030: that is the plan Chapter 2 and then some");
        assert_eq!(text_of(&["0:00".to_string(), " ".to_string()]), None);
    }

    #[tokio::test]
    async fn a_private_address_is_refused_and_a_browser_that_is_not_connected_says_so() {
        let bridge = Bridge::new(crate::Secrets::memory());
        let err = video_transcript(&bridge, "http://127.0.0.1:9/watch?v=x", Policy::Public, RENDER_TIMEOUT).await.unwrap_err();
        assert!(err.downcast_ref::<NotPublic>().is_some(), "{err:#}");
        let err = video_transcript(&bridge, "https://video.example/watch?v=x", Policy::Anything, RENDER_TIMEOUT).await.unwrap_err();
        assert_eq!(err.downcast_ref::<BridgeError>(), Some(&BridgeError::NotConnected), "{err:#}");
        assert!(!format!("{err:#}").contains("video.example"), "{err:#}");
    }

    const WORDS: &str = "I think every company will run an AI factory within five years.";

    /// A page shaped like the video site's: the transcript button sits in the collapsed
    /// description, and the panel fills a moment after the button is pressed.
    fn video_page(segment: &str) -> String {
        format!(
            r#"<html><head><title>Ada Example at the summit</title></head><body><h1>Ada Example at the summit</h1><p>{filler}</p>
<div id="description-inline-expander"><button id="expand">...more</button><div id="slot"></div></div>
<ytd-engagement-panel-section-list-renderer target-id="engagement-panel-searchable-transcript" visibility="ENGAGEMENT_PANEL_VISIBILITY_HIDDEN"><div id="content"></div></ytd-engagement-panel-section-list-renderer>
<script>
document.getElementById('expand').addEventListener('click', () => {{
  document.getElementById('slot').innerHTML = '<ytd-video-description-transcript-section-renderer><button id="show">Transkript anzeigen</button></ytd-video-description-transcript-section-renderer>';
  document.getElementById('show').addEventListener('click', () => setTimeout(() => {{
    const panel = document.querySelector('ytd-engagement-panel-section-list-renderer');
    panel.setAttribute('visibility', 'ENGAGEMENT_PANEL_VISIBILITY_EXPANDED');
    panel.querySelector('#content').innerHTML = {segment};
  }}, 700));
}});
</script></body></html>"#,
            filler = "A talk about what comes next. ".repeat(40),
            segment = serde_json::to_string(segment).unwrap(),
        )
    }

    /// `JOBPILOT_NO_SANDBOX=1 cargo test -p jobpilot-engine -- --ignored the_transcript_panel` (after `bun run ext:build`).
    #[tokio::test]
    #[ignore = "launches Chromium with the extension"]
    async fn the_transcript_panel_is_opened_and_read_in_a_private_tab() {
        use crate::pipeline::ingest::tests::serve;
        let Some(browser) = crate::browser::bridge::testkit::launch_incognito().await else { return };
        let line = |markup: &dyn Fn(&str, &str) -> String| format!("{}{}", markup("0:00", WORDS), markup("0:07", "Engineers will direct agents."));
        let pages = [
            // The current markup, the older one, and a panel whose markup is unknown.
            line(&|t, w| format!("<transcript-segment-view-model><div class=ytwTranscriptSegmentViewModelTimestamp>{t}</div><span role=text>{w}</span></transcript-segment-view-model>")),
            line(&|t, w| format!("<ytd-transcript-segment-renderer><div class=segment-timestamp>{t}</div><yt-formatted-string class=segment-text>{w}</yt-formatted-string></ytd-transcript-segment-renderer>")),
            line(&|t, w| format!("<div class=row><div>{t}</div><div>{w}</div></div>")),
        ];
        for page in pages {
            let url = serve(200, "text/html", video_page(&page)).await.replace("127.0.0.1", "video.test");
            let text = video_transcript(&browser.bridge, &url, Policy::Anything, RENDER_TIMEOUT).await.unwrap();
            assert_eq!(text.as_deref(), Some(format!("{WORDS} Engineers will direct agents.").as_str()));
            assert!(browser.bridge.shared.state().tabs.is_empty(), "the tab was closed");
        }

        // A sign-in wall instead of the video is not read, and not got past.
        let wall = "<html><head><title>Sign in</title></head><body><form><input name=u><input type=password name=p></form></body></html>";
        let url = serve(200, "text/html", wall.into()).await.replace("127.0.0.1", "video.test");
        let err = video_transcript(&browser.bridge, &url, Policy::Anything, RENDER_TIMEOUT).await.unwrap_err();
        assert_eq!(err.downcast_ref::<Walled>(), Some(&Walled(ChallengeKind::Login)), "{err:#}");
    }

    /// A video without captions has no transcript button: `None`, after the short wait.
    #[tokio::test]
    #[ignore = "launches Chromium with the extension"]
    async fn a_video_without_a_transcript_button_has_none() {
        use crate::pipeline::ingest::tests::serve;
        let Some(browser) = crate::browser::bridge::testkit::launch_incognito().await else { return };
        let page = |body: &str| format!("<html><head><title>A talk</title></head><body><h1>A talk</h1><p>{}</p>{body}</body></html>", "No captions here. ".repeat(60));
        let described = page("<div id=description-inline-expander><button id=expand>...more</button></div>");
        let url = serve(200, "text/html", described).await.replace("127.0.0.1", "video.test");
        assert_eq!(video_transcript(&browser.bridge, &url, Policy::Anything, RENDER_TIMEOUT).await.unwrap(), None);

        // A page that never drew the video's description, a player with a message instead of the
        // video, and a panel that shows a message are not "no transcript": the video is read later.
        let message = "<div id=description-inline-expander></div><div class=ytp-error style='width:300px;height:100px'>Sign in to confirm you are not a bot</div>";
        for body in ["", message] {
            let url = serve(200, "text/html", page(body)).await.replace("127.0.0.1", "video.test");
            assert!(video_transcript(&browser.bridge, &url, Policy::Anything, RENDER_TIMEOUT).await.is_err(), "{body}");
        }
        let url = serve(200, "text/html", video_page("<p>Transcript unavailable</p>")).await.replace("127.0.0.1", "video.test");
        assert_eq!(video_transcript(&browser.bridge, &url, Policy::Anything, RENDER_TIMEOUT).await.unwrap(), None, "a message in the panel is not what was said");
    }
}
