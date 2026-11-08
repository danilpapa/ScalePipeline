//! Spec §8.2: pipeline scenarios.

use chrono::{Duration, NaiveDate, NaiveDateTime};
use serde_json::json;

use scale_report::model::rounding::r;
use scale_report::profile::parse_overrides;
use scale_report::trends::Flag;
use scale_report::{Code, Error, InputFile, Measurement, Output, Overrides, Profile, Status};

const PROFILE: &str = r#"
sex = "male"
birth_date = 1980-05-14
height_cm = 176.0
profile_id = 1
timezone = "Europe/Moscow"
goal = "cut"
"#;

const Z: Option<(f64, f64)> = Some((576.7, 514.6));

fn profile() -> Profile {
    Profile::from_toml(PROFILE).unwrap()
}

fn date(s: &str) -> NaiveDate {
    s.parse().unwrap()
}

/// File for a Moscow-local timestamp like `2026-11-09T08:15:30`.
fn meas(local: &str, weight: f64, z: Option<(f64, f64)>) -> InputFile {
    meas_json(
        local,
        json!({
            "ts": format!("{local}+0300"),
            "weight": weight,
            "impedance_low": z.map(|z| z.0),
            "impedance_high": z.map(|z| z.1),
            "heart_rate": null,
        }),
    )
}

fn meas_json(local: &str, body: serde_json::Value) -> InputFile {
    InputFile::new(
        format!("{}.json", local.replace(':', "-")),
        body.to_string(),
    )
}

fn run_with(files: &[InputFile], overrides: &Overrides, today: &str) -> Result<Output, Error> {
    scale_report::process(files, &profile(), overrides, date(today))
}

fn run(files: &[InputFile], today: &str) -> Output {
    run_with(files, &Overrides::new(), today).unwrap()
}

fn get<'a>(out: &'a Output, local: &str) -> &'a Measurement {
    let name = format!("{}.json", local.replace(':', "-"));
    out.measurements.iter().find(|m| m.file == name).unwrap()
}

/// `days` mornings starting at `start`, weight from `weight(i)`.
fn series(start: &str, days: i64, weight: impl Fn(i64) -> f64) -> Vec<InputFile> {
    let start: NaiveDateTime = format!("{start}T07:30:00").parse().unwrap();
    (0..days)
        .map(|i| {
            let t = (start + Duration::days(i))
                .format("%Y-%m-%dT%H:%M:%S")
                .to_string();
            meas(&t, weight(i), Z)
        })
        .collect()
}

#[test]
fn empty_folder_is_no_data() {
    assert!(matches!(
        run_with(&[], &Overrides::new(), "2026-11-09"),
        Err(Error::NoData)
    ));
}

#[test]
fn single_measurement() {
    let out = run(&[meas("2026-11-09T08:15:30", 79.9, Z)], "2026-11-09");
    let rep = &out.report;
    assert_eq!(rep.trend.weight_kg, Some(79.9));
    assert!(rep.flags.contains(&Flag::WarmingUp));
    let rates = &rep.rates;
    assert!(rates.weight_kg_per_week_14d.is_none() && rates.weight_pct_per_week_28d.is_none());
    assert!(rates.fat_mass_kg_per_week_28d.is_none() && rates.lean_mass_kg_per_week_28d.is_none());
    for d in [
        &rep.deltas.weight_kg,
        &rep.deltas.fat_mass_kg,
        &rep.deltas.lean_mass_kg,
    ] {
        assert!(d.d7.is_none() && d.d30.is_none() && d.d90.is_none());
    }
    // Latest matches golden M1 from §8.1.
    assert_eq!(rep.latest.body_fat_pct.map(r), Some(24.6));
    assert_eq!(rep.latest.lean_mass_kg.map(r), Some(60.2));
}

#[test]
fn broken_json_among_good_ones() {
    let files = [
        meas("2026-11-08T08:00:00", 80.0, Z),
        InputFile::new("2026-11-08T09-00-00.json", "{\"ts\": "),
        InputFile::new("2026-11-08T09-30-00.json", ""),
        InputFile::new("notes.txt", "hello"),
        meas("2026-11-09T08:00:00", 79.9, Z),
    ];
    let out = run(&files, "2026-11-09");
    let rejected = &out.report.data_quality.rejected;
    assert_eq!(rejected.len(), 3);
    assert!(rejected.iter().all(|d| d.code == Code::ParseError));
    assert_eq!(out.measurements.iter().filter(|m| m.daily).count(), 2);
}

#[test]
fn socks_weigh_in_is_weight_only() {
    let files = [
        meas("2026-11-08T08:00:00", 80.0, Z),
        meas("2026-11-09T08:00:00", 79.8, None),
    ];
    let out = run(&files, "2026-11-09");
    let m = get(&out, "2026-11-09T08:00:00");
    assert_eq!(m.status, Status::WeightOnly);
    assert!(m.has(Code::NoImpedance));
    assert!(m.weight_trend.is_some());
    assert!(out.report.latest.body_fat_pct.is_none());
    assert!(out.report.latest.fat_mass_kg.is_none());
    assert_eq!(out.report.latest.weight_kg, Some(79.8));
}

#[test]
fn only_z50_gives_partial_composition() {
    let f = meas_json(
        "2026-11-09T08:00:00",
        json!({
            "ts": "2026-11-09T08:00:00+0300", "weight": 79.9, "impedance_low": 576.7, "impedance_high": null,
        }),
    );
    let out = run(&[f], "2026-11-09");
    let l = &out.report.latest;
    assert_eq!(l.status, Status::Ok);
    assert!(l.flags.contains(&Code::NoZ250));
    assert!(l.body_fat_pct.is_some() && l.lean_mass_kg.is_some() && l.bmr_kcal.is_some());
    assert!(l.water_pct.is_none() && l.bone_kg.is_none() && l.muscle_kg.is_none());
    assert!(l.visceral_fat.is_none() && l.metabolic_age.is_none());
}

#[test]
fn swapped_impedances_give_the_same_result() {
    let right = run(
        &[meas("2026-11-09T08:00:00", 79.9, Some((576.7, 514.6)))],
        "2026-11-09",
    );
    let swapped = run(
        &[meas("2026-11-09T08:00:00", 79.9, Some((514.6, 576.7)))],
        "2026-11-09",
    );
    assert_eq!(right.report.json(), swapped.report.json());
}

#[test]
fn impedance_repeated_from_yesterday_is_stale() {
    let files = [
        meas("2026-11-08T08:00:00", 80.0, Z),
        meas("2026-11-09T08:00:00", 79.6, Z),
    ];
    let out = run(&files, "2026-11-09");
    let m = get(&out, "2026-11-09T08:00:00");
    assert!(m.has(Code::StaleImpedance));
    assert_eq!(m.status, Status::WeightOnly);
    assert!(m.z50.is_none());
}

#[test]
fn duplicate_keeps_the_more_complete_one() {
    let first = meas_json(
        "2026-11-09T08:00:00",
        json!({
            "ts": "2026-11-09T08:00:00+0300", "weight": 79.9, "impedance_low": 576.7, "impedance_high": null,
        }),
    );
    let second = meas("2026-11-09T08:02:00", 80.0, Z);
    let out = run(&[first, second], "2026-11-09");
    let a = get(&out, "2026-11-09T08:00:00");
    assert_eq!(
        (a.status, a.reason),
        (Status::Excluded, Some(Code::Duplicate))
    );
    assert!(get(&out, "2026-11-09T08:02:00").daily);
}

#[test]
fn morning_beats_evening() {
    let files = [
        meas("2026-11-09T07:30:00", 80.0, Z),
        meas("2026-11-09T21:00:00", 81.2, Some((560.0, 500.0))),
    ];
    let out = run(&files, "2026-11-09");
    assert!(get(&out, "2026-11-09T07:30:00").daily);
    assert_eq!(get(&out, "2026-11-09T21:00:00").status, Status::Extra);
    assert_eq!(out.report.latest.weight_kg, Some(80.0));
}

#[test]
fn evening_only_day_is_non_morning() {
    let out = run(&[meas("2026-11-09T21:00:00", 80.0, Z)], "2026-11-09");
    assert!(out.report.latest.flags.contains(&Code::NonMorning));
}

#[test]
fn guest_does_not_move_the_trend() {
    let mut files = series("2026-11-01", 8, |_| 80.0);
    let guest = meas("2026-11-05T19:00:00", 62.0, Some((650.0, 580.0)));
    files.push(guest);
    let out = run(&files, "2026-11-08");
    let g = get(&out, "2026-11-05T19:00:00");
    assert_eq!(
        (g.status, g.reason),
        (Status::Excluded, Some(Code::ForeignWeight))
    );
    assert_eq!(out.report.trend.weight_kg, Some(80.0));
}

#[test]
fn other_profile_slot_is_excluded() {
    let other = meas_json(
        "2026-11-09T08:00:00",
        json!({
            "schema": 2, "ts": "2026-11-09T08:00:00+0300", "weight": 60.0, "profile_id": 2,
        }),
    );
    let out = run(&[meas("2026-11-08T08:00:00", 80.0, Z), other], "2026-11-09");
    assert_eq!(
        get(&out, "2026-11-09T08:00:00").reason,
        Some(Code::ForeignProfile)
    );
}

#[test]
fn local_date_uses_profile_timezone() {
    // 23:30 Moscow = 20:30 UTC → Nov 8; 21:30 UTC = 00:30 Moscow → Nov 9.
    let late = meas_json(
        "a",
        json!({"ts": "2026-11-08T20:30:00+0000", "weight": 80.0}),
    );
    let after_midnight = meas_json(
        "b",
        json!({"ts": "2026-11-08T21:30:00+00:00", "weight": 80.0}),
    );
    let out = run(&[late, after_midnight], "2026-11-09");
    assert_eq!(get(&out, "a").date(), Some(date("2026-11-08")));
    assert_eq!(get(&out, "b").date(), Some(date("2026-11-09")));
}

#[test]
fn both_offset_formats() {
    let a = meas_json(
        "a",
        json!({"ts": "2026-11-09T08:15:30+0300", "weight": 80.0}),
    );
    let b = meas_json(
        "b",
        json!({"ts": "2026-11-10T08:15:30+03:00", "weight": 80.0}),
    );
    let out = run(&[a, b], "2026-11-10");
    assert_eq!(
        get(&out, "a").local.unwrap().to_string(),
        "2026-11-09 08:15:30"
    );
    assert_eq!(
        get(&out, "b").local.unwrap().to_string(),
        "2026-11-10 08:15:30"
    );
}

#[test]
fn twenty_day_break_resets_the_trend() {
    let mut files = series("2026-10-01", 10, |_| 85.0);
    files.push(meas("2026-10-30T08:00:00", 80.0, Some((560.0, 500.0))));
    let out = run(&files, "2026-10-30");
    assert!(get(&out, "2026-10-30T08:00:00").has(Code::TrendReset));
    assert_eq!(out.report.trend.weight_kg, Some(80.0));
    assert!(out.report.flags.contains(&Flag::TrendReset));
    assert!(out.report.flags.contains(&Flag::WarmingUp));
}

#[test]
fn birthday_between_measurements() {
    let files = [
        meas("2026-05-13T08:00:00", 80.0, Z),
        meas("2026-05-14T08:00:00", 80.0, Some((560.0, 500.0))),
    ];
    let out = run(&files, "2026-05-14");
    assert_eq!(get(&out, "2026-05-13T08:00:00").age, Some(45));
    assert_eq!(get(&out, "2026-05-14T08:00:00").age, Some(46));
}

#[test]
fn minor_is_a_config_error() {
    let young = PROFILE.replace("1980-05-14", "2010-01-01");
    let p = Profile::from_toml(&young).unwrap();
    let res = scale_report::process(
        &[meas("2026-11-09T08:00:00", 60.0, Z)],
        &p,
        &Overrides::new(),
        date("2026-11-09"),
    );
    assert!(matches!(res, Err(Error::Config(_))));
}

#[test]
fn single_file_checks() {
    let files = [
        meas_json(
            "future",
            json!({"ts": "2026-11-10T08:00:00+0300", "weight": 80.0}),
        ),
        meas_json(
            "no_ntp",
            json!({"ts": "1970-01-01T00:00:10+0000", "weight": 80.0}),
        ),
        meas_json(
            "heavy",
            json!({"ts": "2026-11-09T08:00:00+0300", "weight": 300.0}),
        ),
        meas_json(
            "schema",
            json!({"schema": 3, "ts": "2026-11-09T08:00:00+0300", "weight": 80.0}),
        ),
        meas_json(
            "hr",
            json!({"ts": "2026-11-09T07:00:00+0300", "weight": 80.0, "heart_rate": 240}),
        ),
        meas_json(
            "z",
            json!({"ts": "2026-11-08T07:00:00+0300", "weight": 80.0, "impedance_low": 150.0, "impedance_high": 140.0}),
        ),
        meas_json(
            "ratio",
            json!({"ts": "2026-11-07T07:00:00+0300", "weight": 80.0, "impedance_low": 600.0, "impedance_high": 400.0}),
        ),
    ];
    let out = run(&files, "2026-11-09");
    let code = |f: &str| get(&out, f).reason;
    assert_eq!(code("future"), Some(Code::BadTs));
    assert_eq!(code("no_ntp"), Some(Code::BadTs));
    assert_eq!(code("heavy"), Some(Code::BadWeight));
    assert_eq!(code("schema"), Some(Code::SchemaUnknown));
    assert_eq!(out.report.data_quality.warnings.len(), 1);
    let hr = get(&out, "hr");
    assert!(hr.has(Code::BadHeartRate) && hr.heart_rate.is_none());
    let z = get(&out, "z");
    assert!(z.has(Code::ZOutOfRange) && z.status == Status::WeightOnly);
    let ratio = get(&out, "ratio");
    assert!(ratio.has(Code::ZRatioSuspect) && ratio.metrics.body_fat_pct.is_some());
}

#[test]
fn manual_exclude_and_note() {
    let overrides = parse_overrides(
        r#"
        ["2026-11-08T08-00-00.json"]
        exclude = true
        ["2026-11-09T08-00-00.json"]
        note = "после тренировки"
        "#,
    )
    .unwrap();
    let files = [
        meas("2026-11-08T08:00:00", 80.0, Z),
        meas("2026-11-09T08:00:00", 79.9, Some((560.0, 500.0))),
    ];
    let out = run_with(&files, &overrides, "2026-11-09").unwrap();
    assert_eq!(
        get(&out, "2026-11-08T08:00:00").reason,
        Some(Code::ManualExclude)
    );
    assert_eq!(out.report.latest.note.as_deref(), Some("после тренировки"));
}

/// Deterministic noise in [−amp, amp].
fn noise(seed: &mut u64, amp: f64) -> f64 {
    *seed = seed
        .wrapping_mul(6364136223846793005)
        .wrapping_add(1442695040888963407);
    ((*seed >> 11) as f64 / (1u64 << 53) as f64 * 2.0 - 1.0) * amp
}

/// 60 mornings, −0.4 kg/week ± 0.6 kg, slightly varying impedance.
fn synthetic() -> Vec<InputFile> {
    let start: NaiveDateTime = "2026-09-11T07:30:00".parse().unwrap();
    let mut seed = 42;
    (0..60)
        .map(|i| {
            let t = (start + Duration::days(i))
                .format("%Y-%m-%dT%H:%M:%S")
                .to_string();
            let w = 84.0 - 0.4 / 7.0 * i as f64 + noise(&mut seed, 0.6);
            let z50 = 570.0 + noise(&mut seed, 8.0);
            let hr = 66.0 + noise(&mut seed, 4.0);
            meas_json(
                &t,
                json!({
                    "ts": format!("{t}+0300"),
                    "weight": (w * 10.0).round() / 10.0,
                    "impedance_low": (z50 * 10.0).round() / 10.0,
                    "impedance_high": (z50 * 0.89 * 10.0).round() / 10.0,
                    "heart_rate": hr.round(),
                }),
            )
        })
        .collect()
}

#[test]
fn synthetic_cut_rate_and_phase() {
    let out = run(&synthetic(), "2026-11-09");
    let rate = out.report.rates.weight_kg_per_week_28d.unwrap();
    assert!((rate + 0.4).abs() <= 0.1, "rate {rate}");
    assert_eq!(
        out.report.phase.detected,
        Some(scale_report::profile::Goal::Cut)
    );
    assert!(!out.report.flags.contains(&Flag::GoalMismatch));
    assert!(out.report.deltas.weight_kg.d30.is_some());
}

#[test]
fn same_input_same_bytes() {
    let a = run(&synthetic(), "2026-11-09");
    let b = run(&synthetic(), "2026-11-09");
    assert_eq!(a.report.json(), b.report.json());
    assert_eq!(a.report.markdown(), b.report.markdown());
    assert_eq!(
        scale_report::report::csv(&a.measurements),
        scale_report::report::csv(&b.measurements)
    );
}

#[test]
fn snapshot_synthetic_report() {
    let out = run(&synthetic(), "2026-11-09");
    insta::assert_snapshot!("report_json", out.report.json());
    insta::assert_snapshot!("report_md", out.report.markdown());
    assert!(out.report.markdown().lines().count() <= 10);
}

#[test]
fn year_of_data_is_fast() {
    let start: NaiveDateTime = "2026-01-01T07:30:00".parse().unwrap();
    let files: Vec<_> = (0..365)
        .map(|i| {
            let t = (start + Duration::days(i))
                .format("%Y-%m-%dT%H:%M:%S")
                .to_string();
            meas(
                &t,
                80.0 + (i % 7) as f64 * 0.1,
                Some((560.0 + (i % 5) as f64, 500.0)),
            )
        })
        .collect();
    let t = std::time::Instant::now();
    let out = run(&files, "2026-12-31");
    let _ = (
        out.report.json(),
        scale_report::report::csv(&out.measurements),
    );
    assert!(t.elapsed().as_secs_f64() < 1.0, "{:?}", t.elapsed());
}
