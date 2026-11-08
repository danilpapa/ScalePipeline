# scale-report

CLI and Rust library that turns raw Xiaomi S400 measurements
(`scale-data/weights/*.json`) into body composition, trends and short reports.
Part of [ScalePipeline](../README.md); start there for the hardware setup.
Design spec (in Russian): [`../dock/`](../dock/).

```sh
cargo build --release

# full pipeline → out/report.json, out/report.md, out/measurements.csv
scale-report run --data ../scale-data --out out            # --today defaults to today in profile.timezone
scale-report run --data ../scale-data --today 2026-11-09 --out out

# statuses of every file
scale-report validate --data ../scale-data

# one calculation, to compare with the Xiaomi Home app
scale-report calc --sex male --age 46 --height 176 --weight 79.9 --z50 576.7 --z250 514.6
```

Exit codes: `0` report written · `2` config error (no profile file or env, bad value, age < 18) ·
`3` no valid measurements · `1` I/O error.

## scale-data layout

```
scale-data/
  weights/*.json      # written by the ESP32 only
  profile.toml        # optional if PROFILE_* env variables are set
  overrides.toml      # optional
```

### Profile

From `profile.toml`, from environment variables, or both — **env overrides the
file field by field**. With neither, the exit code is `2`.

| variable | field | required |
| --- | --- | --- |
| `PROFILE_SEX` | `sex` — `male` / `female` | yes |
| `PROFILE_BIRTH_DATE` | `birth_date` — `YYYY-MM-DD` | yes |
| `PROFILE_HEIGHT_CM` | `height_cm` — 120–230 | yes |
| `PROFILE_TIMEZONE` | `timezone` — IANA, default `Europe/Moscow` | no |
| `PROFILE_GOAL` | `goal` — `cut` / `bulk` / `maintain` | no |
| `PROFILE_ID` | `profile_id` — scale slot 1–5 | no |

"Required" means: in env or in the file. Empty variables count as unset.
Thresholds (`[thresholds]`) and `morning_window` are file-only.

```sh
# e.g. secrets of the scheduled task — no personal data in any repo
export PROFILE_SEX=male
export PROFILE_BIRTH_DATE=2003-05-14
export PROFILE_HEIGHT_CM=180
export PROFILE_TIMEZONE=Europe/Moscow   # optional, this is the default
export PROFILE_GOAL=cut                 # optional
export PROFILE_ID=1                     # optional

scale-report run --data ../scale-data --out out
```

`profile.toml`:

```toml
sex = "male"                 # or "female"
birth_date = 2003-05-14
height_cm = 180.0
timezone = "Europe/Moscow"   # optional, this is the default
morning_window = [4, 12]     # optional, local hours [start, end)
profile_id = 1               # optional, scale user slot
goal = "cut"                 # optional: cut | bulk | maintain

[thresholds]                 # optional, any subset — see src/profile.rs
noise_fat_kg = 1.0
```

`overrides.toml` (keys are file names, quoted):

```toml
["2026-11-03T21-02-11.json"]
exclude = true

["2026-11-05T08-10-00.json"]
note = "после тренировки"
```

## As a library

The core is pure (no clock, network or file system), so other tools in the
project can reuse it:

```rust
let data = scale_report::fs::load("../scale-data")?;
let out = scale_report::process(&data.files, &data.profile, &data.overrides, data.profile.today())?;
println!("{}", out.report.json());
```

| module | spec | what |
| --- | --- | --- |
| `raw` | §2.1 | raw JSON, timestamp parsing |
| `profile` | §2.3 | `profile.toml`, thresholds, overrides, age, hash |
| `validate` | §3 | statuses, codes, duplicates, daily pick |
| `model` | §4 | `BodyModel` trait, `xiaomi_home`, firmware rounding |
| `trends` | §5 | EWMA, rolling medians, OLS rates, deltas, phase, flags |
| `report` | §6 | `report.json`, `report.md`, `measurements.csv` |
| `fs` | §7 | the only I/O: load data dir, write outputs |

## CI and releases

`.github/workflows/scale-report.yml` (repo root): every push runs
`cargo fmt --check`, `cargo clippy -- -D warnings` and `cargo test`. A `v*` tag
also builds a static `x86_64-unknown-linux-musl` binary and attaches it to a
GitHub Release as `scale-report-x86_64-linux`:

```sh
git tag v0.1.0 && git push origin v0.1.0

# in the scheduled task
gh release download v0.1.0 -R <owner>/ScalePipeline -p scale-report-x86_64-linux
chmod +x scale-report-x86_64-linux
```

## Tests

`cargo test` runs the golden measurements (§8.1), property tests, all
pipeline scenarios (§8.2) and snapshots (`cargo insta review` after an
intentional output change).

## License

Apache-2.0 — see [`../LICENSE`](../LICENSE). The `xiaomi_home` formulas come
from bodymiscale (Apache-2.0); attribution is in [`NOTICE`](NOTICE).
