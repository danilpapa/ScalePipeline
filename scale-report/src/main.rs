use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::Result;
use chrono::NaiveDate;
use clap::{Parser, Subcommand};

use scale_report::model::rounding::{r, trunc};
use scale_report::model::{BodyInput, BodyModel, Sex, XiaomiHome, evaluate};
use scale_report::report::f1;
use scale_report::{Error, Status, fs};

/// Body composition and trends from Xiaomi S400 scale data.
#[derive(Parser)]
#[command(version)]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Full pipeline → report.json, report.md, measurements.csv
    Run {
        /// `scale-data` checkout (profile.toml, weights/)
        #[arg(long)]
        data: PathBuf,
        /// Report date [default: today in profile.timezone]
        #[arg(long)]
        today: Option<NaiveDate>,
        /// Output directory
        #[arg(long, default_value = "out")]
        out: PathBuf,
    },
    /// Validation only: every file with its status
    Validate {
        #[arg(long)]
        data: PathBuf,
        #[arg(long)]
        today: Option<NaiveDate>,
    },
    /// One calculation without files — to compare with the Xiaomi Home app
    Calc {
        #[arg(long, value_parser = parse_sex)]
        sex: Sex,
        #[arg(long)]
        age: u32,
        /// cm
        #[arg(long)]
        height: f64,
        /// kg
        #[arg(long)]
        weight: f64,
        /// Ω, 50 kHz
        #[arg(long)]
        z50: Option<f64>,
        /// Ω, 250 kHz
        #[arg(long)]
        z250: Option<f64>,
    },
}

fn parse_sex(s: &str) -> Result<Sex, String> {
    match s {
        "male" | "m" => Ok(Sex::Male),
        "female" | "f" => Ok(Sex::Female),
        _ => Err("expected male or female".into()),
    }
}

fn main() -> ExitCode {
    match run(Cli::parse()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("scale-report: {e:#}");
            let code = e.downcast_ref::<Error>().map_or(1, Error::exit_code);
            ExitCode::from(code)
        }
    }
}

fn run(cli: Cli) -> Result<()> {
    match cli.cmd {
        Cmd::Run { data, today, out } => {
            let data = fs::load(&data)?;
            let today = today.unwrap_or_else(|| data.profile.today());
            let output = scale_report::process(&data.files, &data.profile, &data.overrides, today)?;
            for path in fs::write(&out, &output)? {
                eprintln!("wrote {}", path.display());
            }
            print!("{}", output.report.markdown());
        }
        Cmd::Validate { data, today } => {
            let data = fs::load(&data)?;
            let today = today.unwrap_or_else(|| data.profile.today());
            let mut ms = scale_report::validate::validate(
                &data.files,
                &data.profile,
                &data.overrides,
                today,
            )?;
            ms.sort_by(|a, b| a.file.cmp(&b.file));
            println!(
                "{:<26} {:<16} {:<12} {:>6} {:>7} {:>7}  flags",
                "file", "local time", "status", "kg", "Z50", "Z250"
            );
            for m in &ms {
                let local = m
                    .local
                    .map(|l| l.format("%Y-%m-%d %H:%M").to_string())
                    .unwrap_or_default();
                let mark = if m.daily { "*" } else { " " };
                let flags: Vec<&str> = m.flags.iter().map(|c| c.as_str()).collect();
                println!(
                    "{:<26} {:<16} {:<12} {:>6} {:>7} {:>7}  {}",
                    m.file,
                    local,
                    format!("{}{mark}", m.status),
                    f1(m.weight),
                    f1(m.z50),
                    f1(m.z250),
                    flags.join(" ")
                );
            }
            let count = |s: Status| ms.iter().filter(|m| m.status == s).count();
            println!(
                "\n{} files · {} ok · {} weight_only · {} extra · {} excluded · {} rejected   (* = daily)",
                ms.len(),
                count(Status::Ok),
                count(Status::WeightOnly),
                count(Status::Extra),
                count(Status::Excluded),
                count(Status::Rejected)
            );
        }
        Cmd::Calc {
            sex,
            age,
            height,
            weight,
            z50,
            z250,
        } => {
            // Same rule as the pipeline: Z50 is the larger impedance.
            let (z50, z250) = match (z50, z250) {
                (Some(a), Some(b)) => (Some(a.max(b)), Some(a.min(b))),
                other => other,
            };
            let input = BodyInput {
                sex,
                age,
                height_cm: height,
                weight_kg: weight,
                z50,
                z250,
            };
            let m = evaluate(&XiaomiHome, &input);
            let int = |v: Option<i64>| v.map_or("—".into(), |v| v.to_string());
            let one = |v: Option<f64>| v.map_or("—".into(), |v| format!("{:.1}", r(v)));
            let rows = [
                ("Body fat", one(m.body_fat_pct), "%"),
                ("Fat mass", one(m.fat_mass_kg), "kg"),
                ("Lean mass", one(m.lean_mass_kg), "kg"),
                ("Muscle", one(m.muscle_kg), "kg"),
                ("Skeletal muscle", one(m.skeletal_muscle_kg), "kg"),
                ("Water", one(m.water_pct), "%"),
                ("Protein", one(m.protein_pct), "%"),
                ("Bone", one(m.bone_kg), "kg"),
                ("Visceral fat", int(m.visceral_fat), ""),
                ("BMR", int(m.bmr_kcal.map(|v| trunc(v + 0.5))), "kcal"),
                ("Metabolic age", int(m.metabolic_age), "y"),
                ("BMI", one(m.bmi), ""),
                ("FFMI", one(m.ffmi), ""),
                ("FFMI normalized", one(m.ffmi_normalized), ""),
                (
                    "Hydration index",
                    m.hydration_index.map_or("—".into(), |v| format!("{v:.3}")),
                    "",
                ),
            ];
            println!(
                "{} · {sex:?}, {age} y, {height} cm, {weight} kg\n",
                XiaomiHome.name()
            );
            for (name, value, unit) in rows {
                println!("  {name:<16} {value:>7} {unit}");
            }
        }
    }
    Ok(())
}
