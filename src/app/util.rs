//! Formatting and parsing helpers shared by the UI modules.

use std::path::Path;

pub fn nice_step(span: f64, target: usize) -> f64 {
    let raw = span / target.max(1) as f64;
    let mag = 10f64.powf(raw.log10().floor());
    let n = raw / mag;
    let s = if n < 1.5 {
        1.0
    } else if n < 3.5 {
        2.0
    } else if n < 7.5 {
        5.0
    } else {
        10.0
    };
    s * mag
}

/// Like [`nice_step`] but prefers minute-friendly steps above 10 seconds.
pub fn nice_time_step(span: f64, target: usize) -> f64 {
    let raw = span / target.max(1) as f64;
    if raw > 10.0 {
        for s in [15.0, 30.0, 60.0, 120.0, 300.0, 600.0, 900.0, 1800.0, 3600.0] {
            if s >= raw {
                return s;
            }
        }
        return 3600.0 * (raw / 3600.0).ceil();
    }
    nice_step(span, target)
}

/// Formats seconds with as many decimals as `step` needs; `long` uses m:ss.
pub fn format_time(t: f64, step: f64, long: bool) -> String {
    let decimals = (-step.log10().floor()).clamp(0.0, 6.0) as u32;
    let scale = 10i64.pow(decimals);
    let total = (t * scale as f64).round() as i64;
    let (whole, frac) = (total / scale, total % scale);
    let frac = if decimals > 0 {
        format!(".{:0w$}", frac, w = decimals as usize)
    } else {
        String::new()
    };
    if long {
        format!("{}:{:02}{frac}", whole / 60, whole % 60)
    } else {
        format!("{whole}{frac} s")
    }
}

pub fn freq_label(f: f64) -> String {
    if f >= 1000.0 {
        let k = f / 1000.0;
        if (k - k.round()).abs() < 1e-6 {
            format!("{}k", k.round())
        } else {
            format!("{k:.1}k")
        }
    } else {
        format!("{f:.0}")
    }
}

pub fn channel_name(channel: Option<usize>, count: usize) -> String {
    match (channel, count) {
        (None, _) => "Mix".into(),
        (Some(0), 2) => "Left".into(),
        (Some(1), 2) => "Right".into(),
        (Some(c), _) => format!("Ch {}", c + 1),
    }
}

pub fn file_name(path: &Path) -> String {
    path.file_name().map_or_else(
        || path.display().to_string(),
        |n| n.to_string_lossy().into_owned(),
    )
}

/// Parses "m:ss.fff", "h:mm:ss.fff" or plain seconds ("12.5").
pub fn parse_time(text: &str) -> Option<f64> {
    let mut total = 0.0;
    for part in text.trim().split(':') {
        let value: f64 = part.trim().parse().ok()?;
        if value < 0.0 {
            return None;
        }
        total = total * 60.0 + value;
    }
    Some(total)
}

/// "1.23 MB" style size.
pub fn format_bytes(bytes: u64) -> String {
    let b = bytes as f64;
    if b >= 1024.0 * 1024.0 * 1024.0 {
        format!("{:.2} GB", b / (1024.0 * 1024.0 * 1024.0))
    } else if b >= 1024.0 * 1024.0 {
        format!("{:.2} MB", b / (1024.0 * 1024.0))
    } else {
        format!("{:.1} KB", b / 1024.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn time_round_trip() {
        assert_eq!(parse_time("1:02.500"), Some(62.5));
        assert_eq!(parse_time("1:00:00"), Some(3600.0));
        assert_eq!(parse_time("12.25"), Some(12.25));
        assert_eq!(parse_time("abc"), None);
        assert_eq!(format_time(62.5, 0.001, true), "1:02.500");
        assert_eq!(parse_time(&format_time(62.5, 0.001, true)), Some(62.5));
    }
}
