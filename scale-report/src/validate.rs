//! Validation and filtering (spec §3): every file gets a status and codes.
//!
//! Pipeline: parse → overrides → single-measurement checks → sequence checks
//! (foreign profile, duplicates, stale impedance, foreign weight) → daily pick.

use std::collections::{BTreeMap, BTreeSet};

use chrono::{DateTime, Duration, FixedOffset, NaiveDate, NaiveDateTime, TimeZone, Timelike, Utc};

use crate::model::{BodyInput, BodyMetrics, BodyModel, evaluate};
use crate::profile::{Overrides, Profile};
use crate::raw::{KNOWN_SCHEMA, RawMeasurement, parse_ts};
use crate::trends::median;
use crate::{Error, InputFile};

str_enum! {
    pub enum Status {
        /// Weight and body composition.
        Ok => "ok",
        /// Weight only, composition not computed.
        WeightOnly => "weight_only",
        /// Not used in trends.
        Excluded => "excluded",
        /// Valid, but not the measurement picked for its day.
        Extra => "extra",
        /// Garbage.
        Rejected => "rejected",
    }
}

str_enum! {
    /// Reason codes and flags. Declaration order = output order.
    pub enum Code {
        ParseError => "parse_error",
        SchemaUnknown => "schema_unknown",
        BadTs => "bad_ts",
        BadWeight => "bad_weight",
        ManualExclude => "manual_exclude",
        ForeignProfile => "foreign_profile",
        ForeignWeight => "foreign_weight",
        Duplicate => "duplicate",
        NoImpedance => "no_impedance",
        NoZ250 => "no_z250",
        ZOutOfRange => "z_out_of_range",
        ZRatioSuspect => "z_ratio_suspect",
        StaleImpedance => "stale_impedance",
        BadHeartRate => "bad_heart_rate",
        WeightOutlier => "weight_outlier",
        CompositionJump => "composition_jump",
        NonMorning => "non_morning",
        TrendReset => "trend_reset",
    }
}

/// One input file after validation.
#[derive(Debug, Clone)]
pub struct Measurement {
    pub file: String,
    pub ts: Option<DateTime<FixedOffset>>,
    /// `ts` in the profile time zone.
    pub local: Option<NaiveDateTime>,
    pub status: Status,
    /// Why it is `rejected` / `excluded`.
    pub reason: Option<Code>,
    pub flags: BTreeSet<Code>,
    /// The measurement picked for its day.
    pub daily: bool,
    pub schema: u32,
    pub weight: Option<f64>,
    pub z50: Option<f64>,
    pub z250: Option<f64>,
    pub heart_rate: Option<f64>,
    pub profile_id: Option<i64>,
    pub note: Option<String>,
    /// Full years on the measurement date.
    pub age: Option<u32>,
    pub metrics: BodyMetrics,
    /// EWMA weight trend (daily measurements only).
    pub weight_trend: Option<f64>,
}

impl Measurement {
    fn new(file: &str) -> Self {
        Self {
            file: file.to_owned(),
            ts: None,
            local: None,
            status: Status::Ok,
            reason: None,
            flags: BTreeSet::new(),
            daily: false,
            schema: 1,
            weight: None,
            z50: None,
            z250: None,
            heart_rate: None,
            profile_id: None,
            note: None,
            age: None,
            metrics: BodyMetrics::default(),
            weight_trend: None,
        }
    }

    pub fn date(&self) -> Option<NaiveDate> {
        self.local.map(|l| l.date())
    }

    /// Valid and not excluded (`ok` or `weight_only`).
    pub fn is_accepted(&self) -> bool {
        matches!(self.status, Status::Ok | Status::WeightOnly)
    }

    pub fn has(&self, code: Code) -> bool {
        self.flags.contains(&code)
    }

    fn reject(&mut self, code: Code) {
        self.status = Status::Rejected;
        self.reason = Some(code);
        self.flags.insert(code);
    }

    fn exclude(&mut self, code: Code) {
        self.status = Status::Excluded;
        self.reason.get_or_insert(code);
        self.flags.insert(code);
    }

    fn drop_impedance(&mut self, code: Code) {
        self.z50 = None;
        self.z250 = None;
        self.flags.insert(code);
        if self.status == Status::Ok {
            self.status = Status::WeightOnly;
        }
    }

    /// Tie-break for duplicates: Z250 matters most, then filled fields.
    fn completeness(&self) -> (bool, usize) {
        let filled = [self.weight, self.z50, self.z250, self.heart_rate]
            .iter()
            .filter(|v| v.is_some())
            .count()
            + usize::from(self.profile_id.is_some());
        (self.z250.is_some(), filled)
    }

    fn minutes_since(&self, earlier: &Measurement) -> i64 {
        match (self.ts, earlier.ts) {
            (Some(a), Some(b)) => (a - b).num_minutes(),
            _ => i64::MAX,
        }
    }
}

/// Run §3 over all files. Result is sorted by time; rejected files without a
/// timestamp come first.
pub fn validate(
    files: &[InputFile],
    profile: &Profile,
    overrides: &Overrides,
    today: NaiveDate,
) -> Result<Vec<Measurement>, Error> {
    let tz = profile.tz();
    let th = &profile.thresholds;
    let latest_ok = tz
        .from_local_datetime(&(today + Duration::days(1)).and_hms_opt(0, 0, 0).unwrap())
        .earliest()
        .map(|t| t.with_timezone(&Utc))
        .unwrap_or(DateTime::<Utc>::MAX_UTC)
        + Duration::minutes(th.future_tolerance_min);
    let earliest_ok = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();

    // 1. Parse.  2. Overrides.  3. Single-measurement checks.
    let mut ms = Vec::with_capacity(files.len());
    for file in files {
        let mut m = parse(file, profile, latest_ok, earliest_ok);
        if let Some(o) = overrides.get(&file.name) {
            m.note.clone_from(&o.note);
            if o.exclude && m.status != Status::Rejected {
                m.exclude(Code::ManualExclude);
            }
        }
        if m.status != Status::Rejected {
            check_single(&mut m, profile)?;
        }
        ms.push(m);
    }
    ms.sort_by(|a, b| (a.ts, &a.file).cmp(&(b.ts, &b.file)));

    // 4. Sequence checks, in time order.
    foreign_profile(&mut ms, profile);
    duplicates(&mut ms, profile);
    stale_impedance(&mut ms, profile);
    foreign_weight(&mut ms, profile);

    // 5. One measurement per day.
    pick_daily(&mut ms, profile);
    Ok(ms)
}

fn parse(
    file: &InputFile,
    profile: &Profile,
    latest_ok: DateTime<Utc>,
    earliest_ok: NaiveDate,
) -> Measurement {
    let mut m = Measurement::new(&file.name);
    let th = &profile.thresholds;

    let looks_empty = file.bytes.iter().all(u8::is_ascii_whitespace);
    if !file.name.ends_with(".json") || looks_empty {
        m.reject(Code::ParseError);
        return m;
    }
    let Ok(raw) = RawMeasurement::parse(&file.bytes) else {
        m.reject(Code::ParseError);
        return m;
    };
    m.schema = raw.schema();
    m.weight = raw.weight.filter(|w| w.is_finite());
    m.heart_rate = raw.heart_rate.filter(|v| v.is_finite());
    m.profile_id = raw.profile_id;
    m.ts = raw.ts.as_deref().and_then(parse_ts);
    m.local = m.ts.map(|t| t.with_timezone(&profile.tz()).naive_local());

    // ESPHome and Home Assistant disagree on what "low"/"high" mean, so trust
    // physics instead of names: Z50 is always the larger one.
    let low = raw.impedance_low.filter(|v| v.is_finite());
    let high = raw.impedance_high.filter(|v| v.is_finite());
    (m.z50, m.z250) = match (low, high) {
        (Some(a), Some(b)) => (Some(a.max(b)), Some(a.min(b))),
        other => other,
    };

    if m.schema > KNOWN_SCHEMA {
        m.reject(Code::SchemaUnknown);
    } else if m.ts.is_none_or(|t| t.with_timezone(&Utc) > latest_ok)
        || m.date().is_none_or(|d| d < earliest_ok)
    {
        m.reject(Code::BadTs);
    } else if m
        .weight
        .is_none_or(|w| !(th.weight_min_kg..=th.weight_max_kg).contains(&w))
    {
        m.reject(Code::BadWeight);
    }
    m
}

fn check_single(m: &mut Measurement, profile: &Profile) -> Result<(), Error> {
    let th = &profile.thresholds;

    let date = m.date().expect("accepted measurements have a timestamp");
    let age = profile.age_on(date);
    if age < 18 {
        return Err(Error::Config(format!(
            "{}: age {age} on {date} — formulas for under-18s are out of scope",
            m.file
        )));
    }
    m.age = Some(age as u32);

    if let Some(hr) = m.heart_rate
        && !(th.heart_rate_min..=th.heart_rate_max).contains(&hr)
    {
        m.heart_rate = None;
        m.flags.insert(Code::BadHeartRate);
    }

    let in_range = |z: f64| (th.z_min..=th.z_max).contains(&z);
    if [m.z50, m.z250].into_iter().flatten().any(|z| !in_range(z)) {
        m.drop_impedance(Code::ZOutOfRange);
    }
    match (m.z50, m.z250) {
        (None, _) => m.drop_impedance(Code::NoImpedance),
        (Some(_), None) => {
            m.flags.insert(Code::NoZ250);
        }
        (Some(z50), Some(z250)) => {
            let ratio = z250 / z50;
            if z250 >= z50 || !(th.z_ratio_min..=th.z_ratio_max).contains(&ratio) {
                m.flags.insert(Code::ZRatioSuspect);
            }
        }
    }
    Ok(())
}

fn foreign_profile(ms: &mut [Measurement], profile: &Profile) {
    let Some(mine) = profile.profile_id else {
        return;
    };
    for m in ms.iter_mut().filter(|m| m.is_accepted()) {
        if m.profile_id.is_some_and(|id| id != mine) {
            m.exclude(Code::ForeignProfile);
        }
    }
}

/// Two measurements within a few minutes with nearly the same weight: keep
/// the more complete one, later on a tie.
fn duplicates(ms: &mut [Measurement], profile: &Profile) {
    let th = &profile.thresholds;
    let mut kept: Option<usize> = None;
    for i in 0..ms.len() {
        if !ms[i].is_accepted() {
            continue;
        }
        let Some(k) = kept else {
            kept = Some(i);
            continue;
        };
        let close = ms[i].minutes_since(&ms[k]) <= th.duplicate_window_min
            && (ms[i].weight.unwrap() - ms[k].weight.unwrap()).abs()
                <= th.duplicate_weight_kg + 1e-9;
        if !close {
            kept = Some(i);
        } else if ms[i].completeness() >= ms[k].completeness() {
            ms[k].exclude(Code::Duplicate);
            kept = Some(i);
        } else {
            ms[i].exclude(Code::Duplicate);
        }
    }
}

/// FW v1 bug: a sock weighing re-sends the previous impedance.
fn stale_impedance(ms: &mut [Measurement], profile: &Profile) {
    let th = &profile.thresholds;
    let same = |a: Option<f64>, b: Option<f64>| match (a, b) {
        (Some(a), Some(b)) => (a - b).abs() < 0.05,
        (None, None) => true,
        _ => false,
    };
    // Compare against the impedance as received, before any nulling.
    let received: Vec<_> = ms.iter().map(|m| (m.z50, m.z250)).collect();
    let mut prev: Option<usize> = None;
    for i in 0..ms.len() {
        if ms[i].status == Status::Rejected {
            continue;
        }
        if let Some(p) = prev {
            let (z50, z250) = received[i];
            if z50.is_some()
                && same(z50, received[p].0)
                && same(z250, received[p].1)
                && ms[i].minutes_since(&ms[p]) > th.stale_gap_min
            {
                ms[i].drop_impedance(Code::StaleImpedance);
            }
        }
        prev = Some(i);
    }
}

/// A guest on the scale: no `profile_id` and far from the recent median.
fn foreign_weight(ms: &mut [Measurement], profile: &Profile) {
    let th = &profile.thresholds;
    let mut history: Vec<(DateTime<FixedOffset>, f64)> = Vec::new();
    for m in ms.iter_mut().filter(|m| m.is_accepted()) {
        let (ts, w) = (m.ts.unwrap(), m.weight.unwrap());
        if m.profile_id.is_none() && history.len() >= th.foreign_weight_baseline {
            let horizon = ts - Duration::days(th.foreign_weight_days);
            let mut recent: Vec<f64> = history
                .iter()
                .filter(|(t, _)| *t >= horizon)
                .map(|p| p.1)
                .collect();
            if recent.is_empty() {
                // After a long break, compare with the last few known weights.
                let n = history.len().saturating_sub(th.foreign_weight_baseline);
                recent = history[n..].iter().map(|p| p.1).collect();
            }
            let med = median(&recent);
            if (w - med).abs() / med * 100.0 > th.foreign_weight_pct {
                m.exclude(Code::ForeignWeight);
                continue;
            }
        }
        history.push((ts, w));
    }
}

/// First accepted measurement in the morning window, else the first of the
/// day (`non_morning`). The rest of the day becomes `extra`.
fn pick_daily(ms: &mut [Measurement], profile: &Profile) {
    let [start, end] = profile.morning_window;
    let morning = |m: &Measurement| m.local.is_some_and(|l| (start..end).contains(&l.hour()));

    // Accepted measurements grouped by local date, in time order.
    let mut days: BTreeMap<NaiveDate, Vec<usize>> = BTreeMap::new();
    for (i, m) in ms.iter().enumerate().filter(|(_, m)| m.is_accepted()) {
        days.entry(m.date().unwrap()).or_default().push(i);
    }
    for day in days.values() {
        let pick = match day.iter().copied().find(|&j| morning(&ms[j])) {
            Some(j) => j,
            None => {
                ms[day[0]].flags.insert(Code::NonMorning);
                day[0]
            }
        };
        for &j in day {
            if j == pick {
                ms[j].daily = true;
            } else {
                ms[j].status = Status::Extra;
            }
        }
    }
}

/// Body composition for every measurement that has a weight.
pub fn compute_metrics(ms: &mut [Measurement], model: &dyn BodyModel, profile: &Profile) {
    for m in ms.iter_mut().filter(|m| m.status != Status::Rejected) {
        let (Some(weight_kg), Some(age)) = (m.weight, m.age) else {
            continue;
        };
        let input = BodyInput {
            sex: profile.sex,
            age,
            height_cm: profile.height_cm,
            weight_kg,
            z50: m.z50,
            z250: m.z250,
        };
        m.metrics = evaluate(model, &input);
    }
}
