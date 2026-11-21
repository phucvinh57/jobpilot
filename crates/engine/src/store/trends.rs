use std::path::PathBuf;

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use rusqlite::{params, OptionalExtension, Row};

use super::{now, Store};

/// One collected market-trend item (a story, a feed entry, a report), deduplicated by URL.
#[derive(Clone, Debug)]
pub struct TrendItem {
    pub id: i64,
    /// The source that collected it (`None` once that source was deleted).
    pub source_id: Option<i64>,
    pub url: String,
    pub title: String,
    pub published_at: Option<DateTime<Utc>>,
    /// Cleaned text under `<data_dir>/trends/`.
    pub text_path: Option<PathBuf>,
    pub fetched_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Default)]
pub struct NewTrendItem {
    pub source_id: Option<i64>,
    pub url: String,
    pub title: String,
    pub published_at: Option<DateTime<Utc>>,
    pub text_path: Option<PathBuf>,
}

/// A generated digest, one per `(kind, period)` (`kind`: `weekly`, `monthly`, `quarterly`,
/// `yearly`; `period`: `2026-W46`, `2026-M11`, `2026-Q4`, `2026`).
#[derive(Clone, Debug)]
pub struct Report {
    pub id: i64,
    pub kind: String,
    pub period: String,
    pub title: String,
    pub body_md: String,
    /// JSON array of the cited trend items.
    pub citations_json: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Default)]
pub struct NewReport {
    pub kind: String,
    pub period: String,
    pub title: String,
    pub body_md: String,
    pub citations_json: String,
}

const ITEM_COLS: &str = "id, source_id, url, title, published_at, text_path, fetched_at";
const REPORT_COLS: &str = "id, kind, period, title, body_md, citations_json, created_at";

impl TrendItem {
    fn from_row(r: &Row) -> rusqlite::Result<Self> {
        Ok(Self {
            id: r.get(0)?,
            source_id: r.get(1)?,
            url: r.get(2)?,
            title: r.get(3)?,
            published_at: r.get(4)?,
            text_path: r.get::<_, Option<String>>(5)?.map(PathBuf::from),
            fetched_at: r.get(6)?,
        })
    }
}

impl Report {
    fn from_row(r: &Row) -> rusqlite::Result<Self> {
        Ok(Self {
            id: r.get(0)?,
            kind: r.get(1)?,
            period: r.get(2)?,
            title: r.get(3)?,
            body_md: r.get(4)?,
            citations_json: r.get(5)?,
            created_at: r.get(6)?,
        })
    }
}

impl Store {
    /// Store an item unless its URL is already known: `Some(item)` when new, `None` for a duplicate.
    pub fn add_trend_item(&self, new: &NewTrendItem) -> Result<Option<TrendItem>> {
        let id = self.with_conn(|c| {
            let n = c.execute(
                "INSERT INTO trend_items (source_id, url, title, published_at, text_path, fetched_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6) ON CONFLICT(url) DO NOTHING",
                params![
                    new.source_id,
                    new.url,
                    new.title,
                    new.published_at,
                    new.text_path.as_ref().map(|p| p.to_string_lossy().into_owned()),
                    now()
                ],
            )?;
            Ok((n == 1).then(|| c.last_insert_rowid()))
        })?;
        id.map(|id| self.get_trend_item(id)?.context("trend item vanished after insert")).transpose()
    }

    /// Hand the stored item at `new.url` to the source of a followed person: its source, and the
    /// text that source read (a story another source stored with its title alone is the same
    /// address as the interview it links). `false` when there is no such item, or it already
    /// belongs to a person's source (the first one keeps it).
    pub fn claim_trend_item_for_person(&self, new: &NewTrendItem) -> Result<bool> {
        self.with_conn(|c| {
            let n = c.execute(
                "UPDATE trend_items SET source_id = ?1, text_path = COALESCE(?2, text_path)
                 WHERE url = ?3
                   AND (source_id IS NULL OR source_id NOT IN (SELECT id FROM sources WHERE kind = 'person'))",
                params![new.source_id, new.text_path.as_ref().map(|p| p.to_string_lossy().into_owned()), new.url],
            )?;
            Ok(n == 1)
        })
    }

    /// The addresses of the items collected by the sources of followed people.
    pub fn person_item_urls(&self) -> Result<Vec<String>> {
        self.with_conn(|c| {
            let mut stmt =
                c.prepare("SELECT url FROM trend_items WHERE source_id IN (SELECT id FROM sources WHERE kind = 'person')")?;
            let rows = stmt.query_map([], |r| r.get(0))?;
            Ok(rows.collect::<rusqlite::Result<_>>()?)
        })
    }

    pub fn get_trend_item(&self, id: i64) -> Result<Option<TrendItem>> {
        self.with_conn(|c| {
            Ok(c.query_row(&format!("SELECT {ITEM_COLS} FROM trend_items WHERE id = ?1"), [id], TrendItem::from_row)
                .optional()?)
        })
    }

    /// Items published (or, without a date, fetched) at or after `since`, newest first. `None` = all.
    pub fn list_trend_items(&self, since: Option<DateTime<Utc>>) -> Result<Vec<TrendItem>> {
        self.with_conn(|c| {
            let mut stmt = c.prepare(&format!(
                "SELECT {ITEM_COLS} FROM trend_items
                 WHERE ?1 IS NULL OR COALESCE(published_at, fetched_at) >= ?1
                 ORDER BY COALESCE(published_at, fetched_at) DESC, id DESC"
            ))?;
            let rows = stmt.query_map([since], TrendItem::from_row)?;
            Ok(rows.collect::<rusqlite::Result<_>>()?)
        })
    }

    /// Record where the item's text was saved.
    pub fn set_trend_item_text(&self, id: i64, text_path: &std::path::Path) -> Result<()> {
        self.with_conn(|c| {
            let n = c.execute("UPDATE trend_items SET text_path = ?1 WHERE id = ?2", params![text_path.to_string_lossy(), id])?;
            anyhow::ensure!(n == 1, "trend item {id} not found");
            Ok(())
        })
    }

    /// Save the report of `(kind, period)`: a new row, or the existing one replaced in place (its
    /// id stays the same; `created_at` becomes now).
    pub fn add_report(&self, new: &NewReport) -> Result<Report> {
        let citations = if new.citations_json.is_empty() { "[]" } else { &new.citations_json };
        let id: i64 = self.with_conn(|c| {
            Ok(c.query_row(
                "INSERT INTO reports (kind, period, title, body_md, citations_json, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                 ON CONFLICT(kind, period) WHERE period != '' DO UPDATE SET title = excluded.title, body_md = excluded.body_md,
                     citations_json = excluded.citations_json, created_at = excluded.created_at
                 RETURNING id",
                params![new.kind, new.period, new.title, new.body_md, citations, now()],
                |row| row.get(0),
            )?)
        })?;
        self.get_report(id)?
        .context("report vanished after insert")
    }

    /// Delete a report; `false` when there was none with that id.
    pub fn delete_report(&self, id: i64) -> Result<bool> {
        self.with_conn(|c| Ok(c.execute("DELETE FROM reports WHERE id = ?1", [id])? == 1))
    }

    pub fn get_report(&self, id: i64) -> Result<Option<Report>> {
        self.with_conn(|c| {
            Ok(c.query_row(&format!("SELECT {REPORT_COLS} FROM reports WHERE id = ?1"), [id], Report::from_row)
                .optional()?)
        })
    }

    /// The newest report of `kind`.
    pub fn latest_report(&self, kind: &str) -> Result<Option<Report>> {
        self.with_conn(|c| {
            Ok(c.query_row(
                &format!("SELECT {REPORT_COLS} FROM reports WHERE kind = ?1 ORDER BY id DESC LIMIT 1"),
                [kind],
                Report::from_row,
            )
            .optional()?)
        })
    }

    /// Past reports of `kind`, newest first.
    pub fn list_reports(&self, kind: &str) -> Result<Vec<Report>> {
        self.with_conn(|c| {
            let mut stmt =
                c.prepare(&format!("SELECT {REPORT_COLS} FROM reports WHERE kind = ?1 ORDER BY id DESC"))?;
            let rows = stmt.query_map([kind], Report::from_row)?;
            Ok(rows.collect::<rusqlite::Result<_>>()?)
        })
    }
}

#[cfg(test)]
mod tests {
    use chrono::Duration;

    use super::*;
    use crate::store::{NewSource, SourcePurpose};

    fn item(url: &str, published: Option<DateTime<Utc>>) -> NewTrendItem {
        NewTrendItem { url: url.into(), title: format!("title of {url}"), published_at: published, ..Default::default() }
    }

    #[test]
    fn items_are_deduplicated_by_url_and_listed_by_period() {
        let store = Store::open_in_memory().unwrap();
        let src = store.add_source(&NewSource { kind: "rss".into(), purpose: SourcePurpose::Trends, ..Default::default() }).unwrap();
        let old = store.add_trend_item(&item("https://x.test/old", Some(now() - Duration::days(40)))).unwrap().unwrap();
        let recent = store
            .add_trend_item(&NewTrendItem { source_id: Some(src.id), ..item("https://x.test/recent", Some(now() - Duration::days(2))) })
            .unwrap()
            .unwrap();
        let undated = store.add_trend_item(&item("https://x.test/undated", None)).unwrap().unwrap();
        assert_eq!(recent.source_id, Some(src.id));

        assert!(store.add_trend_item(&item("https://x.test/recent", None)).unwrap().is_none(), "duplicate URL");
        assert_eq!(store.list_trend_items(None).unwrap().len(), 3);

        let week = store.list_trend_items(Some(now() - Duration::days(7))).unwrap();
        assert_eq!(week.iter().map(|i| i.id).collect::<Vec<_>>(), [undated.id, recent.id], "newest first; undated by fetch time");
        assert!(!week.iter().any(|i| i.id == old.id));

        store.set_trend_item_text(recent.id, std::path::Path::new("/tmp/trends/1.txt")).unwrap();
        assert_eq!(store.get_trend_item(recent.id).unwrap().unwrap().text_path.as_deref(), Some(std::path::Path::new("/tmp/trends/1.txt")));
        assert!(store.set_trend_item_text(999, std::path::Path::new("/x")).is_err());

        store.delete_source(src.id).unwrap();
        assert_eq!(store.get_trend_item(recent.id).unwrap().unwrap().source_id, None, "items outlive their source");
    }

    #[test]
    fn a_person_source_claims_a_stored_item_once() {
        let store = Store::open_in_memory().unwrap();
        let source = |kind: &str| store.add_source(&NewSource { kind: kind.into(), purpose: SourcePurpose::Trends, ..Default::default() }).unwrap();
        let (hn, ada, bob) = (source("hackernews"), source("person"), source("person"));
        let story = store.add_trend_item(&NewTrendItem { source_id: Some(hn.id), ..item("https://x.test/interview", None) }).unwrap().unwrap();
        assert!(store.person_item_urls().unwrap().is_empty());

        let path = std::path::PathBuf::from("/tmp/trends/full.txt");
        let claim = |source: i64| NewTrendItem { source_id: Some(source), text_path: Some(path.clone()), ..item("https://x.test/interview", None) };
        assert!(store.claim_trend_item_for_person(&claim(ada.id)).unwrap());
        let claimed = store.get_trend_item(story.id).unwrap().unwrap();
        assert_eq!((claimed.source_id, claimed.text_path.as_deref()), (Some(ada.id), Some(path.as_path())));
        assert_eq!(claimed.title, story.title, "the title it was stored under stays");
        assert_eq!(store.person_item_urls().unwrap(), ["https://x.test/interview"]);

        assert!(!store.claim_trend_item_for_person(&claim(bob.id)).unwrap(), "the first person keeps it");
        assert!(!store.claim_trend_item_for_person(&NewTrendItem { source_id: Some(ada.id), ..item("https://x.test/none", None) }).unwrap());
    }

    #[test]
    fn latest_report_per_kind() {
        let store = Store::open_in_memory().unwrap();
        assert!(store.latest_report("weekly").unwrap().is_none());
        let report = |kind: &str, period: &str, body: &str| NewReport {
            kind: kind.into(),
            period: period.into(),
            title: format!("{kind} {period}"),
            body_md: body.into(),
            citations_json: String::new(),
        };
        let w1 = store.add_report(&report("weekly", "2026-W45", "first")).unwrap();
        let w2 = store.add_report(&report("weekly", "2026-W46", "second")).unwrap();
        store.add_report(&report("quarterly", "2026-Q3", "q")).unwrap();
        assert_eq!(w1.citations_json, "[]", "empty citations become an empty list");

        assert_eq!(store.latest_report("weekly").unwrap().unwrap().id, w2.id);
        assert_eq!(store.latest_report("quarterly").unwrap().unwrap().period, "2026-Q3");
        assert_eq!(store.list_reports("weekly").unwrap().iter().map(|r| r.id).collect::<Vec<_>>(), [w2.id, w1.id]);
        assert_eq!(store.get_report(w1.id).unwrap().unwrap().body_md, "first");
        assert!(store.get_report(999).unwrap().is_none());
    }

    #[test]
    fn rerunning_a_period_replaces_its_report_in_place() {
        let store = Store::open_in_memory().unwrap();
        let report = |kind: &str, period: &str, body: &str| NewReport {
            kind: kind.into(),
            period: period.into(),
            title: format!("{kind} {period}"),
            body_md: body.into(),
            citations_json: "{}".into(),
        };
        let first = store.add_report(&report("monthly", "2026-M11", "first")).unwrap();
        let other = store.add_report(&report("yearly", "2026", "year")).unwrap();
        let again = store.add_report(&report("monthly", "2026-M11", "second")).unwrap();
        assert_eq!(again.id, first.id, "same period, same id");
        assert_eq!(again.body_md, "second");
        assert!(again.created_at >= first.created_at);
        assert_eq!(store.list_reports("monthly").unwrap().len(), 1);
        assert_eq!(store.get_report(first.id).unwrap().unwrap().body_md, "second");

        assert!(store.delete_report(first.id).unwrap());
        assert!(!store.delete_report(first.id).unwrap(), "already gone");
        assert!(store.get_report(first.id).unwrap().is_none());
        assert_eq!(store.get_report(other.id).unwrap().unwrap().body_md, "year");
    }
}
