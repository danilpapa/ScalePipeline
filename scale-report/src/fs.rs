//! The thin I/O layer: read a `scale-data` checkout, write report files.
//! Everything else in the crate is pure.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::profile::{Overrides, Profile, parse_overrides};
use crate::{Error, InputFile, Output, report};

/// Contents of a `scale-data` directory.
#[derive(Debug)]
pub struct DataDir {
    pub profile: Profile,
    pub overrides: Overrides,
    /// `weights/*`, sorted by name. Hidden files are skipped.
    pub files: Vec<InputFile>,
}

pub fn load(dir: impl AsRef<Path>) -> Result<DataDir> {
    let dir = dir.as_ref();

    let profile_path = dir.join("profile.toml");
    let text = std::fs::read_to_string(&profile_path)
        .map_err(|e| Error::Config(format!("{}: {e}", profile_path.display())))?;
    let profile = Profile::from_toml(&text)?;

    let overrides_path = dir.join("overrides.toml");
    let overrides = match std::fs::read_to_string(&overrides_path) {
        Ok(text) => parse_overrides(&text)?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Overrides::new(),
        Err(e) => return Err(e).context(overrides_path.display().to_string()),
    };

    let weights = dir.join("weights");
    let mut files = Vec::new();
    if weights.is_dir() {
        for entry in std::fs::read_dir(&weights).with_context(|| weights.display().to_string())? {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') || !entry.file_type()?.is_file() {
                continue;
            }
            let bytes =
                std::fs::read(entry.path()).with_context(|| entry.path().display().to_string())?;
            files.push(InputFile { name, bytes });
        }
    }
    files.sort_by(|a, b| a.name.cmp(&b.name));

    Ok(DataDir {
        profile,
        overrides,
        files,
    })
}

/// Write `report.json`, `report.md` and `measurements.csv` into `out`.
pub fn write(out: impl AsRef<Path>, output: &Output) -> Result<Vec<PathBuf>> {
    let out = out.as_ref();
    std::fs::create_dir_all(out).with_context(|| out.display().to_string())?;
    let files = [
        ("report.json", output.report.json()),
        ("report.md", output.report.markdown()),
        ("measurements.csv", report::csv(&output.measurements)),
    ];
    let mut written = Vec::new();
    for (name, content) in files {
        let path = out.join(name);
        std::fs::write(&path, content).with_context(|| path.display().to_string())?;
        written.push(path);
    }
    Ok(written)
}
