//! Output (spec §6): `report.json` for the agent, `report.md` for Telegram,
//! `measurements.csv` for debugging. Values are stored at full precision and
//! rounded only here.

use chrono::{Datelike, NaiveDate, SecondsFormat};
use serde::Serialize;

use crate::model::rounding::{r, round_to, trunc};
use crate::profile::{Goal, Profile};
use crate::trends::{self, Coverage, Deltas, Flag, Phase, Rates, Significance, Trend};
use crate::validate::{Code, Measurement, Status};

pub const REPORT_VERSION: u32 = 1;
pub const ALGO_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Serde helpers that round on output.
pub mod ser {
    use super::*;
    use serde::Serializer;

    fn opt<S: Serializer>(
        v: &Option<f64>,
        s: S,
        f: impl Fn(f64) -> f64,
    ) -> Result<S::Ok, S::Error> {
        // `+ 0.0` turns −0.0 into 0.0.
        v.map(|x| f(x) + 0.0).serialize(s)
    }
    /// Masses and percentages: firmware `r()`, 0.1.
    pub fn r1<S: Serializer>(v: &Option<f64>, s: S) -> Result<S::Ok, S::Error> {
        opt(v, s, r)
    }
    pub fn r2<S: Serializer>(v: &Option<f64>, s: S) -> Result<S::Ok, S::Error> {
        opt(v, s, |x| round_to(x, 2))
    }
    pub fn r3<S: Serializer>(v: &Option<f64>, s: S) -> Result<S::Ok, S::Error> {
        opt(v, s, |x| round_to(x, 3))
    }
    /// BMR, heart rate: nearest integer.
    pub fn int<S: Serializer>(v: &Option<f64>, s: S) -> Result<S::Ok, S::Error> {
        v.map(|x| trunc(x + 0.5)).serialize(s)
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Report {
    pub report_version: u32,
    pub algo_version: &'static str,
    pub model: &'static str,
    pub generated_for: NaiveDate,
    pub profile_hash: String,
    pub latest: Latest,
    pub trend: Trend,
    pub rates: Rates,
    pub deltas: Deltas,
    pub phase: Phase,
    pub flags: Vec<Flag>,
    pub data_quality: DataQuality,
}

/// The daily measurement of the most recent day.
#[derive(Debug, Clone, Serialize)]
pub struct Latest {
    pub date: NaiveDate,
    pub ts: String,
    pub status: Status,
    pub flags: Vec<Code>,
    pub note: Option<String>,
    #[serde(serialize_with = "ser::r1")]
    pub weight_kg: Option<f64>,
    #[serde(serialize_with = "ser::r1")]
    pub body_fat_pct: Option<f64>,
    #[serde(serialize_with = "ser::r1")]
    pub fat_mass_kg: Option<f64>,
    #[serde(serialize_with = "ser::r1")]
    pub lean_mass_kg: Option<f64>,
    #[serde(serialize_with = "ser::r1")]
    pub muscle_kg: Option<f64>,
    #[serde(serialize_with = "ser::r1")]
    pub skeletal_muscle_kg: Option<f64>,
    #[serde(serialize_with = "ser::r1")]
    pub water_pct: Option<f64>,
    #[serde(serialize_with = "ser::r1")]
    pub protein_pct: Option<f64>,
    #[serde(serialize_with = "ser::r1")]
    pub bone_kg: Option<f64>,
    pub visceral_fat: Option<i64>,
    #[serde(serialize_with = "ser::int")]
    pub bmr_kcal: Option<f64>,
    pub metabolic_age: Option<i64>,
    #[serde(serialize_with = "ser::r1")]
    pub bmi: Option<f64>,
    #[serde(serialize_with = "ser::r1")]
    pub ffmi: Option<f64>,
    #[serde(serialize_with = "ser::r1")]
    pub ffmi_normalized: Option<f64>,
    #[serde(serialize_with = "ser::r3")]
    pub hydration_index: Option<f64>,
    #[serde(serialize_with = "ser::int")]
    pub heart_rate: Option<f64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DataQuality {
    #[serde(flatten)]
    pub coverage: Coverage,
    pub files_total: usize,
    /// Rejected and excluded files with the reason.
    pub rejected: Vec<Dropped>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Dropped {
    pub file: String,
    pub code: Code,
}

impl Latest {
    fn from(m: &Measurement) -> Self {
        let x = &m.metrics;
        Self {
            date: m.date().expect("daily measurement has a date"),
            ts: m
                .ts
                .map(|t| t.to_rfc3339_opts(SecondsFormat::Secs, false))
                .unwrap_or_default(),
            status: m.status,
            flags: m.flags.iter().copied().collect(),
            note: m.note.clone(),
            weight_kg: m.weight,
            body_fat_pct: x.body_fat_pct,
            fat_mass_kg: x.fat_mass_kg,
            lean_mass_kg: x.lean_mass_kg,
            muscle_kg: x.muscle_kg,
            skeletal_muscle_kg: x.skeletal_muscle_kg,
            water_pct: x.water_pct,
            protein_pct: x.protein_pct,
            bone_kg: x.bone_kg,
            visceral_fat: x.visceral_fat,
            bmr_kcal: x.bmr_kcal,
            metabolic_age: x.metabolic_age,
            bmi: x.bmi,
            ffmi: x.ffmi,
            ffmi_normalized: x.ffmi_normalized,
            hydration_index: x.hydration_index,
            heart_rate: m.heart_rate,
        }
    }
}

impl Report {
    /// `None` when there is no usable daily measurement.
    pub fn build(
        ms: &[Measurement],
        profile: &Profile,
        today: NaiveDate,
        model: &'static str,
    ) -> Option<Self> {
        let a = trends::analyze(ms, profile, today)?;

        let mut rejected: Vec<Dropped> = ms
            .iter()
            .filter(|m| matches!(m.status, Status::Rejected | Status::Excluded))
            .filter_map(|m| {
                Some(Dropped {
                    file: m.file.clone(),
                    code: m.reason?,
                })
            })
            .collect();
        rejected.sort_by(|a, b| a.file.cmp(&b.file));
        let warnings = ms
            .iter()
            .filter(|m| m.reason == Some(Code::SchemaUnknown))
            .map(|m| {
                format!(
                    "{}: schema {} is newer than supported ({}) — update scale-report",
                    m.file,
                    m.schema,
                    crate::raw::KNOWN_SCHEMA
                )
            })
            .collect();

        Some(Self {
            report_version: REPORT_VERSION,
            algo_version: ALGO_VERSION,
            model,
            generated_for: today,
            profile_hash: profile.hash(),
            latest: Latest::from(a.latest),
            trend: a.trend,
            rates: a.rates,
            deltas: a.deltas,
            phase: a.phase,
            flags: a.flags,
            data_quality: DataQuality {
                coverage: a.coverage,
                files_total: ms.len(),
                rejected,
                warnings,
            },
        })
    }

    /// `report.json`: pretty, fixed key order, trailing newline.
    pub fn json(&self) -> String {
        let mut s = serde_json::to_string_pretty(self).expect("report serialises");
        s.push('\n');
        s
    }

    /// `report.md`: a short Telegram summary, at most 10 lines, no tables.
    pub fn markdown(&self) -> String {
        let l = &self.latest;
        let d = &self.deltas;
        let mut lines = vec![format!("**Весы · {}**", ru_date(self.generated_for))];

        // Weight.
        let mut weight = format!("Вес {} кг", f1(l.weight_kg));
        let mut extra = vec![];
        if let Some(t) = self.trend.weight_kg {
            extra.push(format!("тренд {}", f1(Some(t))));
        }
        if let Some(d7) = d.weight_kg.d7 {
            extra.push(format!("{} за неделю", signed(d7, 1)));
        }
        if !extra.is_empty() {
            weight += &format!(" ({})", extra.join(", "));
        }
        lines.push(weight);

        // Fat and lean mass.
        match (l.body_fat_pct, l.fat_mass_kg) {
            (Some(bf), Some(fat)) => {
                lines.push(format!(
                    "Жир {}% · {} кг{}",
                    f1(Some(bf)),
                    f1(Some(fat)),
                    change_note(&d.fat_mass_kg)
                ));
                lines.push(format!(
                    "Безжировая масса {} кг{}",
                    f1(l.lean_mass_kg),
                    change_note(&d.lean_mass_kg)
                ));
            }
            _ => lines.push("Состав тела: нет данных (замер без импеданса)".into()),
        }

        // Phase.
        let pct = self.rates.weight_pct_per_week_28d;
        lines.push(match (self.phase.detected, pct) {
            (Some(phase), Some(pct)) => {
                let verdict = if self.flags.contains(&Flag::LossTooFast) {
                    " — слишком быстро".to_owned()
                } else if self.flags.contains(&Flag::GainTooFast) {
                    " — набор слишком быстрый".to_owned()
                } else {
                    match self.phase.goal {
                        Some(goal) if goal == phase => " — в пределах цели".to_owned(),
                        Some(goal) => format!(" — цель: {}", ru_phase(goal)),
                        None => String::new(),
                    }
                };
                format!(
                    "Фаза: {}, {}%/нед{verdict}",
                    ru_phase(phase),
                    signed(pct, 2)
                )
            }
            _ => "Фаза: мало данных для оценки".into(),
        });

        lines.push(format!(
            "Данные: {} из 28 дней",
            self.data_quality.coverage.days_measured_28d
        ));

        let notes: Vec<String> = self.flags.iter().filter_map(|f| self.ru_flag(*f)).collect();
        if !notes.is_empty() {
            lines.push(format!("⚠️ {}", notes.join("; ")));
        }
        lines.join("\n") + "\n"
    }

    fn ru_flag(&self, f: Flag) -> Option<String> {
        Some(match f {
            Flag::WarmingUp => "тренд ещё набирает данные".into(),
            Flag::TrendReset => "тренд перезапущен после перерыва".into(),
            Flag::InsufficientData => "мало замеров состава".into(),
            Flag::StaleData => format!(
                "последний замер {} дн. назад",
                self.data_quality.coverage.days_since_last
            ),
            Flag::LeanLoss => "падает безжировая масса".into(),
            Flag::FatGainOnBulk => "набор идёт в основном жиром".into(),
            Flag::HydrationShift => "сдвиг водного баланса, состав неточен".into(),
            Flag::HrElevated => "пульс выше обычного".into(),
            // Already in the phase line.
            Flag::GoalMismatch | Flag::LossTooFast | Flag::GainTooFast => return None,
        })
    }
}

/// `measurements.csv`: one row per input file, newest first.
pub fn csv(ms: &[Measurement]) -> String {
    let mut rows: Vec<&Measurement> = ms.iter().collect();
    rows.sort_by(|a, b| b.file.cmp(&a.file));

    let mut w = csv::Writer::from_writer(Vec::new());
    w.write_record([
        "file",
        "ts",
        "local_date",
        "status",
        "flags",
        "daily",
        "weight_kg",
        "z50",
        "z250",
        "heart_rate",
        "body_fat_pct",
        "fat_mass_kg",
        "lean_mass_kg",
        "muscle_kg",
        "skeletal_muscle_kg",
        "water_pct",
        "protein_pct",
        "bone_kg",
        "visceral_fat",
        "bmr_kcal",
        "metabolic_age",
        "ffmi",
        "hydration_index",
        "weight_trend_kg",
    ])
    .expect("csv to memory");
    for m in rows {
        let x = &m.metrics;
        let flags: Vec<&str> = m.flags.iter().map(|c| c.as_str()).collect();
        w.write_record([
            m.file.clone(),
            m.ts.map(|t| t.to_rfc3339_opts(SecondsFormat::Secs, false))
                .unwrap_or_default(),
            m.date().map(|d| d.to_string()).unwrap_or_default(),
            m.status.to_string(),
            flags.join(";"),
            m.daily.to_string(),
            f1(m.weight),
            f1(m.z50),
            f1(m.z250),
            int(m.heart_rate),
            f1(x.body_fat_pct),
            f1(x.fat_mass_kg),
            f1(x.lean_mass_kg),
            f1(x.muscle_kg),
            f1(x.skeletal_muscle_kg),
            f1(x.water_pct),
            f1(x.protein_pct),
            f1(x.bone_kg),
            opt(x.visceral_fat),
            int(x.bmr_kcal),
            opt(x.metabolic_age),
            f1(x.ffmi),
            x.hydration_index
                .map(|v| format!("{:.3}", round_to(v, 3)))
                .unwrap_or_default(),
            f1(m.weight_trend),
        ])
        .expect("csv to memory");
    }
    String::from_utf8(w.into_inner().expect("csv flush")).expect("csv is utf-8")
}

// ---------------------------------------------------------------- formatting

/// One decimal after firmware rounding; empty for `None`.
pub fn f1(v: Option<f64>) -> String {
    v.map(|x| format!("{:.1}", r(x) + 0.0)).unwrap_or_default()
}

fn int(v: Option<f64>) -> String {
    v.map(|x| trunc(x + 0.5).to_string()).unwrap_or_default()
}

fn opt(v: Option<i64>) -> String {
    v.map(|x| x.to_string()).unwrap_or_default()
}

/// `+0.4` / `−0.3` with a real minus sign.
fn signed(x: f64, digits: i32) -> String {
    let x = round_to(x, digits);
    let body = format!("{:.*}", digits as usize, x.abs());
    match x {
        x if x > 0.0 => format!("+{body}"),
        x if x < 0.0 => format!("−{body}"),
        _ => body,
    }
}

fn change_note(d: &trends::Delta) -> String {
    match (d.significance, d.d30) {
        (Significance::Real, Some(d30)) => format!(" ({} кг за 30 дней)", signed(d30, 1)),
        (Significance::WithinNoise, _) => " (в пределах шума)".into(),
        _ => String::new(),
    }
}

fn ru_phase(g: Goal) -> &'static str {
    match g {
        Goal::Cut => "сушка",
        Goal::Bulk => "набор",
        Goal::Maintain => "поддержание",
    }
}

fn ru_date(d: NaiveDate) -> String {
    const MONTHS: [&str; 12] = [
        "января",
        "февраля",
        "марта",
        "апреля",
        "мая",
        "июня",
        "июля",
        "августа",
        "сентября",
        "октября",
        "ноября",
        "декабря",
    ];
    format!("{} {}", d.day(), MONTHS[d.month0() as usize])
}
