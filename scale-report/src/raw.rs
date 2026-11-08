//! Raw JSON written by the ESP32 bridge (spec §2.1).

use chrono::{DateTime, FixedOffset};
use serde::Deserialize;

/// Newest `schema` this build understands.
pub const KNOWN_SCHEMA: u32 = 2;

/// One weighing as stored in `weights/*.json`. Unknown fields are ignored.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct RawMeasurement {
    /// Missing = version 1.
    pub schema: Option<u32>,
    pub ts: Option<String>,
    pub weight: Option<f64>,
    pub impedance_low: Option<f64>,
    pub impedance_high: Option<f64>,
    pub heart_rate: Option<f64>,
    pub profile_id: Option<i64>,
    pub fw: Option<String>,
}

impl RawMeasurement {
    pub fn parse(bytes: &[u8]) -> Result<Self, serde_json::Error> {
        serde_json::from_slice(bytes)
    }

    pub fn schema(&self) -> u32 {
        self.schema.unwrap_or(1)
    }
}

/// ISO 8601 with offset; accepts both `+0300` and `+03:00`.
pub fn parse_ts(s: &str) -> Option<DateTime<FixedOffset>> {
    DateTime::parse_from_rfc3339(s)
        .or_else(|_| DateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S%.f%z"))
        .ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn both_offset_styles() {
        let a = parse_ts("2026-11-09T08:15:30+0300").unwrap();
        let b = parse_ts("2026-11-09T08:15:30+03:00").unwrap();
        assert_eq!(a, b);
        assert_eq!(a.offset().local_minus_utc(), 3 * 3600);
    }
}
