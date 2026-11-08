//! Spec §8.1: reference measurements must match the Xiaomi Home app exactly
//! after display rounding.

use scale_report::model::rounding::{r, trunc};
use scale_report::model::xiaomi_home::body_fat;
use scale_report::model::{BodyInput, BodyMetrics, Sex, XiaomiHome, evaluate};

struct Golden {
    id: &'static str,
    sex: Sex,
    age: u32,
    height: f64,
    weight: f64,
    z50: f64,
    z250: f64,
    // expected, as shown in the app
    fat_pct: f64,
    muscle: f64,
    water_pct: f64,
    bone: f64,
    protein_pct: f64,
    bmr: i64,
    visceral: i64,
    metabolic_age: i64,
}

#[rustfmt::skip]
const GOLDEN: &[Golden] = &[
    Golden { id: "P2",  sex: Sex::Male,   age: 26, height: 173.0, weight: 91.1, z50: 505.7, z250: 457.5, fat_pct: 29.2, muscle: 61.2, water_pct: 46.7, bone: 3.3, protein_pct: 19.8, bmr: 1763, visceral: 16, metabolic_age: 24 },
    Golden { id: "P3",  sex: Sex::Male,   age: 48, height: 183.0, weight: 79.1, z50: 458.6, z250: 408.5, fat_pct: 20.0, muscle: 59.7, water_pct: 62.1, bone: 3.6, protein_pct: 12.5, bmr: 1737, visceral: 5,  metabolic_age: 45 },
    Golden { id: "P9",  sex: Sex::Male,   age: 27, height: 178.0, weight: 88.0, z50: 570.0, z250: 510.6, fat_pct: 26.8, muscle: 61.0, water_pct: 49.1, bone: 3.4, protein_pct: 19.5, bmr: 1761, visceral: 14, metabolic_age: 25 },
    Golden { id: "P10", sex: Sex::Female, age: 26, height: 163.0, weight: 63.6, z50: 630.0, z250: 571.5, fat_pct: 33.3, muscle: 40.0, water_pct: 47.8, bone: 2.4, protein_pct: 14.3, bmr: 1286, visceral: 10, metabolic_age: 24 },
    Golden { id: "M1",  sex: Sex::Male,   age: 46, height: 176.0, weight: 79.9, z50: 576.7, z250: 514.6, fat_pct: 24.6, muscle: 57.0, water_pct: 51.3, bone: 3.2, protein_pct: 19.3, bmr: 1670, visceral: 11, metabolic_age: 44 },
];

fn run(g: &Golden) -> BodyMetrics {
    let input = BodyInput {
        sex: g.sex,
        age: g.age,
        height_cm: g.height,
        weight_kg: g.weight,
        z50: Some(g.z50),
        z250: Some(g.z250),
    };
    evaluate(&XiaomiHome, &input)
}

#[test]
fn golden_measurements_match_app() {
    for g in GOLDEN {
        let m = run(g);
        let got = (
            r(m.body_fat_pct.unwrap()),
            r(m.muscle_kg.unwrap()),
            r(m.water_pct.unwrap()),
            r(m.bone_kg.unwrap()),
            r(m.protein_pct.unwrap()),
            trunc(m.bmr_kcal.unwrap() + 0.5),
            m.visceral_fat.unwrap(),
            m.metabolic_age.unwrap(),
        );
        let want = (
            g.fat_pct,
            g.muscle,
            g.water_pct,
            g.bone,
            g.protein_pct,
            g.bmr,
            g.visceral,
            g.metabolic_age,
        );
        assert_eq!(got, want, "golden {}", g.id);
    }
}

#[test]
fn m1_lean_and_skeletal() {
    let m = run(&GOLDEN[4]);
    assert!((m.lean_mass_kg.unwrap() - 60.2).abs() <= 0.05);
    assert_eq!(r(m.skeletal_muscle_kg.unwrap()), 29.6);
}

#[test]
fn body_fat_lower_bound() {
    assert_eq!(body_fat(Sex::Male, 167.0, 41.4, 12, 701.0), 3.0);
}

#[test]
fn body_fat_single_point() {
    assert_eq!(r(body_fat(Sex::Male, 178.0, 79.0, 41, 606.3)), 23.5);
}

mod props {
    use super::*;
    use proptest::prelude::*;

    fn input() -> impl Strategy<Value = BodyInput> {
        (
            prop_oneof![Just(Sex::Male), Just(Sex::Female)],
            18u32..90,
            150.0..210.0f64,
            40.0..160.0f64,
            350.0..900.0f64,
            0.80..0.97f64,
        )
            .prop_map(|(sex, age, h, w, z50, ratio)| BodyInput {
                sex,
                age,
                height_cm: h,
                weight_kg: w,
                z50: Some(z50),
                z250: Some(z50 * ratio),
            })
    }

    proptest! {
        #[test]
        fn higher_z50_never_lowers_body_fat(i in input(), dz in 0.0..300.0f64) {
            let a = body_fat(i.sex, i.height_cm, i.weight_kg, i.age, i.z50.unwrap());
            let b = body_fat(i.sex, i.height_cm, i.weight_kg, i.age, i.z50.unwrap() + dz);
            prop_assert!(b >= a);
        }

        #[test]
        fn fat_plus_lean_is_weight(i in input()) {
            let m = evaluate(&XiaomiHome, &i);
            let sum = m.fat_mass_kg.unwrap() + m.lean_mass_kg.unwrap();
            prop_assert!((sum - i.weight_kg).abs() <= 0.1 + 1e-9);
        }

        #[test]
        fn skeletal_within_45_70_pct_of_muscle(i in input()) {
            let m = evaluate(&XiaomiHome, &i);
            let (sk, mu) = (m.skeletal_muscle_kg.unwrap(), r(m.muscle_kg.unwrap()));
            prop_assert!(sk >= 0.45 * mu - 1e-9 && sk <= 0.70 * mu + 1e-9);
        }

        #[test]
        fn all_metrics_finite(i in input()) {
            let m = evaluate(&XiaomiHome, &i);
            let floats = [
                m.body_fat_pct, m.fat_mass_kg, m.lean_mass_kg, m.muscle_kg,
                m.skeletal_muscle_kg, m.water_pct, m.protein_pct, m.bone_kg,
                m.bmr_kcal, m.bmi, m.ffmi, m.ffmi_normalized, m.hydration_index,
            ];
            for v in floats {
                prop_assert!(v.unwrap().is_finite());
            }
        }
    }
}
