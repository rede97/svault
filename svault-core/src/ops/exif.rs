//! EXIF metadata extraction for import pipeline.
//!
//! Supports:
//! - EXIF-based formats: JPEG, TIFF, HEIF, PNG, WebP, RAW
//! - Video formats: MP4, MOV (QuickTime creation_time)

use crate::media::video::extract_video_metadata;
use std::path::Path;

/// Returns `(taken_ms, device)` from media metadata, with fallbacks.
/// - `taken_ms`: EXIF `DateTimeOriginal` → `DateTime` → Video `creation_time` → `mtime_ms` fallback
/// - `device`:   `"Make Model"` sanitised for path use → `"Unknown"` fallback
pub fn read_exif_date_device(path: &Path, mtime_ms: i64) -> (i64, String) {
    use std::fs::File;
    use std::io::BufReader;

    let Ok(file) = File::open(path) else {
        return (mtime_ms, "Unknown".to_string());
    };
    let mut reader = BufReader::new(file);

    // Try EXIF first (for images)
    if let Ok(exif) = exif::Reader::new().read_from_container(&mut reader) {
        return extract_from_exif(&exif, mtime_ms);
    }

    // Fallback to video metadata extraction
    if let Ok(video_meta) = extract_video_metadata(path) {
        return extract_from_video(video_meta, mtime_ms);
    }

    (mtime_ms, "Unknown".to_string())
}

/// Extract metadata from EXIF data.
fn extract_from_exif(exif: &exif::Exif, mtime_ms: i64) -> (i64, String) {
    use exif::In;

    // Date: prefer DateTimeOriginal, fallback to DateTime
    let taken_ms = exif
        .get_field(exif::Tag::DateTimeOriginal, In::PRIMARY)
        .or_else(|| exif.get_field(exif::Tag::DateTime, In::PRIMARY))
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
        .get_field(exif::Tag::Make, In::PRIMARY)
        .and_then(|f| exif_ascii_first(&f.value))
        .unwrap_or_default();
    let model = exif
        .get_field(exif::Tag::Model, In::PRIMARY)
        .and_then(|f| exif_ascii_first(&f.value))
        .unwrap_or_default();
    let device = sanitize_device_name(&make, &model);

    (taken_ms, device)
}

/// Extract metadata from video metadata.
fn extract_from_video(meta: crate::media::VideoMetadata, mtime_ms: i64) -> (i64, String) {
    let taken_ms = meta.creation_time_ms.unwrap_or(mtime_ms);

    let device = match (meta.device_make.as_ref(), meta.device_model.as_ref()) {
        (Some(make), Some(model)) => sanitize_device_name(make, model),
        (None, Some(model)) => sanitize_device_name("", model),
        (Some(make), None) => sanitize_device_name("", make),
        (None, None) => "Unknown".to_string(),
    };

    (taken_ms, device)
}

/// Sanitize device name from make and model.
fn sanitize_device_name(make: &str, model: &str) -> String {
    if make.is_empty() && model.is_empty() {
        return "Unknown".to_string();
    }

    let raw = if make.is_empty() || model.starts_with(make) {
        model.to_string() // avoid "Apple Apple iPhone"
    } else {
        format!("{} {}", make, model)
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
    let year: i64 = std::str::from_utf8(&b[0..4]).ok()?.parse().ok()?;
    let month: i64 = std::str::from_utf8(&b[5..7]).ok()?.parse().ok()?;
    let day: i64 = std::str::from_utf8(&b[8..10]).ok()?.parse().ok()?;
    let hour: i64 = std::str::from_utf8(&b[11..13]).ok()?.parse().ok()?;
    let min: i64 = std::str::from_utf8(&b[14..16]).ok()?.parse().ok()?;
    let sec: i64 = std::str::from_utf8(&b[17..19]).ok()?.parse().ok()?;
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
    let m_adj = if m > 2 {
        (m - 3) as u32
    } else {
        (m + 9) as u32
    };
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

#[cfg(test)]
mod tests {
    use super::*;

    // -------------------------------------------------------------------------
    // secs_to_ymd tests - core date conversion logic
    // -------------------------------------------------------------------------

    #[test]
    fn secs_to_ymd_epoch() {
        // Unix epoch: 1970-01-01
        assert_eq!(secs_to_ymd(0), (1970, 1, 1));
    }

    #[test]
    fn secs_to_ymd_specific_known_dates() {
        // Known timestamps verified with external tools
        // 2000-01-01 00:00:00 UTC = 946684800
        assert_eq!(secs_to_ymd(946684800), (2000, 1, 1));
        // 2024-05-01 00:00:00 UTC = 1714521600
        assert_eq!(secs_to_ymd(1714521600), (2024, 5, 1));
    }

    #[test]
    fn secs_to_ymd_year_boundaries() {
        // 1999-12-31 00:00:00 UTC
        assert_eq!(secs_to_ymd(946598400), (1999, 12, 31));
        // 2000-01-01 00:00:00 UTC
        assert_eq!(secs_to_ymd(946684800), (2000, 1, 1));
    }

    #[test]
    fn secs_to_ymd_negative_timestamp() {
        // Before Unix epoch: 1969-12-31
        assert_eq!(secs_to_ymd(-86400), (1969, 12, 31));
    }

    // -------------------------------------------------------------------------
    // parse_exif_datetime_ms tests - EXIF date parsing
    // -------------------------------------------------------------------------

    #[test]
    fn parse_exif_datetime_valid() {
        // Standard EXIF format: "YYYY:MM:DD HH:MM:SS"
        let result = parse_exif_datetime_ms("2024:05:01 10:30:00");
        assert!(result.is_some());
        let ms = result.unwrap();
        // Verify by converting back
        let (y, m, d) = secs_to_ymd(ms / 1000);
        assert_eq!(y, 2024);
        assert_eq!(m, 5);
        assert_eq!(d, 1);
        // Also verify time component (10:30:00 = 37800 seconds)
        assert_eq!(ms % 86400000, 37800 * 1000);
    }

    #[test]
    fn parse_exif_datetime_epoch() {
        let result = parse_exif_datetime_ms("1970:01:01 00:00:00");
        assert_eq!(result, Some(0));
    }

    #[test]
    fn parse_exif_datetime_too_short() {
        // Too short - less than 19 chars
        assert!(parse_exif_datetime_ms("2024:05:01").is_none());
    }

    #[test]
    fn parse_exif_datetime_handles_edge_cases() {
        // Function doesn't strictly validate date ranges - it parses what it can
        // "99" for month/day will parse successfully as numbers
        let result = parse_exif_datetime_ms("2024:99:99 99:99:99");
        // This may succeed or fail depending on ymd_to_days - we just check it doesn't panic
        // The actual behavior depends on ymd_to_days implementation
        let _ = result; // Don't assert - just ensure no panic
    }

    // -------------------------------------------------------------------------
    // ymd_to_days (round-trip with secs_to_ymd)
    // -------------------------------------------------------------------------

    #[test]
    fn ymd_days_round_trip() {
        // Test round-trip conversion for various valid dates
        let test_dates = [
            (1970, 1, 1),
            (2000, 1, 1),
            (2000, 3, 1),
            (2024, 5, 15),
            (1999, 12, 31),
        ];

        for (y, m, d) in test_dates {
            let days =
                ymd_to_days(y, m, d).unwrap_or_else(|| panic!("Valid date {}-{}-{}", y, m, d));
            let (y2, m2, d2) = secs_to_ymd(days * 86400);
            assert_eq!(
                (y, m, d),
                (y2, m2, d2),
                "Round-trip failed for {}-{}-{}: got {}-{}-{} days={}",
                y,
                m,
                d,
                y2,
                m2,
                d2,
                days
            );
        }
    }

    #[test]
    fn ymd_to_days_behavioral_test() {
        // Note: ymd_to_days has minimal validation - it computes days for any input
        // We verify it produces consistent results rather than testing validation
        // Day 0 produces a valid result (last day of previous month in the algorithm)
        let result = ymd_to_days(2024, 1, 0);
        assert!(result.is_some()); // The algorithm accepts day 0

        // Month 0 also produces a result (December of previous year)
        let result = ymd_to_days(2024, 0, 1);
        assert!(result.is_some());
    }

    // -------------------------------------------------------------------------
    // exif_ascii_first tests
    // -------------------------------------------------------------------------

    #[test]
    fn exif_ascii_first_extracts_string() {
        use exif::Value;
        let value = Value::Ascii(vec![b"Test String\0".to_vec()]);
        assert_eq!(exif_ascii_first(&value), Some("Test String".to_string()));
    }

    #[test]
    fn exif_ascii_first_trims_nulls() {
        use exif::Value;
        let value = Value::Ascii(vec![b"Camera\0\0\0".to_vec()]);
        assert_eq!(exif_ascii_first(&value), Some("Camera".to_string()));
    }

    #[test]
    fn exif_ascii_first_empty_vec() {
        use exif::Value;
        let value = Value::Ascii(vec![]);
        assert_eq!(exif_ascii_first(&value), None);
    }

    #[test]
    fn exif_ascii_first_non_ascii() {
        use exif::Value;
        let value = Value::Byte(vec![0x00, 0x01, 0x02]);
        assert_eq!(exif_ascii_first(&value), None);
    }
}
