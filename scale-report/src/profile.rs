//! `profile.toml` and `overrides.toml` (spec §2.3, §3).

use std::collections::BTreeMap;

use chrono::{Datelike, NaiveDate, Utc};
use chrono_tz::Tz;
use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize};
use sha2::{Digest, Sha256};

use crate::Error;
use crate::model::Sex;

/// Environment variables → profile fields. The first three are required
/// unless `profile.toml` provides them.
pub const ENV_VARS: [(&str, &str); 6] = [
    ("PROFILE_SEX", "sex"),
    ("PROFILE_BIRTH_DATE", "birth_date"),
    ("PROFILE_HEIGHT_CM", "height_cm"),
    ("PROFILE_TIMEZONE", "timezone"),
    ("PROFILE_GOAL", "goal"),
    ("PROFILE_ID", "profile_id"),
];

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
    /// IANA name: day boundary and morning window. Default `Europe/Moscow`.
    #[serde(default = "default_timezone")]
    pub timezone: String,
    /// `[start, end)` local hours for the daily measurement.
    #[serde(default = "default_morning_window")]
    pub morning_window: [u32; 2],
    #[serde(default)]
    pub goal: Option<Goal>,
    #[serde(default)]
    pub thresholds: Thresholds,
}

fn default_timezone() -> String {
    "Europe/Moscow".into()
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
        Self::resolve(Some(s), |_| None)
    }

    /// Build the profile from `profile.toml` (optional) and environment
    /// variables ([`ENV_VARS`]); env wins over the file. `env` is a lookup
    /// function so the core stays free of global state —
    /// pass `|k| std::env::var(k).ok()` in real use.
    pub fn resolve(
        toml_text: Option<&str>,
        env: impl Fn(&str) -> Option<String>,
    ) -> Result<Self, Error> {
        let mut table: toml::Table = match toml_text {
            Some(text) => text
                .parse()
                .map_err(|e| Error::Config(format!("profile.toml: {e}")))?,
            None => toml::Table::new(),
        };

        let mut from_env = false;
        for (var, key) in ENV_VARS {
            let Some(raw) = env(var) else { continue };
            let raw = raw.trim();
            if raw.is_empty() {
                continue;
            }
            let not_a = |what: &str| Error::Config(format!("{var}={raw:?} is not {what}"));
            let value = match key {
                "height_cm" => toml::Value::Float(raw.parse().map_err(|_| not_a("a number"))?),
                "profile_id" => toml::Value::Integer(raw.parse().map_err(|_| not_a("an integer"))?),
                _ => toml::Value::String(raw.to_owned()),
            };
            table.insert(key.into(), value);
            from_env = true;
        }

        if toml_text.is_none() && !from_env {
            return Err(Error::Config(
                "no profile: create profile.toml or set PROFILE_SEX, PROFILE_BIRTH_DATE, \
                 PROFILE_HEIGHT_CM"
                    .into(),
            ));
        }
        for (var, key) in &ENV_VARS[..3] {
            if !table.contains_key(*key) {
                return Err(Error::Config(format!(
                    "profile: `{key}` is missing — set it in profile.toml or {var}"
                )));
            }
        }

        let p: Profile = toml::Value::Table(table)
            .try_into()
            .map_err(|e| Error::Config(format!("profile: {e}")))?;
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

    fn env(vars: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let vars: Vec<(String, String)> = vars
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        move |k| {
            vars.iter()
                .rfind(|(name, _)| name == k) // later entries win
                .map(|(_, v)| v.clone())
        }
    }

    const ENV_PROFILE: &[(&str, &str)] = &[
        ("PROFILE_SEX", "female"),
        ("PROFILE_BIRTH_DATE", "1995-02-03"),
        ("PROFILE_HEIGHT_CM", "165"),
    ];

    #[test]
    fn env_only_profile_with_defaults() {
        let p = Profile::resolve(None, env(ENV_PROFILE)).unwrap();
        assert_eq!(p.sex, Sex::Female);
        assert_eq!(p.birth_date, NaiveDate::from_ymd_opt(1995, 2, 3).unwrap());
        assert_eq!(p.height_cm, 165.0);
        assert_eq!(p.timezone, "Europe/Moscow");
        assert_eq!((p.goal, p.profile_id), (None, None));
    }

    #[test]
    fn env_overrides_file() {
        let vars = [
            ("PROFILE_HEIGHT_CM", "181.5"),
            ("PROFILE_GOAL", "bulk"),
            ("PROFILE_ID", "2"),
            ("PROFILE_TIMEZONE", "Asia/Yekaterinburg"),
            ("PROFILE_SEX", ""), // empty = unset, the file value stays
        ];
        let p = Profile::resolve(Some(PROFILE), env(&vars)).unwrap();
        assert_eq!(p.sex, Sex::Male);
        assert_eq!(p.height_cm, 181.5);
        assert_eq!(p.goal, Some(Goal::Bulk));
        assert_eq!(p.profile_id, Some(2));
        assert_eq!(p.timezone, "Asia/Yekaterinburg");
        assert_eq!(p.thresholds.noise_fat_kg, 1.5, "file-only fields are kept");
    }

    #[test]
    fn no_file_no_env_is_config_error() {
        let err = Profile::resolve(None, env(&[])).unwrap_err();
        assert!(err.to_string().contains("PROFILE_SEX"), "{err}");
    }

    #[test]
    fn missing_required_names_the_variable() {
        let err = Profile::resolve(None, env(&ENV_PROFILE[..2])).unwrap_err();
        assert!(err.to_string().contains("PROFILE_HEIGHT_CM"), "{err}");
    }

    #[test]
    fn bad_env_values_are_config_errors() {
        let bad = |k: &'static str, v: &'static str| {
            let mut vars = ENV_PROFILE.to_vec();
            vars.push((k, v));
            Profile::resolve(None, env(&vars))
        };
        assert!(matches!(
            bad("PROFILE_HEIGHT_CM", "tall"),
            Err(Error::Config(_))
        ));
        assert!(matches!(bad("PROFILE_SEX", "other"), Err(Error::Config(_))));
        assert!(matches!(
            bad("PROFILE_BIRTH_DATE", "03.02.1995"),
            Err(Error::Config(_))
        ));
        assert!(matches!(
            bad("PROFILE_GOAL", "shred"),
            Err(Error::Config(_))
        ));
        assert!(matches!(
            bad("PROFILE_TIMEZONE", "Mars/Olympus"),
            Err(Error::Config(_))
        ));
    }

    #[test]
    fn rejects_bad_height() {
        let bad = PROFILE.replace("180.0", "300.0");
        assert!(matches!(Profile::from_toml(&bad), Err(Error::Config(_))));
    }
}
