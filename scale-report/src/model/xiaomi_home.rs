//! `xiaomi_home`: formulas extracted from the S400 firmware and published by
//! the bodymiscale project (Apache-2.0, see NOTICE). Spec §4.
//!
//! The intermediate `r()` calls are part of the algorithm — without them the
//! numbers drift from the Xiaomi Home app by 0.1.

use super::rounding::{clamp, r, trunc};
use super::{BodyInput, BodyMetrics, BodyModel, Sex};

pub struct XiaomiHome;

/// Regression coefficients, spec §4.3:
/// `[h, w, Z50, Z250, √h, √w, √Z50, √Z250, 1]`.
type Coef = [f64; 9];

const WATER: Coef = [
    2935892.0,
    149065.0,
    -468183.0,
    589094.0,
    -63286091.0,
    -778045.0,
    23941529.0,
    -29784715.0,
    424044966.0,
];
const BONE: Coef = [
    118543.0, 26106.0, -45544.0, 48749.0, -2271994.0, -155662.0, 2151182.0, -2248649.0, 12272558.0,
];
const SOLIDS: Coef = [
    160696.0, 28769.0, -60093.0, 65412.0, -3163648.0, -169504.0, 2860478.0, -3043837.0, 18045896.0,
];
const SKELETAL_MUSCLE: Coef = [
    2475795.0,
    118673.0,
    -457174.0,
    556218.0,
    -53616294.0,
    -574893.0,
    23989719.0,
    -28648257.0,
    352065003.0,
];
const VISCERAL_FAT: Coef = [
    -1849883.0,
    424124.0,
    366784.0,
    -455322.0,
    39648934.0,
    -359012.0,
    -20035428.0,
    24213934.0,
    -265509571.0,
];

fn regression(k: &Coef, h: f64, w: f64, z50: f64, z250: f64) -> f64 {
    let x = [
        h,
        w,
        z50,
        z250,
        h.sqrt(),
        w.sqrt(),
        z50.sqrt(),
        z250.sqrt(),
        1.0,
    ];
    1e-6 * k.iter().zip(x).map(|(k, x)| k * x).sum::<f64>()
}

/// Body fat, % (spec §4.2). Depends on Z50 only.
pub fn body_fat(sex: Sex, height_cm: f64, weight_kg: f64, age: u32, z50: f64) -> f64 {
    let [ch, cw, ca, cz, c0] = match sex {
        Sex::Male => [-0.3315, 0.6216, 0.0183, 0.0085, 22.554],
        Sex::Female => [-0.3332, 0.7509, 0.0196, 0.0072, 22.7193],
    };
    let raw = ch * height_cm + cw * weight_kg + ca * f64::from(age) + cz * z50 + c0;
    clamp(100.0 * raw / weight_kg, 3.0, 55.0)
}

impl BodyModel for XiaomiHome {
    fn name(&self) -> &'static str {
        "xiaomi_home"
    }

    fn compute(&self, input: &BodyInput) -> BodyMetrics {
        let BodyInput {
            sex,
            age,
            height_cm: h,
            weight_kg: w,
            ..
        } = *input;
        let Some(z50) = input.z50 else {
            return BodyMetrics::default();
        };

        // §4.2 — needs Z50 only.
        let bf = body_fat(sex, h, w, age, z50);
        let lbm = w - r(r(bf) * w / 100.0);
        let bmr = clamp(21.6 * r(lbm) + 370.0, 400.0, 3500.0);
        let mut m = BodyMetrics {
            body_fat_pct: Some(bf),
            // `w − LBM` rather than `w·BF/100`: matches the reference report
            // (M1 → 19.7 kg) and keeps fat + lean == weight exactly.
            fat_mass_kg: Some(w - lbm),
            lean_mass_kg: Some(lbm),
            bmr_kcal: Some(bmr),
            ..BodyMetrics::default()
        };

        // §4.3–4.6 — need both impedances.
        let Some(z250) = input.z250 else {
            return m;
        };
        let reg = |k: &Coef| regression(k, h, w, z50, z250);

        // §4.4 — water and protein; the order matters.
        let lbm_r = r(lbm);
        let solids = r(reg(&SOLIDS));
        let water = r(r(reg(&WATER)) + 0.03 * w);
        let water = r(clamp(r(water / w * 100.0), 20.0, 85.0) * w / 100.0);
        let protein_pct = (lbm_r - solids - water) / w * 100.0;
        let protein = r(clamp(r(protein_pct), 5.0, 32.0) * w / 100.0);
        let water_kg = lbm_r - solids - protein;

        // §4.5 — bones, muscle, visceral fat.
        let bone_raw = reg(&BONE);
        let bone = if r(bone_raw) < 0.3 {
            0.0
        } else {
            clamp(bone_raw, 0.0, 5.0)
        };
        let muscle = clamp(lbm - r(bone), 7.0, 141.5);
        let skeletal = clamp(reg(&SKELETAL_MUSCLE), 0.45 * r(muscle), 0.70 * r(muscle));
        let visceral = trunc(reg(&VISCERAL_FAT) + 0.5).clamp(1, 59);

        // §4.6 — metabolic age, adult branch.
        let (bmi_mid, bfp_mid) = match sex {
            Sex::Male => (22.0, 15.0),
            Sex::Female => (21.0, 23.0),
        };
        let sw = r(bmi_mid * h * h / 10000.0);
        let ref_lbm = r((100.0 - bfp_mid) * sw / 100.0);
        let ref_bmr = trunc(21.6 * ref_lbm + 370.5) as f64;
        let ref_muscle = r((100.0 - bfp_mid) * 0.944);
        let bmr_shown = trunc(bmr + 0.5) as f64;
        let muscle_pct = r(muscle / w * 100.0);
        let score = ((w - sw) * -1.48615435
            + (bmr_shown - ref_bmr) * 0.129089458
            + (r(bf) - bfp_mid) * 0.591458258
            + (muscle_pct - ref_muscle) * -0.000405611446)
            / 2.5;
        let shift = if score < -10.0 {
            -10
        } else if score >= 2.0 {
            2
        } else if score >= 0.0 {
            trunc(score) + 1
        } else {
            trunc(score)
        };
        let metabolic_age = trunc(f64::from(age) - shift as f64 - 4.0).clamp(18, 100);

        m.muscle_kg = Some(muscle);
        m.skeletal_muscle_kg = Some(skeletal);
        m.water_pct = Some(r(water_kg) / w * 100.0);
        m.protein_pct = Some(clamp(protein_pct, 5.0, 32.0));
        m.bone_kg = Some(bone);
        m.visceral_fat = Some(visceral);
        m.metabolic_age = Some(metabolic_age);
        m
    }
}
