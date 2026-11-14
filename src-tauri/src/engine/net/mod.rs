//! Network plumbing shared by the parts that read third-party data: [`http`] is the one safe
//! HTTP client (public hosts only, per-host delay, size cap, hop-by-hop redirects).

pub mod http;

#[cfg(test)]
pub(crate) mod fixtures;
