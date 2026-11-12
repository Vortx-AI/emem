//! `canopy_loss_seasonal_nbr@1`: the methods lab's `lab:seasonal_canopy_loss@1`,
//! registered from its record. A cell lost canopy on the first date t in the
//! scoring window where the median NBR of the 45 days from t sits at least 0.20
//! below both the median of the 90 days before t and the median of the same 45
//! days a year earlier, from a pre-event median of at least 0.4.
//!
//! It is a line-for-line port of the lab's Python (emem-world `agents/lab.py`,
//! `_seasonal_loss(nbr, win, 0.20, 0.4, 45)`, source BLAKE3 in [`LAB_SOURCE_BLAKE3`]),
//! held to it by a parity test on 40 cells of real signed series. Medians follow
//! Python's `statistics.median`: the mean of the two middle values for an even
//! count.

pub(crate) const KEY: &str = "canopy_loss_seasonal_nbr@1";
pub(crate) const LAB_KEY: &str = "lab:seasonal_canopy_loss@1";
pub(crate) const LAB_SOURCE_BLAKE3: &str =
    "908ffad5ce9e619f31bebcc93b35f26197396d9ea95b8755e43270a83cf30d08";
const DROP: f64 = 0.20;
const FROM_AT_LEAST: f64 = 0.4;
const POST_DAYS: i64 = 45;
const PRE_DAYS: i64 = 90;
const YEAR_DAYS: i64 = 365;
const YEAR_SLACK_DAYS: i64 = 10;
/// The earliest series start that lets every date of the window have its
/// year-ago reference: a year and its slack before the window opens.
pub(crate) const HISTORY_DAYS: i64 = YEAR_DAYS + YEAR_SLACK_DAYS;

/// The first date the rule flags, with the three medians it compared.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Loss {
    pub day: i64,
    pub pre: f64,
    pub year_ago: f64,
    pub post: f64,
}

fn median(v: &[f64]) -> f64 {
    let mut s = v.to_vec();
    s.sort_by(f64::total_cmp);
    let n = s.len();
    if n % 2 == 1 {
        s[n / 2]
    } else {
        (s[n / 2 - 1] + s[n / 2]) / 2.0
    }
}

/// `days` ascending (ties allowed), `vals` the cell's median NBR on each.
/// `lo`..=`hi` is the scoring window, in the same day units.
pub(crate) fn seasonal_canopy_loss(days: &[i64], vals: &[f64], lo: i64, hi: i64) -> Option<Loss> {
    let left = |x: i64| days.partition_point(|d| *d < x);
    let right = |x: i64| days.partition_point(|d| *d <= x);
    for (i, &t) in days.iter().enumerate() {
        if t < lo || t > hi {
            continue;
        }
        let post = &vals[i..right(t + POST_DAYS).max(i)];
        let pre = &vals[left(t - PRE_DAYS).min(i)..i];
        let a0 = left(t - YEAR_DAYS - YEAR_SLACK_DAYS);
        let a1 = right(t - YEAR_DAYS + POST_DAYS + YEAR_SLACK_DAYS);
        let ago = &vals[a0..a1.max(a0)];
        if post.len() < 2 || pre.len() < 2 || ago.is_empty() {
            continue;
        }
        let (mp, ma, mq) = (median(pre), median(ago), median(post));
        if mp >= FROM_AT_LEAST && mp.min(ma) - mq >= DROP {
            return Some(Loss {
                day: t,
                pre: mp,
                year_ago: ma,
                post: mq,
            });
        }
    }
    None
}

/// The rule as one sentence, for the record.
pub(crate) const RULE: &str = "loss on the first date t in the window where the median NBR of the 45 days from t is at least 0.20 below both the median of the 90 days before t and the median of the same 45 days a year earlier, from a pre-event median of at least 0.4";

#[cfg(test)]
mod tests {
    use super::*;

    /// The port flags exactly the cells, on exactly the dates, that the lab's
    /// Python flagged on the same signed series: 40 cells from ten forests,
    /// half of them flagged.
    #[test]
    fn matches_the_lab_rule_on_real_series() {
        let fx: serde_json::Value =
            serde_json::from_str(include_str!("testdata/canopy_loss_parity.json")).unwrap();
        assert_eq!(fx["source_blake3"], LAB_SOURCE_BLAKE3);
        let w = fx["window"].as_array().unwrap();
        let day_of = |s: &str| {
            let (y, m, d) = crate::band_raster::parse_ymd(s).unwrap();
            crate::days_from_civil(y, m, d)
        };
        let (lo, hi) = (
            day_of(w[0].as_str().unwrap()),
            day_of(w[1].as_str().unwrap()),
        );
        let cases = fx["cases"].as_array().unwrap();
        assert_eq!(cases.len(), 40);
        let mut flagged = 0;
        for c in cases {
            let days: Vec<i64> = c["days"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_i64().unwrap())
                .collect();
            let vals: Vec<f64> = c["values"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_f64().unwrap())
                .collect();
            let want = c["loss_date"].as_str().map(|s| day_of(&s[..10]));
            let got = seasonal_canopy_loss(&days, &vals, lo, hi).map(|l| l.day);
            assert_eq!(got, want, "{} {}", c["site"], c["cell"]);
            flagged += usize::from(got.is_some());
        }
        assert_eq!(flagged, 20);
    }

    #[test]
    fn needs_two_passes_each_side_and_a_year_ago() {
        // A clear drop, but nothing a year earlier: not judged.
        let days = [0, 30, 60, 100, 120, 130];
        let vals = [0.6, 0.6, 0.6, 0.2, 0.2, 0.2];
        assert_eq!(seasonal_canopy_loss(&days, &vals, 0, 200), None);
        // The same drop with a year of history at the same level. Day 455 already
        // qualifies: its 45 days hold three passes at 0.2 and one at 0.6.
        let days = [365, 380, 395, 425, 455, 465, 475, 485];
        let vals = [0.6, 0.6, 0.6, 0.6, 0.6, 0.2, 0.2, 0.2];
        let mut d = vec![0, 15, 30, 60, 90, 100, 110, 120];
        let mut v = vec![0.6; 8];
        d.extend(days);
        v.extend(vals);
        let hit = seasonal_canopy_loss(&d, &v, 400, 600).unwrap();
        assert_eq!(hit.day, 455);
        assert!((hit.pre - 0.6).abs() < 1e-12 && (hit.post - 0.2).abs() < 1e-12);
    }

    #[test]
    fn median_is_pythons() {
        assert_eq!(median(&[0.3, 0.1]), 0.2);
        assert_eq!(median(&[0.5, 0.1, 0.3]), 0.3);
    }
}
