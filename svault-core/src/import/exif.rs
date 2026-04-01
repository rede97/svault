//! EXIF metadata extraction for import pipeline.

use std::path::Path;

/// Returns `(taken_ms, device)` from EXIF metadata, with fallbacks.
/// - `taken_ms`: EXIF `DateTimeOriginal` → `DateTime` → `mtime_ms` fallback
/// - `device`:   `"Make Model"` sanitised for path use → `"Unknown"` fallback
pub fn read_exif_date_device(path: &Path, mtime_ms: i64) -> (i64, String) {
    use std::fs::File;
    use std::io::BufReader;

    let Ok(file) = File::open(path) else {
        return (mtime_ms, "Unknown".to_string());
    };
    let mut reader = BufReader::new(file);
    let Ok(exif) = exif::Reader::new().read_from_container(&mut reader) else {
        return (mtime_ms, "Unknown".to_string());
    };

    // Date: prefer DateTimeOriginal, fallback to DateTime
    let taken_ms = exif
        .get_field(exif::Tag::DateTimeOriginal, exif::In::PRIMARY)
        .or_else(|| exif.get_field(exif::Tag::DateTime, exif::In::PRIMARY))
        .and_then(|f| {
            if let exif::Value::Ascii(ref vec) = f.value {
                vec.first().and_then(|b| {
                    let s = std::str::from_utf8(b).ok()?;
                    parse_exif_datetime_ms(s)
                })
            } else {
                None
            }
        })
        .unwrap_or(mtime_ms);

    // Device: "Make Model", sanitised for use as a path component
    let make = exif
        .get_field(exif::Tag::Make, exif::In::PRIMARY)
        .and_then(|f| exif_ascii_first(&f.value))
        .unwrap_or_default();
    let model = exif
        .get_field(exif::Tag::Model, exif::In::PRIMARY)
        .and_then(|f| exif_ascii_first(&f.value))
        .unwrap_or_default();
    let device = if make.is_empty() && model.is_empty() {
        "Unknown".to_string()
    } else {
        let raw = if make.is_empty() || model.starts_with(&make) {
            model // avoid "Apple Apple iPhone"
        } else {
            format!("{make} {model}")
        };
        // Replace path-unsafe chars with '_'
        raw.chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == ' ' {
                    c
                } else {
                    '_'
                }
            })
            .collect::<String>()
            .trim()
            .to_string()
    };

    (taken_ms, device)
}

fn exif_ascii_first(v: &exif::Value) -> Option<String> {
    if let exif::Value::Ascii(vec) = v {
        vec.first()
            .and_then(|b| std::str::from_utf8(b).ok())
            .map(|s| s.trim_end_matches('\0').trim().to_string())
    } else {
        None
    }
}

/// Parse EXIF datetime string `"YYYY:MM:DD HH:MM:SS"` → Unix milliseconds.
fn parse_exif_datetime_ms(s: &str) -> Option<i64> {
    let b = s.as_bytes();
    if b.len() < 19 {
        return None;
    }
    let year:  i64 = std::str::from_utf8(&b[0..4]).ok()?.parse().ok()?;
    let month: i64 = std::str::from_utf8(&b[5..7]).ok()?.parse().ok()?;
    let day:   i64 = std::str::from_utf8(&b[8..10]).ok()?.parse().ok()?;
    let hour:  i64 = std::str::from_utf8(&b[11..13]).ok()?.parse().ok()?;
    let min:   i64 = std::str::from_utf8(&b[14..16]).ok()?.parse().ok()?;
    let sec:   i64 = std::str::from_utf8(&b[17..19]).ok()?.parse().ok()?;
    let days = ymd_to_days(year as i32, month as u32, day as u32)?;
    let secs = days * 86400 + hour * 3600 + min * 60 + sec;
    Some(secs * 1000)
}

/// Calendar date → days since 1970-01-01 (inverse of `secs_to_ymd`).
fn ymd_to_days(y: i32, m: u32, d: u32) -> Option<i64> {
    let m = m as i32;
    let d = d as i32;
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = (y - era * 400) as u32;
    let m_adj = if m > 2 { (m - 3) as u32 } else { (m + 9) as u32 };
    let doy = (153 * m_adj + 2) / 5 + d as u32 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    Some((era as i64) * 146097 + doe as i64 - 719468)
}

/// Naive Unix timestamp → (year, month, day) without external crates.
pub fn secs_to_ymd(secs: i64) -> (i32, u32, u32) {
    // Days since 1970-01-01
    let days = (secs / 86400) as i32;
    // Shift epoch to 1 Mar 2000 for the leap-year algorithm
    let z = days + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = (z - era * 146097) as u32;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe as i32 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y, m, d)
}
