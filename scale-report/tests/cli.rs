//! End-to-end: the binary on an on-disk `scale-data` fixture.

use std::path::Path;
use std::process::Command;

const BIN: &str = env!("CARGO_BIN_EXE_scale-report");
const FIXTURE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/basic");

const PROFILE_VARS: [&str; 6] = [
    "PROFILE_SEX",
    "PROFILE_BIRTH_DATE",
    "PROFILE_HEIGHT_CM",
    "PROFILE_TIMEZONE",
    "PROFILE_GOAL",
    "PROFILE_ID",
];

/// `scale-report run` with a clean PROFILE_* environment plus `env`.
fn run_env(data: &Path, out: &Path, env: &[(&str, &str)]) -> std::process::Output {
    let mut cmd = Command::new(BIN);
    for var in PROFILE_VARS {
        cmd.env_remove(var);
    }
    cmd.envs(env.iter().copied())
        .args(["run", "--today", "2026-11-09", "--data"])
        .arg(data)
        .arg("--out")
        .arg(out)
        .output()
        .unwrap()
}

fn run(data: &Path, out: &Path) -> std::process::Output {
    run_env(data, out, &[])
}

#[test]
fn run_writes_three_files() {
    let out = tempfile::tempdir().unwrap();
    let res = run(Path::new(FIXTURE), out.path());
    assert_eq!(
        res.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&res.stderr)
    );

    let json = std::fs::read_to_string(out.path().join("report.json")).unwrap();
    let md = std::fs::read_to_string(out.path().join("report.md")).unwrap();
    let csv = std::fs::read_to_string(out.path().join("measurements.csv")).unwrap();
    assert_eq!(csv.lines().count(), 1 + 7, "header + one row per file");
    assert!(
        csv.lines()
            .nth(1)
            .unwrap()
            .starts_with("2026-11-09T07-31-00.json,"),
        "newest first"
    );
    insta::assert_snapshot!("fixture_report_json", json);
    insta::assert_snapshot!("fixture_report_md", md);
    insta::assert_snapshot!("fixture_measurements_csv", csv);
}

#[test]
fn exit_3_without_measurements() {
    let data = tempfile::tempdir().unwrap();
    std::fs::copy(
        Path::new(FIXTURE).join("profile.toml"),
        data.path().join("profile.toml"),
    )
    .unwrap();
    std::fs::create_dir(data.path().join("weights")).unwrap();
    let out = tempfile::tempdir().unwrap();
    assert_eq!(run(data.path(), out.path()).status.code(), Some(3));
}

#[test]
fn profile_from_env_only() {
    let data = tempfile::tempdir().unwrap();
    std::fs::create_dir(data.path().join("weights")).unwrap();
    for entry in std::fs::read_dir(Path::new(FIXTURE).join("weights")).unwrap() {
        let entry = entry.unwrap();
        std::fs::copy(
            entry.path(),
            data.path().join("weights").join(entry.file_name()),
        )
        .unwrap();
    }
    let out = tempfile::tempdir().unwrap();
    let env = [
        ("PROFILE_SEX", "male"),
        ("PROFILE_BIRTH_DATE", "1980-05-14"),
        ("PROFILE_HEIGHT_CM", "176"),
        ("PROFILE_GOAL", "maintain"),
    ];
    let res = run_env(data.path(), out.path(), &env);
    assert_eq!(
        res.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&res.stderr)
    );

    // Same profile as the fixture's profile.toml → same report, same hash.
    let from_file = tempfile::tempdir().unwrap();
    run(Path::new(FIXTURE), from_file.path());
    let read = |dir: &Path| std::fs::read_to_string(dir.join("report.json")).unwrap();
    assert_eq!(read(out.path()), read(from_file.path()));
}

#[test]
fn env_overrides_profile_file() {
    let out = tempfile::tempdir().unwrap();
    let res = run_env(Path::new(FIXTURE), out.path(), &[("PROFILE_GOAL", "bulk")]);
    assert_eq!(res.status.code(), Some(0));
    let json = std::fs::read_to_string(out.path().join("report.json")).unwrap();
    assert!(json.contains("\"goal\": \"bulk\""));
}

#[test]
fn exit_2_with_incomplete_env() {
    let data = tempfile::tempdir().unwrap();
    let out = tempfile::tempdir().unwrap();
    let res = run_env(data.path(), out.path(), &[("PROFILE_SEX", "male")]);
    assert_eq!(res.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&res.stderr).contains("PROFILE_BIRTH_DATE"));
}

#[test]
fn exit_2_without_profile() {
    let data = tempfile::tempdir().unwrap();
    let out = tempfile::tempdir().unwrap();
    let res = run(data.path(), out.path());
    assert_eq!(res.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&res.stderr);
    assert!(
        stderr.contains("profile.toml") && stderr.contains("PROFILE_SEX"),
        "{stderr}"
    );
}
