//! Body-composition models. The pipeline only knows the [`BodyModel`] trait,
//! so an alternative model can be added without touching anything else.

pub mod rounding;
pub mod xiaomi_home;

use serde::{Deserialize, Serialize};

pub use xiaomi_home::XiaomiHome;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Sex {
    Male,
    Female,
}

/// Everything a model needs for one measurement.
#[derive(Debug, Clone, Copy)]
pub struct BodyInput {
    pub sex: Sex,
    /// Full years on the measurement date.
    pub age: u32,
    pub height_cm: f64,
    pub weight_kg: f64,
    /// Impedance at 50 kHz, Ω (the larger value).
    pub z50: Option<f64>,
    /// Impedance at 250 kHz, Ω (the smaller value).
    pub z250: Option<f64>,
}

/// Full-precision metrics. `None` = cannot be computed from the input.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct BodyMetrics {
    pub body_fat_pct: Option<f64>,
    pub fat_mass_kg: Option<f64>,
    pub lean_mass_kg: Option<f64>,
    pub muscle_kg: Option<f64>,
    pub skeletal_muscle_kg: Option<f64>,
    pub water_pct: Option<f64>,
    pub protein_pct: Option<f64>,
    pub bone_kg: Option<f64>,
    pub visceral_fat: Option<i64>,
    pub bmr_kcal: Option<f64>,
    pub metabolic_age: Option<i64>,
    pub bmi: Option<f64>,
    pub ffmi: Option<f64>,
    pub ffmi_normalized: Option<f64>,
    pub hydration_index: Option<f64>,
}

pub trait BodyModel {
    /// Written to the report as `model`.
    fn name(&self) -> &'static str;
    /// Model-specific metrics (everything except the derived ones in §4.7).
    fn compute(&self, input: &BodyInput) -> BodyMetrics;
}

/// Run `model` and add the model-independent metrics (spec §4.7).
pub fn evaluate(model: &dyn BodyModel, input: &BodyInput) -> BodyMetrics {
    let mut m = model.compute(input);
    let h = input.height_cm / 100.0;
    m.bmi = Some(input.weight_kg / (h * h));
    m.ffmi = m.lean_mass_kg.map(|lbm| lbm / (h * h));
    m.ffmi_normalized = m.ffmi.map(|ffmi| ffmi + 6.1 * (1.8 - h));
    m.hydration_index = match (input.z50, input.z250) {
        (Some(z50), Some(z250)) => Some(z250 / z50),
        _ => None,
    };
    m
}
