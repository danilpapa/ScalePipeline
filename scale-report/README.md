# scale-report

Turns raw Xiaomi S400 measurements (`scale-data/weights/*.json`) into body
composition, trends and a short report for the fitness agent.
Spec: `../dock/ТЗ обработчик данных весов S400.md`.

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

Exit codes: `0` report written · `2` config error (no/bad profile, age < 18) ·
`3` no valid measurements · `1` I/O error.

## scale-data layout

```
scale-data/
  weights/*.json      # written by the ESP32 only
  profile.toml
  overrides.toml      # optional
```

`profile.toml`:

```toml
sex = "male"                 # or "female"
birth_date = 2003-05-14
height_cm = 180.0
timezone = "Europe/Moscow"
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

## Tests

`cargo test` runs the golden measurements (§8.1), property tests, all
pipeline scenarios (§8.2) and snapshots (`cargo insta review` after an
intentional output change).
