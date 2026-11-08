# ScalePipeline

Track body composition from a **Xiaomi Body Composition Scale S400** without the
Xiaomi app. An ESP32 reads the scale over Bluetooth and saves each weigh-in as a
JSON file in a private GitHub repo. The `scale-report` CLI turns those files into
metrics, trends and a short report.

```
S400 ──Bluetooth──▶ ESP32 (ESPHome) ──▶ private GitHub repo: weights/*.json
                                                    │
                                    scale-report run ◀┘
                                     ├─ report.json        (for scripts / an AI agent)
                                     ├─ report.md          (short summary, Russian)
                                     └─ measurements.csv   (every file, for debugging)
```

| Folder | What |
| --- | --- |
| `firmware/` | ESPHome config for the ESP32 |
| `scale-report/` | Rust CLI and library ([details](scale-report/README.md)) |
| `dock/` | Design spec with every formula and threshold (Russian) |

## Setup

**1. Data repo.** Create a private GitHub repo with a `weights/` folder. Create a
[fine-grained token](https://github.com/settings/personal-access-tokens) for that
repo only, with *Contents: Read and write*. The ESP32 uses it to upload files.

**2. ESP32.** In `firmware/scale-bridge.yaml`, replace `danilpapa/ScaleData` in the
upload URL with your repo, and set `timezone:` under `time:`. Then create
`firmware/secrets.yaml`:

```yaml
wifi_ssid: "..."
wifi_password: "..."
ota_password: "..."          # any string, for over-the-air updates
scale_mac: "AA:BB:CC:DD:EE:FF"
scale_bindkey: "..."         # the scale's BLE encryption key
gh_auth: "Bearer github_pat_..."
```

Flash it with `esphome run firmware/scale-bridge.yaml`. For how to find the MAC
and bindkey, see the ESPHome docs for the `xiaomi_body_scale` sensor.

**3. Weigh in.** About 10 seconds after a weigh-in, the ESP32 uploads a file such as
`weights/2026-11-09T07-30-00.json`. The board LED lights for 3 seconds when GitHub
accepts it:

```json
{"schema":2,"fw":"scale-bridge/2","ts":"2026-11-09T07:30:00+0300","weight":78.80,
 "impedance_low":470.40,"impedance_high":429.10,"heart_rate":68.00,"profile_id":1}
```

**4. Run the report.** Clone your data repo, then build and run the CLI with Rust
installed:

```sh
cd scale-report && cargo build --release

export PROFILE_SEX=male            # male | female
export PROFILE_BIRTH_DATE=1990-04-21
export PROFILE_HEIGHT_CM=180
# optional: PROFILE_TIMEZONE (default Europe/Moscow), PROFILE_GOAL (cut|bulk|maintain), PROFILE_ID

./target/release/scale-report run --data path/to/your-data-repo --out out
```

Instead of env variables, you can put a `profile.toml` in the data repo. See
[scale-report/README.md](scale-report/README.md) for that, for manual exclusions
(`overrides.toml`) and for the other commands.

Exit codes: `0` report written · `2` bad or missing profile, or invalid arguments ·
`3` no valid measurements · `1` file error.

## What you get

`report.json` includes today's metrics (weight, body fat, fat and lean mass,
muscle, water, protein, bone, visceral fat, BMR, metabolic age, BMI, FFMI). It
also has smoothed trends, weekly rates, 7/30/90-day changes marked as `real` or
`within_noise`, the detected phase (`cut` / `bulk` / `maintain`), warning flags,
and a list of every rejected or excluded file with the reason. A metric that can't
be computed is `null`.

Bad weigh-ins are detected and kept out of the trends. That covers weigh-ins in
socks (no impedance), duplicates, guests, impedance repeated from the previous
reading, and extra readings later in the day.

## Limitations

- These are bathroom-scale estimates, not medical measurements. The formulas come
  from the S400 firmware and match the reference values exactly, but those values
  have a large absolute error. The useful part is the trend, measured the same way
  every morning, barefoot.
- Body composition works with the S400 only. Weight trends work with any data in
  this JSON format.
- One person per setup, adults only.
- `report.md` and the design spec are in Russian.

Not medical advice.

## Credits and license

Body-composition formulas from [bodymiscale](https://github.com/dckiller51/bodymiscale)
(Apache-2.0), see [`scale-report/NOTICE`](scale-report/NOTICE). Bluetooth decoding
by [ESPHome](https://esphome.io).

Licensed under the [Apache License 2.0](LICENSE).
