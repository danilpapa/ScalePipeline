//! `scale-report` — turns raw Xiaomi S400 measurements into body-composition
//! metrics and trends (see `dock/ТЗ обработчик данных весов S400.md`).
//!
//! The core is pure: [`process`] takes file contents, a profile and the
//! "today" date, and returns measurements plus a [`Report`]. No clock, no
//! network, no file system — reading and writing files lives in [`fs`].
//!
//! ```no_run
//! let data = scale_report::fs::load("../scale-data")?;
//! let today = data.profile.today();
//! let out = scale_report::process(&data.files, &data.profile, &data.overrides, today)?;
//! println!("{}", out.report.markdown());
//! # Ok::<(), anyhow::Error>(())
//! ```

/// A string-backed enum: `as_str`, `Display` and `Serialize` from one table.
macro_rules! str_enum {
    ($(#[$meta:meta])* pub enum $name:ident { $($(#[$vmeta:meta])* $variant:ident => $s:literal,)* }) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub enum $name { $($(#[$vmeta])* $variant,)* }

        impl $name {
            pub fn as_str(self) -> &'static str {
                match self { $(Self::$variant => $s,)* }
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str(self.as_str())
            }
        }

        impl serde::Serialize for $name {
            fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
                s.serialize_str(self.as_str())
            }
        }
    };
}

pub mod fs;
pub mod model;
pub mod profile;
pub mod raw;
pub mod report;
pub mod trends;
pub mod validate;

use chrono::NaiveDate;

pub use model::{BodyModel, XiaomiHome};
pub use profile::{Overrides, Profile};
pub use report::Report;
pub use validate::{Code, Measurement, Status};

/// One file from `weights/`.
#[derive(Debug, Clone)]
pub struct InputFile {
    pub name: String,
    pub bytes: Vec<u8>,
}

impl InputFile {
    pub fn new(name: impl Into<String>, bytes: impl Into<Vec<u8>>) -> Self {
        Self {
            name: name.into(),
            bytes: bytes.into(),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// Bad or missing profile, age < 18, … → exit code 2.
    #[error("configuration error: {0}")]
    Config(String),
    /// Not a single usable measurement → exit code 3.
    #[error("no valid measurements in weights/")]
    NoData,
}

impl Error {
    pub fn exit_code(&self) -> u8 {
        match self {
            Error::Config(_) => 2,
            Error::NoData => 3,
        }
    }
}

/// Result of a full pipeline run.
#[derive(Debug)]
pub struct Output {
    /// Every input file, in time order (rejected ones included).
    pub measurements: Vec<Measurement>,
    pub report: Report,
}

/// Full pipeline with the default `xiaomi_home` model.
pub fn process(
    files: &[InputFile],
    profile: &Profile,
    overrides: &Overrides,
    today: NaiveDate,
) -> Result<Output, Error> {
    process_with(&XiaomiHome, files, profile, overrides, today)
}

/// Full pipeline with any [`BodyModel`].
pub fn process_with(
    model: &dyn BodyModel,
    files: &[InputFile],
    profile: &Profile,
    overrides: &Overrides,
    today: NaiveDate,
) -> Result<Output, Error> {
    let mut measurements = validate::validate(files, profile, overrides, today)?;
    validate::compute_metrics(&mut measurements, model, profile);
    trends::annotate(&mut measurements, &profile.thresholds);
    let report = Report::build(&measurements, profile, today, model.name()).ok_or(Error::NoData)?;
    Ok(Output {
        measurements,
        report,
    })
}
