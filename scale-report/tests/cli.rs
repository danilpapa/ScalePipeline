//! End-to-end: the binary on an on-disk `scale-data` fixture.

use std::path::Path;
use std::process::Command;

const BIN: &str = env!("CARGO_BIN_EXE_scale-report");
const FIXTURE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/basic");

fn run(data: &Path, out: &Path) -> std::process::Output {
    Command::new(BIN)
        .args(["run", "--today", "2026-11-09", "--data"])
        .arg(data)
        .arg("--out")
        .arg(out)
        .output()
        .unwrap()
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
fn exit_2_without_profile() {
    let data = tempfile::tempdir().unwrap();
    let out = tempfile::tempdir().unwrap();
    let res = run(data.path(), out.path());
    assert_eq!(res.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&res.stderr).contains("profile.toml"));
}
