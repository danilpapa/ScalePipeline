//! Trends and sport analytics (spec §5). All series use daily measurements
//! and full-precision values; rounding happens only on output.

use std::collections::BTreeSet;

use chrono::NaiveDate;
use serde::Serialize;

use crate::profile::{Goal, Profile, Thresholds};
use crate::report::ser;
use crate::validate::{Code, Measurement, Status};

str_enum! {
    /// Report-level flags for the agent. Declaration order = output order.
    pub enum Flag {
        WarmingUp => "warming_up",
        TrendReset => "trend_reset",
        InsufficientData => "insufficient_data",
        StaleData => "stale_data",
        GoalMismatch => "goal_mismatch",
        LossTooFast => "loss_too_fast",
        GainTooFast => "gain_too_fast",
        LeanLoss => "lean_loss",
        FatGainOnBulk => "fat_gain_on_bulk",
        HydrationShift => "hydration_shift",
        HrElevated => "hr_elevated",
    }
}

str_enum! {
    pub enum Significance {
        Real => "real",
        WithinNoise => "within_noise",
        InsufficientData => "insufficient_data",
    }
}

/// Smoothed values as of the latest daily measurement.
#[derive(Debug, Clone, Default, Serialize)]
pub struct Trend {
    #[serde(serialize_with = "ser::r1")]
    pub weight_kg: Option<f64>,
    #[serde(serialize_with = "ser::r1")]
    pub fat_mass_kg: Option<f64>,
    #[serde(serialize_with = "ser::r1")]
    pub lean_mass_kg: Option<f64>,
    #[serde(serialize_with = "ser::r1")]
    pub skeletal_muscle_kg: Option<f64>,
    #[serde(serialize_with = "ser::r1")]
    pub body_fat_pct: Option<f64>,
    #[serde(serialize_with = "ser::r1")]
    pub visceral_fat: Option<f64>,
    #[serde(serialize_with = "ser::r3")]
    pub hydration_index: Option<f64>,
    #[serde(serialize_with = "ser::int")]
    pub heart_rate: Option<f64>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct Rates {
    #[serde(serialize_with = "ser::r2")]
    pub weight_kg_per_week_14d: Option<f64>,
    #[serde(serialize_with = "ser::r2")]
    pub weight_pct_per_week_14d: Option<f64>,
    #[serde(serialize_with = "ser::r2")]
    pub weight_kg_per_week_28d: Option<f64>,
    #[serde(serialize_with = "ser::r2")]
    pub weight_pct_per_week_28d: Option<f64>,
    #[serde(serialize_with = "ser::r2")]
    pub fat_mass_kg_per_week_28d: Option<f64>,
    #[serde(serialize_with = "ser::r2")]
    pub lean_mass_kg_per_week_28d: Option<f64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Delta {
    #[serde(serialize_with = "ser::r1")]
    pub d7: Option<f64>,
    #[serde(serialize_with = "ser::r1")]
    pub d30: Option<f64>,
    #[serde(serialize_with = "ser::r1")]
    pub d90: Option<f64>,
    /// Is the 30-day change bigger than the noise threshold?
    pub significance: Significance,
}

#[derive(Debug, Clone, Serialize)]
pub struct Deltas {
    pub weight_kg: Delta,
    pub fat_mass_kg: Delta,
    pub lean_mass_kg: Delta,
    pub skeletal_muscle_kg: Delta,
    pub body_fat_pct: Delta,
}

#[derive(Debug, Clone, Serialize)]
pub struct Phase {
    pub detected: Option<Goal>,
    pub goal: Option<Goal>,
    #[serde(serialize_with = "ser::r2")]
    pub lean_share_28d: Option<f64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Coverage {
    pub days_measured_7d: usize,
    pub days_measured_28d: usize,
    #[serde(serialize_with = "ser::r2")]
    pub morning_share_28d: Option<f64>,
    #[serde(serialize_with = "ser::r2")]
    pub impedance_share_28d: Option<f64>,
    pub days_since_last: i64,
}

#[derive(Debug, Clone)]
pub struct Analysis<'a> {
    pub latest: &'a Measurement,
    pub trend: Trend,
    pub rates: Rates,
    pub deltas: Deltas,
    pub phase: Phase,
    pub flags: Vec<Flag>,
    pub coverage: Coverage,
}

type Series = Vec<(NaiveDate, f64)>;

/// Daily measurement that may feed composition trends (§5.2).
pub fn composition_ok(m: &Measurement) -> bool {
    m.daily
        && m.status == Status::Ok
        && !m.has(Code::WeightOutlier)
        && !m.has(Code::CompositionJump)
        && !m.has(Code::ZRatioSuspect)
}

/// Sequence flags that need the weight trend: EWMA (§5.1), `weight_outlier`,
/// `trend_reset`, `composition_jump`.
pub fn annotate(ms: &mut [Measurement], th: &Thresholds) {
    // Weight EWMA, 10 % per day, gaps handled by exponent (Hacker's Diet).
    let mut prev: Option<(NaiveDate, f64)> = None;
    for m in ms.iter_mut().filter(|m| m.daily) {
        let (date, w) = (m.date().unwrap(), m.weight.unwrap());
        let trend = match prev {
            Some((d0, t0)) if (date - d0).num_days() <= th.trend_reset_days => {
                if (w - t0).abs() / t0 * 100.0 > th.weight_outlier_pct {
                    m.flags.insert(Code::WeightOutlier);
                }
                let gap = (date - d0).num_days() as i32;
                t0 + (1.0 - 0.9f64.powi(gap)) * (w - t0)
            }
            Some(_) => {
                m.flags.insert(Code::TrendReset);
                w
            }
            None => w,
        };
        m.weight_trend = Some(trend);
        prev = Some((date, trend));
    }

    // Body fat far from the 7-day median: probably water.
    let mut history: Series = Vec::new();
    for m in ms.iter_mut().filter(|m| m.daily && m.status == Status::Ok) {
        let (Some(date), Some(bf)) = (m.date(), m.metrics.body_fat_pct) else {
            continue;
        };
        let recent: Vec<f64> = history
            .iter()
            .filter(|(d, _)| (date - *d).num_days() <= 7)
            .map(|p| p.1)
            .collect();
        if recent.len() >= 3 && (bf - median(&recent)).abs() > th.composition_jump_pp {
            m.flags.insert(Code::CompositionJump);
        }
        if composition_ok(m) {
            history.push((date, bf));
        }
    }
}

/// Trends, rates, deltas, phase and flags as of `today`. `None` when there is
/// no daily measurement at all.
pub fn analyze<'a>(
    ms: &'a [Measurement],
    profile: &Profile,
    today: NaiveDate,
) -> Option<Analysis<'a>> {
    let th = &profile.thresholds;
    let daily: Vec<&Measurement> = ms.iter().filter(|m| m.daily).collect();
    let latest = *daily.last()?;
    let last_date = latest.date()?;
    let first_date = daily[0].date()?;

    // Weight trend and the current segment (after the last reset).
    let weight: Series = daily
        .iter()
        .map(|m| (m.date().unwrap(), m.weight_trend.unwrap()))
        .collect();
    let segment_start = daily
        .iter()
        .rev()
        .find(|m| m.has(Code::TrendReset))
        .and_then(|m| m.date())
        .unwrap_or(first_date);
    let segment: Series = weight
        .iter()
        .copied()
        .filter(|(d, _)| *d >= segment_start)
        .collect();

    // Composition series: raw daily values and their 7-day rolling medians.
    let comp: Vec<&Measurement> = daily
        .iter()
        .copied()
        .filter(|m| composition_ok(m))
        .collect();
    let series = |get: fn(&Measurement) -> Option<f64>| -> Series {
        comp.iter()
            .filter_map(|m| Some((m.date()?, get(m)?)))
            .collect()
    };
    let fat = series(|m| m.metrics.fat_mass_kg);
    let lean = series(|m| m.metrics.lean_mass_kg);
    let skeletal = series(|m| m.metrics.skeletal_muscle_kg);
    let bf = series(|m| m.metrics.body_fat_pct);
    let visceral = series(|m| m.metrics.visceral_fat.map(|v| v as f64));
    let hydration = series(|m| m.metrics.hydration_index);
    let hr = series(|m| m.heart_rate);
    let (fat_s, lean_s, skeletal_s, bf_s) = (
        smoothed(&fat),
        smoothed(&lean),
        smoothed(&skeletal),
        smoothed(&bf),
    );

    let week_median = |s: &Series| window_median(s, last_date, 7, 3);
    let trend = Trend {
        weight_kg: latest.weight_trend,
        fat_mass_kg: week_median(&fat),
        lean_mass_kg: week_median(&lean),
        skeletal_muscle_kg: week_median(&skeletal),
        body_fat_pct: week_median(&bf),
        visceral_fat: week_median(&visceral),
        hydration_index: week_median(&hydration),
        heart_rate: week_median(&hr),
    };

    let w14 = rate(&segment, today, 14, 7);
    let w28 = rate(&segment, today, 28, 12);
    let rates = Rates {
        weight_kg_per_week_14d: w14.map(|r| r.0),
        weight_pct_per_week_14d: w14.map(|r| r.1),
        weight_kg_per_week_28d: w28.map(|r| r.0),
        weight_pct_per_week_28d: w28.map(|r| r.1),
        fat_mass_kg_per_week_28d: rate(&fat, today, 28, 10).map(|r| r.0),
        lean_mass_kg_per_week_28d: rate(&lean, today, 28, 10).map(|r| r.0),
    };

    let delta_of = |s: &Series, noise: f64| {
        let d30 = delta(s, today, 30);
        Delta {
            d7: delta(s, today, 7),
            d30,
            d90: delta(s, today, 90),
            significance: match d30 {
                None => Significance::InsufficientData,
                Some(d) if d.abs() < noise => Significance::WithinNoise,
                Some(_) => Significance::Real,
            },
        }
    };
    let deltas = Deltas {
        weight_kg: delta_of(&weight, th.noise_weight_kg),
        fat_mass_kg: delta_of(&fat_s, th.noise_fat_kg),
        lean_mass_kg: delta_of(&lean_s, th.noise_lean_kg),
        skeletal_muscle_kg: delta_of(&skeletal_s, th.noise_lean_kg),
        body_fat_pct: delta_of(&bf_s, th.noise_body_fat_pct),
    };

    let pct28 = rates.weight_pct_per_week_28d;
    let detected = pct28.map(|p| {
        if p < -th.phase_pct_per_week {
            Goal::Cut
        } else if p > th.phase_pct_per_week {
            Goal::Bulk
        } else {
            Goal::Maintain
        }
    });
    let weight_28 = delta(&weight, today, 28);
    let lean_28 = delta(&lean_s, today, 28);
    let lean_share = match (lean_28, weight_28) {
        (Some(l), Some(w)) if w.abs() >= 1.0 => Some(l / w),
        _ => None,
    };
    let phase = Phase {
        detected,
        goal: profile.goal,
        lean_share_28d: lean_share,
    };

    // Report flags.
    let mut flags = BTreeSet::new();
    let mut flag = |cond: bool, f: Flag| {
        if cond {
            flags.insert(f);
        }
    };
    let days_since_last = (today - last_date).num_days();
    let pct = pct28.or(rates.weight_pct_per_week_14d);
    flag(
        (last_date - segment_start).num_days() < th.warmup_days,
        Flag::WarmingUp,
    );
    flag(
        segment_start != first_date && (today - segment_start).num_days() < 28,
        Flag::TrendReset,
    );
    flag(trend.fat_mass_kg.is_none(), Flag::InsufficientData);
    flag(days_since_last > th.stale_data_days, Flag::StaleData);
    flag(
        matches!((profile.goal, detected), (Some(g), Some(d)) if g != d),
        Flag::GoalMismatch,
    );
    flag(
        pct.is_some_and(|p| p < -th.loss_too_fast_pct_per_week),
        Flag::LossTooFast,
    );
    flag(
        pct.is_some_and(|p| p > th.gain_too_fast_pct_per_week),
        Flag::GainTooFast,
    );
    flag(
        lean_28.is_some_and(|d| d < -th.noise_lean_kg),
        Flag::LeanLoss,
    );
    flag(
        detected == Some(Goal::Bulk) && lean_share.is_some_and(|s| s < th.fat_gain_lean_share),
        Flag::FatGainOnBulk,
    );
    let hydration_28 = window_median(&hydration, today, 28, 3);
    flag(
        matches!((latest.metrics.hydration_index, hydration_28),
            (Some(hi), Some(med)) if (hi - med).abs() / med * 100.0 > th.hydration_shift_pct),
        Flag::HydrationShift,
    );
    flag(
        matches!((window_median(&hr, today, 7, 3), window_median(&hr, today, 28, 10)),
            (Some(a), Some(b)) if a - b >= th.noise_heart_rate),
        Flag::HrElevated,
    );

    // Coverage.
    let last_28: Vec<&Measurement> = daily
        .iter()
        .copied()
        .filter(|m| in_window(m.date().unwrap(), today, 28))
        .collect();
    let share = |pred: fn(&Measurement) -> bool| {
        (!last_28.is_empty())
            .then(|| last_28.iter().filter(|m| pred(m)).count() as f64 / last_28.len() as f64)
    };
    let coverage = Coverage {
        days_measured_7d: last_28
            .iter()
            .filter(|m| in_window(m.date().unwrap(), today, 7))
            .count(),
        days_measured_28d: last_28.len(),
        morning_share_28d: share(|m| !m.has(Code::NonMorning)),
        impedance_share_28d: share(|m| m.z50.is_some()),
        days_since_last,
    };

    Some(Analysis {
        latest,
        trend,
        rates,
        deltas,
        phase,
        flags: flags.into_iter().collect(),
        coverage,
    })
}

// ---------------------------------------------------------------- helpers

pub fn median(values: &[f64]) -> f64 {
    let mut v = values.to_vec();
    v.sort_by(f64::total_cmp);
    let n = v.len();
    if n % 2 == 1 {
        v[n / 2]
    } else {
        (v[n / 2 - 1] + v[n / 2]) / 2.0
    }
}

/// `date` falls in the `days` calendar days ending at `end` (inclusive).
fn in_window(date: NaiveDate, end: NaiveDate, days: i64) -> bool {
    (0..days).contains(&(end - date).num_days())
}

fn window(s: &Series, end: NaiveDate, days: i64) -> Series {
    s.iter()
        .copied()
        .filter(|(d, _)| in_window(*d, end, days))
        .collect()
}

/// Median of the window, or `None` if fewer than `min` points.
fn window_median(s: &Series, end: NaiveDate, days: i64, min: usize) -> Option<f64> {
    let values: Vec<f64> = window(s, end, days).into_iter().map(|p| p.1).collect();
    (values.len() >= min).then(|| median(&values))
}

/// 7-day rolling median at every point that has enough data.
fn smoothed(s: &Series) -> Series {
    s.iter()
        .filter_map(|&(d, _)| Some((d, window_median(s, d, 7, 3)?)))
        .collect()
}

/// Value at the point nearest to `target`, at most 2 days away.
fn value_near(s: &Series, target: NaiveDate) -> Option<f64> {
    s.iter()
        .map(|&(d, v)| ((d - target).num_days().abs(), std::cmp::Reverse(d), v))
        .filter(|(dist, _, _)| *dist <= 2)
        .min_by(|a, b| (a.0, a.1).cmp(&(b.0, b.1)))
        .map(|p| p.2)
}

/// Trend today minus trend `days` ago; `None` unless both points exist.
fn delta(s: &Series, today: NaiveDate, days: i64) -> Option<f64> {
    Some(value_near(s, today)? - value_near(s, today - chrono::Duration::days(days))?)
}

/// Least-squares slope over the window: `(per week, % of window mean per week)`.
fn rate(s: &Series, today: NaiveDate, days: i64, min_points: usize) -> Option<(f64, f64)> {
    let pts = window(s, today, days);
    if pts.len() < min_points {
        return None;
    }
    let n = pts.len() as f64;
    let xs: Vec<f64> = pts
        .iter()
        .map(|(d, _)| (*d - pts[0].0).num_days() as f64)
        .collect();
    let mean_x = xs.iter().sum::<f64>() / n;
    let mean_y = pts.iter().map(|p| p.1).sum::<f64>() / n;
    let sxx: f64 = xs.iter().map(|x| (x - mean_x).powi(2)).sum();
    if sxx == 0.0 {
        return None;
    }
    let sxy: f64 = xs
        .iter()
        .zip(&pts)
        .map(|(x, p)| (x - mean_x) * (p.1 - mean_y))
        .sum();
    let per_week = sxy / sxx * 7.0;
    Some((per_week, per_week / mean_y * 100.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 10, day).unwrap()
    }

    #[test]
    fn median_odd_even() {
        assert_eq!(median(&[3.0, 1.0, 2.0]), 2.0);
        assert_eq!(median(&[4.0, 1.0, 2.0, 3.0]), 2.5);
    }

    #[test]
    fn slope_of_a_line() {
        let s: Series = (1..=20).map(|i| (d(i), 80.0 - 0.1 * i as f64)).collect();
        let (kg, _) = rate(&s, d(20), 14, 7).unwrap();
        assert!((kg + 0.7).abs() < 1e-9);
    }

    #[test]
    fn value_near_prefers_closest_then_latest() {
        let s: Series = vec![(d(1), 1.0), (d(3), 3.0), (d(5), 5.0)];
        assert_eq!(value_near(&s, d(4)), Some(5.0));
        assert_eq!(value_near(&s, d(10)), None);
    }
}
