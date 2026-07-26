pub const SPARK_TICKS: [char; 8] = [
    '\u{2581}', '\u{2582}', '\u{2583}', '\u{2584}', '\u{2585}', '\u{2586}', '\u{2587}', '\u{2588}',
];

/// Map values onto the eight block glyphs. Non-finite values are skipped;
/// explicit bounds clamp instead of stretching.
pub fn sparkline(values: &[f64], min: Option<f64>, max: Option<f64>) -> String {
    let finite: Vec<f64> = values.iter().copied().filter(|v| v.is_finite()).collect();
    if finite.is_empty() {
        return String::new();
    }
    let lo = min.unwrap_or_else(|| finite.iter().copied().fold(f64::INFINITY, f64::min));
    let hi = max.unwrap_or_else(|| finite.iter().copied().fold(f64::NEG_INFINITY, f64::max));
    let span = hi - lo;
    finite
        .iter()
        .map(|&v| {
            let idx = if span <= 0.0 {
                0
            } else {
                let norm = ((v - lo) / span).clamp(0.0, 1.0);
                (norm * 7.0).round() as usize
            };
            SPARK_TICKS[idx]
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_input_is_empty_output() {
        assert_eq!(sparkline(&[], None, None), "");
    }

    #[test]
    fn full_ramp() {
        let values: Vec<f64> = (0..8).map(f64::from).collect();
        assert_eq!(sparkline(&values, None, None), "▁▂▃▄▅▆▇█");
    }

    #[test]
    fn all_equal_is_flat() {
        assert_eq!(sparkline(&[5.0, 5.0, 5.0], None, None), "▁▁▁");
    }

    #[test]
    fn single_value_is_lowest() {
        assert_eq!(sparkline(&[42.0], None, None), "▁");
    }

    #[test]
    fn explicit_bounds_clamp() {
        assert_eq!(
            sparkline(&[-10.0, 0.0, 10.0, 20.0], Some(0.0), Some(10.0)),
            "▁▁██"
        );
    }

    #[test]
    fn non_finite_values_are_skipped() {
        assert_eq!(
            sparkline(&[0.0, f64::NAN, 7.0, f64::INFINITY], None, None),
            "▁█"
        );
    }
}
