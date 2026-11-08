//! Rounding helpers that reproduce the S400 firmware (spec §4.1).

/// Round to 0.1, half away from zero — exactly as the firmware does.
pub fn r(x: f64) -> f64 {
    if x >= 0.0 {
        (10.0 * x + 0.5).floor() / 10.0
    } else {
        (10.0 * x - 0.5).ceil() / 10.0
    }
}

/// Drop the fractional part towards zero.
pub fn trunc(x: f64) -> i64 {
    x as i64
}

/// `clamp` that never panics (bounds may coincide or come from data).
pub fn clamp(x: f64, lo: f64, hi: f64) -> f64 {
    x.max(lo).min(hi)
}

/// Round half away from zero to `digits` decimals — for report output only.
pub fn round_to(x: f64, digits: i32) -> f64 {
    let k = 10f64.powi(digits);
    (x * k).round() / k
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn r_matches_firmware() {
        assert_eq!(r(23.349), 23.3);
        assert_eq!(r(23.35), 23.4);
        assert_eq!(r(23.3499), 23.3);
        assert_eq!(r(-23.35), -23.4);
    }
}
