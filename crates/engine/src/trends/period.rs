//! The period a trend run covers: an ISO week (`2026-W46`, weekly digest) or a quarter
//! (`2026-Q4`, quarterly summary). It names the graph thread (`trends:<period>`) and the
//! report, and bounds the queries built from the preferences.

use chrono::{DateTime, Datelike, NaiveDate, TimeZone, Utc, Weekday};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Period {
    /// ISO week `week` (1-53) of ISO year `year`.
    Week { year: i32, week: u32 },
    /// Calendar quarter `quarter` (1-4).
    Quarter { year: i32, quarter: u32 },
}

impl Period {
    /// The ISO week containing `date`.
    pub fn week_of(date: NaiveDate) -> Self {
        let iso = date.iso_week();
        Period::Week { year: iso.year(), week: iso.week() }
    }

    /// The quarter containing `date`.
    pub fn quarter_of(date: NaiveDate) -> Self {
        Period::Quarter { year: date.year(), quarter: (date.month() - 1) / 3 + 1 }
    }

    /// The current ISO week (UTC).
    pub fn current_week() -> Self {
        Self::week_of(Utc::now().date_naive())
    }

    /// `2026-W46` / `2026-Q4`, as used in thread ids and `reports.period`.
    pub fn label(&self) -> String {
        match *self {
            Period::Week { year, week } => format!("{year}-W{week:02}"),
            Period::Quarter { year, quarter } => format!("{year}-Q{quarter}"),
        }
    }

    /// Inverse of [`label`](Self::label); `None` for anything else (including a week or quarter
    /// that does not exist).
    pub fn parse(label: &str) -> Option<Self> {
        let (year, rest) = label.trim().split_once('-')?;
        let year: i32 = year.parse().ok()?;
        let period = if let Some(week) = rest.strip_prefix('W') {
            Period::Week { year, week: week.parse().ok()? }
        } else {
            Period::Quarter { year, quarter: rest.strip_prefix('Q')?.parse().ok()? }
        };
        period.first_day().map(|_| period)
    }

    /// The `reports.kind` a digest of this period is saved under.
    pub fn kind(&self) -> &'static str {
        match self {
            Period::Week { .. } => "weekly",
            Period::Quarter { .. } => "quarterly",
        }
    }

    /// The year a query should name (the ISO year: the year of the week's Thursday).
    pub fn year(&self) -> i32 {
        match *self {
            Period::Week { year, .. } | Period::Quarter { year, .. } => year,
        }
    }

    /// `Q4`: the quarter of the period (of the Thursday, for a week, like its year).
    pub fn quarter_label(&self) -> String {
        match *self {
            Period::Quarter { quarter, .. } => format!("Q{quarter}"),
            Period::Week { .. } => {
                let thursday = match *self {
                    Period::Week { year, week } => NaiveDate::from_isoywd_opt(year, week, Weekday::Thu),
                    Period::Quarter { .. } => None,
                };
                format!("Q{}", (thursday.map_or(1, |d| d.month()) - 1) / 3 + 1)
            }
        }
    }

    fn first_day(&self) -> Option<NaiveDate> {
        match *self {
            Period::Week { year, week } => NaiveDate::from_isoywd_opt(year, week, Weekday::Mon),
            Period::Quarter { year, quarter } => {
                if !(1..=4).contains(&quarter) {
                    return None;
                }
                NaiveDate::from_ymd_opt(year, (quarter - 1) * 3 + 1, 1)
            }
        }
    }

    /// First instant of the period (UTC).
    pub fn start(&self) -> DateTime<Utc> {
        let day = self.first_day().unwrap_or_default();
        Utc.from_utc_datetime(&day.and_hms_opt(0, 0, 0).unwrap_or_default())
    }

    /// First instant after the period (UTC).
    pub fn end(&self) -> DateTime<Utc> {
        let next = match *self {
            Period::Week { .. } => self.first_day().and_then(|d| d.checked_add_days(chrono::Days::new(7))),
            Period::Quarter { year, quarter } => {
                if quarter >= 4 {
                    NaiveDate::from_ymd_opt(year + 1, 1, 1)
                } else {
                    NaiveDate::from_ymd_opt(year, quarter * 3 + 1, 1)
                }
            }
        };
        Utc.from_utc_datetime(&next.unwrap_or_default().and_hms_opt(0, 0, 0).unwrap_or_default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    #[test]
    fn labels_round_trip_and_invalid_ones_are_rejected() {
        let week = Period::week_of(day(2026, 11, 14));
        assert_eq!(week, Period::Week { year: 2026, week: 46 });
        assert_eq!(week.label(), "2026-W46");
        assert_eq!(week.kind(), "weekly");
        let quarter = Period::quarter_of(day(2026, 11, 14));
        assert_eq!(quarter.label(), "2026-Q4");
        assert_eq!(quarter.kind(), "quarterly");
        for p in [week, quarter, Period::Week { year: 2026, week: 1 }] {
            assert_eq!(Period::parse(&p.label()), Some(p));
        }
        for bad in ["", "2026", "2026-W54", "2026-W00", "2026-Q5", "2026-Q0", "x-W1", "2026-46", "2026-W"] {
            assert_eq!(Period::parse(bad), None, "{bad}");
        }
    }

    #[test]
    fn ranges_cover_the_period_exactly() {
        let week = Period::parse("2026-W46").unwrap();
        assert_eq!(week.start().to_rfc3339(), "2026-11-09T00:00:00+00:00");
        assert_eq!(week.end().to_rfc3339(), "2026-11-16T00:00:00+00:00");
        let q4 = Period::parse("2026-Q4").unwrap();
        assert_eq!(q4.start().to_rfc3339(), "2026-10-01T00:00:00+00:00");
        assert_eq!(q4.end().to_rfc3339(), "2027-01-01T00:00:00+00:00");
        let q1 = Period::parse("2026-Q1").unwrap();
        assert_eq!(q1.end().to_rfc3339(), "2026-04-01T00:00:00+00:00");
    }

    #[test]
    fn year_and_quarter_for_queries() {
        // 2026 has 53 ISO weeks; W53 ends on 2027-01-03, its Thursday is 2026-12-31.
        let w53 = Period::parse("2026-W53").unwrap();
        assert_eq!(w53.start().to_rfc3339(), "2026-12-28T00:00:00+00:00");
        assert_eq!((w53.year(), w53.quarter_label().as_str()), (2026, "Q4"));
        assert_eq!(Period::parse("2027-W53"), None, "2027 has 52 weeks");
        // 2026-W01 starts on Monday 2025-12-29 but belongs to 2026 (Thursday 2026-01-01).
        let w1 = Period::parse("2026-W01").unwrap();
        assert_eq!(w1.start().to_rfc3339(), "2025-12-29T00:00:00+00:00");
        assert_eq!((w1.year(), w1.quarter_label().as_str()), (2026, "Q1"));
        assert_eq!(Period::parse("2026-Q2").unwrap().quarter_label(), "Q2");
    }
}
