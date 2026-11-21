//! What is said in a YouTube video: its transcript, for an interview or a talk that a story or a
//! feed entry links.
//!
//! The site gives captions to its own player only, so the transcript is read from the video's
//! page in a private tab of the user's Chrome (`browser::transcript`), never over plain HTTP and
//! never by posing as the site's app. That needs the JobPilot extension connected. When the
//! browser cannot be used, or the site shows a consent, login or robot check, nothing is got
//! past: the video is left for a later run ([`Read::Later`]) and no further tab is opened in this
//! run. A video without a transcript keeps its headline ([`Read::NoTranscript`]).
//!
//! The address opened is always `<root>/watch?v=<id>`, built here from the video's id: a link
//! never chooses the page.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::browser::transcript::{video_transcript, Walled};
use crate::browser::{Bridge, BridgeError, Policy};
use crate::trends::text::cap;

use super::Ctx;

/// The reader of a run's videos. Cheap to clone.
#[derive(Clone)]
pub struct Videos {
    bridge: Bridge,
    /// Set once the browser or the site refused: the rest of the run opens no tab.
    off: Arc<AtomicBool>,
}

impl Videos {
    pub fn new(bridge: Bridge) -> Self {
        Self { bridge, off: Arc::new(AtomicBool::new(false)) }
    }
}

/// What reading a video gave.
#[derive(Debug, PartialEq, Eq)]
pub enum Read {
    /// The transcript (at most `max_text_chars`).
    Text(String),
    /// The video has no transcript: its headline and link are what there is.
    NoTranscript,
    /// It could not be read now (no browser, a wall, a page that did not load): try another run.
    Later,
}

/// The id of the video a YouTube link points at (`watch?v=`, `youtu.be/`, `/live/`, `/shorts/`,
/// `/embed/`), or `None` for any other address (a channel, a playlist, another site).
pub fn video_id(url: &str) -> Option<String> {
    let url = url::Url::parse(url).ok()?;
    let host = url.host_str()?.to_ascii_lowercase();
    let host = ["www.", "m.", "music."].iter().find_map(|p| host.strip_prefix(p)).unwrap_or(&host);
    let mut segments = url.path_segments()?;
    let id = match host {
        "youtu.be" => segments.next().map(str::to_string),
        "youtube.com" | "youtube-nocookie.com" => match segments.next()? {
            "watch" => url.query_pairs().find(|(k, _)| k == "v").map(|(_, v)| v.into_owned()),
            "live" | "shorts" | "embed" | "v" => segments.next().map(str::to_string),
            _ => None,
        },
        _ => None,
    }?;
    (id.len() == 11 && id != "videoseries" && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')).then_some(id)
}

/// The share of the source's time after which no video is started, and what a read needs at
/// least: a video is only started when it can finish inside the source's time.
const START_SHARE: f64 = 2.0 / 3.0;
const MIN_READ: Duration = Duration::from_secs(20);
const MARGIN: Duration = Duration::from_secs(5);

/// Read the transcript of the video `id` (see the module docs). `started` is when the source
/// began: the read is bounded by what is left of the source's time, and not started
/// ([`Read::Later`]) when too little is left.
pub async fn transcript(ctx: &Ctx, id: &str, started: Instant) -> Read {
    let Some(videos) = &ctx.videos else { return Read::Later };
    if videos.off.load(Ordering::Relaxed) {
        return Read::Later;
    }
    let total = ctx.limits().source_timeout;
    let left = total.saturating_sub(started.elapsed()).saturating_sub(MARGIN);
    if started.elapsed() >= total.mul_f64(START_SHARE) || left < MIN_READ {
        return Read::Later;
    }
    let url = format!("{}/watch?v={id}", ctx.endpoints().youtube.trim_end_matches('/'));
    let policy = Policy::new(ctx.limits().allow_private_hosts);
    match video_transcript(&videos.bridge, &url, policy, left).await {
        Ok(Some(text)) => Read::Text(cap(&text, ctx.limits().max_text_chars)),
        Ok(None) => Read::NoTranscript,
        Err(e) => {
            // A browser that cannot be used, or a wall, holds for every video: no further tab is
            // opened in this run. Anything else is this video's own trouble. The error never
            // names the page.
            if e.downcast_ref::<BridgeError>().is_some() || e.downcast_ref::<Walled>().is_some() {
                videos.off.store(true, Ordering::Relaxed);
                tracing::warn!("video transcripts cannot be read in this run; the videos are left for a later one: {e:#}");
            } else {
                tracing::warn!("a video's transcript could not be read; it is left for a later run: {e:#}");
            }
            Read::Later
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::trends::testutil::{ctx, Fixtures};

    #[test]
    fn a_video_link_gives_its_id_and_other_addresses_none() {
        for url in [
            "https://www.youtube.com/watch?v=cdiD-9MMpb0",
            "https://youtube.com/watch?t=120&v=cdiD-9MMpb0&feature=share",
            "https://m.youtube.com/watch?v=cdiD-9MMpb0",
            "https://youtu.be/cdiD-9MMpb0?si=abc",
            "https://www.youtube.com/live/cdiD-9MMpb0",
            "https://www.youtube.com/shorts/cdiD-9MMpb0",
            "https://www.youtube-nocookie.com/embed/cdiD-9MMpb0",
            "https://music.youtube.com/watch?v=cdiD-9MMpb0",
        ] {
            assert_eq!(video_id(url).as_deref(), Some("cdiD-9MMpb0"), "{url}");
        }
        for url in [
            "https://www.youtube.com/@channel",
            "https://www.youtube.com/playlist?list=PL123",
            "https://www.youtube.com/watch?v=short",
            "https://www.youtube.com/embed/videoseries?list=PL123",
            "https://www.youtube.com/watch?v=cdiD-9MMpb0%2F..",
            "https://youtube.com.evil.test/watch?v=cdiD-9MMpb0",
            "https://example.com/watch?v=cdiD-9MMpb0",
            "not a url",
        ] {
            assert_eq!(video_id(url), None, "{url}");
        }
    }

    #[tokio::test]
    async fn without_a_usable_browser_a_video_is_left_for_later_and_asked_once() {
        let fx = Fixtures::start().await;
        let mut c = ctx(&fx, &[]);
        assert_eq!(transcript(&c, "cdiD-9MMpb0", Instant::now()).await, Read::Later, "no browser at all");

        // A bridge nothing is connected to: refused once, then not asked again in this run.
        let videos = Videos::new(Bridge::new(crate::Secrets::memory()));
        c.videos = Some(videos.clone());
        assert!(!videos.off.load(Ordering::Relaxed));
        assert_eq!(transcript(&c, "cdiD-9MMpb0", Instant::now()).await, Read::Later);
        assert!(videos.off.load(Ordering::Relaxed));
        assert_eq!(transcript(&c, "cdiD-9MMpb0", Instant::now()).await, Read::Later);
        assert!(fx.hits().is_empty(), "a video is never asked for over plain HTTP");

        // Too little of the source's time left: not started, and the browser is not blamed.
        let fresh = Videos::new(Bridge::new(crate::Secrets::memory()));
        c.videos = Some(fresh.clone());
        let mut config = (*c.config).clone();
        config.limits.source_timeout = Duration::from_secs(10);
        c.config = Arc::new(config);
        assert_eq!(transcript(&c, "cdiD-9MMpb0", Instant::now()).await, Read::Later);
        assert!(!fresh.off.load(Ordering::Relaxed));
    }
}
