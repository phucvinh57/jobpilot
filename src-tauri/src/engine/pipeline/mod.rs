//! JD ingest, CV generation, fact guard and Typst rendering.
//!
//! ```text
//! // UI side (inside runtime::spawn):
//! let app = pipeline::generate(&core, JdInput::Url(url), |p| tx.send(p)).await?;   // Draft + cv.yaml + cv.pdf
//! pipeline::revise(&core, app.id, "shorter, stress Rust", |p| ..).await?;           // "Ask AI"
//! let out = pipeline::rerender(&core, app.id).await?;                                // after manual YAML edits
//! ```
//!
//! Files per application (`Application::dir`): `jd.md`, `cv.yaml`, `cv.pdf`, `page-<n>.png`.
//!
//! Layers, each usable on its own:
//! - [`ingest`]: URL / file / text -> JD text ([`NeedsBrowser`] when a plain fetch is not enough).
//! - [`analyze`]: JD -> [`Analysis`] (LLM step 1).
//! - [`insight`]: JD + the company's pages -> [`Insight`], a brief per job (LLM, optional, started in the background next to step 2).
//! - [`tailor`]: profile + JD -> CV YAML, plus revise / repair / trim rounds (LLM step 2).
//! - [`cv`] + [`guard`]: typed CV and the deterministic fact guard.
//! - [`render`]: YAML -> PDF + PNG previews with the `typst` CLI.
//! - [`generate`]: the orchestrator ([`Pipeline`]) with progress events.

pub mod analyze;
pub mod cv;
pub mod generate;
pub mod guard;
pub mod ingest;
pub mod insight;
pub mod render;
pub mod tailor;

pub use analyze::Analysis;
pub use cv::Cv;
pub use generate::{
    generate, generate_into, read_jd_text, regenerate, require_open, rerender, revise, GuardFailed, Pipeline, Progress,
    RerenderOutcome, Stage, MAX_PAGES_KEY,
};
pub use guard::{guard, Violation};
pub use ingest::{ingest, ingest_html, JdInput, JdText, NeedsBrowser};
pub use insight::Insight;
pub use render::{render, RenderResult, TypstError};
pub use tailor::TailorOutput;
