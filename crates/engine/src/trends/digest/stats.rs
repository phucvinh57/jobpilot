//! The statistics behind the counts: how sure a share is, and whether it moved.
//!
//! Everything here is a pure function of counts (`k` documents naming a thing out of `n` read), so
//! the same postings always give the same numbers; the model has no part in it.
//!
//! - **level**: [`wilson`], the 95% interval of a share. At a few hundred postings a month this is
//!   the dependable output.
//! - **sudden change**: [`log_odds_z`], the target months against the baseline months.
//! - **slow drift**: [`trend_z`], a trend over every month of the window (Cochran-Armitage).
//! - **many tests at once**: [`benjamini_hochberg`], which of them survive false-discovery control.
//! - **the call**: [`call_direction`] turns that evidence into a [`Direction`].

use super::Direction;

/// The normal quantile of a two-sided 95% interval.
const Z95: f64 = 1.959_963_985;

/// The 95% Wilson interval of the share `k / n`, in per mille, as `(low, high)`. `(0, 0)` without
/// documents, like [`permille`](super::permille). Unlike the plain `p ± 1.96·se` it stays inside
/// 0..=1000 and is not empty at `k = 0` or `k = n`.
pub fn wilson(k: u32, n: u32) -> (u32, u32) {
    if n == 0 {
        return (0, 0);
    }
    let (k, n) = (f64::from(k.min(n)), f64::from(n));
    let p = k / n;
    let z2 = Z95 * Z95;
    let denom = 1.0 + z2 / n;
    let centre = (p + z2 / (2.0 * n)) / denom;
    let half = Z95 * (p * (1.0 - p) / n + z2 / (4.0 * n * n)).sqrt() / denom;
    let per_mille = |x: f64| (x.clamp(0.0, 1.0) * 1000.0).round() as u32;
    (per_mille(centre - half), per_mille(centre + half))
}

/// How far the share in the target (`k_t / n_t`) is from the share in the baseline (`k_b / n_b`),
/// as a z-score of the log-odds ratio: positive when the target's share is higher.
///
/// Both sides are first pulled towards the pooled share by a prior worth `prior` documents
/// (Monroe, Colaresi and Quinn 2008): with a small prior (1) the counts speak for themselves, a
/// larger one (20) steadies the estimate for a term seen in a handful of documents or missing
/// from one side. `0.0` when a side has no documents or the thing is in none or all of them.
pub fn log_odds_z(k_t: u32, n_t: u32, k_b: u32, n_b: u32, prior: f64) -> f64 {
    if n_t == 0 || n_b == 0 {
        return 0.0;
    }
    let (k_t, n_t, k_b, n_b) = (f64::from(k_t.min(n_t)), f64::from(n_t), f64::from(k_b.min(n_b)), f64::from(n_b));
    let pooled = (k_t + k_b) / (n_t + n_b);
    let (yes, no) = (prior * pooled, prior * (1.0 - pooled));
    if yes <= 0.0 || no <= 0.0 {
        return 0.0;
    }
    let (t_yes, t_no, b_yes, b_no) = (k_t + yes, n_t - k_t + no, k_b + yes, n_b - k_b + no);
    let delta = (t_yes / t_no).ln() - (b_yes / b_no).ln();
    let variance = 1.0 / t_yes + 1.0 / t_no + 1.0 / b_yes + 1.0 / b_no;
    delta / variance.sqrt()
}

/// Is the share `k[i] / n[i]` going up or down over the months (oldest first)? The
/// Cochran-Armitage trend test with the month's index as its score, as a z-score: positive when
/// the share grows. It uses every month, so it sees a slow drift that the comparison of two sides
/// misses. `0.0` with fewer than two months holding documents, or a share of 0 or 1 throughout.
pub fn trend_z(k: &[u32], n: &[u32]) -> f64 {
    let months = k.len().min(n.len());
    let total: f64 = n[..months].iter().map(|&d| f64::from(d)).sum();
    if total == 0.0 {
        return 0.0;
    }
    let hits: f64 = k[..months].iter().zip(n).map(|(&m, &d)| f64::from(m.min(d))).sum();
    let p = hits / total;
    let mean: f64 = (0..months).map(|i| f64::from(n[i]) * i as f64).sum::<f64>() / total;
    let (mut t, mut spread) = (0.0, 0.0);
    for i in 0..months {
        let x = i as f64 - mean;
        t += f64::from(k[i].min(n[i])) * x;
        spread += f64::from(n[i]) * x * x;
    }
    let variance = p * (1.0 - p) * spread;
    if variance <= 0.0 {
        return 0.0;
    }
    t / variance.sqrt()
}

/// The two-sided p-value of a z-score: `erfc(|z| / sqrt 2)`. A NaN counts as no evidence (1).
fn p_value(z: f64) -> f64 {
    if z.is_nan() {
        return 1.0;
    }
    erfc(z.abs() / std::f64::consts::SQRT_2)
}

/// The complementary error function for `x >= 0`, by the Chebyshev fit of Numerical Recipes
/// (`erfcc`): its relative error is below 1.2e-7 everywhere, tails included, which is what a
/// comparison of small p-values needs.
fn erfc(x: f64) -> f64 {
    let t = 1.0 / (1.0 + 0.5 * x);
    let poly = -1.265_512_23
        + t * (1.000_023_68
            + t * (0.374_091_96
                + t * (0.096_784_18
                    + t * (-0.186_288_06
                        + t * (0.278_868_07 + t * (-1.135_203_98 + t * (1.488_515_87 + t * (-0.822_152_23 + t * 0.170_872_77))))))));
    t * (-x * x + poly).exp()
}

/// The share of false discoveries the things called in one family of tests may hold, on average
/// (Benjamini-Hochberg's `q`): at 0.10, of ten skills called rising or falling about one is noise.
/// One family is a scope's skills (`count`) or a stratum's terms (`terms`); the rate is applied to
/// all the tests of a family together, so a longer vocabulary does not call more movements out of
/// chance.
pub(super) const FALSE_DISCOVERY_RATE: f64 = 0.10;

/// Which of the tests (given as z-scores, in their order) survive when the expected share of
/// false discoveries among the survivors is held at `q` (Benjamini and Hochberg 1995). Testing a
/// hundred skills at 5% each would call five movements out of pure noise; this does not.
pub(super) fn benjamini_hochberg(z: &[f64], q: f64) -> Vec<bool> {
    let mut order: Vec<(usize, f64)> = z.iter().map(|&z| p_value(z)).enumerate().collect();
    order.sort_by(|a, b| a.1.total_cmp(&b.1).then(a.0.cmp(&b.0)));
    let m = order.len() as f64;
    // The largest rank whose p-value is under its own bar; every test up to it survives.
    let last = order.iter().enumerate().rev().find(|(rank, (_, p))| *p <= (*rank as f64 + 1.0) / m * q).map(|(rank, _)| rank);
    let mut survives = vec![false; z.len()];
    if let Some(last) = last {
        for (at, _) in &order[..=last] {
            survives[*at] = true;
        }
    }
    survives
}

/// A z-score in hundredths, for the stored counts (integers keep them comparable with `Eq`).
pub fn centi(z: f64) -> i32 {
    if z.is_nan() {
        return 0;
    }
    (z * 100.0).round().clamp(f64::from(i32::MIN), f64::from(i32::MAX)) as i32
}

/// What the counts say about one skill in one scope, before a direction is called.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Evidence {
    /// [`log_odds_z`]: the target months against the baseline months.
    pub step_z: f64,
    /// [`trend_z`]: the trend over every month of the window.
    pub trend_z: f64,
    /// The step test survived [`benjamini_hochberg`] among all tests of the scope.
    pub step_survives: bool,
    /// The trend test survived it.
    pub trend_survives: bool,
    /// The target's share minus the baseline's, in per mille of the documents (10 = one point).
    pub change_permille: i32,
    /// Documents on the smaller of the two sides (target, baseline).
    pub min_side_documents: u32,
}

/// A step must move the share by at least this much (per mille of the documents: one point) to
/// count: on a large sample a trivial change is significant too.
pub(super) const MIN_CHANGE_PERMILLE: i32 = 10;
/// With at least this many documents on each side, no surviving test reads as "steady"; with
/// fewer, the data could not have shown a movement, and the direction is "unclear".
pub(super) const MIN_SIDE_DOCS: u32 = 30;

/// The direction the evidence supports. Either test may call a movement once it survived the
/// false-discovery control (the step also needs [`MIN_CHANGE_PERMILLE`]); two surviving tests
/// that point opposite ways (a long slide, then a jump back) are `Unclear`. Without a surviving
/// test it is `Steady` when both sides hold [`MIN_SIDE_DOCS`] documents, `Unclear` otherwise.
pub(super) fn call_direction(evidence: &Evidence) -> Direction {
    let up = |z: f64| if z > 0.0 { Direction::Rising } else { Direction::Falling };
    let step = (evidence.step_survives && evidence.change_permille.abs() >= MIN_CHANGE_PERMILLE).then(|| up(evidence.step_z));
    let trend = evidence.trend_survives.then(|| up(evidence.trend_z));
    match (step, trend) {
        (Some(a), Some(b)) if a != b => Direction::Unclear,
        (Some(direction), _) | (None, Some(direction)) => direction,
        (None, None) if evidence.min_side_documents >= MIN_SIDE_DOCS => Direction::Steady,
        (None, None) => Direction::Unclear,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_interval_of_a_share_narrows_with_more_documents_and_stays_in_range() {
        assert_eq!(wilson(0, 0), (0, 0));
        // Known values: 5 of 10 is 23.7% .. 76.3%, 50 of 100 is 40.4% .. 59.6%.
        assert_eq!(wilson(5, 10), (237, 763));
        assert_eq!(wilson(50, 100), (404, 596));
        let (low, high) = wilson(0, 20);
        assert_eq!(low, 0);
        assert!(high > 0 && high < 200, "none of 20 still leaves room: {high}");
        assert_eq!(wilson(20, 20).1, 1000);
        assert_eq!(wilson(30, 20), wilson(20, 20), "more mentions than documents is capped");
    }

    #[test]
    fn the_step_test_follows_the_change_and_is_quiet_on_thin_data() {
        assert_eq!(log_odds_z(10, 0, 10, 100, 1.0), 0.0);
        assert_eq!(log_odds_z(0, 100, 0, 100, 1.0), 0.0, "named nowhere: nothing to compare");
        assert!(log_odds_z(20, 100, 20, 100, 1.0).abs() < 1e-9, "equal shares");
        assert!(log_odds_z(40, 200, 20, 200, 1.0) > 1.96);
        assert!(log_odds_z(20, 200, 40, 200, 1.0) < -1.96);
        // The same shares on a tenth of the documents are not evidence.
        assert!(log_odds_z(4, 20, 2, 20, 1.0).abs() < 1.96);
        // Doubling the documents at the same share is no change.
        assert!(log_odds_z(40, 400, 20, 200, 1.0).abs() < 1e-9);
    }

    #[test]
    fn a_jump_on_a_handful_of_documents_is_not_news_under_either_prior() {
        // 2 -> 5 postings out of 250 a side: more than double, and still no evidence.
        let (weak, strong) = (log_odds_z(5, 250, 2, 250, 1.0), log_odds_z(5, 250, 2, 250, 20.0));
        assert!(weak > 0.0 && weak < 1.96, "{weak}");
        assert!(strong > 0.0 && strong < 1.96, "{strong}");
        // An empty side does not blow the score up: the prior fills it in.
        assert!(log_odds_z(6, 100, 0, 100, 20.0).is_finite());
        // With plenty of documents the prior hardly matters.
        let (weak, strong) = (log_odds_z(150, 500, 100, 500, 1.0), log_odds_z(150, 500, 100, 500, 20.0));
        assert!((weak - strong).abs() < 0.2 && strong > 1.96);
    }

    #[test]
    fn the_trend_test_sees_a_slow_drift_the_step_test_misses() {
        let n = [250, 250, 250, 250, 250, 250];
        assert_eq!(trend_z(&[], &[]), 0.0);
        assert_eq!(trend_z(&[0; 6], &n), 0.0);
        assert!(trend_z(&[50; 6], &n).abs() < 1e-9, "a flat share");
        // 53, 54, 52, 47, 46, 38 percent: a drift down.
        let k = [133, 135, 130, 118, 115, 95];
        let drift = trend_z(&k, &n);
        assert!(drift < -1.96, "{drift}");
        assert!(trend_z(&[95, 115, 118, 130, 135, 133], &n) > 1.96, "and up, mirrored");
        // Month 6 against months 1..5, the two-sample view of the same data, is weaker.
        let step = log_odds_z(95, 250, 631, 1250, 1.0);
        assert!(step < 0.0 && step.abs() < drift.abs() + 1.0);
        // A busy month does not make a trend.
        assert!(trend_z(&[20, 20, 20, 20, 20, 200], &[100, 100, 100, 100, 100, 1000]).abs() < 1e-9);
    }

    #[test]
    fn p_values_match_the_normal_table() {
        assert!((p_value(0.0) - 1.0).abs() < 1e-6);
        assert!((p_value(1.959_964) - 0.05).abs() < 1e-5);
        assert!((p_value(-2.575_829) - 0.01).abs() < 1e-5);
        assert!((p_value(4.0) / 6.334e-5 - 1.0).abs() < 1e-3, "the tail keeps its relative accuracy");
        assert_eq!(p_value(f64::NAN), 1.0);
    }

    #[test]
    fn false_discovery_control_keeps_real_movements_and_no_noise() {
        assert!(benjamini_hochberg(&[], 0.1).is_empty());
        // A hundred tests of pure noise at |z| = 1.7 (p = 0.09 each): 5% apiece would pass none
        // here, and neither does the control.
        assert!(benjamini_hochberg(&[1.7; 100], 0.05).iter().all(|&s| !s));
        // One clear movement among noise survives, on either side of zero.
        let mut z = vec![0.3; 50];
        z[7] = 4.5;
        z[20] = -4.2;
        let survives = benjamini_hochberg(&z, 0.1);
        assert_eq!(survives.iter().filter(|&&s| s).count(), 2);
        assert!(survives[7] && survives[20]);
        // A borderline 2.0 passes alone (p = 0.0455 <= 0.1) but not among fifty tests.
        assert_eq!(benjamini_hochberg(&[2.0], 0.1), [true]);
        let mut z = vec![0.3; 50];
        z[0] = 2.0;
        assert!(!benjamini_hochberg(&z, 0.1)[0]);
    }

    #[test]
    fn a_direction_is_called_only_on_a_surviving_test() {
        let quiet = Evidence { step_z: 1.2, trend_z: -0.4, step_survives: false, trend_survives: false, change_permille: 30, min_side_documents: 200 };
        assert_eq!(call_direction(&quiet), Direction::Steady);
        assert_eq!(call_direction(&Evidence { min_side_documents: 29, ..quiet }), Direction::Unclear, "too thin to say steady");
        let step = Evidence { step_z: 3.4, step_survives: true, ..quiet };
        assert_eq!(call_direction(&step), Direction::Rising);
        assert_eq!(call_direction(&Evidence { step_z: -3.4, change_permille: -30, ..step }), Direction::Falling);
        assert_eq!(call_direction(&Evidence { change_permille: 9, ..step }), Direction::Steady, "significant, but under a point");
        // A slow drift alone is enough, whatever the last step did.
        let drift = Evidence { trend_z: -3.1, trend_survives: true, ..quiet };
        assert_eq!(call_direction(&drift), Direction::Falling);
        // A slide and a jump back, both real: no single direction.
        assert_eq!(call_direction(&Evidence { step_z: 3.4, step_survives: true, ..drift }), Direction::Unclear);
        // A movement is called even on a thin side: the tests already account for the size.
        assert_eq!(call_direction(&Evidence { min_side_documents: 12, ..step }), Direction::Rising);
    }

    #[test]
    fn a_z_score_is_stored_in_hundredths() {
        assert_eq!(centi(1.964), 196);
        assert_eq!(centi(-2.005), -201);
        assert_eq!(centi(f64::NAN), 0);
        assert_eq!(centi(f64::INFINITY), i32::MAX);
    }
}
