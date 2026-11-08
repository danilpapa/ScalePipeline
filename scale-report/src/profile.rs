//! `profile.toml` and `overrides.toml` (spec §2.3, §3).

use std::collections::BTreeMap;

use chrono::{Datelike, NaiveDate, Utc};
use chrono_tz::Tz;
use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize};
use sha2::{Digest, Sha256};

use crate::Error;
use crate::model::Sex;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Goal {
    Cut,
    Bulk,
    Maintain,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Profile {
    pub sex: Sex,
    #[serde(deserialize_with = "de_date")]
    pub birth_date: NaiveDate,
    pub height_cm: f64,
    /// Scale user slot; measurements from other slots are excluded.
    #[serde(default)]
    pub profile_id: Option<i64>,
    /// IANA name, e.g. `Europe/Moscow`: day boundary and morning window.
    pub timezone: String,
    /// `[start, end)` local hours for the daily measurement.
    #[serde(default = "default_morning_window")]
    pub morning_window: [u32; 2],
    #[serde(default)]
    pub goal: Option<Goal>,
    #[serde(default)]
    pub thresholds: Thresholds,
}

fn default_morning_window() -> [u32; 2] {
    [4, 12]
}

/// TOML date (`2003-05-14`) or string (`"2003-05-14"`).
fn de_date<'de, D: Deserializer<'de>>(d: D) -> Result<NaiveDate, D::Error> {
    let s = match toml::Value::deserialize(d)? {
        toml::Value::Datetime(dt) => dt.to_string(),
        toml::Value::String(s) => s,
        _ => return Err(D::Error::custom("expected a date like 2003-05-14")),
    };
    NaiveDate::parse_from_str(&s, "%Y-%m-%d").map_err(D::Error::custom)
}

impl Profile {
    pub fn from_toml(s: &str) -> Result<Self, Error> {
        let p: Profile =
            toml::from_str(s).map_err(|e| Error::Config(format!("profile.toml: {e}")))?;
        p.check()?;
        Ok(p)
    }

    fn check(&self) -> Result<(), Error> {
        let bad = |msg: String| Err(Error::Config(format!("profile.toml: {msg}")));
        if !(120.0..=230.0).contains(&self.height_cm) {
            return bad(format!("height_cm {} is outside 120–230", self.height_cm));
        }
        if self.timezone.parse::<Tz>().is_err() {
            return bad(format!("unknown timezone {:?}", self.timezone));
        }
        let [start, end] = self.morning_window;
        if start >= end || end > 24 {
            return bad(format!(
                "morning_window [{start}, {end}] must be 0 ≤ start < end ≤ 24"
            ));
        }
        Ok(())
    }

    pub fn tz(&self) -> Tz {
        self.timezone.parse().unwrap_or(Tz::UTC)
    }

    /// Today in the profile time zone — the only place that reads the clock.
    pub fn today(&self) -> NaiveDate {
        Utc::now().with_timezone(&self.tz()).date_naive()
    }

    /// Full years on `date`.
    pub fn age_on(&self, date: NaiveDate) -> i32 {
        let b = self.birth_date;
        let before_birthday = (date.month(), date.day()) < (b.month(), b.day());
        date.year() - b.year() - i32::from(before_birthday)
    }

    /// `sha256:<hex>` of the normalised profile — changes when the base changes.
    pub fn hash(&self) -> String {
        let canonical = serde_json::to_vec(self).expect("profile serialises");
        let digest = Sha256::digest(&canonical);
        let hex: String = digest.iter().map(|b| format!("{b:02x}")).collect();
        format!("sha256:{hex}")
    }
}

/// Heuristic thresholds (spec §3, §5). All overridable in `[thresholds]`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Thresholds {
    pub weight_min_kg: f64,
    pub weight_max_kg: f64,
    pub z_min: f64,
    pub z_max: f64,
    pub z_ratio_min: f64,
    pub z_ratio_max: f64,
    pub heart_rate_min: f64,
    pub heart_rate_max: f64,
    pub future_tolerance_min: i64,
    pub duplicate_window_min: i64,
    pub duplicate_weight_kg: f64,
    pub stale_gap_min: i64,
    pub foreign_weight_pct: f64,
    pub foreign_weight_days: i64,
    pub foreign_weight_baseline: usize,
    pub weight_outlier_pct: f64,
    pub composition_jump_pp: f64,
    pub trend_reset_days: i64,
    pub warmup_days: i64,
    pub noise_weight_kg: f64,
    pub noise_fat_kg: f64,
    pub noise_lean_kg: f64,
    pub noise_body_fat_pct: f64,
    pub noise_heart_rate: f64,
    pub phase_pct_per_week: f64,
    pub loss_too_fast_pct_per_week: f64,
    pub gain_too_fast_pct_per_week: f64,
    pub fat_gain_lean_share: f64,
    pub hydration_shift_pct: f64,
    pub stale_data_days: i64,
}

impl Default for Thresholds {
    fn default() -> Self {
        Self {
            weight_min_kg: 20.0,
            weight_max_kg: 250.0,
            z_min: 200.0,
            z_max: 1500.0,
            z_ratio_min: 0.80,
            z_ratio_max: 0.97,
            heart_rate_min: 35.0,
            heart_rate_max: 200.0,
            future_tolerance_min: 5,
            duplicate_window_min: 5,
            duplicate_weight_kg: 0.3,
            stale_gap_min: 10,
            foreign_weight_pct: 8.0,
            foreign_weight_days: 14,
            foreign_weight_baseline: 3,
            weight_outlier_pct: 3.0,
            composition_jump_pp: 3.0,
            trend_reset_days: 14,
            warmup_days: 7,
            noise_weight_kg: 0.5,
            noise_fat_kg: 1.0,
            noise_lean_kg: 1.0,
            noise_body_fat_pct: 1.0,
            noise_heart_rate: 5.0,
            phase_pct_per_week: 0.25,
            loss_too_fast_pct_per_week: 1.0,
            gain_too_fast_pct_per_week: 0.5,
            fat_gain_lean_share: 0.3,
            hydration_shift_pct: 2.0,
            stale_data_days: 3,
        }
    }
}

/// Manual fix for one file in `overrides.toml`:
///
/// ```toml
/// ["2026-11-03T21-02-11.json"]
/// exclude = true
/// note = "после тренировки"
/// ```
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Override {
    #[serde(default)]
    pub exclude: bool,
    pub note: Option<String>,
}

/// File name → override.
pub type Overrides = BTreeMap<String, Override>;

pub fn parse_overrides(s: &str) -> Result<Overrides, Error> {
    toml::from_str(s).map_err(|e| Error::Config(format!("overrides.toml: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    const PROFILE: &str = r#"
        sex = "male"
        birth_date = 2003-05-14
        height_cm = 180.0
        timezone = "Europe/Moscow"
        [thresholds]
        noise_fat_kg = 1.5
    "#;

    #[test]
    fn parses_and_defaults() {
        let p = Profile::from_toml(PROFILE).unwrap();
        assert_eq!(p.morning_window, [4, 12]);
        assert_eq!(p.thresholds.noise_fat_kg, 1.5);
        assert_eq!(p.thresholds.noise_lean_kg, 1.0);
        assert!(p.hash().starts_with("sha256:"));
    }

    #[test]
    fn age_counts_full_years() {
        let p = Profile::from_toml(PROFILE).unwrap();
        let d = |s| NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap();
        assert_eq!(p.age_on(d("2026-05-13")), 22);
        assert_eq!(p.age_on(d("2026-05-14")), 23);
    }

    #[test]
    fn rejects_bad_height() {
        let bad = PROFILE.replace("180.0", "300.0");
        assert!(matches!(Profile::from_toml(&bad), Err(Error::Config(_))));
    }
}
